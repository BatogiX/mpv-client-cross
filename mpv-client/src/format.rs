use super::Result;
use super::{mpv_format_MPV_FORMAT_NONE, mpv_free, mpv_free_node_contents, mpv_node, mpv_node__bindgen_ty_1};

use std::ffi::{CStr, CString, c_char, c_int, c_void};

use super::node::Node;

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
        Ok(unsafe { CStr::from_ptr(*ptr) }.to_str()?.to_owned())
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
        let result = unsafe { CStr::from_ptr(ptr) }.to_str().map(ToOwned::to_owned);
        unsafe { mpv_free(ptr.cast::<c_void>()) };
        Ok(result?)
    }
}

impl Format for bool {
    const MPV_FORMAT: u32 = 3;

    fn from_ptr(ptr: *const c_void) -> Result<Self> {
        Ok(unsafe { *ptr.cast::<c_int>() != 0 })
    }

    fn to_mpv<F: Fn(*mut c_void) -> Result<()>>(self, fun: F) -> Result<()> {
        let data = c_int::from(self);
        fun(&raw const data as *mut c_void)
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
        fun(&raw const self as *mut c_void)
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
        fun(&raw const self as *mut c_void)
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

        let node = unsafe { &mut *(ptr as *mut mpv_node) };
        let result = Self::from(node);
        Ok(result)
    }

    fn to_mpv<F: Fn(*mut c_void) -> Result<()>>(self, fun: F) -> Result<()> {
        let mpv_node_ptr = <*mut mpv_node>::from(&self);
        let res = fun(mpv_node_ptr.cast::<c_void>());
        unsafe { mpv_free_node_contents(mpv_node_ptr) };
        res
    }

    fn from_mpv<F: Fn(*mut c_void) -> Result<()>>(fun: F) -> Result<Self> {
        let mut node = mpv_node {
            format: mpv_format_MPV_FORMAT_NONE,
            u: mpv_node__bindgen_ty_1 { int64: 0 },
        };

        fun((&raw mut node).cast::<c_void>())?;
        let result = Self::from(&mut node);
        unsafe { mpv_free_node_contents(&raw mut node) };
        Ok(result)
    }
}
