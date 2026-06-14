use mpv_client_sys::{mpv_byte_array, mpv_node, mpv_node__bindgen_ty_1, mpv_node_list};
use std::{
    cmp,
    collections::HashMap,
    ffi::{CStr, CString, c_void},
    fmt::Display,
    ops::Deref,
    ptr, slice,
};

use crate::{Handle, format::Format};

#[derive(Debug, Clone, Default)]
pub enum Node {
    #[default]
    None,
    String(String),
    Int(i64),
    Double(f64),
    Bool(bool),
    ByteArray(Vec<u8>),
    Array(Vec<Self>),
    Map(HashMap<String, Self>),
}

impl Display for Node {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::None => f.write_str("None"),
            Self::String(v) => write!(f, "{v}"),
            Self::Int(v) => write!(f, "{v}"),
            Self::Double(v) => write!(f, "{v}"),
            Self::Bool(v) => write!(f, "{v}"),
            Self::ByteArray(items) => write!(f, "{items:#?}"),
            Self::Array(nodes) => write!(f, "{nodes:#?}"),
            Self::Map(hash_map) => write!(f, "{hash_map:#?}"),
        }
    }
}

/// Shared methods
pub trait MpvNode: Sized {
    fn as_ref(&self) -> BorrowedMpvNode<'_>;
    fn as_mut_ptr(&mut self) -> *mut mpv_node;
    fn to_node(self) -> crate::Result<Node> {
        let mpv_node = self.as_ref();
        let node = match Format::from(mpv_node.format) {
            Format::String => {
                let string = unsafe { mpv_node.u.string };
                if string.is_null() {
                    Node::None
                } else {
                    Node::String(unsafe { CStr::from_ptr(string) }.to_string_lossy().into_owned())
                }
            }
            Format::Int => Node::Int(unsafe { mpv_node.u.int64 }),
            Format::Double => Node::Double(unsafe { mpv_node.u.double_ }),
            Format::Bool => Node::Bool(unsafe { mpv_node.u.flag } != 0),
            Format::NodeArray => {
                let list = unsafe { mpv_node.u.list };
                if list.is_null() {
                    return Ok(Node::Array(Vec::default()));
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
                        .collect::<crate::Result<Vec<Node>>>()?,
                )
            }
            Format::NodeMap => {
                let list = unsafe { mpv_node.u.list };
                if list.is_null() {
                    return Ok(Node::Map(HashMap::default()));
                }

                let list = unsafe { &*mpv_node.u.list };
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
                            Some(BorrowedMpvNode(v).to_node().map(|node| (key, node)))
                        }
                    })
                    .collect::<crate::Result<HashMap<String, Node>>>()?;

                Node::Map(map)
            }
            Format::ByteArray => {
                let ba = unsafe { mpv_node.u.ba };
                if ba.is_null() {
                    return Ok(Node::ByteArray(Vec::default()));
                }

                let ba = unsafe { &*mpv_node.u.ba };
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
        };

        Ok(node)
    }

    fn to_node_array(self) -> crate::Result<Vec<Node>> {
        let mpv_node = self.as_ref();
        let list = unsafe { mpv_node.u.list };
        if list.is_null() {
            return Ok(Vec::default());
        }

        let list = unsafe { &*list };
        let num = list.num;
        let values = list.values;
        if num <= 0 || values.is_null() {
            return Ok(Vec::default());
        }

        #[allow(clippy::cast_sign_loss)]
        let num = num as usize;
        let values = unsafe { slice::from_raw_parts(values, num) };

        values
            .iter()
            .map(|mpv_node| BorrowedMpvNode(mpv_node).to_node())
            .collect::<crate::Result<Vec<Node>>>()
    }

    fn to_node_map(self) -> crate::Result<HashMap<String, Node>> {
        let mpv_node = self.as_ref();
        let list = unsafe { mpv_node.u.list };
        if list.is_null() {
            return Ok(HashMap::default());
        }

        let list = unsafe { &*list };
        let num = list.num;
        let keys = list.keys;
        let values = list.values;
        if num <= 0 || values.is_null() || keys.is_null() {
            return Ok(HashMap::default());
        }

        #[allow(clippy::cast_sign_loss)]
        let num = num as usize;
        let keys = unsafe { slice::from_raw_parts(keys, num) };
        let values = unsafe { slice::from_raw_parts(values, num) };

        let mut node_map = HashMap::with_capacity(num);
        for (key, value) in keys.iter().zip(values) {
            if key.is_null() {
                return Ok(HashMap::default());
            }

            let key = unsafe { CStr::from_ptr(*key) }.to_string_lossy().into_owned();
            let value = BorrowedMpvNode(value).to_node()?;
            node_map.insert(key, value);
        }

        Ok(node_map)
    }

    fn to_node_byte_array(self) -> Vec<u8> {
        let mpv_node = self.as_ref();
        let ba = unsafe { mpv_node.u.ba };
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

/// Cleaned by libmpv.
/// Created by libmpv.
#[repr(transparent)]
#[derive(Clone, Copy)]
pub struct BorrowedMpvNode<'a>(&'a mpv_node);

impl BorrowedMpvNode<'_> {
    pub const fn from_ptr(ptr: *const c_void) -> Option<Self> {
        if ptr.is_null() {
            None
        } else {
            Some(Self(unsafe { &*ptr.cast::<mpv_node>() }))
        }
    }
}

impl<'a> Deref for BorrowedMpvNode<'a> {
    type Target = &'a mpv_node;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl MpvNode for BorrowedMpvNode<'_> {
    fn as_ref(&self) -> Self {
        *self
    }

    fn as_mut_ptr(&mut self) -> *mut mpv_node {
        (&raw const (*self.0)).cast_mut()
    }
}

/// Must be cleaned on drop via [`free_node_contents()`](Handle::free_node_contents()).
/// Created by libmpv.
#[repr(transparent)]
pub struct ClonedMpvNode(mpv_node);

impl Default for ClonedMpvNode {
    fn default() -> Self {
        Self(mpv_node {
            format: Format::None.as_u32(),
            u: mpv_node__bindgen_ty_1 { int64: 0 },
        })
    }
}

impl MpvNode for ClonedMpvNode {
    fn as_ref(&self) -> BorrowedMpvNode<'_> {
        BorrowedMpvNode(&self.0)
    }

    fn as_mut_ptr(&mut self) -> *mut mpv_node {
        &raw mut self.0
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
pub struct RawMpvNode(mpv_node);

impl RawMpvNode {
    pub fn from_node(node: Node) -> Self {
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

impl MpvNode for RawMpvNode {
    fn as_ref(&self) -> BorrowedMpvNode<'_> {
        BorrowedMpvNode(&self.0)
    }

    fn as_mut_ptr(&mut self) -> *mut mpv_node {
        &raw mut self.0
    }
}

impl Drop for RawMpvNode {
    fn drop(&mut self) {
        fn drop_mpv_node(mpv_node: &mut mpv_node) {
            unsafe {
                match Format::from(mpv_node.format) {
                    Format::String => {
                        if mpv_node.u.string.is_null() {
                            return;
                        }
                        drop(CString::from_raw(mpv_node.u.string));
                    }
                    Format::NodeArray | Format::NodeMap => {
                        let list = mpv_node.u.list;
                        if list.is_null() {
                            return;
                        }

                        let list = Box::from_raw(list);
                        let len = usize::try_from(list.num).unwrap_or(0);

                        let keys = list.keys;
                        if !keys.is_null() {
                            for &key in slice::from_raw_parts(keys, len) {
                                if !key.is_null() {
                                    drop(CString::from_raw(key));
                                }
                            }
                            drop(Box::from_raw(ptr::slice_from_raw_parts_mut(keys, len)));
                        }

                        let values = list.values;
                        if !values.is_null() {
                            for mpv_node_child in slice::from_raw_parts_mut(values, len) {
                                drop_mpv_node(mpv_node_child);
                            }
                            drop(Box::from_raw(ptr::slice_from_raw_parts_mut(values, len)));
                        }
                    }
                    Format::ByteArray => {
                        let ba = mpv_node.u.ba;
                        if ba.is_null() {
                            return;
                        }

                        let ba = Box::from_raw(mpv_node.u.ba);
                        let data = ba.data;
                        if data.is_null() {
                            return;
                        }

                        drop(Box::from_raw(ptr::slice_from_raw_parts_mut(data.cast::<u8>(), ba.size)));
                    }
                    _ => {}
                }
            }
        }

        drop_mpv_node(&mut self.0);
    }
}

fn mpv_node_list_from_node_array(node_array: Vec<Node>) -> *mut mpv_node_list {
    let num = cmp::min(node_array.len(), i32::MAX as usize) as i32;
    let keys = ptr::null_mut();
    if num == 0 {
        return Box::into_raw(Box::new(mpv_node_list {
            num,
            values: ptr::null_mut(),
            keys,
        }));
    }

    let values = Box::into_raw(
        node_array
            .into_iter()
            .take(num as usize)
            .map(RawMpvNode::from_node)
            .collect::<Vec<RawMpvNode>>()
            .into_boxed_slice(),
    )
    .cast();

    Box::into_raw(Box::new(mpv_node_list { num, keys, values }))
}

fn mpv_node_list_from_node_map(node_map: HashMap<String, Node>) -> *mut mpv_node_list {
    let num = cmp::min(node_map.len(), i32::MAX as usize) as i32;
    if num == 0 {
        return Box::into_raw(Box::new(mpv_node_list {
            num,
            values: ptr::null_mut(),
            keys: ptr::null_mut(),
        }));
    }

    let (keys, values): (Vec<*mut i8>, Vec<RawMpvNode>) = node_map
        .into_iter()
        .take(num as usize)
        .map(|(key, node)| {
            (
                CString::new(key).expect("CString::new() failed").into_raw(),
                RawMpvNode::from_node(node),
            )
        })
        .collect();

    let (keys, values) = (
        Box::into_raw(keys.into_boxed_slice()).cast(),
        Box::into_raw(values.into_boxed_slice()).cast(),
    );

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
