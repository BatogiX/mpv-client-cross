#![allow(non_upper_case_globals)]
#![allow(non_camel_case_types)]
#![allow(non_snake_case)]

mod error;
mod format;
mod logging;
mod node;
mod options;

pub use error::{Error, Result};
pub use format::Format;
pub use mpv_client_sys::mpv_handle;
pub use node::Node;

use crate::{node::MpvNodeContentsGuard, options::CoercingString};

use mpv_client_sys::{
    mpv_abort_async_command, mpv_client_api_version, mpv_client_id, mpv_client_name, mpv_command, mpv_command_async,
    mpv_command_ret, mpv_create, mpv_create_client, mpv_create_weak_client, mpv_destroy, mpv_error_MPV_ERROR_NOMEM,
    mpv_error_MPV_ERROR_SUCCESS, mpv_error_string, mpv_event, mpv_event_client_message, mpv_event_end_file,
    mpv_event_hook, mpv_event_id_MPV_EVENT_AUDIO_RECONFIG, mpv_event_id_MPV_EVENT_CLIENT_MESSAGE,
    mpv_event_id_MPV_EVENT_COMMAND_REPLY, mpv_event_id_MPV_EVENT_END_FILE, mpv_event_id_MPV_EVENT_FILE_LOADED,
    mpv_event_id_MPV_EVENT_GET_PROPERTY_REPLY, mpv_event_id_MPV_EVENT_HOOK, mpv_event_id_MPV_EVENT_LOG_MESSAGE,
    mpv_event_id_MPV_EVENT_NONE, mpv_event_id_MPV_EVENT_PLAYBACK_RESTART, mpv_event_id_MPV_EVENT_PROPERTY_CHANGE,
    mpv_event_id_MPV_EVENT_QUEUE_OVERFLOW, mpv_event_id_MPV_EVENT_SEEK, mpv_event_id_MPV_EVENT_SET_PROPERTY_REPLY,
    mpv_event_id_MPV_EVENT_SHUTDOWN, mpv_event_id_MPV_EVENT_START_FILE, mpv_event_id_MPV_EVENT_VIDEO_RECONFIG,
    mpv_event_log_message, mpv_event_name, mpv_event_property, mpv_event_start_file, mpv_get_property, mpv_get_time_ns,
    mpv_get_time_us, mpv_hook_add, mpv_hook_continue, mpv_initialize, mpv_node, mpv_observe_property, mpv_set_property,
    mpv_unobserve_property, mpv_wait_event, mpv_wakeup,
};
use serde::de::{self, DeserializeOwned};
use std::{
    borrow::Cow,
    collections::HashMap,
    ffi::{CStr, CString, c_char, c_void},
    fmt, fs, iter,
    marker::PhantomData,
    mem::MaybeUninit,
    ops::Deref,
    path::{Path, PathBuf},
    ptr,
};

#[cfg(feature = "macros")]
pub use mpv_client_macros::main;

/// Representation of a borrowed client context used by the client API.
/// Every client has its own private handle.
#[repr(transparent)]
pub struct Handle {
    inner: [mpv_handle],
}

#[derive(Debug)]
pub struct EventQueueToken(i64);

/// A type representing an owned client context.
pub struct Client(*mut mpv_handle);

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
    CommandReply(Result<()>, u64),
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

/// Data associated with [`Event::GetPropertyReply`] and [`Event::PropertyChange`].
pub struct Property<'h>(*const mpv_event_property, PhantomData<&'h Handle>);

/// Data associated with [`Event::LogMessage`].
pub struct LogMessage<'h>(*const mpv_event_log_message, PhantomData<&'h Handle>);

/// Data associated with [`Event::StartFile`].
pub struct StartFile<'h>(*const mpv_event_start_file, PhantomData<&'h Handle>);

/// Data associated with [`Event::EndFile`].
pub struct EndFile<'h>(*const mpv_event_end_file, PhantomData<&'h Handle>);

/// Data associated with [`Event::ClientMessage`].
pub struct ClientMessage<'h>(*const mpv_event_client_message, PhantomData<&'h Handle>);

/// Data associated with [`Event::Hook`].
pub struct Hook<'h>(*const mpv_event_hook, PhantomData<&'h Handle>);

macro_rules! result {
    ($f:expr) => {
        match $f {
            mpv_error_MPV_ERROR_SUCCESS => Ok(()),
            e => Err(Error::new(e)),
        }
    };
}

macro_rules! result_with_code {
    ($f:expr) => {
        if $f >= mpv_error_MPV_ERROR_SUCCESS {
            Ok($f)
        } else {
            Err(Error::new($f))
        }
    };
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
    pub fn create_client<'a, S: Into<Cow<'a, str>>>(&self, name: Option<S>) -> Result<(Client, EventQueueToken)> {
        let name = name.map(|n| n.into()).filter(|n| !n.is_empty());
        let c_name = match name {
            Some(n) => Some(CString::new(n.into_owned())?),
            None => None,
        };

        let name_ptr = c_name.as_ref().map_or_else(ptr::null, |cstring| cstring.as_ptr());
        let handle = unsafe { mpv_create_client(self.as_ptr().cast_mut(), name_ptr) };
        if handle.is_null() {
            Err(Error::new(mpv_error_MPV_ERROR_NOMEM))
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
    pub fn create_weak_client<'a, S: Into<Cow<'a, str>>>(&self, name: Option<S>) -> Result<(Client, EventQueueToken)> {
        let name = name.map(|n| n.into()).filter(|n| !n.is_empty());
        let c_name = match name {
            Some(n) => Some(CString::new(n.into_owned())?),
            None => None,
        };

        let name_ptr = c_name.as_ref().map_or_else(ptr::null, |cstring| cstring.as_ptr());
        let handle = unsafe { mpv_create_weak_client(self.as_ptr().cast_mut(), name_ptr) };
        if handle.is_null() {
            Err(Error::new(mpv_error_MPV_ERROR_NOMEM))
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

        unsafe { Event::from_ptr(mpv_wait_event(self.as_ptr().cast_mut(), timeout)) }
    }

    /// Return the name of this client handle. Every client has its own unique
    /// name, which is mostly used for user interface purposes.
    #[must_use]
    pub fn name(&self) -> &str {
        unsafe {
            CStr::from_ptr(mpv_client_name(self.as_ptr().cast_mut()))
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
        unsafe { mpv_client_id(self.as_ptr().cast_mut()) }
    }

    /// Send a command to the player. Commands are the same as those used in
    /// input.conf, except that this function takes parameters in a pre-split
    /// form.
    ///
    /// # Errors
    /// Returns an mpv error if the command fails.
    ///
    /// # Panics
    /// Panics if any argument contains a null byte.
    pub fn command<'a, I, S>(&self, args: I) -> Result<()>
    where
        I: IntoIterator<Item = S>,
        S: Into<Cow<'a, str>>,
    {
        let args: Vec<CString> = args
            .into_iter()
            .map(|s| CString::new(s.into().into_owned()).expect("input contains null byte"))
            .collect();

        let mut raw_args: Vec<*const c_char> = args.iter().map(|s| s.as_ptr()).chain(iter::once(ptr::null())).collect();
        unsafe { result!(mpv_command(self.as_ptr().cast_mut(), raw_args.as_mut_ptr())) }
    }

    /// Send a command and return the result as a [`Node`].
    ///
    /// # Errors
    /// Returns an mpv error if the command fails, or if the result cannot be
    /// converted to a [`Node`].
    ///
    /// # Panics
    /// Panics if any argument contains a null byte.
    pub fn command_ret<'a, I, S>(&self, args: I) -> Result<Node>
    where
        I: IntoIterator<Item = S>,
        S: Into<Cow<'a, str>>,
    {
        let args: Vec<CString> = args
            .into_iter()
            .map(|s| CString::new(s.into().into_owned()).expect("input contains null byte"))
            .collect();

        let mut raw_args: Vec<*const c_char> = args.iter().map(|s| s.as_ptr()).chain(iter::once(ptr::null())).collect();
        let mut res = MaybeUninit::<mpv_node>::zeroed();
        let res_ptr = res.as_mut_ptr();
        let ret = unsafe { mpv_command_ret(self.as_ptr().cast_mut(), raw_args.as_mut_ptr(), res_ptr) };
        let _guard = MpvNodeContentsGuard(res_ptr);
        result!(ret)?;
        let result = unsafe { Node::from(res.assume_init_ref()) };
        Ok(result)
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
    ///
    /// # Panics
    /// Panics if any argument contains a null byte.
    pub fn command_async<'a, I, S>(&self, reply: u64, args: I) -> Result<()>
    where
        I: IntoIterator<Item = S>,
        S: Into<Cow<'a, str>>,
    {
        let args: Vec<CString> = args
            .into_iter()
            .map(|s| CString::new(s.into().into_owned()).expect("input contains null byte"))
            .collect();

        let mut raw_args: Vec<*const c_char> = args.iter().map(|s| s.as_ptr()).chain(iter::once(ptr::null())).collect();
        unsafe {
            result!(mpv_command_async(
                self.as_ptr().cast_mut(),
                reply,
                raw_args.as_mut_ptr()
            ))
        }
    }

    /// # Errors
    /// Returns an mpv error if the property cannot be set.
    pub fn set_property<'a, S: Into<Cow<'a, str>>, T: Format>(&self, name: S, data: T) -> Result<()> {
        let name = CString::new(name.into().into_owned())?;
        let handle = self.as_ptr().cast_mut();
        data.to_mpv(|data| unsafe { result!(mpv_set_property(handle, name.as_ptr(), T::MPV_FORMAT, data)) })
    }

    /// Read the value of the given property.
    ///
    /// If the format doesn't match with the internal format of the property, access
    /// usually will fail with [`mpv_client_sys::mpv_error_MPV_ERROR_PROPERTY_FORMAT`]. In some cases, the data
    /// is automatically converted and access succeeds. For example, i64 is always
    /// converted to f64, and access using String usually invokes a string formatter.
    /// # Errors
    /// Returns an mpv error if the property cannot be read, or if the format
    /// doesn't match the internal format.
    pub fn get_property<'a, S: Into<Cow<'a, str>>, T: Format>(&self, name: S) -> Result<T> {
        let name = CString::new(name.into().into_owned())?;
        let handle = self.as_ptr().cast_mut();
        T::from_mpv(|data| unsafe { result!(mpv_get_property(handle, name.as_ptr(), T::MPV_FORMAT, data)) })
    }

    /// # Errors
    /// Returns an mpv error if property observation fails.
    pub fn observe_property<'a, S: Into<Cow<'a, str>>, T: Format>(&self, reply: u64, name: S) -> Result<()> {
        let name = CString::new(name.into().into_owned())?;
        unsafe {
            result!(mpv_observe_property(
                self.as_ptr().cast_mut(),
                reply,
                name.as_ptr(),
                T::MPV_FORMAT
            ))
        }
    }

    /// Undo [`Handle::observe_property`]. This will remove all observed properties for
    /// which the given number was passed as reply to [`Handle::observe_property`].
    ///
    /// Safe to be called from mpv render API threads.
    /// # Errors
    /// Returns an mpv error code, or 0 on success.
    pub fn unobserve_property(&self, registered_reply: u64) -> Result<i32> {
        unsafe { result_with_code!(mpv_unobserve_property(self.as_ptr().cast_mut(), registered_reply)) }
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
    /// Only the `mpv_handle` on which this was called will receive the hook events,
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
    pub fn hook_add<'a, S: Into<Cow<'a, str>>>(&self, reply: u64, name: S, priority: i32) -> Result<()> {
        let name = CString::new(name.into().into_owned())?;
        unsafe { result!(mpv_hook_add(self.as_ptr().cast_mut(), reply, name.as_ptr(), priority)) }
    }

    /// # Errors
    /// Returns an mpv error if hook continuation fails.
    pub fn hook_continue(&self, id: u64) -> Result<()> {
        unsafe { result!(mpv_hook_continue(self.as_ptr().cast_mut(), id)) }
    }

    pub fn request_log_messages<'a, S: Into<Cow<'a, str>>>(&self, min_level: S) -> Result<()> {
        unimplemented!()
    }

    pub fn get_property_async<'a, S: Into<Cow<'a, str>>>(&self, reply: u64, name: S) -> Result<()> {
        unimplemented!()
    }

    pub fn set_property_async<'a, S: Into<Cow<'a, str>>, T: Format>(&self, reply: u64, name: S, data: T) -> Result<()> {
        unimplemented!()
    }

    /// Return the `MPV_CLIENT_API_VERSION` the mpv source has been compiled with.
    #[must_use]
    pub fn api_version() -> u64 {
        unsafe { u64::from(mpv_client_api_version()) }
    }

    #[must_use]
    pub fn error_string(error: i32) -> &'static str {
        unsafe {
            CStr::from_ptr(mpv_error_string(error))
                .to_str()
                .unwrap_or("unknown error")
        }
    }

    pub fn load_config_file<P: AsRef<Path>>(&self, filename: P) -> Result<()> {
        unimplemented!()
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
        unsafe { mpv_get_time_ns(self.as_ptr().cast_mut()) }
    }

    /// Same as [`Handle::get_time_ns`] but in microseconds.
    #[must_use]
    pub fn get_time_us(&self) -> i64 {
        unsafe { mpv_get_time_us(self.as_ptr().cast_mut()) }
    }

    /// Signals to all async requests with the matching ID to abort.
    ///
    /// This affects the following API calls:
    /// * [`Handle::command_async()`]
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

    pub fn request_event(&self, event: u32, enable: i32) -> Result<()> {
        unimplemented!()
    }

    /// Interrupts the current [`Handle::wait_event()`] call.
    ///
    /// This will wake up the thread currently waiting in [`Handle::wait_event()`]. If no
    /// thread is waiting, the next [`Handle::wait_event()`] call will return immediately
    /// (this is to avoid lost wakeups).
    ///
    /// [`Handle::wait_event()`] will receive a [`mpv_event_id_MPV_EVENT_NONE`] if it is woken up due to
    /// this call. However, note that this dummy event might be skipped if there are
    /// already other events queued. All that matters is that the waiting thread
    /// is woken up at all.
    ///
    /// This function is **safe** to be called from `mpv` render API threads.
    pub fn wakeup(&self) {
        unsafe { mpv_wakeup(self.as_ptr().cast_mut()) }
    }

    pub fn set_wakeup_callback(&self) {
        unimplemented!()
    }

    pub fn wait_async_requests(&self) {
        unimplemented!()
    }

    /// # Panics
    /// Panics if `expand-path` or `script-opts` commands fail or return unexpected types.
    #[must_use]
    pub fn read_options<T>(&self) -> T
    where
        T: DeserializeOwned + Default,
    {
        let plugin_name = self.name();
        let mut raw_map = HashMap::new();

        let Node::String(config_dir) = self
            .command_ret(["expand-path", "~~/"])
            .expect("'expand-path ~~/' failed")
        else {
            unreachable!("'expand-path ~~/' always return a String variant")
        };

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

        let Node::Map(script_opts) = self
            .get_property::<&str, Node>("script-opts")
            .expect("'script-opts' property unavailable")
        else {
            unreachable!("'script-opts' always return a Map variant")
        };

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
    pub fn initialize_logging(&self) -> std::result::Result<(), log::SetLoggerError> {
        logging::init(self)
    }

    #[inline]
    #[must_use]
    const fn as_ptr(&self) -> *const mpv_handle {
        self.inner.as_ptr()
    }
}

/// SAFETY: libmpv guarantees that the same `mpv_handle` is safe to be called from multiple
/// threads concurrently. The single exception is [`mpv_wait_event`], which is strictly
/// protected at compile-time by requiring a unique &mut [`EventQueueToken`].
unsafe impl Sync for Handle {}

impl Client {
    /// Create a new standalone mpv client.
    ///
    /// # Errors
    /// Returns an error if mpv instance creation fails (out of memory).
    pub fn create() -> Result<(UninitializedClient, EventQueueToken)> {
        let handle = unsafe { mpv_create() };

        if handle.is_null() {
            Err(Error::new(mpv_error_MPV_ERROR_NOMEM))
        } else {
            let id = unsafe { mpv_client_id(handle) };
            Ok((UninitializedClient(handle), EventQueueToken(id)))
        }
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        unsafe { mpv_destroy(self.0) }
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

impl Drop for UninitializedClient {
    fn drop(&mut self) {
        unsafe { mpv_destroy(self.0) }
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

impl Event<'_> {
    unsafe fn from_ptr(event: *const mpv_event) -> Self {
        if event.is_null() {
            return Self::None;
        }

        unsafe {
            match (*event).event_id {
                mpv_event_id_MPV_EVENT_SHUTDOWN => Self::Shutdown,
                mpv_event_id_MPV_EVENT_LOG_MESSAGE => Self::LogMessage(LogMessage::from_ptr((*event).data)),
                mpv_event_id_MPV_EVENT_GET_PROPERTY_REPLY => {
                    let err = result!((*event).error);
                    let prop = if (*event).data.is_null() {
                        None
                    } else {
                        Some(Property::from_ptr((*event).data))
                    };
                    Self::GetPropertyReply(err, (*event).reply_userdata, prop)
                }
                mpv_event_id_MPV_EVENT_SET_PROPERTY_REPLY => {
                    Self::SetPropertyReply(result!((*event).error), (*event).reply_userdata)
                }
                mpv_event_id_MPV_EVENT_COMMAND_REPLY => {
                    Self::CommandReply(result!((*event).error), (*event).reply_userdata)
                }
                mpv_event_id_MPV_EVENT_START_FILE => Self::StartFile(StartFile::from_ptr((*event).data)),
                mpv_event_id_MPV_EVENT_END_FILE => Self::EndFile(EndFile::from_ptr((*event).data)),
                mpv_event_id_MPV_EVENT_FILE_LOADED => Self::FileLoaded,
                mpv_event_id_MPV_EVENT_CLIENT_MESSAGE => Self::ClientMessage(ClientMessage::from_ptr((*event).data)),
                mpv_event_id_MPV_EVENT_VIDEO_RECONFIG => Self::VideoReconfig,
                mpv_event_id_MPV_EVENT_AUDIO_RECONFIG => Self::AudioReconfig,
                mpv_event_id_MPV_EVENT_SEEK => Self::Seek,
                mpv_event_id_MPV_EVENT_PLAYBACK_RESTART => Self::PlaybackRestart,
                mpv_event_id_MPV_EVENT_PROPERTY_CHANGE => {
                    Self::PropertyChange((*event).reply_userdata, Property::from_ptr((*event).data))
                }
                mpv_event_id_MPV_EVENT_QUEUE_OVERFLOW => Self::QueueOverflow,
                mpv_event_id_MPV_EVENT_HOOK => Self::Hook((*event).reply_userdata, Hook::from_ptr((*event).data)),
                _ => Self::None,
            }
        }
    }
}

impl fmt::Display for Event<'_> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let event = match *self {
            Self::Shutdown => mpv_event_id_MPV_EVENT_SHUTDOWN,
            Self::LogMessage(..) => mpv_event_id_MPV_EVENT_LOG_MESSAGE,
            Self::GetPropertyReply(..) => mpv_event_id_MPV_EVENT_GET_PROPERTY_REPLY,
            Self::SetPropertyReply(..) => mpv_event_id_MPV_EVENT_SET_PROPERTY_REPLY,
            Self::CommandReply(..) => mpv_event_id_MPV_EVENT_COMMAND_REPLY,
            Self::StartFile(..) => mpv_event_id_MPV_EVENT_START_FILE,
            Self::EndFile(..) => mpv_event_id_MPV_EVENT_END_FILE,
            Self::FileLoaded => mpv_event_id_MPV_EVENT_FILE_LOADED,
            Self::ClientMessage(..) => mpv_event_id_MPV_EVENT_CLIENT_MESSAGE,
            Self::VideoReconfig => mpv_event_id_MPV_EVENT_VIDEO_RECONFIG,
            Self::AudioReconfig => mpv_event_id_MPV_EVENT_AUDIO_RECONFIG,
            Self::Seek => mpv_event_id_MPV_EVENT_SEEK,
            Self::PlaybackRestart => mpv_event_id_MPV_EVENT_PLAYBACK_RESTART,
            Self::PropertyChange(..) => mpv_event_id_MPV_EVENT_PROPERTY_CHANGE,
            Self::QueueOverflow => mpv_event_id_MPV_EVENT_QUEUE_OVERFLOW,
            Self::Hook(..) => mpv_event_id_MPV_EVENT_HOOK,
            Self::None => mpv_event_id_MPV_EVENT_NONE,
        };

        f.write_str(unsafe {
            CStr::from_ptr(mpv_event_name(event))
                .to_str()
                .unwrap_or("unknown event")
        })
    }
}

impl<'h> Property<'h> {
    /// Wrap a raw [`mpv_event_property`]
    /// The pointer must not be null
    fn from_ptr(ptr: *const c_void) -> Self {
        assert!(!ptr.is_null());
        Self(ptr.cast::<mpv_event_property>(), PhantomData)
    }

    /// Name of the property.
    #[must_use]
    pub fn name(&self) -> &'h str {
        unsafe { CStr::from_ptr((*self.0).name) }.to_str().unwrap_or("unknown")
    }

    #[must_use]
    pub fn data<T: Format>(&self) -> Option<T> {
        unsafe {
            if (*self.0).format == T::MPV_FORMAT {
                T::from_ptr((*self.0).data).ok()
            } else {
                None
            }
        }
    }
}

impl fmt::Display for Property<'_> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str(self.name())
    }
}

impl LogMessage<'_> {
    /// Wrap a raw [`mpv_event_log_message`]
    /// The pointer must not be null
    fn from_ptr(ptr: *const c_void) -> Self {
        assert!(!ptr.is_null());
        Self(ptr.cast::<mpv_event_log_message>(), PhantomData)
    }
}

impl fmt::Display for LogMessage<'_> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str("log message")
    }
}

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
        f.write_str("start file")
    }
}

impl EndFile<'_> {
    /// Wrap a raw [`mpv_event_end_file`]
    /// The pointer must not be null
    fn from_ptr(ptr: *const c_void) -> Self {
        assert!(!ptr.is_null());
        Self(ptr.cast::<mpv_event_end_file>(), PhantomData)
    }
}

impl fmt::Display for EndFile<'_> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str("end file")
    }
}

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
                std::slice::from_raw_parts((*self.0).args, num_args)
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
        f.write_str("client-message")
    }
}

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
        f.write_str(self.name())
    }
}
