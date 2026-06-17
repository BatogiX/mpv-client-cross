use crate::{
    Handle,
    node::{MpvNodeCloned, MpvNodeOwned, MpvNodeRef, Node},
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
pub trait AsFormat<S: BuildHasher + Default = RandomState>: Sealed<S> {
    const FORMAT: Format;
}

impl AsFormat for () {
    const FORMAT: Format = Format::None;
}

impl AsFormat for String {
    const FORMAT: Format = Format::String;
}

impl AsFormat for OsdString {
    const FORMAT: Format = Format::OsdString;
}

impl AsFormat for bool {
    const FORMAT: Format = Format::Bool;
}

impl AsFormat for i64 {
    const FORMAT: Format = Format::Int;
}

impl AsFormat for f64 {
    const FORMAT: Format = Format::Double;
}

impl<S: BuildHasher + Default> AsFormat<S> for Node<S> {
    const FORMAT: Format = Format::Node;
}

impl<S: BuildHasher + Default> AsFormat<S> for Vec<Node<S>> {
    const FORMAT: Format = Format::Node;
}

impl<S: BuildHasher + Default> AsFormat<S> for HashMap<String, Node<S>, S> {
    const FORMAT: Format = Format::Node;
}

impl AsFormat for Vec<u8> {
    const FORMAT: Format = Format::Node;
}

pub trait Sealed<S: BuildHasher + Default = RandomState>: Sized + Default {
    fn from_ptr(ptr: *const c_void) -> Self;

    /// # Errors
    /// If the FFI callback fails.
    fn to_mpv<F: Fn(*mut c_void) -> crate::Result<()>>(self, fun: F) -> crate::Result<()>;

    /// # Errors
    /// If the FFI callback fails or the stored value cannot be recovered.
    fn from_mpv<F: Fn(*mut c_void) -> crate::Result<()>>(fun: F) -> crate::Result<Self>;

    fn from_node(node: Node<S>) -> crate::Result<Self>;
}

impl Sealed for () {
    fn from_ptr(_ptr: *const c_void) -> Self {}

    fn to_mpv<F: Fn(*mut c_void) -> crate::Result<()>>(self, fun: F) -> crate::Result<()> {
        fun(ptr::null_mut())
    }

    fn from_mpv<F: Fn(*mut c_void) -> crate::Result<()>>(fun: F) -> crate::Result<Self> {
        fun(ptr::null_mut())
    }

    fn from_node(_node: Node) -> crate::Result<Self> {
        Ok(())
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

    fn from_node(node: Node) -> crate::Result<Self> {
        if let Node::String(string) = node {
            Ok(string)
        } else {
            Err(crate::Error::FormatMismatch(Format::from_node(&node)))
        }
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

    fn from_node(node: Node) -> crate::Result<Self> {
        Ok(Self(String::from_node(node)?))
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

    fn from_node(node: Node) -> crate::Result<Self> {
        if let Node::Bool(boolean) = node {
            Ok(boolean)
        } else {
            Err(crate::Error::FormatMismatch(Format::from_node(&node)))
        }
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

    fn from_node(node: Node) -> crate::Result<Self> {
        if let Node::Int(int64) = node {
            Ok(int64)
        } else {
            Err(crate::Error::FormatMismatch(Format::from_node(&node)))
        }
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

    fn from_node(node: Node) -> crate::Result<Self> {
        if let Node::Double(float64) = node {
            Ok(float64)
        } else {
            Err(crate::Error::FormatMismatch(Format::from_node(&node)))
        }
    }
}

impl<S: BuildHasher + Default> Sealed<S> for Node<S> {
    fn from_ptr(ptr: *const c_void) -> Self {
        let Some(mpv_node) = MpvNodeRef::from_ptr(ptr) else {
            return Self::None;
        };

        mpv_node.to_node()
    }

    fn to_mpv<F: Fn(*mut c_void) -> crate::Result<()>>(self, fun: F) -> crate::Result<()> {
        let mut mpv_node = MpvNodeOwned::from_node(self);
        fun(mpv_node.as_mut_ptr().cast::<c_void>())
    }

    fn from_mpv<F: Fn(*mut c_void) -> crate::Result<()>>(fun: F) -> crate::Result<Self> {
        let mut mpv_node = MpvNodeCloned::default();
        fun(mpv_node.as_mut_ptr().cast())?;
        Ok(mpv_node.as_ref().to_node())
    }

    fn from_node(node: Self) -> crate::Result<Self> {
        Ok(node)
    }
}

impl<S: BuildHasher + Default> Sealed<S> for Vec<Node<S>> {
    fn from_ptr(ptr: *const c_void) -> Self {
        let Some(mpv_node) = MpvNodeRef::from_ptr(ptr) else {
            return Self::default();
        };

        mpv_node.to_node_array()
    }

    fn to_mpv<F: Fn(*mut c_void) -> crate::Result<()>>(self, fun: F) -> crate::Result<()> {
        let mut mpv_node = MpvNodeOwned::from_node(Node::Array(self));
        fun(mpv_node.as_mut_ptr().cast::<c_void>())
    }

    fn from_mpv<F: Fn(*mut c_void) -> crate::Result<()>>(fun: F) -> crate::Result<Self> {
        let mut mpv_node = MpvNodeCloned::default();
        fun(mpv_node.as_mut_ptr().cast())?;
        Ok(mpv_node.as_ref().to_node_array())
    }

    fn from_node(node: Node<S>) -> crate::Result<Self> {
        if let Node::Array(array) = node {
            Ok(array)
        } else {
            Err(crate::Error::FormatMismatch(Format::from_node(&node)))
        }
    }
}

impl<S: BuildHasher + Default> Sealed<S> for HashMap<String, Node<S>, S> {
    fn from_ptr(ptr: *const c_void) -> Self {
        let Some(mpv_node) = MpvNodeRef::from_ptr(ptr) else {
            return Self::default();
        };

        mpv_node.to_node_map()
    }

    fn to_mpv<F: Fn(*mut c_void) -> crate::Result<()>>(self, fun: F) -> crate::Result<()> {
        let mut mpv_node = MpvNodeOwned::from_node(Node::<S>::Map(self));
        fun(mpv_node.as_mut_ptr().cast::<c_void>())
    }

    fn from_mpv<F: Fn(*mut c_void) -> crate::Result<()>>(fun: F) -> crate::Result<Self> {
        let mut mpv_node = MpvNodeCloned::default();
        fun(mpv_node.as_mut_ptr().cast())?;
        Ok(mpv_node.as_ref().to_node_map())
    }

    fn from_node(node: Node<S>) -> crate::Result<Self> {
        if let Node::Map(map) = node {
            Ok(map)
        } else {
            Err(crate::Error::FormatMismatch(Format::from_node(&node)))
        }
    }
}

impl Sealed for Vec<u8> {
    fn from_ptr(ptr: *const c_void) -> Self {
        let Some(mpv_node) = MpvNodeRef::from_ptr(ptr) else {
            return Self::default();
        };

        mpv_node.to_node_byte_array()
    }

    fn to_mpv<F: Fn(*mut c_void) -> crate::Result<()>>(self, fun: F) -> crate::Result<()> {
        let mut mpv_node = MpvNodeOwned::from_node(Node::<RandomState>::Bytes(self));
        fun(mpv_node.as_mut_ptr().cast::<c_void>())
    }

    fn from_mpv<F: Fn(*mut c_void) -> crate::Result<()>>(fun: F) -> crate::Result<Self> {
        let mut mpv_node = MpvNodeCloned::default();
        fun(mpv_node.as_mut_ptr().cast())?;
        Ok(mpv_node.as_ref().to_node_byte_array())
    }

    fn from_node(node: Node) -> crate::Result<Self> {
        if let Node::Bytes(bytes) = node {
            Ok(bytes)
        } else {
            Err(crate::Error::FormatMismatch(Format::from_node(&node)))
        }
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
    Array = mpv_format_MPV_FORMAT_NODE_ARRAY,
    Map = mpv_format_MPV_FORMAT_NODE_MAP,
    Bytes = mpv_format_MPV_FORMAT_BYTE_ARRAY,
    Unknown(u32),
}

impl Format {
    #[must_use]
    pub const fn as_u32(self) -> u32 {
        match self {
            Self::None => mpv_format_MPV_FORMAT_NONE,
            Self::String => mpv_format_MPV_FORMAT_STRING,
            Self::OsdString => mpv_format_MPV_FORMAT_OSD_STRING,
            Self::Bool => mpv_format_MPV_FORMAT_FLAG,
            Self::Int => mpv_format_MPV_FORMAT_INT64,
            Self::Double => mpv_format_MPV_FORMAT_DOUBLE,
            Self::Node => mpv_format_MPV_FORMAT_NODE,
            Self::Array => mpv_format_MPV_FORMAT_NODE_ARRAY,
            Self::Map => mpv_format_MPV_FORMAT_NODE_MAP,
            Self::Bytes => mpv_format_MPV_FORMAT_BYTE_ARRAY,
            Self::Unknown(value) => value,
        }
    }

    #[must_use]
    pub const fn from_u32(value: u32) -> Self {
        match value {
            mpv_format_MPV_FORMAT_NONE => Self::None,
            mpv_format_MPV_FORMAT_STRING => Self::String,
            mpv_format_MPV_FORMAT_OSD_STRING => Self::OsdString,
            mpv_format_MPV_FORMAT_FLAG => Self::Bool,
            mpv_format_MPV_FORMAT_INT64 => Self::Int,
            mpv_format_MPV_FORMAT_DOUBLE => Self::Double,
            mpv_format_MPV_FORMAT_NODE => Self::Node,
            mpv_format_MPV_FORMAT_NODE_ARRAY => Self::Array,
            mpv_format_MPV_FORMAT_NODE_MAP => Self::Map,
            mpv_format_MPV_FORMAT_BYTE_ARRAY => Self::Bytes,
            value => Self::Unknown(value),
        }
    }

    #[must_use]
    pub const fn from_node<S: BuildHasher + Default>(node: &Node<S>) -> Self {
        match node {
            Node::None => Self::None,
            Node::String(_) => Self::String,
            Node::Int(_) => Self::Int,
            Node::Double(_) => Self::Double,
            Node::Bool(_) => Self::Bool,
            Node::Bytes(_) => Self::Bytes,
            Node::Array(_) => Self::Array,
            Node::Map(_) => Self::Map,
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
            Self::Array => f.write_str("Array"),
            Self::Map => f.write_str("Map"),
            Self::Bytes => f.write_str("Bytes"),
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
