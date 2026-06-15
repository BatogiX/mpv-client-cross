#![allow(non_upper_case_globals)]
#![allow(non_camel_case_types)]
#![allow(non_snake_case)]

mod error;
mod format;
mod logging;
mod node;
mod options;

use crate::{error::MpvError, format::Sealed as _, node::MpvNodeRef, options::CoercingString};
pub use error::{Error, Result};
pub use format::{AsFormat, Format, OsdString};
pub use mpv_client_sys::mpv_handle;
use mpv_client_sys::{
    mpv_abort_async_command, mpv_client_api_version, mpv_client_id, mpv_client_name, mpv_command, mpv_command_async,
    mpv_command_ret, mpv_create, mpv_create_client, mpv_create_weak_client, mpv_del_property, mpv_destroy,
    mpv_end_file_reason_MPV_END_FILE_REASON_EOF, mpv_end_file_reason_MPV_END_FILE_REASON_ERROR,
    mpv_end_file_reason_MPV_END_FILE_REASON_QUIT, mpv_end_file_reason_MPV_END_FILE_REASON_REDIRECT,
    mpv_end_file_reason_MPV_END_FILE_REASON_STOP, mpv_error_string, mpv_event, mpv_event_client_message,
    mpv_event_command, mpv_event_end_file, mpv_event_hook, mpv_event_id_MPV_EVENT_AUDIO_RECONFIG,
    mpv_event_id_MPV_EVENT_CLIENT_MESSAGE, mpv_event_id_MPV_EVENT_COMMAND_REPLY, mpv_event_id_MPV_EVENT_END_FILE,
    mpv_event_id_MPV_EVENT_FILE_LOADED, mpv_event_id_MPV_EVENT_GET_PROPERTY_REPLY, mpv_event_id_MPV_EVENT_HOOK,
    mpv_event_id_MPV_EVENT_LOG_MESSAGE, mpv_event_id_MPV_EVENT_NONE, mpv_event_id_MPV_EVENT_PLAYBACK_RESTART,
    mpv_event_id_MPV_EVENT_PROPERTY_CHANGE, mpv_event_id_MPV_EVENT_QUEUE_OVERFLOW, mpv_event_id_MPV_EVENT_SEEK,
    mpv_event_id_MPV_EVENT_SET_PROPERTY_REPLY, mpv_event_id_MPV_EVENT_SHUTDOWN, mpv_event_id_MPV_EVENT_START_FILE,
    mpv_event_id_MPV_EVENT_VIDEO_RECONFIG, mpv_event_log_message, mpv_event_name, mpv_event_property,
    mpv_event_start_file, mpv_format_MPV_FORMAT_NONE, mpv_free, mpv_free_node_contents, mpv_get_property,
    mpv_get_property_async, mpv_get_time_ns, mpv_get_time_us, mpv_hook_add, mpv_hook_continue, mpv_initialize,
    mpv_load_config_file, mpv_node, mpv_observe_property, mpv_request_event, mpv_request_log_messages,
    mpv_set_property, mpv_set_property_async, mpv_unobserve_property, mpv_wait_async_requests, mpv_wait_event,
    mpv_wakeup,
};
pub use node::Node;
use serde::de::{self, DeserializeOwned};
use std::{
    collections::HashMap,
    convert::Into,
    ffi::{CStr, CString, NulError, c_char, c_void},
    fmt::{self, Display},
    fs, iter,
    marker::PhantomData,
    ops::Deref,
    path::{Path, PathBuf},
    ptr, result, slice,
};

#[cfg(feature = "macros")]
pub use mpv_client_macros::main;

macro_rules! result {
    ($f:expr) => {{
        let code = $f;
        if code >= mpv_client_sys::mpv_error_MPV_ERROR_SUCCESS {
            Ok(())
        } else {
            Err(crate::error::Error::from(code))
        }
    }};
}

macro_rules! result_with_code {
    ($f:expr) => {{
        let code = $f;
        if code >= mpv_client_sys::mpv_error_MPV_ERROR_SUCCESS {
            Ok(code)
        } else {
            Err(crate::error::Error::from(code))
        }
    }};
}

#[macro_export]
macro_rules! osd {
    ($client:expr, $duration:expr, $($arg:tt)*) => {
        $client.command(["show-text", &format!($($arg)*), &$duration.as_millis().to_string()])
    }
}

#[macro_export]
macro_rules! osd_async {
    ($client:expr, $reply:expr, $duration:expr, $($arg:tt)*) => {
        $client.command_async($reply, ["show-text", &format!($($arg)*), &$duration.as_millis().to_string()])
    }
}

/// Representation of a borrowed client context used by the client API.
/// Every client has its own private handle.
#[repr(transparent)]
pub struct Handle {
    inner: [mpv_handle],
}

/// SAFETY: libmpv guarantees that the same `mpv_handle` is safe to be called from multiple
/// threads concurrently. The single exception is [`mpv_wait_event`], which is strictly
/// protected at compile-time by requiring a unique &mut [`EventQueueToken`].
unsafe impl Sync for Handle {}
unsafe impl Send for Handle {}

impl Handle {
    /// Safely bind an [`mpv_handle`] pointer to a shared reference and mint its
    /// associated exclusive [`EventQueueToken`].
    ///
    /// # Safety
    ///
    /// * `ptr` must point to a valid, fully initialized [`mpv_handle`] allocated by `libmpv`.
    ///
    /// * The underlying memory referenced by the returned [`Handle`] must remain valid and
    ///   unfreed for the entire duration of lifetime `'a`.
    ///
    /// * No aliasing mutable references to the same [`mpv_handle`] may exist anywhere for
    ///   the duration of lifetime `'a`.
    ///
    /// * The caller must guarantee that this is the **only** active [`EventQueueToken`]
    ///   associated with this specific [`mpv_handle`]. Minting duplicate tokens breaks the
    ///   compile-time single-threaded safety model enforced by [`Handle::wait_event`],
    ///   introducing runtime data races inside the C library.
    ///
    /// # Panics
    ///
    /// Panics if the provided `ptr` is null.
    #[inline]
    #[must_use]
    pub unsafe fn from_ptr<'a>(ptr: *const mpv_handle) -> (&'a Self, EventQueueToken) {
        assert!(!ptr.is_null(), "mpv_handle pointer must not be null");
        let handle = unsafe { &*(ptr::slice_from_raw_parts(ptr, 1) as *const Self) };
        let id = handle.id();
        (handle, EventQueueToken(id))
    }

    /// Create a new client handle connected to the same player core as [`Handle`]. This
    /// context has its own event queue, its own [`Self::request_event()`] state, its own
    /// [`Self::request_log_messages()`] state, its own set of observed properties, and
    /// its own state for asynchronous operations. Otherwise, everything is shared.
    ///
    /// # Arguments
    ///
    /// * `name` - The client name. This will be returned by [`Self::name()`]. If
    ///   the name is already in use, or contains non-alphanumeric
    ///   characters (other than `'_'`), the name is modified to fit.
    ///   If [`None`], an arbitrary name is automatically chosen.
    ///
    /// # Returns
    ///
    /// * A new [`Client`] paired with an [`EventQueueToken`], or an error.
    ///
    /// # Errors
    ///
    /// Returns an error if the mpv API call fails.
    pub fn create_client(&self, name: Option<&str>) -> Result<(Client, EventQueueToken)> {
        let c_name = name
            .map(Into::into)
            .filter(|n: &Vec<u8>| !n.is_empty())
            .map(CString::new)
            .transpose()?;

        let name_ptr = c_name.as_ref().map_or_else(ptr::null, |cstring| cstring.as_ptr());
        let handle = unsafe { mpv_create_client(self.as_mut_ptr(), name_ptr) };
        if handle.is_null() {
            Err(Error::MpvKnown(MpvError::Nomem))
        } else {
            let id = unsafe { mpv_client_id(handle) };
            Ok((Client(handle), EventQueueToken(id)))
        }
    }

    /// This is the same as [`Self::create_client`], but the created [`Client`] handle is
    /// treated as a weak reference. If all handles referencing a core are
    /// weak references, the core is automatically destroyed.
    ///
    /// Effectively, if the last non-weak handle is destroyed (dropped), then the
    /// weak handles receive [`mpv_event_id_MPV_EVENT_SHUTDOWN`] and are asked to terminate as well.
    ///
    /// # Arguments
    ///
    /// * `name` - The client name. This will be returned by [`Self::name()`]. If
    ///   the name is already in use, or contains non-alphanumeric
    ///   characters (other than `'_'`), the name is modified to fit.
    ///   If [`None`], an arbitrary name is automatically chosen.
    ///
    /// # Returns
    ///
    /// * A new weak [`Client`] paired with an [`EventQueueToken`], or an error.
    ///
    /// # Errors
    ///
    /// Returns an error if the mpv API call fails (e.g. out of memory).
    pub fn create_weak_client(&self, name: Option<&str>) -> Result<(Client, EventQueueToken)> {
        let c_name = name
            .map(Into::into)
            .filter(|n: &Vec<u8>| !n.is_empty())
            .map(CString::new)
            .transpose()?;

        let name_ptr = c_name.as_ref().map_or_else(ptr::null, |cstring| cstring.as_ptr());
        let handle = unsafe { mpv_create_weak_client(self.as_mut_ptr(), name_ptr) };
        if handle.is_null() {
            Err(Error::MpvKnown(MpvError::Nomem))
        } else {
            let id = unsafe { mpv_client_id(handle) };
            Ok((Client(handle), EventQueueToken(id)))
        }
    }

    /// Wait for the next event, or until the timeout expires, or if another thread
    /// makes a call to [`mpv_client_sys::mpv_wakeup()`]. Passing 0 as timeout will never wait, and
    /// is suitable for polling.
    ///
    /// The internal event queue has a limited size (per client handle). If you
    /// don't empty the event queue quickly enough with [`Handle::wait_event`], it will
    /// overflow and silently discard further events. If this happens, making
    /// asynchronous requests will fail as well (with [`mpv_client_sys::mpv_error_MPV_ERROR_EVENT_QUEUE_FULL`]).
    ///
    /// Only one thread is allowed to call this on the same [`Handle`] at a time.
    /// The API won't complain if more than one thread calls this, but it will cause
    /// race conditions in the client when accessing the shared [`mpv_event`] struct.
    /// Note that most other API functions are not restricted by this, and no API
    /// function internally calls [`mpv_wait_event()`]. Additionally, concurrent calls
    /// to different handles are always safe.
    ///
    /// As long as the timeout is 0, this is safe to be called from mpv render API
    /// threads.
    ///
    /// # Arguments
    ///
    /// * `token` - An exclusive capability token (&mut [`EventQueueToken`]) that enforces
    ///   the single-threaded event polling invariant at compile-time. Because it requires
    ///   a unique mutable reference, Rust's borrow checker guarantees that no two threads
    ///   can concurrently poll the event queue on the same handle, entirely preventing
    ///   the race conditions mentioned above.
    ///
    ///   Crucially, separating this exclusive access into a dedicated token allows `self`
    ///   to remain a shared reference (`&self`), enabling you to safely send commands
    ///   or change properties from within the event loop or other threads concurrently.
    ///
    /// # Panics
    ///
    /// Panics if the provided [`EventQueueToken`] is mismatched and does not belong
    /// to this specific `Handle` instance.
    pub fn wait_event<'h>(&'h self, token: &'h mut EventQueueToken, timeout: f64) -> Event<'h> {
        assert_eq!(
            self.id(),
            token.0,
            "mismatched EventQueueToken: this token does not belong to this MPV handle!"
        );

        unsafe { Event::from_ptr(mpv_wait_event(self.as_mut_ptr(), timeout)) }
    }

    /// Return the name of this client handle. Every client has its own unique
    /// name, which is mostly used for user interface purposes.
    #[must_use]
    pub fn name(&self) -> &str {
        unsafe {
            CStr::from_ptr(mpv_client_name(self.as_mut_ptr()))
                .to_str()
                .unwrap_or("unknown")
        }
    }

    /// Return the ID of this client handle. Every client has its own unique ID. This
    /// ID is never reused by the core, even if the [`mpv_handle`] at hand gets destroyed
    /// and new handles get allocated.
    ///
    /// IDs are never 0 or negative.
    ///
    /// Some mpv APIs (not necessarily all) accept a name in the form "@<id>" in
    /// addition of the proper [`Handle::name()`], where "<id>" is the ID in decimal
    /// form (e.g. "@123"). For example, the "script-message-to" command takes the
    /// client name as first argument, but also accepts the client ID formatted in
    /// this manner.
    #[inline]
    #[must_use]
    pub fn id(&self) -> i64 {
        unsafe { mpv_client_id(self.as_mut_ptr()) }
    }

    /// Send a command to the player. Commands are the same as those used in
    /// input.conf, except that this function takes parameters in a pre-split
    /// form.
    ///
    /// # Errors
    /// Returns an mpv error if the command fails.
    pub fn command<I, S>(&self, args: I) -> Result<()>
    where
        I: IntoIterator<Item = S>,
        S: Into<Vec<u8>>,
    {
        let args: Vec<CString> = args
            .into_iter()
            .map(|s| CString::new(s.into()))
            .collect::<result::Result<Vec<CString>, NulError>>()?;

        let mut raw_args: Vec<*const c_char> = args.iter().map(|s| s.as_ptr()).chain(iter::once(ptr::null())).collect();
        unsafe { result!(mpv_command(self.as_mut_ptr(), raw_args.as_mut_ptr())) }
    }

    /// Send a command and return the result as a [`Node`].
    ///
    /// # Errors
    /// Returns an mpv error if the command fails, or if the result cannot be
    /// converted to a [`Node`].
    pub fn command_ret<I, S, T>(&self, args: I) -> Result<T>
    where
        I: IntoIterator<Item = S>,
        S: Into<Vec<u8>>,
        T: AsFormat,
    {
        let handle = self.as_mut_ptr();
        let c_args: Vec<CString> = args
            .into_iter()
            .map(|s| CString::new(s.into()))
            .collect::<result::Result<Vec<CString>, NulError>>()?;

        let mut c_args_raw: Vec<*const c_char> = c_args
            .iter()
            .map(|s| s.as_ptr())
            .chain(iter::once(ptr::null()))
            .collect();

        let args = c_args_raw.as_mut_ptr();
        let node: Node = Node::from_mpv(|result| result!(unsafe { mpv_command_ret(handle, args, result.cast()) }))?;

        T::from_node(node)
    }

    /// Same as [`Handle::command`], but run the command asynchronously.
    ///
    /// Commands are executed asynchronously. You will receive a
    /// [`Event::CommandReply`] event. This event will also have an
    /// error code set if running the command failed. For commands that
    /// return data, the data is put into [`mpv_client_sys::mpv_event_command::result`].
    ///
    /// The only case when you do not receive an event is when the function call
    /// itself fails. This happens only if parsing the command itself (or otherwise
    /// validating it) fails, i.e. the return code of the API call is not 0 or
    /// positive.
    ///
    /// Safe to be called from mpv render API threads.
    ///
    /// # Errors
    /// Returns an mpv error if the command fails.
    pub fn command_async<I, S>(&self, reply: u64, args: I) -> Result<()>
    where
        I: IntoIterator<Item = S>,
        S: Into<Vec<u8>>,
    {
        let args: Vec<CString> = args
            .into_iter()
            .map(|s| CString::new(s.into()))
            .collect::<result::Result<Vec<CString>, NulError>>()?;

        let mut raw_args: Vec<*const c_char> = args.iter().map(|s| s.as_ptr()).chain(iter::once(ptr::null())).collect();
        unsafe { result!(mpv_command_async(self.as_mut_ptr(), reply, raw_args.as_mut_ptr())) }
    }

    /// Sets a property to a given value.
    ///
    /// Properties are variables that can be altered at runtime to control the player.
    /// For example, setting the `"pause"` property will pause or unpause playback.
    ///
    /// # Data Conversion
    ///
    /// If the requested `T::MPV_FORMAT` does not strictly match the internal format of the
    /// property, `libmpv` will attempt automatic type coercion where possible:
    /// * `MPV_FORMAT_INT64` values are automatically converted to `MPV_FORMAT_DOUBLE`.
    /// * `MPV_FORMAT_STRING` values typically invoke an internal string parser.
    /// * Failing a fallback conversion, the operation will fail with `MPV_ERROR_PROPERTY_FORMAT`.
    ///
    /// # Arguments
    ///
    /// * `name` - The property name. See `input.rst` for a list of available properties.
    /// * `data` - The value to assign, passed as any type implementing [`Format`].
    ///
    /// # Errors
    ///
    /// Returns a [`Error`](`crate::Error`) if:
    /// * The property name contains an internal null byte.
    /// * The format parameter is incompatible with the target property type.
    /// * The property is modified before initialization but is not backed by an option
    ///   ([`mpv_error_MPV_ERROR_PROPERTY_UNAVAILABLE`](mpv_client_sys::mpv_error_MPV_ERROR_PROPERTY_UNAVAILABLE)).
    /// * The underlying `mpv_set_property` call returns any other negative error code.
    ///
    /// # Notes
    ///
    /// This function can also be used to configure global initialization options
    /// *before* the instance is explicitly initialized via `mpv_initialize()`.
    pub fn set_property<T: AsFormat>(&self, name: impl Into<Vec<u8>>, data: T) -> Result<()> {
        let name = CString::new(name.into())?;
        let handle = self.as_mut_ptr();
        data.to_mpv(|data| unsafe { result!(mpv_set_property(handle, name.as_ptr(), T::FORMAT.as_u32(), data)) })
    }

    /// Set a property asynchronously.
    ///
    /// You will receive the result of the operation as an [`mpv_event_id_MPV_EVENT_SET_PROPERTY_REPLY`]
    /// event. The [`mpv_event::error`] field will contain the result status of the operation.
    /// Otherwise, this function is similar to [`mpv_set_property`].
    ///
    /// # Thread Safety
    ///
    /// Safe to be called from mpv render API threads.
    ///
    /// # Arguments
    ///
    /// * `reply` - An arbitrary 64-bit value used to identify the asynchronous response.
    ///   See the section about asynchronous calls in the mpv documentation.
    /// * `name` - The property name as a null-terminated C-string.
    /// * `data` - A pointer to the option value. The value will be copied internally by `mpv`
    ///   during the call, so the caller retains ownership of the original memory.
    ///
    /// # Returns
    ///
    /// Returns `Ok(())` if the asynchronous request was successfully queued.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - The `name` contains an internal null byte, making it an invalid C-string.
    /// - The underlying [`mpv_set_property_async`] call fails to send the request (e.g., if the context is invalid).
    pub fn set_property_async<T: AsFormat>(&self, reply: u64, name: impl Into<Vec<u8>>, data: T) -> Result<()> {
        let c_name = CString::new(name.into())?;
        let c_name_ptr = c_name.as_ptr();
        let handle = self.as_mut_ptr();
        data.to_mpv(|data| unsafe {
            result!(mpv_set_property_async(
                handle,
                reply,
                c_name_ptr,
                T::FORMAT.as_u32(),
                data
            ))
        })
    }

    /// Read the value of the given property.
    ///
    /// If the format doesn't match with the internal format of the property, access
    /// usually will fail with [`mpv_error_MPV_ERROR_PROPERTY_FORMAT`](`mpv_client_sys::mpv_error_MPV_ERROR_PROPERTY_FORMAT`). In some cases, the data
    /// is automatically converted and access succeeds. For example, [`i64`] is always
    /// converted to [`f64`], and access using String usually invokes a string formatter.
    /// # Errors
    /// Returns an mpv error if the property cannot be read, or if the format
    /// doesn't match the internal format.
    pub fn get_property<T: AsFormat>(&self, name: impl Into<Vec<u8>>) -> Result<T> {
        let name = CString::new(name.into())?;
        let handle = self.as_mut_ptr();
        T::from_mpv(|data| unsafe { result!(mpv_get_property(handle, name.as_ptr(), T::FORMAT.as_u32(), data)) })
    }

    /// Get a property asynchronously.
    ///
    /// You will receive the result of the operation as well as the property data
    /// with the [`mpv_event_id_MPV_EVENT_GET_PROPERTY_REPLY`] event. You should check the
    /// [`mpv_event::error`] field on the reply event.
    ///
    /// # Thread Safety
    ///
    /// Safe to be called from mpv render API threads.
    ///
    /// # Arguments
    ///
    /// * `reply` - An arbitrary 64-bit value used to identify the asynchronous response.
    ///   See the section about asynchronous calls in the mpv documentation.
    /// * `name` - The property name as a null-terminated C-string.
    ///
    /// # Returns
    ///
    /// Returns `Ok(())` if the asynchronous request was successfully queued.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - The `name` contains an internal null byte, making it an invalid C-string.
    /// - The underlying `mpv_get_property_async` call fails to send the request (e.g., if the context is invalid).
    pub fn get_property_async<T: AsFormat>(&self, reply: u64, name: impl Into<Vec<u8>>) -> Result<()> {
        let name = CString::new(name.into())?;
        let handle = self.as_mut_ptr();
        unsafe { result!(mpv_get_property_async(handle, reply, name.as_ptr(), T::FORMAT.as_u32())) }
    }

    /// Registers a notification callback to trigger whenever the given property changes.
    ///
    /// You will receive updates asynchronously as [`mpv_event_id_MPV_EVENT_PROPERTY_CHANGE`] events.
    ///
    /// # Behavior & Performance
    ///
    /// * **Coalescing:** Property changes are coalesced. Change events are only
    ///   returned once the internal event queue becomes empty, yielding exactly
    ///   one event per changed property.
    /// * **Initial Value:** You will always receive an immediate initial change
    ///   notification. This is intended to initialize your application state
    ///   with the current property value.
    /// * **Precision:** This mechanism is not perfectly precise for all properties.
    ///   For highly active or dynamic properties (such as `"clock"`), update frequency
    ///   might not reflect every localized state change.
    ///
    /// # Arguments
    ///
    /// * `reply` - An identifier mapping directly to `mpv_event.reply_userdata`
    ///   on the received event payload. Pass `0` if unused. See also [`Self::unobserve_property`].
    /// * `name` - The property name. Observing a non-existent property name is
    ///   allowed but may emit sporadic change events.
    ///
    /// # Errors
    ///
    /// Returns a [`Error`](`crate::Error`) if:
    /// * The property name contains an internal null byte.
    /// * The `Format` type parameter mapping `T::MPV_FORMAT` is unsupported by `libmpv`.
    /// * Memory allocation fails ([`mpv_error_MPV_ERROR_NOMEM`](`mpv_client_sys::mpv_error_MPV_ERROR_NOMEM`)).
    ///
    /// # Warnings
    ///
    /// * **Feedback Loops:** You will receive change notifications even if you mutate
    ///   the property yourself from this handle. Take precautions to prevent infinite
    ///   cascading feedback loops.
    /// * **Error Fallback:** If a property becomes unavailable or errors out during extraction,
    ///   the emitted event format drops to [`mpv_format_MPV_FORMAT_NONE`] regardless of `T`, rendering the
    ///   underlying event data pointer invalid.
    ///
    /// # Notes
    ///
    /// Only the specific [`Handle`] that registers this observation will receive the
    /// resulting change events or be permitted to unobserve them.
    pub fn observe_property<T: AsFormat>(&self, reply: u64, name: impl Into<Vec<u8>>) -> Result<()> {
        let name = CString::new(name.into())?;
        unsafe {
            result!(mpv_observe_property(
                self.as_mut_ptr(),
                reply,
                name.as_ptr(),
                T::FORMAT.as_u32()
            ))
        }
    }

    /// Undo [`Handle::observe_property`]. This will remove all observed properties for
    /// which the given number was passed as reply to [`Handle::observe_property`].
    ///
    /// Safe to be called from mpv render API threads.
    ///
    /// # Errors
    ///
    /// Returns an mpv error code, or 0 on success.
    pub fn unobserve_property(&self, registered_reply: u64) -> Result<i32> {
        unsafe { result_with_code!(mpv_unobserve_property(self.as_mut_ptr(), registered_reply)) }
    }

    /// A hook is like a synchronous event that blocks the player. You register
    /// a hook handler with this function. You will get an event, which you need
    /// to handle, and once things are ready, you can let the player continue with
    /// [`Handle::hook_continue()`].
    ///
    /// Currently, hooks can't be removed explicitly. But they will be implicitly
    /// removed if the [`mpv_handle`] it was registered with is destroyed. This also
    /// continues the hook if it was being handled by the destroyed [`mpv_handle`] (but
    /// this should be avoided, as it might mess up order of hook execution).
    ///
    /// Hook handlers are ordered globally by priority and order of registration.
    /// Handlers for the same hook with same priority are invoked in order of
    /// registration (the handler registered first is run first). Handlers with
    /// lower priority are run first (which seems backward).
    ///
    /// See the "Hooks" section in the manpage to see which hooks are currently
    /// defined.
    ///
    /// Some hooks might be reentrant (so you get multiple [`mpv_event_id_MPV_EVENT_HOOK`] for the
    /// same hook). If this can happen for a specific hook type, it will be
    /// explicitly documented in the manpage.
    ///
    /// Only the [`mpv_handle`] on which this was called will receive the hook events,
    /// or can "continue" them.
    ///
    /// # Arguments
    ///
    /// * `reply` - This will be used for the `mpv_event.reply_userdata`
    ///   field for the received [`mpv_event_id_MPV_EVENT_HOOK`] events.
    ///   If you have no use for this, pass 0.
    /// * `name` - The hook name. This should be one of the documented names. But
    ///   if the name is unknown, the hook event will simply be never
    ///   raised.
    /// * `priority` - See remarks above. Use 0 as a neutral default.
    ///
    /// # Returns
    ///
    /// * Error code (usually fails only on OOM).
    ///
    /// # Errors
    /// Returns an mpv error if the hook cannot be added.
    pub fn hook_add(&self, reply: u64, name: impl Into<Vec<u8>>, priority: i32) -> Result<()> {
        let name = CString::new(name.into())?;
        unsafe { result!(mpv_hook_add(self.as_mut_ptr(), reply, name.as_ptr(), priority)) }
    }

    /// Responds to an `MPV_EVENT_HOOK` event.
    ///
    /// You must call this after you have handled the hook event. There is no way
    /// to "cancel" or "stop" a hook.
    ///
    /// Calling this will typically unblock the player for whatever the hook is
    /// responsible for (e.g., for the `"on_load"` hook, it allows it to continue
    /// playback).
    ///
    /// # Arguments
    ///
    /// * `id` - This must be the value of the `mpv_event_hook.id` field from the
    ///   corresponding `MPV_EVENT_HOOK` event.
    ///
    /// # Errors
    ///
    /// Returns an error if the underlying [`Handle::hook_continue`] call fails.
    pub fn hook_continue(&self, id: u64) -> Result<()> {
        unsafe { result!(mpv_hook_continue(self.as_mut_ptr(), id)) }
    }

    /// Enable or disable receiving of log messages.
    ///
    /// These are the messages the command line player prints to the terminal.
    /// This call sets the minimum required log level for a message to be
    /// received with [`mpv_event_id_MPV_EVENT_LOG_MESSAGE`].
    ///
    /// # Arguments
    ///
    /// * `min_level` - Minimal log level. [`LogLevel::None`] disables all messages.
    ///
    /// # Errors
    ///
    /// Returns an error if the underlying [`mpv_request_log_messages`] call fails.
    pub fn request_log_messages(&self, min_level: LogLevel) -> Result<()> {
        let handle = self.as_mut_ptr();
        let c_min_level_ptr = min_level.as_cstr().as_ptr();
        unsafe { result!(mpv_request_log_messages(handle, c_min_level_ptr)) }
    }

    /// Return the `MPV_CLIENT_API_VERSION` the mpv source has been compiled with.
    #[must_use]
    pub fn api_version() -> u64 {
        unsafe { u64::from(mpv_client_api_version()) }
    }

    /// Returns a string describing the error.
    ///
    /// For unknown errors, the string `"unknown error"` is returned.
    ///
    /// # Arguments
    ///
    /// * `error` - The error number corresponding to an [`mpv_error`](`mpv_client_sys::mpv_error`).
    ///
    /// # Returns
    ///
    /// A static string slice (`&'static str`) describing the error. The string is
    /// completely static (managed by `libmpv`), does not need to be deallocated,
    /// and remains valid for the lifetime of the program.
    ///
    /// # Notes
    ///
    /// If the string returned by `libmpv` is not valid UTF-8, this function safely
    /// falls back to returning `"unknown error"`.
    #[must_use]
    pub fn error_string(error: i32) -> &'static str {
        unsafe {
            CStr::from_ptr(mpv_error_string(error))
                .to_str()
                .unwrap_or("unknown error")
        }
    }

    /// Loads a configuration file.
    ///
    /// This function loads and parses the file, and sets every entry in the config
    /// file's default section as if [`mpv_set_option_string()`] was called.
    ///
    /// # Path Requirements
    ///
    /// The `filename` should be an **absolute path**. If it isn't, the actual path used
    /// is unspecified. (Note: an absolute path starts with `/` on UNIX-like systems).
    ///
    /// # Error Handling
    ///
    /// If a fatal error happens when parsing a config file, errors when setting options
    /// as well as other types of errors are ignored (even if options do not exist).
    /// You can still try to capture the resulting error messages with [`Handle::request_log_messages()`].
    /// Note that it's possible that some options were successfully set even if an error occurs.
    ///
    /// # Arguments
    ///
    /// * `filename` - Absolute path to the config file on the local filesystem.
    ///
    /// # Errors
    ///
    /// Returns an error if the configuration file could not be loaded or parsed correctly.
    /// Common error variants include:
    /// * [`mpv_error_MPV_ERROR_INVALID_PARAMETER`](`mpv_client_sys::mpv_error_MPV_ERROR_INVALID_PARAMETER`) - The file was not found.
    /// * [`mpv_error_MPV_ERROR_OPTION_ERROR`](`mpv_client_sys::mpv_error_MPV_ERROR_OPTION_ERROR`) - A fatal error happened while parsing the config file.
    /// * [`NulError`](`std::ffi::NulError`) - Returned if `filename` contains an internal null byte, making it an invalid C-string.
    pub fn load_config_file(&self, filename: impl AsRef<Path>) -> crate::Result<()> {
        let handle = self.as_mut_ptr();
        let filename = filename.as_ref();

        let filename_c = {
            #[cfg(unix)]
            let bytes = filename.as_os_str().as_encoded_bytes();

            #[cfg(not(unix))]
            let bytes = filename.to_string_lossy();

            CString::new(bytes.as_ref())?
        };

        unsafe { result!(mpv_load_config_file(handle, filename_c.as_ptr())) }
    }

    /// Returns the internal time in nanoseconds.
    ///
    /// This has an arbitrary start offset, but will never wrap or go backwards.
    ///
    /// # Note
    ///
    /// This is always the *real time*, and doesn't necessarily have to do with playback time.
    /// For example, playback could go faster or slower due to playback speed, or due to
    /// playback being paused. Use the `"time-pos"` property instead to get the playback status.
    ///
    /// # Safety / Context
    ///
    /// Unlike other `libmpv` APIs, this can be called at absolutely any time (even
    /// within wakeup callbacks), as long as the context is valid.
    ///
    /// **Thread Safety:** Safe to be called from mpv render API threads.
    #[must_use]
    pub fn get_time_ns(&self) -> i64 {
        unsafe { mpv_get_time_ns(self.as_mut_ptr()) }
    }

    /// Same as [`Handle::get_time_ns`] but in microseconds.
    #[must_use]
    pub fn get_time_us(&self) -> i64 {
        unsafe { mpv_get_time_us(self.as_mut_ptr()) }
    }

    /// Signals to all async requests with the matching ID to abort.
    ///
    /// This affects the following API calls:
    /// * [`command_async()`](Handle::command_async())
    /// * [`mpv_command_node_async()`]
    ///
    /// All of these functions take a `reply` parameter. This function
    /// tells all requests with the matching `reply` value to try to return
    /// as soon as possible. If there are multiple requests with a matching ID, it
    /// aborts all of them.
    ///
    /// # Async Behavior
    ///
    /// This function is mostly asynchronous itself. It will not wait until the
    /// command is aborted. Instead, the command will terminate as usual, but with
    /// some work left undone.
    /// * How this is signaled depends on the specific command (for example, the `subprocess`
    ///   command will indicate it by setting `killed_by_us` to `true` in the result).
    /// * How long it takes also depends on the situation. The aborting process is
    ///   completely asynchronous.
    ///
    /// Not all commands may support this functionality; if unsupported, this function
    /// will have no effect. The same is true if the request using the passed `reply`
    /// has already terminated, has not been started yet, or was never in use at all.
    ///
    /// # Race Conditions
    ///
    /// You have to be careful of race conditions: the time during which the abort
    /// request will be effective is **after** the asynchronous command (e.g., [`Handle::command_async()`])
    /// has returned, and **before** the command has signaled completion with [`mpv_event_id_MPV_EVENT_COMMAND_REPLY`].
    ///
    /// # Arguments
    ///
    /// * `reply` - The ID of the request to be aborted.
    pub fn abort_async_command(&self, reply: u64) {
        unsafe { mpv_abort_async_command(self.as_ptr().cast_mut(), reply) }
    }

    /// Returns a string describing the event.
    ///
    /// For unknown events, [`None`] is returned. Note that all events actually
    /// returned by the API will also yield a `Some(&str)` with this function.
    ///
    /// The returned string is completely static (valid for the lifetime of the program)
    /// and does not need to be deallocated.
    ///
    /// # Arguments
    ///
    /// * `event` - The event ID (corresponding to [`mpv_client_sys::mpv_event_id`]).
    ///
    /// # Returns
    ///
    /// A short symbolic name of the event suitable for use in scripting interfaces.
    /// It consists of lower-case alphanumeric characters and can include `-` characters.
    #[must_use]
    pub fn event_name(event: u32) -> Option<&'static str> {
        unsafe {
            let ptr = mpv_event_name(event);
            if ptr.is_null() {
                return None;
            }

            CStr::from_ptr(ptr).to_str().ok()
        }
    }

    /// Enable or disable the given event.
    ///
    /// Some events are enabled by default. Some events can't be disabled.
    ///
    /// *(Informational note: currently, all events are enabled by default, except
    /// [`MPV_EVENT_TICK`](mpv_client_sys::mpv_event_id_MPV_EVENT_TICK).)*
    ///
    /// # Thread Safety
    ///
    /// Safe to be called from mpv render API threads.
    ///
    /// # Arguments
    ///
    /// * `event` - The event to enable or disable. See [`mpv_event_id`](mpv_client_sys::mpv_event_id).
    /// * `enable` - `1` to enable receiving this event, `0` to disable it.
    ///
    /// # Returns
    ///
    /// Returns `Ok(())` if the event state was successfully updated.
    ///
    /// # Errors
    ///
    /// Returns an error if the underlying [`mpv_request_event`] call fails (returns a negative error code),
    /// which can happen if the event API is uninitialized or the event cannot be disabled.
    pub fn request_event(&self, event: u32, enable: bool) -> Result<()> {
        unsafe { result!(mpv_request_event(self.as_mut_ptr(), event, i32::from(enable))) }
    }

    /// Interrupts the current [`Handle::wait_event()`] call.
    ///
    /// This will wake up the thread currently waiting in [`Handle::wait_event()`]. If no
    /// thread is waiting, the next [`Handle::wait_event()`] call will return immediately
    /// (this is to avoid lost wakeups).
    ///
    /// [`Handle::wait_event()`] will receive a [`MPV_EVENT_NONE`](mpv_event_id_MPV_EVENT_NONE) if it is woken up due to
    /// this call. However, note that this dummy event might be skipped if there are
    /// already other events queued. All that matters is that the waiting thread
    /// is woken up at all.
    ///
    /// This function is **safe** to be called from `mpv` render API threads.
    pub fn wakeup(&self) {
        unsafe { mpv_wakeup(self.as_ptr().cast_mut()) }
    }

    // /// Set a custom function that should be called when there are new events. Use
    // /// this if blocking in [`mpv_wait_event()`] to wait for new events is not feasible.
    // ///
    // /// In general, the client API expects you to call [`mpv_wait_event()`] to receive
    // /// notifications, and the wakeup callback is merely a helper utility to make
    // /// this easier in certain situations. Note that it's possible that there's
    // /// only one wakeup callback invocation for multiple events. You should call
    // /// [`mpv_wait_event()`] with no timeout until `MPV_EVENT_NONE` is reached, at which
    // /// point the event queue is empty.
    // ///
    // /// If you actually want to do processing in a callback, spawn a thread that
    // /// does nothing but call [`mpv_wait_event()`] in a loop and dispatches the result
    // /// to a callback.
    // ///
    // /// Only one wakeup callback can be set.
    // ///
    // /// # Arguments
    // ///
    // /// * `cb` - Function that should be called if a wakeup is required.
    // /// * `d` - Arbitrary userdata passed back to `cb`.
    // pub fn set_wakeup_callback(&mut self) {
    //     mpv_set_wakeup_callback()
    //     unimplemented!()
    // }

    pub fn wait_async_requests(&self) {
        unsafe { mpv_wait_async_requests(self.as_mut_ptr()) }
    }

    /// Convenience function to delete a property.
    ///
    /// This is equivalent to running the command `"del [name]"`.
    ///
    /// # Arguments
    ///
    /// * `name` - The property name. See `input.rst` for a list of properties.
    ///
    /// # Errors
    ///
    /// Returns a [`Error`](`crate::Error`) if the property name contains an internal null byte,
    /// or if the underlying `mpv` call returns an error code.
    pub fn del_property(&self, name: impl Into<Vec<u8>>) -> crate::Result<()> {
        let name = CString::new(name.into())?;
        result!(unsafe { mpv_del_property(self.as_mut_ptr(), name.as_ptr()) })
    }

    // fn event_to_node(event: *mut mpv_event) -> crate::Result<Node> {
    //     let mut mpv_node = ClonedMpvNode::default();
    //     result!(unsafe { mpv_event_to_node(mpv_node.as_mut_ptr(), event) })?;
    //     Ok(mpv_node.to_node())
    // }

    /// # Panics
    /// Panics if `expand-path` or `script-opts` commands fail or return unexpected types.
    #[must_use]
    pub fn read_options<T>(&self) -> T
    where
        T: DeserializeOwned + Default,
    {
        let plugin_name = self.name();
        let mut raw_map = HashMap::new();

        let config_dir: String = self
            .command_ret(["expand-path", "~~/"])
            .expect("'expand-path ~~/' failed");

        let config_path = PathBuf::from(config_dir)
            .join("script-opts")
            .join(format!("{plugin_name}.conf"));

        if config_path.exists()
            && let Ok(content) = fs::read_to_string(config_path)
        {
            for line in content.lines() {
                let line = line.trim_start();
                if line.is_empty() || line.starts_with('#') {
                    continue;
                }

                if let Some((key, value)) = line.split_once('=') {
                    raw_map.insert(key.trim().to_owned(), value.to_owned());
                }
            }
        }

        let script_opts: HashMap<String, Node> = self
            .get_property("script-opts")
            .expect("'script-opts' property unavailable");

        let prefix = format!("{plugin_name}-");
        for (full_key, node_value) in script_opts {
            if full_key.starts_with(&prefix) {
                let clean_key = &full_key[prefix.len()..];

                if let Node::String(value) = node_value {
                    raw_map.insert(clean_key.to_owned(), value);
                }
            }
        }

        let deserializer_map: HashMap<String, CoercingString> =
            raw_map.into_iter().map(|(k, v)| (k, CoercingString(v))).collect();

        let map_deserializer = de::value::MapDeserializer::new(deserializer_map.into_iter());
        T::deserialize(map_deserializer).unwrap_or_default()
    }

    /// # Errors
    /// Returns [`log::SetLoggerError`] if a logger is already set.
    pub fn initialize_logging(&self) -> result::Result<(), log::SetLoggerError> {
        logging::init(self)
    }

    #[inline]
    #[must_use]
    const fn as_ptr(&self) -> *const mpv_handle {
        self.inner.as_ptr()
    }

    #[inline]
    #[must_use]
    const fn as_mut_ptr(&self) -> *mut mpv_handle {
        self.inner.as_ptr().cast_mut()
    }

    /// Disconnect and destroy the [`mpv_handle`]. The `ctx` pointer will be deallocated
    /// with this API call.
    ///
    /// If the last [`mpv_handle`] is detached, the core player is destroyed. In
    /// addition, if there are only weak `mpv_handles` (such as created by
    /// [`create_weak_client()`](Handle::create_weak_client) or internal scripts), these `mpv_handles` will
    /// be sent [`MPV_EVENT_SHUTDOWN`](mpv_event_id_MPV_EVENT_SHUTDOWN).
    ///
    /// This function may block until these clients have responded to the shutdown
    /// event, and the core is finally destroyed.
    ///
    /// # Safety
    ///
    /// * `ctx` must be a valid, non-null pointer to an initialized [`mpv_handle`].
    /// * After this function returns, `ctx` becomes a **dangling pointer**. Any further
    ///   use of this pointer (including passing it to other `mpv_` functions) is a
    ///   use-after-free violation and results in **undefined behavior**.
    ///
    /// # Blocking
    ///
    /// This call can **block** the current thread while waiting for weak clients and
    /// internal scripts to process the shutdown event and terminate properly. Avoid
    /// calling this on a thread that cannot afford to block (e.g., a UI thread or an
    /// audio callback thread).
    fn destroy(ctx: *mut mpv_handle) {
        if !ctx.is_null() {
            unsafe { mpv_destroy(ctx) }
        }
    }

    /// General function to deallocate memory returned by some of the API functions.
    ///
    /// Call this only if it's explicitly documented as allowed. Calling this on
    /// `mpv` memory not owned by the caller will lead to undefined behavior.
    ///
    /// # Safety
    ///
    /// * The `data` parameter must be a valid pointer returned by the API,
    ///   or a null pointer (e.g., [`std::ptr::null()`](core::ptr::null) / [`std::ptr::null_mut()`](core::ptr::null_mut)).
    /// * Passing a pointer to memory not owned by the caller, or a pointer that
    ///   has already been deallocated (double-free), results in **undefined behavior**.
    ///
    /// # Arguments
    ///
    /// * `data` - A pointer to the memory block to be deallocated.
    fn free(data: *mut c_void) {
        unsafe { mpv_free(data) }
    }

    /// Frees any data referenced by the node. It doesn't free the node itself.
    ///
    /// Call this only if the `mpv` client API set the node. If you constructed the
    /// node yourself (manually), you have to free it yourself.
    ///
    /// If [`mpv_node::format`] is [`mpv_format_MPV_FORMAT_NONE`], this call does nothing. Likewise, if
    /// the client API sets a node with this format, this function doesn't need to
    /// be called. (This is just a clarification that there's no danger of anything
    /// strange happening in these cases.)
    ///
    /// # Safety
    ///
    /// * The `node` must point to a valid, initialized structure that was populated
    ///   by the `mpv` client API.
    /// * Calling this function on a node you manually allocated or constructed yourself
    ///   will result in **undefined behavior**. You must free manual allocations using
    ///   your own allocation wrappers.
    /// * Ensure that the referenced data is not used after this function is called,
    ///   as the pointers inside the node will become dangling.
    fn free_node_contents(node: *mut mpv_node) {
        unsafe {
            if !node.is_null() && (*node).format != mpv_format_MPV_FORMAT_NONE {
                mpv_free_node_contents(node);
            }
        }
    }
}

#[derive(Debug)]
pub struct EventQueueToken(i64);

/// A type representing an owned client context.
pub struct Client(*mut mpv_handle);

impl Client {
    /// Create a new standalone mpv client.
    ///
    /// # Errors
    /// Returns an error if mpv instance creation fails (out of memory).
    pub fn create() -> Result<(UninitializedClient, EventQueueToken)> {
        let handle = unsafe { mpv_create() };

        if handle.is_null() {
            Err(Error::MpvKnown(MpvError::Nomem))
        } else {
            let id = unsafe { mpv_client_id(handle) };
            Ok((UninitializedClient(handle), EventQueueToken(id)))
        }
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        Handle::destroy(self.0);
    }
}

impl Deref for Client {
    type Target = Handle;

    #[inline]
    fn deref(&self) -> &Self::Target {
        unsafe { &*(ptr::slice_from_raw_parts(self.0, 1) as *const Handle) }
    }
}

/// SAFETY: [`Client`] uniquely owns the underlying [`mpv_handle`] and its destruction
/// via [`mpv_destroy`] is entirely thread-safe. Since [`Handle`] is [`Send`] and [`Sync`],
/// it is also perfectly safe to transfer or share ownership of [`Client`] across threads.
unsafe impl Sync for Client {}
unsafe impl Send for Client {}

pub struct UninitializedClient(*mut mpv_handle);

unsafe impl Sync for UninitializedClient {}
unsafe impl Send for UninitializedClient {}

impl Drop for UninitializedClient {
    fn drop(&mut self) {
        Handle::destroy(self.0);
    }
}

impl UninitializedClient {
    /// Initialize the mpv core. Consumes the uninitialized client and returns
    /// a ready-to-use [`Client`].
    ///
    /// # Errors
    /// Returns an mpv error if initialization fails.
    pub fn initialize(self) -> Result<Client> {
        let handle = self.0;
        match result!(unsafe { mpv_initialize(handle) }) {
            Ok(()) => {
                std::mem::forget(self);
                Ok(Client(handle))
            }
            Err(e) => Err(e),
        }
    }
}

/// An enum representing the available events that can be received by
/// [`Handle::wait_event`].
pub enum Event<'h> {
    /// Nothing happened. Happens on timeouts or sporadic wakeups.
    None,
    /// Happens when the player quits. The player enters a state where it tries
    /// to disconnect all clients.
    Shutdown,
    /// See [`Handle::request_log_messages`].
    /// See also [`LogMessage`].
    LogMessage(LogMessage<'h>),
    /// Reply to a [`Handle::get_property_async`] request.
    /// See also [`Property`].
    GetPropertyReply(Result<()>, u64, Option<Property<'h>>),
    /// Reply to a [`Handle::set_property_async`] request.
    /// (Unlike [`Event::GetPropertyReply`], [`Property`] is not used.)
    SetPropertyReply(Result<()>, u64),
    /// Reply to a [`Handle::command_async`] or [`mpv_client_sys::mpv_command_node_async()`] request.
    CommandReply(Result<()>, u64, Command<'h>),
    /// Notification before playback start of a file (before the file is loaded).
    /// See also [`StartFile`].
    StartFile(StartFile<'h>),
    /// Notification after playback end (after the file was unloaded).
    /// See also [`EndFile`].
    EndFile(EndFile<'h>),
    /// Notification when the file has been loaded (headers were read etc.), and
    /// decoding starts.
    FileLoaded,
    /// Triggered by the script-message input command. The command uses the
    /// first argument of the command as client name (see [`Handle::name`]) to
    /// dispatch the message, and passes along all arguments starting from the
    /// second argument as strings.
    /// See also [`ClientMessage`].
    ClientMessage(ClientMessage<'h>),
    /// Happens after video changed in some way. This can happen on resolution
    /// changes, pixel format changes, or video filter changes. The event is
    /// sent after the video filters and the VO are reconfigured. Applications
    /// embedding a mpv window should listen to this event in order to resize
    /// the window if needed.
    /// Note that this event can happen sporadically, and you should check
    /// yourself whether the video parameters really changed before doing
    /// something expensive.
    VideoReconfig,
    /// Similar to [`Event::VideoReconfig`]. This is relatively uninteresting,
    /// because there is no such thing as audio output embedding.
    AudioReconfig,
    /// Happens when a seek was initiated. Playback stops. Usually it will
    /// resume with [`Event::PlaybackRestart`] as soon as the seek is finished.
    Seek,
    /// There was a discontinuity of some sort (like a seek), and playback
    /// was reinitialized. Usually happens on start of playback and after
    /// seeking. The main purpose is allowing the client to detect when a seek
    /// request is finished.
    PlaybackRestart,
    /// Event sent due to [`mpv_observe_property()`].
    /// See also [`Property`].
    PropertyChange(u64, Property<'h>),
    /// Happens if the internal per-mpv_handle ringbuffer overflows, and at
    /// least 1 event had to be dropped. This can happen if the client doesn't
    /// read the event queue quickly enough with [`Handle::wait_event`], or if the
    /// client makes a very large number of asynchronous calls at once.
    ///
    /// Event delivery will continue normally once this event was returned
    /// (this forces the client to empty the queue completely).
    QueueOverflow,
    /// Triggered if a hook handler was registered with [`Handle::hook_add`], and the
    /// hook is invoked. If you receive this, you must handle it, and continue
    /// the hook with [`Handle::hook_continue`].
    /// See also [`Hook`].
    Hook(u64, Hook<'h>),
}

impl Event<'_> {
    fn from_ptr(event: *const mpv_event) -> Self {
        if event.is_null() {
            return Self::None;
        }

        let event = unsafe { *event };
        match event.event_id {
            mpv_event_id_MPV_EVENT_SHUTDOWN => Self::Shutdown,
            mpv_event_id_MPV_EVENT_LOG_MESSAGE => Self::LogMessage(LogMessage::from_ptr(event.data)),
            mpv_event_id_MPV_EVENT_GET_PROPERTY_REPLY => {
                let err = result!(event.error);
                let prop = if event.data.is_null() {
                    None
                } else {
                    Some(Property::from_ptr(event.data))
                };
                Self::GetPropertyReply(err, event.reply_userdata, prop)
            }
            mpv_event_id_MPV_EVENT_SET_PROPERTY_REPLY => {
                Self::SetPropertyReply(result!(event.error), event.reply_userdata)
            }
            mpv_event_id_MPV_EVENT_COMMAND_REPLY => Self::CommandReply(
                result!(event.error),
                event.reply_userdata,
                Command::from_ptr(event.data),
            ),
            mpv_event_id_MPV_EVENT_START_FILE => Self::StartFile(StartFile::from_ptr(event.data)),
            mpv_event_id_MPV_EVENT_END_FILE => Self::EndFile(EndFile::from_ptr(event.data)),
            mpv_event_id_MPV_EVENT_FILE_LOADED => Self::FileLoaded,
            mpv_event_id_MPV_EVENT_CLIENT_MESSAGE => Self::ClientMessage(ClientMessage::from_ptr(event.data)),
            mpv_event_id_MPV_EVENT_VIDEO_RECONFIG => Self::VideoReconfig,
            mpv_event_id_MPV_EVENT_AUDIO_RECONFIG => Self::AudioReconfig,
            mpv_event_id_MPV_EVENT_SEEK => Self::Seek,
            mpv_event_id_MPV_EVENT_PLAYBACK_RESTART => Self::PlaybackRestart,
            mpv_event_id_MPV_EVENT_PROPERTY_CHANGE => {
                Self::PropertyChange(event.reply_userdata, Property::from_ptr(event.data))
            }
            mpv_event_id_MPV_EVENT_QUEUE_OVERFLOW => Self::QueueOverflow,
            mpv_event_id_MPV_EVENT_HOOK => Self::Hook(event.reply_userdata, Hook::from_ptr(event.data)),
            _ => Self::None,
        }
    }
}

impl fmt::Display for Event<'_> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let event_name = Handle::event_name(u32::from(self)).unwrap_or("unknown");

        match self {
            Event::None => Ok(()),
            Event::LogMessage(log_message) => write!(f, "{event_name}: {log_message}"),
            Event::GetPropertyReply(error, reply, property) => {
                write!(f, "{event_name}: (")?;
                if let Err(error) = error {
                    write!(f, "err: {error:?}, ")?;
                }
                write!(f, "reply: {reply}): ")?;

                if let Some(property) = property {
                    write!(f, "{property}")
                } else {
                    f.write_str("None")
                }
            }
            Event::SetPropertyReply(error, reply) => {
                write!(f, "{event_name}: (")?;
                if let Err(error) = error {
                    write!(f, "err: {error:?}, ")?;
                }
                write!(f, "reply: {reply}): ")
            }
            Event::CommandReply(error, reply, command) => {
                write!(f, "{event_name}: (")?;
                if let Err(error) = error {
                    write!(f, "err: {error:?}, ")?;
                }
                write!(f, "reply: {reply}): {command}")
            }
            Event::StartFile(start_file) => write!(f, "{event_name}: {start_file}"),
            Event::EndFile(end_file) => write!(f, "{event_name}: {end_file}"),
            Event::ClientMessage(client_message) => write!(f, "{event_name}: {client_message}"),
            Event::PropertyChange(reply, property) => write!(f, "{event_name}: (reply: {reply}): {property}"),
            Event::Hook(reply, hook) => write!(f, "{event_name}: (reply: {reply}): {hook}"),
            _ => f.write_str(event_name),
        }
    }
}

/// Data associated with [`Event::GetPropertyReply`] and [`Event::PropertyChange`].
#[derive(Debug)]
#[repr(transparent)]
pub struct Property<'h>(*const mpv_event_property, PhantomData<&'h Handle>);

impl Property<'_> {
    /// Wrap a raw [`mpv_event_property`]
    /// The pointer must not be null
    fn from_ptr(ptr: *const c_void) -> Self {
        assert!(!ptr.is_null());
        Self(ptr.cast::<mpv_event_property>(), PhantomData)
    }

    /// Name of the property.
    #[must_use]
    pub fn name(&self) -> &str {
        unsafe { CStr::from_ptr((*self.0).name) }.to_str().unwrap_or("unknown")
    }

    #[must_use]
    pub const fn format(&self) -> Format {
        Format::from_u32(unsafe { (*self.0).format })
    }

    #[must_use]
    pub fn data<T: AsFormat>(&self) -> Option<T> {
        unsafe {
            if self.format() == T::FORMAT {
                Some(T::from_ptr((*self.0).data))
            } else {
                None
            }
        }
    }
}

impl fmt::Display for Property<'_> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        fn write_opt<T: fmt::Debug>(f: &mut fmt::Formatter, value: Option<T>) -> fmt::Result {
            match value {
                Some(v) => write!(f, "{v:?}"),
                None => f.write_str("None"),
            }
        }

        write!(f, "name: {}, format: {}, data: ", self.name(), self.format())?;

        match self.format() {
            Format::String => write_opt(f, self.data::<String>()),
            Format::OsdString => write_opt(f, self.data::<OsdString>()),
            Format::Bool => write_opt(f, self.data::<bool>()),
            Format::Int => write_opt(f, self.data::<i64>()),
            Format::Double => write_opt(f, self.data::<f64>()),
            Format::Node => write_opt(f, self.data::<Node>()),
            Format::Array => write_opt(f, self.data::<Vec<Node>>()),
            Format::Map => write_opt(f, self.data::<HashMap<String, Node>>()),
            Format::Bytes => write_opt(f, self.data::<Vec<u8>>()),
            Format::None | Format::Unknown(_) => f.write_str("None"),
        }
    }
}

/// Data associated with [`Event::LogMessage`].
#[repr(transparent)]
pub struct LogMessage<'h>(*const mpv_event_log_message, PhantomData<&'h Handle>);

impl LogMessage<'_> {
    /// Wrap a raw [`mpv_event_log_message`]
    /// The pointer must not be null
    fn from_ptr(ptr: *const c_void) -> Self {
        assert!(!ptr.is_null());
        Self(ptr.cast::<mpv_event_log_message>(), PhantomData)
    }

    #[must_use]
    pub fn prefix(&self) -> &str {
        unsafe { CStr::from_ptr((*self.0).prefix).to_str().unwrap_or("unknown") }
    }

    #[must_use]
    pub fn level(&self) -> &str {
        unsafe { CStr::from_ptr((*self.0).level).to_str().unwrap_or("unknown") }
    }

    #[must_use]
    pub fn text(&self) -> &str {
        unsafe { CStr::from_ptr((*self.0).text).to_str().unwrap_or("unknown") }
    }

    #[must_use]
    pub const fn log_level(&self) -> u32 {
        unsafe { (*self.0).log_level }
    }
}

impl fmt::Display for LogMessage<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "[{}] [{}] {}", self.level(), self.prefix(), self.text())
    }
}

/// Data associated with [`Event::StartFile`].
#[repr(transparent)]
pub struct StartFile<'h>(*const mpv_event_start_file, PhantomData<&'h Handle>);

impl StartFile<'_> {
    /// Wrap a raw [`mpv_event_start_file`]
    /// The pointer must not be null
    fn from_ptr(ptr: *const c_void) -> Self {
        assert!(!ptr.is_null());
        Self(ptr.cast::<mpv_event_start_file>(), PhantomData)
    }

    /// Playlist entry ID of the file being loaded now.
    #[must_use]
    pub const fn playlist_entry_id(&self) -> i64 {
        unsafe { (*self.0).playlist_entry_id }
    }
}

impl fmt::Display for StartFile<'_> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "playlist_entry_id: {}", self.playlist_entry_id())
    }
}

/// Data associated with [`Event::EndFile`].
#[repr(transparent)]
pub struct EndFile<'h>(*const mpv_event_end_file, PhantomData<&'h Handle>);

impl EndFile<'_> {
    /// Wrap a raw [`mpv_event_end_file`]
    /// The pointer must not be null
    fn from_ptr(ptr: *const c_void) -> Self {
        assert!(!ptr.is_null());
        Self(ptr.cast::<mpv_event_end_file>(), PhantomData)
    }

    #[must_use]
    pub fn reason(&self) -> EndFileReason {
        unsafe { EndFileReason::from((*self.0).reason) }
    }

    /// Check if the file playback ended due to an error.
    ///
    /// # Returns
    ///
    /// Returns `Ok(())` if the file finished playing normally (e.g., reached EOF,
    /// stopped, or quit).
    ///
    /// # Errors
    ///
    /// Returns an [`Error`] if the [`reason`](Self::reason) for ending the file
    /// is [`EndFileReason::Error`].
    pub fn error(&self) -> Result<()> {
        let code = unsafe { (*self.0).error };
        match self.reason() {
            EndFileReason::Error => Err(Error::from(code)),
            _ => Ok(()),
        }
    }

    #[must_use]
    pub const fn playlist_entry_id(&self) -> i64 {
        unsafe { (*self.0).playlist_entry_id }
    }

    #[must_use]
    pub const fn playlist_insert_id(&self) -> i64 {
        unsafe { (*self.0).playlist_insert_id }
    }

    #[must_use]
    pub const fn playlist_insert_num_entries(&self) -> i32 {
        unsafe { (*self.0).playlist_insert_num_entries }
    }
}

impl fmt::Display for EndFile<'_> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "reason: {}", self.reason())?;

        if let Err(error) = self.error() {
            write!(f, ", error: {error}")?;
        }

        write!(
            f,
            ", playlist_entry_id: {}, playlist_insert_id: {}, playlist_insert_num_entries: {}",
            self.playlist_entry_id(),
            self.playlist_insert_id(),
            self.playlist_insert_num_entries()
        )
    }
}

/// Data associated with [`Event::ClientMessage`].
#[repr(transparent)]
pub struct ClientMessage<'h>(*const mpv_event_client_message, PhantomData<&'h Handle>);

impl<'h> ClientMessage<'h> {
    /// Wrap a raw [`mpv_event_client_message`].
    /// The pointer must not be null
    fn from_ptr(ptr: *const c_void) -> Self {
        assert!(!ptr.is_null());
        Self(ptr.cast::<mpv_event_client_message>(), PhantomData)
    }

    #[must_use]
    /// # Panics
    /// Panics if `num_args` is negative, or if event args contain invalid UTF-8.
    pub fn args(&self) -> Vec<&'h str> {
        unsafe {
            let num_args: usize = (*self.0).num_args.try_into().expect("negative num_args");

            let args = if num_args == 0 || (*self.0).args.is_null() {
                &[]
            } else {
                slice::from_raw_parts((*self.0).args, num_args)
            };

            args.iter()
                .map(|arg| {
                    CStr::from_ptr(*arg)
                        .to_str()
                        .expect("mpv event args contain invalid UTF-8")
                })
                .collect()
        }
    }
}

impl fmt::Display for ClientMessage<'_> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{:?}", self.args())
    }
}

/// Data associated with [`Event::Hook`].
#[repr(transparent)]
pub struct Hook<'h>(*const mpv_event_hook, PhantomData<&'h Handle>);

impl<'h> Hook<'h> {
    /// Wrap a raw [`mpv_event_hook`].
    /// The pointer must not be null
    fn from_ptr(ptr: *const c_void) -> Self {
        assert!(!ptr.is_null());
        Self(ptr.cast::<mpv_event_hook>(), PhantomData)
    }

    /// The hook name as passed to [`Handle::hook_add`].
    #[must_use]
    pub fn name(&self) -> &'h str {
        unsafe { CStr::from_ptr((*self.0).name).to_str().unwrap_or("unknown") }
    }

    /// Internal ID that must be passed to [`Handle::hook_continue`].
    #[must_use]
    pub const fn id(&self) -> u64 {
        unsafe { (*self.0).id }
    }
}

impl fmt::Display for Hook<'_> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "name: {}, id: {}", self.name(), self.id())
    }
}

/// Data associated with [`Event::CommandReply`].
#[repr(transparent)]
pub struct Command<'h>(*const mpv_event_command, PhantomData<&'h Handle>);

impl Display for Command<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.result())
    }
}

impl Command<'_> {
    /// Wrap a raw [`mpv_event_command`]
    /// The pointer must not be null
    fn from_ptr(ptr: *const c_void) -> Self {
        assert!(!ptr.is_null());
        Self(ptr.cast::<mpv_event_command>(), PhantomData)
    }

    #[must_use]
    pub fn result(&self) -> Node {
        MpvNodeRef(&unsafe { *self.0 }.result).to_node()
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub enum LogLevel {
    #[default]
    None,
    Fatal,
    Error,
    Warn,
    Info,
    V,
    Debug,
    Trace,
    TerminalDefault,
}

impl LogLevel {
    #[must_use]
    pub const fn as_cstr(&self) -> &'static CStr {
        match self {
            Self::None => c"no",
            Self::Fatal => c"fatal",
            Self::Error => c"error",
            Self::Warn => c"warn",
            Self::Info => c"info",
            Self::V => c"v",
            Self::Debug => c"debug",
            Self::Trace => c"trace",
            Self::TerminalDefault => c"terminal-default",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum EndFileReason {
    Eof = mpv_end_file_reason_MPV_END_FILE_REASON_EOF,
    Stop = mpv_end_file_reason_MPV_END_FILE_REASON_STOP,
    Quit = mpv_end_file_reason_MPV_END_FILE_REASON_QUIT,
    Error = mpv_end_file_reason_MPV_END_FILE_REASON_ERROR,
    Redirect = mpv_end_file_reason_MPV_END_FILE_REASON_REDIRECT,
    Unknown(u32),
}

impl Display for EndFileReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Eof => f.write_str("End of file"),
            Self::Stop => f.write_str("Stop"),
            Self::Quit => f.write_str("Quit"),
            Self::Error => f.write_str("Error"),
            Self::Redirect => f.write_str("Redirect"),
            Self::Unknown(v) => write!(f, "Unknown: {v}"),
        }
    }
}

impl From<u32> for EndFileReason {
    fn from(value: u32) -> Self {
        match value {
            mpv_end_file_reason_MPV_END_FILE_REASON_EOF => Self::Eof,
            mpv_end_file_reason_MPV_END_FILE_REASON_STOP => Self::Stop,
            mpv_end_file_reason_MPV_END_FILE_REASON_QUIT => Self::Quit,
            mpv_end_file_reason_MPV_END_FILE_REASON_ERROR => Self::Error,
            mpv_end_file_reason_MPV_END_FILE_REASON_REDIRECT => Self::Redirect,
            value => Self::Unknown(value),
        }
    }
}

impl From<&Event<'_>> for u32 {
    fn from(value: &Event) -> Self {
        match value {
            Event::None => mpv_event_id_MPV_EVENT_NONE,
            Event::Shutdown => mpv_event_id_MPV_EVENT_SHUTDOWN,
            Event::LogMessage(..) => mpv_event_id_MPV_EVENT_LOG_MESSAGE,
            Event::GetPropertyReply(..) => mpv_event_id_MPV_EVENT_GET_PROPERTY_REPLY,
            Event::SetPropertyReply(..) => mpv_event_id_MPV_EVENT_SET_PROPERTY_REPLY,
            Event::CommandReply(..) => mpv_event_id_MPV_EVENT_COMMAND_REPLY,
            Event::StartFile(..) => mpv_event_id_MPV_EVENT_START_FILE,
            Event::EndFile(..) => mpv_event_id_MPV_EVENT_END_FILE,
            Event::FileLoaded => mpv_event_id_MPV_EVENT_FILE_LOADED,
            Event::ClientMessage(..) => mpv_event_id_MPV_EVENT_CLIENT_MESSAGE,
            Event::VideoReconfig => mpv_event_id_MPV_EVENT_VIDEO_RECONFIG,
            Event::AudioReconfig => mpv_event_id_MPV_EVENT_AUDIO_RECONFIG,
            Event::Seek => mpv_event_id_MPV_EVENT_SEEK,
            Event::PlaybackRestart => mpv_event_id_MPV_EVENT_PLAYBACK_RESTART,
            Event::PropertyChange(..) => mpv_event_id_MPV_EVENT_PROPERTY_CHANGE,
            Event::QueueOverflow => mpv_event_id_MPV_EVENT_QUEUE_OVERFLOW,
            Event::Hook(..) => mpv_event_id_MPV_EVENT_HOOK,
        }
    }
}
