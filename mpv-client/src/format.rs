use crate::{
    Handle, Result,
    node::{BorrowedMpvNode, ClonedMpvNode, MpvNode as _, Node, RawMpvNode},
};
use mpv_client_sys::{
    mpv_format_MPV_FORMAT_BYTE_ARRAY, mpv_format_MPV_FORMAT_DOUBLE, mpv_format_MPV_FORMAT_FLAG,
    mpv_format_MPV_FORMAT_INT64, mpv_format_MPV_FORMAT_NODE, mpv_format_MPV_FORMAT_NODE_ARRAY,
    mpv_format_MPV_FORMAT_NODE_MAP, mpv_format_MPV_FORMAT_NONE, mpv_format_MPV_FORMAT_OSD_STRING,
    mpv_format_MPV_FORMAT_STRING,
};
use std::{
    collections::HashMap,
    ffi::{CStr, CString, c_char, c_int, c_void},
    fmt::Display,
    ptr,
};

#[allow(private_bounds)]
pub trait Format: Sized + Default + Sealed {
    const MPV_FORMAT: u32;

    /// # Errors
    /// If the pointer does not point to a valid value of this format.
    fn from_ptr(ptr: *const c_void) -> Self;

    /// # Errors
    /// If the FFI callback fails.
    fn to_mpv<F: Fn(*mut c_void) -> Result<()>>(self, fun: F) -> Result<()>;

    /// # Errors
    /// If the FFI callback fails or the stored value cannot be recovered.
    fn from_mpv<F: Fn(*mut c_void) -> Result<()>>(fun: F) -> Result<Self>;
}

impl Format for () {
    const MPV_FORMAT: u32 = FormatType::None as u32;

    fn from_ptr(_ptr: *const c_void) -> Self {}

    fn to_mpv<F: Fn(*mut c_void) -> Result<()>>(self, fun: F) -> Result<()> {
        fun(ptr::null_mut())
    }

    fn from_mpv<F: Fn(*mut c_void) -> Result<()>>(fun: F) -> Result<Self> {
        fun(ptr::null_mut())?;
        Ok(())
    }
}

impl Format for String {
    const MPV_FORMAT: u32 = FormatType::String as u32;

    /// # Errors
    /// Returns an error if the C string is not valid UTF-8.
    fn from_ptr(ptr: *const c_void) -> Self {
        let ptr = ptr.cast::<*const c_char>();
        let string_ptr = unsafe { *ptr };

        if string_ptr.is_null() {
            return Self::new();
        }

        unsafe { CStr::from_ptr(string_ptr) }.to_string_lossy().into_owned()
    }

    fn to_mpv<F: Fn(*mut c_void) -> Result<()>>(self, fun: F) -> Result<()> {
        let cstr = CString::new(self)?;
        let mut ptr = cstr.as_ptr();
        fun((&raw mut ptr).cast::<c_void>())
    }

    /// # Errors
    /// Returns an error if the FFI callback fails or the returned pointer is null/invalid UTF-8.
    fn from_mpv<F: Fn(*mut c_void) -> Result<()>>(fun: F) -> Result<Self> {
        let mut mpv_string_ptr: *mut c_char = ptr::null_mut();
        fun((&raw mut mpv_string_ptr).cast::<c_void>())?;
        let _mpv_string = ClonedMpvString(mpv_string_ptr);

        if mpv_string_ptr.is_null() {
            return Ok(Self::new());
        }

        let string = unsafe { CStr::from_ptr(mpv_string_ptr) }
            .to_str()
            .map(ToOwned::to_owned)?;

        Ok(string)
    }
}

impl Format for OsdString {
    const MPV_FORMAT: u32 = FormatType::OsdString as u32;

    /// # Errors
    /// Returns an error if the C string is not valid UTF-8.
    fn from_ptr(ptr: *const c_void) -> Self {
        Self(String::from_ptr(ptr))
    }

    fn to_mpv<F: Fn(*mut c_void) -> Result<()>>(self, fun: F) -> Result<()> {
        String::to_mpv(self.0, fun)
    }

    /// # Errors
    /// Returns an error if the FFI callback fails or the returned pointer is null/invalid UTF-8.
    fn from_mpv<F: Fn(*mut c_void) -> Result<()>>(fun: F) -> Result<Self> {
        Ok(Self(String::from_mpv(fun)?))
    }
}

impl Format for bool {
    const MPV_FORMAT: u32 = FormatType::Bool as u32;

    fn from_ptr(ptr: *const c_void) -> Self {
        unsafe { *ptr.cast::<i32>() != 0 }
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
    const MPV_FORMAT: u32 = FormatType::Int as u32;

    fn from_ptr(ptr: *const c_void) -> Self {
        unsafe { *ptr.cast() }
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
    const MPV_FORMAT: u32 = FormatType::Double as u32;

    fn from_ptr(ptr: *const c_void) -> Self {
        unsafe { *ptr.cast() }
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
    const MPV_FORMAT: u32 = FormatType::Node as u32;

    fn from_ptr(ptr: *const c_void) -> Self {
        let Some(mpv_node) = BorrowedMpvNode::from_ptr(ptr) else {
            return Self::None;
        };

        mpv_node.to_node()
    }

    fn to_mpv<F: Fn(*mut c_void) -> Result<()>>(self, fun: F) -> Result<()> {
        let mut mpv_node = RawMpvNode::from_node(self);
        fun(mpv_node.as_mut_ptr().cast::<c_void>())
    }

    fn from_mpv<F: Fn(*mut c_void) -> Result<()>>(fun: F) -> Result<Self> {
        let mut mpv_node = ClonedMpvNode::default();
        fun(mpv_node.as_mut_ptr().cast())?;
        Ok(mpv_node.to_node())
    }
}

impl Format for Vec<Node> {
    const MPV_FORMAT: u32 = FormatType::NodeArray as u32;

    fn from_ptr(ptr: *const c_void) -> Self {
        let Some(mpv_node) = BorrowedMpvNode::from_ptr(ptr) else {
            return vec![];
        };

        mpv_node.to_node_array()
    }

    fn to_mpv<F: Fn(*mut c_void) -> Result<()>>(self, fun: F) -> Result<()> {
        let mut mpv_node = RawMpvNode::from_node_array(self);
        fun(mpv_node.as_mut_ptr().cast::<c_void>())
    }

    fn from_mpv<F: Fn(*mut c_void) -> Result<()>>(fun: F) -> Result<Self> {
        let mut mpv_node = ClonedMpvNode::default();
        fun(mpv_node.as_mut_ptr().cast())?;
        Ok(mpv_node.to_node_array())
    }
}

trait Sealed {}
impl Sealed for () {}
impl Sealed for String {}
impl Sealed for OsdString {}
impl Sealed for bool {}
impl Sealed for i64 {}
impl Sealed for f64 {}
impl Sealed for Node {}
impl Sealed for Vec<Node> {}
impl Sealed for HashMap<String, Node> {}
impl Sealed for Vec<u8> {}

struct ClonedMpvString(*mut c_char);
impl Drop for ClonedMpvString {
    fn drop(&mut self) {
        Handle::free(self.0.cast::<c_void>());
    }
}

#[repr(u32)]
#[derive(Debug, Clone, Copy)]
pub enum FormatType {
    None = mpv_format_MPV_FORMAT_NONE,
    String = mpv_format_MPV_FORMAT_STRING,
    OsdString = mpv_format_MPV_FORMAT_OSD_STRING,
    Bool = mpv_format_MPV_FORMAT_FLAG,
    Int = mpv_format_MPV_FORMAT_INT64,
    Double = mpv_format_MPV_FORMAT_DOUBLE,
    Node = mpv_format_MPV_FORMAT_NODE,
    NodeArray = mpv_format_MPV_FORMAT_NODE_ARRAY,
    NodeMap = mpv_format_MPV_FORMAT_NODE_MAP,
    ByteArray = mpv_format_MPV_FORMAT_BYTE_ARRAY,
}

impl TryFrom<u32> for FormatType {
    type Error = crate::Error;

    fn try_from(value: u32) -> std::result::Result<Self, Self::Error> {
        match value {
            mpv_format_MPV_FORMAT_NONE => Ok(Self::None),
            mpv_format_MPV_FORMAT_STRING => Ok(Self::String),
            mpv_format_MPV_FORMAT_OSD_STRING => Ok(Self::OsdString),
            mpv_format_MPV_FORMAT_FLAG => Ok(Self::Bool),
            mpv_format_MPV_FORMAT_INT64 => Ok(Self::Int),
            mpv_format_MPV_FORMAT_DOUBLE => Ok(Self::Double),
            mpv_format_MPV_FORMAT_NODE => Ok(Self::Node),
            mpv_format_MPV_FORMAT_NODE_ARRAY => Ok(Self::NodeArray),
            mpv_format_MPV_FORMAT_NODE_MAP => Ok(Self::NodeMap),
            mpv_format_MPV_FORMAT_BYTE_ARRAY => Ok(Self::ByteArray),
            value => Err(crate::Error::UnknownFormat(value)),
        }
    }
}

impl Display for FormatType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::None => f.write_str("None"),
            Self::String => f.write_str("String"),
            Self::OsdString => f.write_str("OsdString"),
            Self::Bool => f.write_str("Bool"),
            Self::Int => f.write_str("Int"),
            Self::Double => f.write_str("Double"),
            Self::Node => f.write_str("Node"),
            Self::NodeArray => f.write_str("NodeArray"),
            Self::NodeMap => f.write_str("NodeMap"),
            Self::ByteArray => f.write_str("ByteArray"),
        }
    }
}

#[derive(Debug, Default)]
pub struct OsdString(pub String);
