#![allow(non_upper_case_globals)]
#![allow(non_camel_case_types)]
#![allow(non_snake_case)]

mod error;
mod format;
mod logging;
pub mod node;
mod options;

pub use error::{Error, Result};
pub use format::Format;
pub use node::Node;
use serde::de::{self, DeserializeOwned};

use std::collections::HashMap;
use std::ffi::{CStr, CString, c_char, c_void};
use std::marker::PhantomData;
use std::mem::MaybeUninit;
use std::ops::Deref;
use std::path::PathBuf;
use std::{fmt, fs};

pub use mpv_client_sys::mpv_handle;
use mpv_client_sys::{
    mpv_byte_array, mpv_client_id, mpv_client_name, mpv_command, mpv_command_async, mpv_command_ret, mpv_create,
    mpv_create_client, mpv_create_weak_client, mpv_destroy, mpv_error, mpv_error_MPV_ERROR_GENERIC,
    mpv_error_MPV_ERROR_NOMEM, mpv_error_MPV_ERROR_SUCCESS, mpv_error_string, mpv_event, mpv_event_client_message,
    mpv_event_end_file, mpv_event_hook, mpv_event_id_MPV_EVENT_AUDIO_RECONFIG, mpv_event_id_MPV_EVENT_CLIENT_MESSAGE,
    mpv_event_id_MPV_EVENT_COMMAND_REPLY, mpv_event_id_MPV_EVENT_END_FILE, mpv_event_id_MPV_EVENT_FILE_LOADED,
    mpv_event_id_MPV_EVENT_GET_PROPERTY_REPLY, mpv_event_id_MPV_EVENT_HOOK, mpv_event_id_MPV_EVENT_LOG_MESSAGE,
    mpv_event_id_MPV_EVENT_NONE, mpv_event_id_MPV_EVENT_PLAYBACK_RESTART, mpv_event_id_MPV_EVENT_PROPERTY_CHANGE,
    mpv_event_id_MPV_EVENT_QUEUE_OVERFLOW, mpv_event_id_MPV_EVENT_SEEK, mpv_event_id_MPV_EVENT_SET_PROPERTY_REPLY,
    mpv_event_id_MPV_EVENT_SHUTDOWN, mpv_event_id_MPV_EVENT_START_FILE, mpv_event_id_MPV_EVENT_VIDEO_RECONFIG,
    mpv_event_log_message, mpv_event_name, mpv_event_property, mpv_event_start_file, mpv_format_MPV_FORMAT_BYTE_ARRAY,
    mpv_format_MPV_FORMAT_DOUBLE, mpv_format_MPV_FORMAT_FLAG, mpv_format_MPV_FORMAT_INT64,
    mpv_format_MPV_FORMAT_NODE_ARRAY, mpv_format_MPV_FORMAT_NODE_MAP, mpv_format_MPV_FORMAT_NONE,
    mpv_format_MPV_FORMAT_STRING, mpv_free, mpv_free_node_contents, mpv_get_property, mpv_hook_add, mpv_hook_continue,
    mpv_initialize, mpv_node, mpv_node__bindgen_ty_1, mpv_node_list, mpv_observe_property, mpv_set_property,
    mpv_unobserve_property, mpv_wait_event,
};

use crate::node::from_mpv_node_value;
use crate::options::CoercingString;

#[cfg(feature = "macros")]
pub use mpv_client_macros::main;

/// Representation of a borrowed client context used by the client API.
/// Every client has its own private handle.
#[repr(transparent)]
pub struct Handle {
    inner: [mpv_handle],
}

pub struct EventQueueToken(*const mpv_handle);

/// A type representing an owned client context.
pub struct Client(*mut mpv_handle);

/// An enum representing the available events that can be received by
/// `Handle::wait_event`.
pub enum Event<'h> {
    /// Nothing happened. Happens on timeouts or sporadic wakeups.
    None,
    /// Happens when the player quits. The player enters a state where it tries
    /// to disconnect all clients.
    Shutdown,
    /// See `Handle::request_log_messages`.
    /// See also `LogMessage`.
    LogMessage(LogMessage<'h>),
    /// Reply to a `Handle::get_property_async` request.
    /// See also `Property`.
    GetPropertyReply(Result<()>, u64, Property<'h>),
    /// Reply to a `Handle::set_property_async` request.
    /// (Unlike `GetPropertyReply`, `Property` is not used.)
    SetPropertyReply(Result<()>, u64),
    /// Reply to a `Handle::command_async` or `mpv_command_node_async()` request.
    /// See also `Command`.
    CommandReply(Result<()>, u64), // TODO mpv_event_command and mpv_node
    /// Notification before playback start of a file (before the file is loaded).
    /// See also `StartFile`.
    StartFile(StartFile<'h>),
    /// Notification after playback end (after the file was unloaded).
    /// See also `EndFile`.
    EndFile(EndFile<'h>),
    /// Notification when the file has been loaded (headers were read etc.), and
    /// decoding starts.
    FileLoaded,
    /// Triggered by the script-message input command. The command uses the
    /// first argument of the command as client name (see `Handle::client_name`) to
    /// dispatch the message, and passes along all arguments starting from the
    /// second argument as strings.
    /// See also `ClientMessage`.
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
    /// Similar to `VideoReconfig`. This is relatively uninteresting,
    /// because there is no such thing as audio output embedding.
    AudioReconfig,
    /// Happens when a seek was initiated. Playback stops. Usually it will
    /// resume with `PlaybackRestart` as soon as the seek is finished.
    Seek,
    /// There was a discontinuity of some sort (like a seek), and playback
    /// was reinitialized. Usually happens on start of playback and after
    /// seeking. The main purpose is allowing the client to detect when a seek
    /// request is finished.
    PlaybackRestart,
    /// Event sent due to `mpv_observe_property()`.
    /// See also `Property`.
    PropertyChange(u64, Property<'h>),
    /// Happens if the internal per-mpv_handle ringbuffer overflows, and at
    /// least 1 event had to be dropped. This can happen if the client doesn't
    /// read the event queue quickly enough with `Handle::wait_event`, or if the
    /// client makes a very large number of asynchronous calls at once.
    ///
    /// Event delivery will continue normally once this event was returned
    /// (this forces the client to empty the queue completely).
    QueueOverflow,
    /// Triggered if a hook handler was registered with `Handle::hook_add`, and the
    /// hook is invoked. If you receive this, you must handle it, and continue
    /// the hook with `Handle::hook_continue`.
    /// See also `Hook`.
    Hook(u64, Hook<'h>),
}

/// Data associated with `Event::GetPropertyReply` and `Event::PropertyChange`.
pub struct Property<'h>(*const mpv_event_property, PhantomData<&'h Handle>);

/// Data associated with `Event::LogMessage`.
#[allow(dead_code)]
pub struct LogMessage<'h>(*const mpv_event_log_message, PhantomData<&'h Handle>);

/// Data associated with `Event::StartFile`.
pub struct StartFile<'h>(*const mpv_event_start_file, PhantomData<&'h Handle>);

/// Data associated with `Event::EndFile`.
#[allow(dead_code)]
pub struct EndFile<'h>(*const mpv_event_end_file, PhantomData<&'h Handle>);

/// Data associated with `Event::ClientMessage`.
pub struct ClientMessage<'h>(*const mpv_event_client_message, PhantomData<&'h Handle>);

/// Data associated with `Event::Hook`.
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
        $client.command(&["show-text", &format!($($arg)*), &$duration.as_millis().to_string()])
    }
}

#[macro_export]
macro_rules! osd_async {
    ($client:expr, $reply:expr, $duration:expr, $($arg:tt)*) => {
        $client.command_async($reply, &["show-text", &format!($($arg)*), &$duration.as_millis().to_string()])
    }
}

impl Handle {
    /// Wrap a raw `mpv_handle` as a shared reference.
    ///
    /// # Safety
    ///
    /// * `ptr` must be non-null.
    ///
    /// * The memory referenced by the returned `Handle` must not be freed for
    ///   the duration of lifetime `'a`.
    ///
    /// * No mutable references to the same `mpv_handle` may exist for the
    ///   duration of lifetime `'a`.
    #[inline]
    #[must_use]
    pub const unsafe fn from_ptr<'a>(ptr: *const mpv_handle) -> (&'a Self, EventQueueToken) {
        (
            unsafe { &*(std::ptr::slice_from_raw_parts(ptr, 1) as *const Self) },
            EventQueueToken(ptr),
        )
    }

    #[inline]
    #[must_use]
    pub const fn as_ptr(&self) -> *const mpv_handle {
        self.inner.as_ptr()
    }

    #[inline]
    pub const fn as_mut_ptr(&mut self) -> *mut mpv_handle {
        self.inner.as_mut_ptr()
    }

    /// # Errors
    ///
    /// Returns an error if the mpv API call fails.
    pub fn create_client(&self, name: impl AsRef<str>) -> Result<(Client, EventQueueToken)> {
        let name = CString::new(name.as_ref())?;
        let handle = unsafe { mpv_create_client(self.as_ptr().cast_mut(), name.as_ptr()) };
        if handle.is_null() {
            Err(Error::new(mpv_error_MPV_ERROR_NOMEM))
        } else {
            Ok((Client(handle), EventQueueToken(handle)))
        }
    }

    /// # Errors
    ///
    /// Returns an error if the mpv API call fails.
    pub fn create_weak_client(&self, name: impl AsRef<str>) -> Result<(Client, EventQueueToken)> {
        let name = CString::new(name.as_ref())?;
        let handle = unsafe { mpv_create_weak_client(self.as_ptr().cast_mut(), name.as_ptr()) };
        if handle.is_null() {
            Err(Error::new(mpv_error_MPV_ERROR_NOMEM))
        } else {
            Ok((Client(handle), EventQueueToken(handle)))
        }
    }

    /// Wait for the next event, or until the timeout expires, or if another thread
    /// makes a call to `mpv_wakeup()`. Passing 0 as timeout will never wait, and
    /// is suitable for polling.
    ///
    /// The internal event queue has a limited size (per client handle). If you
    /// don't empty the event queue quickly enough with `Handle::wait_event`, it will
    /// overflow and silently discard further events. If this happens, making
    /// asynchronous requests will fail as well (with `MPV_ERROR_EVENT_QUEUE_FULL`).
    ///
    /// Only one thread is allowed to call this on the same `Handle` at a time.
    /// The API won't complain if more than one thread calls this, but it will cause
    /// race conditions in the client when accessing the shared `mpv_event` struct.
    /// Note that most other API functions are not restricted by this, and no API
    /// function internally calls `mpv_wait_event()`. Additionally, concurrent calls
    /// to different handles are always safe.
    ///
    /// As long as the timeout is 0, this is safe to be called from mpv render API
    /// threads.
    ///
    /// # Arguments
    ///
    /// * `_token` - An exclusive capability token (`&mut EventQueueToken`) that enforces
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
    /// Panics if the provided `EventQueueToken` is mismatched and does not belong
    /// to this specific `Handle` instance.
    pub fn wait_event<'h>(&'h self, token: &'h mut EventQueueToken, timeout: f64) -> Event<'h> {
        assert_eq!(
            self.as_ptr(),
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
    /// ID is never reused by the core, even if the `mpv_handle` at hand gets destroyed
    /// and new handles get allocated.
    ///
    /// IDs are never 0 or negative.
    ///
    /// Some mpv APIs (not necessarily all) accept a name in the form "@<id>" in
    /// addition of the proper `mpv_client_name()`, where "<id>" is the ID in decimal
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
    pub fn command<I, S>(&self, args: I) -> Result<()>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let args: Vec<CString> = args
            .into_iter()
            .map(|s| CString::new(s.as_ref()).expect("input contains null byte"))
            .collect();
        let mut raw_args: Vec<*const c_char> = args.iter().map(|s| s.as_ptr()).collect();
        raw_args.push(std::ptr::null()); // Adding null at the end
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
    pub fn command_ret<I, S>(&self, args: I) -> Result<Node>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let args: Vec<CString> = args
            .into_iter()
            .map(|s| CString::new(s.as_ref()).expect("input contains null byte"))
            .collect();

        let mut raw_args: Vec<*const c_char> = args.iter().map(|s| s.as_ptr()).collect();
        raw_args.push(std::ptr::null()); // Adding null at the end

        let mut res = MaybeUninit::<mpv_node>::zeroed();
        let ret = unsafe { mpv_command_ret(self.as_ptr().cast_mut(), raw_args.as_mut_ptr(), res.as_mut_ptr()) };

        result!(ret)?;
        let result = unsafe { from_mpv_node_value(res.assume_init_mut()) };
        unsafe { mpv_free_node_contents(res.as_mut_ptr()) };
        Ok(result)
    }

    /// Same as `Handle::command`, but run the command asynchronously.
    ///
    /// Commands are executed asynchronously. You will receive a
    /// `CommandReply` event. This event will also have an
    /// error code set if running the command failed. For commands that
    /// return data, the data is put into `mpv_event_command.result`.
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
    pub fn command_async<I, S>(&self, reply: u64, args: I) -> Result<()>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let args: Vec<CString> = args
            .into_iter()
            .map(|s| CString::new(s.as_ref()).expect("input contains null byte"))
            .collect();
        let mut raw_args: Vec<*const c_char> = args.iter().map(|s| s.as_ptr()).collect();
        raw_args.push(std::ptr::null()); // Adding null at the end
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
    pub fn set_property<T: Format>(&self, name: impl AsRef<str>, data: T) -> Result<()> {
        let name = CString::new(name.as_ref())?;
        let handle = self.as_ptr().cast_mut();
        data.to_mpv(|data| unsafe { result!(mpv_set_property(handle, name.as_ptr(), T::MPV_FORMAT, data)) })
    }

    /// Read the value of the given property.
    ///
    /// If the format doesn't match with the internal format of the property, access
    /// usually will fail with `MPV_ERROR_PROPERTY_FORMAT`. In some cases, the data
    /// is automatically converted and access succeeds. For example, i64 is always
    /// converted to f64, and access using String usually invokes a string formatter.
    /// # Errors
    /// Returns an mpv error if the property cannot be read, or if the format
    /// doesn't match the internal format.
    pub fn get_property<T: Format>(&self, name: impl AsRef<str>) -> Result<T> {
        let name = CString::new(name.as_ref())?;
        let handle = self.as_ptr().cast_mut();
        T::from_mpv(|data| unsafe { result!(mpv_get_property(handle, name.as_ptr(), T::MPV_FORMAT, data)) })
    }

    /// # Errors
    /// Returns an mpv error if property observation fails.
    pub fn observe_property<T: Format>(&self, reply: u64, name: impl AsRef<str>) -> Result<()> {
        let name = CString::new(name.as_ref())?;
        unsafe {
            result!(mpv_observe_property(
                self.as_ptr().cast_mut(),
                reply,
                name.as_ptr(),
                T::MPV_FORMAT
            ))
        }
    }

    /// Undo `Handle::observe_property`. This will remove all observed properties for
    /// which the given number was passed as reply to `Handle::observe_property`.
    ///
    /// Safe to be called from mpv render API threads.
    /// # Errors
    /// Returns an mpv error code, or 0 on success.
    pub fn unobserve_property(&self, registered_reply: u64) -> Result<i32> {
        unsafe { result_with_code!(mpv_unobserve_property(self.as_ptr().cast_mut(), registered_reply)) }
    }

    /// # Errors
    /// Returns an mpv error if the hook cannot be added.
    pub fn hook_add(&self, reply: u64, name: &str, priority: i32) -> Result<()> {
        let name = CString::new(name)?;
        unsafe { result!(mpv_hook_add(self.as_ptr().cast_mut(), reply, name.as_ptr(), priority)) }
    }

    /// # Errors
    /// Returns an mpv error if hook continuation fails.
    pub fn hook_continue(&self, id: u64) -> Result<()> {
        unsafe { result!(mpv_hook_continue(self.as_ptr().cast_mut(), id)) }
    }

    #[must_use]
    /// # Panics
    /// Panics if `expand-path` or `script-opts` commands fail or return unexpected types.
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
            .get_property::<Node>("script-opts")
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
    /// Returns `log::SetLoggerError` if a logger is already set.
    pub fn init_logger(&self) -> std::result::Result<(), log::SetLoggerError> {
        logging::init(self)
    }
}

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
            Ok((UninitializedClient(handle), EventQueueToken(handle)))
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
        unsafe { Handle::from_ptr(self.0).0 }
    }
}

unsafe impl Send for Client {}

pub struct UninitializedClient(*mut mpv_handle);

impl Drop for UninitializedClient {
    fn drop(&mut self) {
        unsafe { mpv_destroy(self.0) }
    }
}

impl UninitializedClient {
    /// Initialize the mpv core. Consumes the uninitialized client and returns
    /// a ready-to-use `Client`.
    ///
    /// # Errors
    /// Returns an mpv error if initialization fails.
    pub fn initialize(self) -> Result<Client> {
        let handle = self.0;
        std::mem::forget(self);

        unsafe { result!(mpv_initialize(handle)).map(|()| Client(handle)) }
    }
}

impl Event<'_> {
    unsafe fn from_ptr(event: *const mpv_event) -> Self {
        unsafe {
            match (*event).event_id {
                mpv_event_id_MPV_EVENT_SHUTDOWN => Self::Shutdown,
                mpv_event_id_MPV_EVENT_LOG_MESSAGE => Self::LogMessage(LogMessage::from_ptr((*event).data)),
                mpv_event_id_MPV_EVENT_GET_PROPERTY_REPLY => Self::GetPropertyReply(
                    result!((*event).error),
                    (*event).reply_userdata,
                    Property::from_ptr((*event).data),
                ),
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
    /// Wrap a raw `mpv_event_property`
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
    /// Wrap a raw `mpv_event_log_message`
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
    /// Wrap a raw `mpv_event_start_file`
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
    /// Wrap a raw `mpv_event_end_file`
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
    /// Wrap a raw `mpv_event_client_message`.
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
            let args = std::slice::from_raw_parts(
                (*self.0).args,
                (*self.0).num_args.try_into().expect("negative num_args"),
            );
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
    /// Wrap a raw `mpv_event_hook`.
    /// The pointer must not be null
    fn from_ptr(ptr: *const c_void) -> Self {
        assert!(!ptr.is_null());
        Self(ptr.cast::<mpv_event_hook>(), PhantomData)
    }

    /// The hook name as passed to `Handle::hook_add`.
    #[must_use]
    pub fn name(&self) -> &'h str {
        unsafe { CStr::from_ptr((*self.0).name).to_str().unwrap_or("unknown") }
    }

    /// Internal ID that must be passed to `Handle::hook_continue`.
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
