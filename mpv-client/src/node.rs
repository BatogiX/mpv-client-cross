use core::fmt;
use mpv_client_sys::{mpv_byte_array, mpv_node, mpv_node__bindgen_ty_1, mpv_node_list};
use std::{
    cmp,
    collections::HashMap,
    ffi::{CStr, CString, c_void},
    fmt::Display,
    hash::{BuildHasher, RandomState},
    ops::Deref,
    ptr, slice,
};

use crate::{Handle, format::Format};

#[derive(Debug, Clone, Default)]
pub enum Node<S = RandomState> {
    #[default]
    None,
    String(String),
    Int(i64),
    Double(f64),
    Bool(bool),
    ByteArray(Vec<u8>),
    Array(Vec<Self>),
    Map(HashMap<String, Self, S>),
}

impl Node<RandomState> {
    pub fn string(s: impl Into<String>) -> Self {
        Self::String(s.into())
    }

    #[must_use]
    pub const fn int(i: i64) -> Self {
        Self::Int(i)
    }

    #[must_use]
    pub const fn double(d: f64) -> Self {
        Self::Double(d)
    }

    #[must_use]
    pub const fn bool(b: bool) -> Self {
        Self::Bool(b)
    }

    pub fn byte_array(ba: impl Into<Vec<u8>>) -> Self {
        Self::ByteArray(ba.into())
    }

    pub fn array(a: impl Into<Vec<Self>>) -> Self {
        Self::Array(a.into())
    }

    pub fn map(m: impl Into<HashMap<String, Self>>) -> Self {
        Self::Map(m.into())
    }
}

impl Display for Node {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fn fmt_join<T, F>(f: &mut fmt::Formatter<'_>, iter: impl IntoIterator<Item = T>, mut format: F) -> fmt::Result
        where
            F: FnMut(T, &mut fmt::Formatter<'_>) -> fmt::Result,
        {
            let mut iter = iter.into_iter();
            if let Some(first) = iter.next() {
                format(first, f)?;
                for item in iter {
                    f.write_str(", ")?;
                    format(item, f)?;
                }
            }
            Ok(())
        }

        match self {
            Self::None => f.write_str("None"),
            Self::String(v) => write!(f, "\"{v}\""),
            Self::Int(v) => write!(f, "{v}"),
            Self::Double(v) => write!(f, "{v}"),
            Self::Bool(v) => write!(f, "{v}"),
            Self::ByteArray(bytes) => {
                f.write_str("[")?;
                fmt_join(f, bytes, |b, f| write!(f, "{b}"))?;
                f.write_str("]")
            }
            Self::Array(nodes) => {
                f.write_str("[")?;
                fmt_join(f, nodes, |n, f| write!(f, "{n}"))?;
                f.write_str("]")
            }
            Self::Map(hash_map) => {
                f.write_str("{")?;
                fmt_join(f, hash_map, |(k, v), f| write!(f, "\"{k}\": {v}"))?;
                f.write_str("}")
            }
        }
    }
}

/// Cleaned by libmpv.
/// Created by libmpv.
#[repr(transparent)]
#[derive(Clone, Copy)]
pub struct BorrowedMpvNode<'a>(pub &'a mpv_node);

impl BorrowedMpvNode<'_> {
    pub const fn from_ptr(ptr: *const c_void) -> Option<Self> {
        if ptr.is_null() {
            None
        } else {
            Some(Self(unsafe { &*ptr.cast::<mpv_node>() }))
        }
    }

    pub fn to_node<S: BuildHasher + Default>(self) -> Node<S> {
        match Format::from_u32(self.format) {
            Format::String => {
                let string = unsafe { self.u.string };
                if string.is_null() {
                    Node::None
                } else {
                    Node::String(unsafe { CStr::from_ptr(string) }.to_string_lossy().into_owned())
                }
            }
            Format::Int => Node::Int(unsafe { self.u.int64 }),
            Format::Double => Node::Double(unsafe { self.u.double_ }),
            Format::Bool => Node::Bool(unsafe { self.u.flag } != 0),
            Format::NodeArray => {
                let list = unsafe { self.u.list };
                if list.is_null() {
                    return Node::Array(Vec::default());
                }

                let list = unsafe { &*list };
                let len: usize = list.num.try_into().expect("num fits in usize");

                let values = list.values;
                let values = if len == 0 || values.is_null() {
                    &[]
                } else {
                    unsafe { slice::from_raw_parts(values, len) }
                };

                Node::Array(
                    values
                        .iter()
                        .map(|raw_node| BorrowedMpvNode(raw_node).to_node())
                        .collect(),
                )
            }
            Format::NodeMap => {
                let list = unsafe { self.u.list };
                if list.is_null() {
                    return Node::Map(HashMap::default());
                }

                let list = unsafe { &*self.u.list };
                let len: usize = list.num.try_into().expect("num fits in usize");

                let values = list.values;
                let values = if len == 0 || values.is_null() {
                    &[]
                } else {
                    unsafe { slice::from_raw_parts(values, len) }
                };

                let keys = list.keys;
                let keys = if len == 0 || keys.is_null() {
                    &[]
                } else {
                    unsafe { slice::from_raw_parts(keys, len) }
                };

                let map = keys
                    .iter()
                    .zip(values.iter())
                    .filter_map(|(&k, v)| {
                        if k.is_null() {
                            None
                        } else {
                            let key = unsafe { CStr::from_ptr(k) }.to_string_lossy().into_owned();
                            let node = BorrowedMpvNode(v).to_node();
                            Some((key, node))
                        }
                    })
                    .collect();

                Node::Map(map)
            }
            Format::ByteArray => {
                let ba = unsafe { self.u.ba };
                if ba.is_null() {
                    return Node::ByteArray(Vec::default());
                }

                let ba = unsafe { &*self.u.ba };
                let size = ba.size;

                let data = ba.data;
                let data = if size == 0 || data.is_null() {
                    &[]
                } else {
                    unsafe { slice::from_raw_parts(data.cast(), size) }
                };

                Node::ByteArray(data.to_vec())
            }
            _ => Node::None,
        }
    }

    pub fn to_node_array<S: BuildHasher + Default>(self) -> Vec<Node<S>> {
        let list = unsafe { self.u.list };
        if list.is_null() {
            return Vec::default();
        }

        let list = unsafe { &*list };
        let num = list.num;
        let values = list.values;
        if num <= 0 || values.is_null() {
            return Vec::default();
        }

        #[allow(clippy::cast_sign_loss)]
        let num = num as usize;
        let values = unsafe { slice::from_raw_parts(values, num) };

        values
            .iter()
            .map(|mpv_node| BorrowedMpvNode(mpv_node).to_node())
            .collect()
    }

    pub fn to_node_map<S: BuildHasher + Default>(self) -> HashMap<String, Node<S>, S> {
        let list = unsafe { self.u.list };
        if list.is_null() {
            return HashMap::default();
        }

        let list = unsafe { &*list };
        let num = list.num;
        let keys = list.keys;
        let values = list.values;
        if num <= 0 || values.is_null() || keys.is_null() {
            return HashMap::default();
        }

        #[allow(clippy::cast_sign_loss)]
        let num = num as usize;
        let keys = unsafe { slice::from_raw_parts(keys, num) };
        let values = unsafe { slice::from_raw_parts(values, num) };

        let mut node_map = HashMap::with_capacity_and_hasher(num, S::default());
        for (key, value) in keys.iter().zip(values) {
            if key.is_null() {
                return HashMap::default();
            }

            let key = unsafe { CStr::from_ptr(*key) }.to_string_lossy().into_owned();
            let value = BorrowedMpvNode(value).to_node();
            node_map.insert(key, value);
        }

        node_map
    }

    pub fn to_node_byte_array(self) -> Vec<u8> {
        let ba = unsafe { self.u.ba };
        if ba.is_null() {
            return Vec::default();
        }

        let ba = unsafe { &*ba };
        let data = ba.data;
        if data.is_null() {
            return Vec::default();
        }

        let size = ba.size;
        let data = data.cast::<u8>();

        unsafe { slice::from_raw_parts(data, size).to_vec() }
    }
}

impl Deref for BorrowedMpvNode<'_> {
    type Target = mpv_node;

    fn deref(&self) -> &Self::Target {
        self.0
    }
}

/// Must be cleaned on drop via [`free_node_contents()`](Handle::free_node_contents()).
/// Created by libmpv.
#[repr(transparent)]
pub struct ClonedMpvNode(mpv_node);

impl ClonedMpvNode {
    pub const fn as_mut_ptr(&mut self) -> *mut mpv_node {
        &raw mut self.0
    }

    pub const fn as_ref(&self) -> BorrowedMpvNode<'_> {
        BorrowedMpvNode(&self.0)
    }
}

impl Default for ClonedMpvNode {
    fn default() -> Self {
        Self(mpv_node {
            format: Format::None.as_u32(),
            u: mpv_node__bindgen_ty_1 { int64: 0 },
        })
    }
}

impl Drop for ClonedMpvNode {
    fn drop(&mut self) {
        Handle::free_node_contents(&raw mut self.0);
    }
}

/// Must be cleaned on drop manually.
/// Created manually
#[repr(transparent)]
pub struct OwnedMpvNode(mpv_node);

impl OwnedMpvNode {
    pub const fn as_mut_ptr(&mut self) -> *mut mpv_node {
        &raw mut self.0
    }

    pub const fn as_ref(&self) -> BorrowedMpvNode<'_> {
        BorrowedMpvNode(&self.0)
    }

    pub fn from_node<S: BuildHasher + Default>(node: Node<S>) -> Self {
        let mut mpv_node = mpv_node {
            format: 0,
            u: mpv_node__bindgen_ty_1 { int64: 0 },
        };

        match node {
            Node::None => {
                mpv_node.format = Format::None.as_u32();
            }
            Node::String(string) => {
                mpv_node.format = Format::String.as_u32();
                mpv_node.u.string = CString::new(string).expect("CString::new failed").into_raw();
            }
            Node::Int(int64) => {
                mpv_node.format = Format::Int.as_u32();
                mpv_node.u.int64 = int64;
            }
            Node::Double(float64) => {
                mpv_node.format = Format::Double.as_u32();
                mpv_node.u.double_ = float64;
            }
            Node::Bool(bool) => {
                mpv_node.format = Format::Bool.as_u32();
                mpv_node.u.flag = i32::from(bool);
            }
            Node::Array(node_array) => {
                mpv_node.format = Format::NodeArray.as_u32();
                mpv_node.u.list = mpv_node_list_from_node_array(node_array);
            }
            Node::Map(node_map) => {
                mpv_node.format = Format::NodeMap.as_u32();
                mpv_node.u.list = mpv_node_list_from_node_map(node_map);
            }
            Node::ByteArray(byte_array) => {
                mpv_node.format = Format::ByteArray.as_u32();
                mpv_node.u.ba = mpv_byte_array_from_byte_array(byte_array);
            }
        }

        Self(mpv_node)
    }
}

impl Default for OwnedMpvNode {
    fn default() -> Self {
        Self(mpv_node {
            format: Format::None.as_u32(),
            u: mpv_node__bindgen_ty_1 { int64: 0 },
        })
    }
}

impl Drop for OwnedMpvNode {
    fn drop(&mut self) {
        unsafe {
            match Format::from_u32(self.0.format) {
                Format::String => {
                    let string_ptr = self.0.u.string;
                    if !string_ptr.is_null() {
                        drop(CString::from_raw(string_ptr));
                    }
                }
                Format::NodeArray | Format::NodeMap => {
                    let list_ptr = self.0.u.list;
                    if !list_ptr.is_null() {
                        drop(OwnedMpvNodeList::from_raw(list_ptr));
                    }
                }
                Format::ByteArray => {
                    let ba_ptr = self.0.u.ba;
                    if !ba_ptr.is_null() {
                        drop(OwnedMpvNodeByteArray::from_raw(ba_ptr));
                    }
                }
                _ => {}
            }
        }
    }
}

fn mpv_node_list_from_node_array<S: BuildHasher + Default>(node_array: Vec<Node<S>>) -> *mut mpv_node_list {
    let len = node_array.len().min(i32::MAX as usize);
    let keys = ptr::null_mut();
    if len == 0 {
        return Box::into_raw(Box::new(mpv_node_list {
            num: 0,
            values: ptr::null_mut(),
            keys,
        }));
    }

    let values = Box::into_raw(
        node_array
            .into_iter()
            .take(len)
            .map(OwnedMpvNode::from_node)
            .collect::<Vec<OwnedMpvNode>>()
            .into_boxed_slice(),
    )
    .cast();

    let num = i32::try_from(len).unwrap_or(i32::MAX);
    Box::into_raw(Box::new(mpv_node_list { num, keys, values }))
}

fn mpv_node_list_from_node_map<S: BuildHasher + Default>(node_map: HashMap<String, Node<S>, S>) -> *mut mpv_node_list {
    let len = node_map.len().min(i32::MAX as usize);
    if len == 0 {
        return Box::into_raw(Box::new(mpv_node_list {
            num: 0,
            values: ptr::null_mut(),
            keys: ptr::null_mut(),
        }));
    }

    let (keys, values): (Vec<*mut i8>, Vec<OwnedMpvNode>) = node_map
        .into_iter()
        .take(len)
        .map(|(key, node)| {
            (
                CString::new(key).expect("CString::new() failed").into_raw(),
                OwnedMpvNode::from_node(node),
            )
        })
        .collect();

    let (keys, values) = (
        Box::into_raw(keys.into_boxed_slice()).cast(),
        Box::into_raw(values.into_boxed_slice()).cast(),
    );

    let num = i32::try_from(len).unwrap_or(i32::MAX);
    Box::into_raw(Box::new(mpv_node_list { num, keys, values }))
}

fn mpv_byte_array_from_byte_array(byte_array: Vec<u8>) -> *mut mpv_byte_array {
    let size = byte_array.len();
    let data = if byte_array.is_empty() {
        ptr::null_mut()
    } else {
        Box::into_raw(byte_array.into_boxed_slice()).cast()
    };

    Box::into_raw(Box::new(mpv_byte_array { data, size }))
}

#[repr(transparent)]
struct OwnedMpvNodeList(*mut mpv_node_list);

impl OwnedMpvNodeList {
    const unsafe fn from_raw(ptr: *mut mpv_node_list) -> Self {
        Self(ptr)
    }
}

impl Drop for OwnedMpvNodeList {
    fn drop(&mut self) {
        unsafe {
            let list = Box::from_raw(self.0);
            let len = list.num as usize;

            if !list.keys.is_null() {
                let keys_slice = ptr::slice_from_raw_parts_mut(list.keys, len);
                let keys_box = Box::from_raw(keys_slice);

                for &key in &keys_box {
                    if !key.is_null() {
                        drop(CString::from_raw(key));
                    }
                }
            }

            if !list.values.is_null() {
                let values_slice = ptr::slice_from_raw_parts_mut(list.values.cast::<OwnedMpvNode>(), len);
                let _values_box = Box::from_raw(values_slice);
            }
        }
    }
}

struct OwnedMpvNodeByteArray(*mut mpv_byte_array);

impl OwnedMpvNodeByteArray {
    const unsafe fn from_raw(ptr: *mut mpv_byte_array) -> Self {
        Self(ptr)
    }
}

impl Drop for OwnedMpvNodeByteArray {
    fn drop(&mut self) {
        unsafe {
            let ba = Box::from_raw(self.0);
            if !ba.data.is_null() {
                let data_slice = ptr::slice_from_raw_parts_mut(ba.data.cast::<u8>(), ba.size);
                drop(Box::from_raw(data_slice));
            }
        }
    }
}
