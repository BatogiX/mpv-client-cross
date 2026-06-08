use crate::{
    Result,
    node::{MpvNodeGuard, Node},
};
use mpv_client_sys::{mpv_format_MPV_FORMAT_NONE, mpv_free, mpv_free_node_contents, mpv_node, mpv_node__bindgen_ty_1};
use std::ffi::{CStr, CString, c_char, c_int, c_void};

pub trait Format: Sized + Default {
    const MPV_FORMAT: u32;

    /// # Errors
    /// If the pointer does not point to a valid value of this format.
    fn from_ptr(ptr: *const c_void) -> Result<Self>;

    /// # Errors
    /// If the FFI callback fails.
    fn to_mpv<F: Fn(*mut c_void) -> Result<()>>(self, fun: F) -> Result<()>;

    /// # Errors
    /// If the FFI callback fails or the stored value cannot be recovered.
    fn from_mpv<F: Fn(*mut c_void) -> Result<()>>(fun: F) -> Result<Self>;
}

impl Format for String {
    const MPV_FORMAT: u32 = 1;

    /// # Errors
    /// Returns an error if the C string is not valid UTF-8.
    fn from_ptr(ptr: *const c_void) -> Result<Self> {
        let ptr = ptr.cast::<*const c_char>();
        let string_ptr = unsafe { *ptr };

        if string_ptr.is_null() {
            return Ok(Self::new());
        }

        Ok(unsafe { CStr::from_ptr(string_ptr) }.to_str()?.to_owned())
    }

    fn to_mpv<F: Fn(*mut c_void) -> Result<()>>(self, fun: F) -> Result<()> {
        let cstr = CString::new(self)?;
        let mut ptr = cstr.as_ptr();
        fun((&raw mut ptr).cast::<c_void>())
    }

    /// # Errors
    /// Returns an error if the FFI callback fails or the returned pointer is null/invalid UTF-8.
    fn from_mpv<F: Fn(*mut c_void) -> Result<()>>(fun: F) -> Result<Self> {
        let mut ptr: *mut c_char = std::ptr::null_mut();
        fun((&raw mut ptr).cast::<c_void>())?;
        let _guard = MpvFreeGuard(ptr);

        if ptr.is_null() {
            return Ok(Self::new());
        }

        let result = unsafe { CStr::from_ptr(ptr) }.to_str().map(ToOwned::to_owned);
        Ok(result?)
    }
}

impl Format for bool {
    const MPV_FORMAT: u32 = 3;

    fn from_ptr(ptr: *const c_void) -> Result<Self> {
        Ok(unsafe { *ptr.cast::<c_int>() != 0 })
    }

    fn to_mpv<F: Fn(*mut c_void) -> Result<()>>(self, fun: F) -> Result<()> {
        let mut data = c_int::from(self);
        fun((&raw mut data).cast::<c_void>())
    }

    fn from_mpv<F: Fn(*mut c_void) -> Result<()>>(fun: F) -> Result<Self> {
        let mut data = c_int::from(Self::default());
        fun((&raw mut data).cast::<c_void>()).map(|()| data != 0)
    }
}

impl Format for i64 {
    const MPV_FORMAT: u32 = 4;

    fn from_ptr(ptr: *const c_void) -> Result<Self> {
        Ok(unsafe { *ptr.cast::<Self>() })
    }

    fn to_mpv<F: Fn(*mut c_void) -> Result<()>>(self, fun: F) -> Result<()> {
        let mut data = self;
        fun((&raw mut data).cast::<c_void>())
    }

    fn from_mpv<F: Fn(*mut c_void) -> Result<()>>(fun: F) -> Result<Self> {
        let mut data = Self::default();
        fun((&raw mut data).cast::<c_void>()).map(|()| data)
    }
}

impl Format for f64 {
    const MPV_FORMAT: u32 = 5;

    fn from_ptr(ptr: *const c_void) -> Result<Self> {
        Ok(unsafe { *ptr.cast::<Self>() })
    }

    fn to_mpv<F: Fn(*mut c_void) -> Result<()>>(self, fun: F) -> Result<()> {
        let mut data = self;
        fun((&raw mut data).cast::<c_void>())
    }

    fn from_mpv<F: Fn(*mut c_void) -> Result<()>>(fun: F) -> Result<Self> {
        let mut data = Self::default();
        fun((&raw mut data).cast::<c_void>()).map(|()| data)
    }
}

impl Format for Node {
    const MPV_FORMAT: u32 = 6;

    fn from_ptr(ptr: *const c_void) -> Result<Self> {
        if ptr.is_null() {
            return Ok(Self::None);
        }

        let node = unsafe { &*ptr.cast::<mpv_node>() };
        let result = Self::from(node);
        Ok(result)
    }

    fn to_mpv<F: Fn(*mut c_void) -> Result<()>>(self, fun: F) -> Result<()> {
        let guard = MpvNodeGuard::from(&self);
        fun(guard.as_ptr().cast::<c_void>())
    }

    fn from_mpv<F: Fn(*mut c_void) -> Result<()>>(fun: F) -> Result<Self> {
        let mut node = mpv_node {
            format: mpv_format_MPV_FORMAT_NONE,
            u: mpv_node__bindgen_ty_1 { int64: 0 },
        };

        let _guard = MpvNodeContentsGuard(&raw mut node);
        fun((&raw mut node).cast::<c_void>())?;
        let result = Self::from(&node);
        Ok(result)
    }
}

struct MpvFreeGuard(*mut c_char);
impl Drop for MpvFreeGuard {
    fn drop(&mut self) {
        unsafe { mpv_free(self.0.cast::<c_void>()) };
    }
}

struct MpvNodeContentsGuard(*mut mpv_node);
impl Drop for MpvNodeContentsGuard {
    fn drop(&mut self) {
        unsafe { mpv_free_node_contents(self.0) };
    }
}
