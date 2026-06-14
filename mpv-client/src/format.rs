use crate::{
    Handle,
    node::{BorrowedMpvNode, ClonedMpvNode, MpvNode as _, Node, RawMpvNode},
};
use ::std::hash::BuildHasher;
use mpv_client_sys::{
    mpv_format_MPV_FORMAT_BYTE_ARRAY, mpv_format_MPV_FORMAT_DOUBLE, mpv_format_MPV_FORMAT_FLAG,
    mpv_format_MPV_FORMAT_INT64, mpv_format_MPV_FORMAT_NODE, mpv_format_MPV_FORMAT_NODE_ARRAY,
    mpv_format_MPV_FORMAT_NODE_MAP, mpv_format_MPV_FORMAT_NONE, mpv_format_MPV_FORMAT_OSD_STRING,
    mpv_format_MPV_FORMAT_STRING,
};
use std::{
    borrow::Borrow,
    collections::HashMap,
    convert,
    ffi::{CStr, CString, c_char, c_int, c_void},
    fmt::{self, Display},
    hash::RandomState,
    ops::{Deref, DerefMut},
    ptr,
    str::FromStr,
};

#[allow(private_bounds)]
pub trait AsFormat: Sealed {
    const MPV_FORMAT: u32;
}

impl AsFormat for () {
    const MPV_FORMAT: u32 = Format::None.as_u32();
}

impl AsFormat for String {
    const MPV_FORMAT: u32 = Format::String.as_u32();
}

impl AsFormat for OsdString {
    const MPV_FORMAT: u32 = Format::OsdString.as_u32();
}

impl AsFormat for bool {
    const MPV_FORMAT: u32 = Format::Bool.as_u32();
}

impl AsFormat for i64 {
    const MPV_FORMAT: u32 = Format::Int.as_u32();
}

impl AsFormat for f64 {
    const MPV_FORMAT: u32 = Format::Double.as_u32();
}

impl<S: BuildHasher + Default> AsFormat for Node<S> {
    const MPV_FORMAT: u32 = Format::Node.as_u32();
}

impl<S: BuildHasher + Default> AsFormat for Vec<Node<S>> {
    const MPV_FORMAT: u32 = Format::Node.as_u32();
}

impl<S: BuildHasher + Default> AsFormat for HashMap<String, Node<S>, S> {
    const MPV_FORMAT: u32 = Format::Node.as_u32();
}

impl AsFormat for Vec<u8> {
    const MPV_FORMAT: u32 = Format::Node.as_u32();
}

pub trait Sealed: Sized + Default {
    fn from_ptr(ptr: *const c_void) -> Self;

    /// # Errors
    /// If the FFI callback fails.
    fn to_mpv<F: Fn(*mut c_void) -> crate::Result<()>>(self, fun: F) -> crate::Result<()>;

    /// # Errors
    /// If the FFI callback fails or the stored value cannot be recovered.
    fn from_mpv<F: Fn(*mut c_void) -> crate::Result<()>>(fun: F) -> crate::Result<Self>;
}

impl Sealed for () {
    fn from_ptr(_ptr: *const c_void) -> Self {}

    fn to_mpv<F: Fn(*mut c_void) -> crate::Result<()>>(self, fun: F) -> crate::Result<()> {
        fun(ptr::null_mut())
    }

    fn from_mpv<F: Fn(*mut c_void) -> crate::Result<()>>(fun: F) -> crate::Result<Self> {
        fun(ptr::null_mut())
    }
}

impl Sealed for String {
    fn from_ptr(ptr: *const c_void) -> Self {
        let ptr = ptr.cast::<*const c_char>();
        let string_ptr = unsafe { *ptr };

        if string_ptr.is_null() {
            return Self::default();
        }

        unsafe { CStr::from_ptr(string_ptr) }.to_string_lossy().into_owned()
    }

    fn to_mpv<F: Fn(*mut c_void) -> crate::Result<()>>(self, fun: F) -> crate::Result<()> {
        let cstr = CString::new(self)?;
        let mut ptr = cstr.as_ptr();
        fun((&raw mut ptr).cast::<c_void>())
    }

    /// # Errors
    /// Returns an error if the FFI callback fails or the returned pointer is null/invalid UTF-8.
    fn from_mpv<F: Fn(*mut c_void) -> crate::Result<()>>(fun: F) -> crate::Result<Self> {
        let mut mpv_string_ptr: *mut c_char = ptr::null_mut();
        fun((&raw mut mpv_string_ptr).cast::<c_void>())?;

        if mpv_string_ptr.is_null() {
            return Ok(Self::default());
        }

        let _mpv_string = ClonedMpvString(mpv_string_ptr);
        Ok(unsafe { CStr::from_ptr(mpv_string_ptr) }.to_string_lossy().into_owned())
    }
}

impl Sealed for OsdString {
    fn from_ptr(ptr: *const c_void) -> Self {
        Self(String::from_ptr(ptr))
    }

    fn to_mpv<F: Fn(*mut c_void) -> crate::Result<()>>(self, fun: F) -> crate::Result<()> {
        String::to_mpv(self.0, fun)
    }

    /// # Errors
    /// Returns an error if the FFI callback fails or the returned pointer is null/invalid UTF-8.
    fn from_mpv<F: Fn(*mut c_void) -> crate::Result<()>>(fun: F) -> crate::Result<Self> {
        Ok(Self(String::from_mpv(fun)?))
    }
}

impl Sealed for bool {
    fn from_ptr(ptr: *const c_void) -> Self {
        unsafe { *ptr.cast::<c_int>() != 0 }
    }

    fn to_mpv<F: Fn(*mut c_void) -> crate::Result<()>>(self, fun: F) -> crate::Result<()> {
        let mut data = c_int::from(self);
        fun((&raw mut data).cast::<c_void>())
    }

    fn from_mpv<F: Fn(*mut c_void) -> crate::Result<()>>(fun: F) -> crate::Result<Self> {
        let mut data = c_int::from(Self::default());
        fun((&raw mut data).cast::<c_void>()).map(|()| data != 0)
    }
}

impl Sealed for i64 {
    fn from_ptr(ptr: *const c_void) -> Self {
        unsafe { *ptr.cast() }
    }

    fn to_mpv<F: Fn(*mut c_void) -> crate::Result<()>>(self, fun: F) -> crate::Result<()> {
        let mut data = self;
        fun((&raw mut data).cast::<c_void>())
    }

    fn from_mpv<F: Fn(*mut c_void) -> crate::Result<()>>(fun: F) -> crate::Result<Self> {
        let mut data = Self::default();
        fun((&raw mut data).cast::<c_void>()).map(|()| data)
    }
}

impl Sealed for f64 {
    fn from_ptr(ptr: *const c_void) -> Self {
        unsafe { *ptr.cast() }
    }

    fn to_mpv<F: Fn(*mut c_void) -> crate::Result<()>>(self, fun: F) -> crate::Result<()> {
        let mut data = self;
        fun((&raw mut data).cast::<c_void>())
    }

    fn from_mpv<F: Fn(*mut c_void) -> crate::Result<()>>(fun: F) -> crate::Result<Self> {
        let mut data = Self::default();
        fun((&raw mut data).cast::<c_void>()).map(|()| data)
    }
}

impl<S: BuildHasher + Default> Sealed for Node<S> {
    fn from_ptr(ptr: *const c_void) -> Self {
        let Some(mpv_node) = BorrowedMpvNode::from_ptr(ptr) else {
            return Self::None;
        };

        mpv_node.to_node()
    }

    fn to_mpv<F: Fn(*mut c_void) -> crate::Result<()>>(self, fun: F) -> crate::Result<()> {
        let mut mpv_node = RawMpvNode::from_node(self);
        fun(mpv_node.as_mut_ptr().cast::<c_void>())
    }

    fn from_mpv<F: Fn(*mut c_void) -> crate::Result<()>>(fun: F) -> crate::Result<Self> {
        let mut mpv_node = ClonedMpvNode::default();
        fun(mpv_node.as_mut_ptr().cast())?;
        Ok(mpv_node.to_node())
    }
}

impl<S: BuildHasher + Default> Sealed for Vec<Node<S>> {
    fn from_ptr(ptr: *const c_void) -> Self {
        let Some(mpv_node) = BorrowedMpvNode::from_ptr(ptr) else {
            return Self::default();
        };

        mpv_node.to_node_array()
    }

    fn to_mpv<F: Fn(*mut c_void) -> crate::Result<()>>(self, fun: F) -> crate::Result<()> {
        let mut mpv_node = RawMpvNode::from_node(Node::Array(self));
        fun(mpv_node.as_mut_ptr().cast::<c_void>())
    }

    fn from_mpv<F: Fn(*mut c_void) -> crate::Result<()>>(fun: F) -> crate::Result<Self> {
        let mut mpv_node = ClonedMpvNode::default();
        fun(mpv_node.as_mut_ptr().cast())?;
        Ok(mpv_node.to_node_array())
    }
}

impl<S: BuildHasher + Default> Sealed for HashMap<String, Node<S>, S> {
    fn from_ptr(ptr: *const c_void) -> Self {
        let Some(mpv_node) = BorrowedMpvNode::from_ptr(ptr) else {
            return Self::default();
        };

        mpv_node.to_node_map()
    }

    fn to_mpv<F: Fn(*mut c_void) -> crate::Result<()>>(self, fun: F) -> crate::Result<()> {
        let mut mpv_node = RawMpvNode::from_node(Node::<S>::Map(self));
        fun(mpv_node.as_mut_ptr().cast::<c_void>())
    }

    fn from_mpv<F: Fn(*mut c_void) -> crate::Result<()>>(fun: F) -> crate::Result<Self> {
        let mut mpv_node = ClonedMpvNode::default();
        fun(mpv_node.as_mut_ptr().cast())?;
        Ok(mpv_node.to_node_map())
    }
}

impl Sealed for Vec<u8> {
    fn from_ptr(ptr: *const c_void) -> Self {
        let Some(mpv_node) = BorrowedMpvNode::from_ptr(ptr) else {
            return Self::default();
        };

        mpv_node.to_node_byte_array()
    }

    fn to_mpv<F: Fn(*mut c_void) -> crate::Result<()>>(self, fun: F) -> crate::Result<()> {
        let mut mpv_node = RawMpvNode::from_node(Node::<RandomState>::ByteArray(self));
        fun(mpv_node.as_mut_ptr().cast::<c_void>())
    }

    fn from_mpv<F: Fn(*mut c_void) -> crate::Result<()>>(fun: F) -> crate::Result<Self> {
        let mut mpv_node = ClonedMpvNode::default();
        fun(mpv_node.as_mut_ptr().cast())?;
        Ok(mpv_node.to_node_byte_array())
    }
}

struct ClonedMpvString(*mut c_char);
impl Drop for ClonedMpvString {
    fn drop(&mut self) {
        Handle::free(self.0.cast::<c_void>());
    }
}

#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
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
    Unknown(u32),
}

impl Format {
    pub const fn as_u32(self) -> u32 {
        match self {
            Self::None => mpv_format_MPV_FORMAT_NONE,
            Self::String => mpv_format_MPV_FORMAT_STRING,
            Self::OsdString => mpv_format_MPV_FORMAT_OSD_STRING,
            Self::Bool => mpv_format_MPV_FORMAT_FLAG,
            Self::Int => mpv_format_MPV_FORMAT_INT64,
            Self::Double => mpv_format_MPV_FORMAT_DOUBLE,
            Self::Node => mpv_format_MPV_FORMAT_NODE,
            Self::NodeArray => mpv_format_MPV_FORMAT_NODE_ARRAY,
            Self::NodeMap => mpv_format_MPV_FORMAT_NODE_MAP,
            Self::ByteArray => mpv_format_MPV_FORMAT_BYTE_ARRAY,
            Self::Unknown(value) => value,
        }
    }
}

impl From<u32> for Format {
    fn from(v: u32) -> Self {
        match v {
            v if v == const { mpv_format_MPV_FORMAT_NONE } => Self::None,
            v if v == const { mpv_format_MPV_FORMAT_STRING } => Self::String,
            v if v == const { mpv_format_MPV_FORMAT_OSD_STRING } => Self::OsdString,
            v if v == const { mpv_format_MPV_FORMAT_FLAG } => Self::Bool,
            v if v == const { mpv_format_MPV_FORMAT_INT64 } => Self::Int,
            v if v == const { mpv_format_MPV_FORMAT_DOUBLE } => Self::Double,
            v if v == const { mpv_format_MPV_FORMAT_NODE } => Self::Node,
            v if v == const { mpv_format_MPV_FORMAT_NODE_ARRAY } => Self::NodeArray,
            v if v == const { mpv_format_MPV_FORMAT_NODE_MAP } => Self::NodeMap,
            v if v == const { mpv_format_MPV_FORMAT_BYTE_ARRAY } => Self::ByteArray,
            value => Self::Unknown(value),
        }
    }
}

impl Display for Format {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
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
            Self::Unknown(value) => write!(f, "Unknown: {value}"),
        }
    }
}

#[derive(Debug, Default, Clone, PartialEq, Eq, Hash)]
pub struct OsdString(pub String);

impl Deref for OsdString {
    type Target = str;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for OsdString {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl AsRef<str> for OsdString {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl Borrow<str> for OsdString {
    fn borrow(&self) -> &str {
        &self.0
    }
}

impl Display for OsdString {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        Display::fmt(&self.0, f)
    }
}

impl<T: Into<String>> From<T> for OsdString {
    fn from(value: T) -> Self {
        Self(value.into())
    }
}

impl FromStr for OsdString {
    type Err = convert::Infallible;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(Self(s.to_owned()))
    }
}
