use mpv_client_sys::{
    mpv_byte_array, mpv_format_MPV_FORMAT_BYTE_ARRAY, mpv_format_MPV_FORMAT_DOUBLE, mpv_format_MPV_FORMAT_FLAG,
    mpv_format_MPV_FORMAT_INT64, mpv_format_MPV_FORMAT_NODE_ARRAY, mpv_format_MPV_FORMAT_NODE_MAP,
    mpv_format_MPV_FORMAT_NONE, mpv_format_MPV_FORMAT_STRING, mpv_node, mpv_node__bindgen_ty_1, mpv_node_list,
};
use std::{
    cmp,
    collections::HashMap,
    ffi::{CStr, CString, c_void},
    fmt::Display,
    mem,
    ops::Deref,
    ptr, slice,
};

use crate::{Handle, format::FormatType};

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

// pub struct MpvNodeGuard(*mut mpv_node);

// impl MpvNodeGuard {
//     pub fn new(node: Node) -> Self {
//         Self(<*mut mpv_node>::from(node))
//     }

//     #[must_use]
//     pub const fn as_ptr(&self) -> *mut mpv_node {
//         self.0
//     }
// }

// impl Drop for MpvNodeGuard {
//     fn drop(&mut self) {
//         let ptr = self.0;
//         if ptr.is_null() {
//             return;
//         }

//         unsafe { drop_mpv_node_contents(&mut *ptr) };
//         unsafe { drop(Box::from_raw(ptr)) }
//     }
// }

// pub struct MpvNodeContentsGuard(pub *mut mpv_node);

// impl Drop for MpvNodeContentsGuard {
//     fn drop(&mut self) {
//         Handle::free_node_contents(self.0);
//     }
// }

// pub struct MpvNodeListGuard {
//     values: Vec<mpv_node>,
//     keys: Vec<*mut c_char>,
// }

// impl MpvNodeListGuard {
//     pub fn as_mut_ptr(&mut self) -> *mut mpv_node {
//         let mpv_node_list = Box::into_raw(Box::new(mpv_node_list {
//             num: self.values.len() as i32,
//             values: self.values.as_mut_ptr(),
//             keys: self.keys.as_mut_ptr(),
//         }));

//         Box::into_raw(Box::new(mpv_node {
//             format: mpv_format_MPV_FORMAT_NODE_ARRAY,
//             u: mpv_node__bindgen_ty_1 { list: mpv_node_list },
//         }))
//     }
// }

// impl From<Vec<Node>> for MpvNodeListGuard {
//     fn from(node_array: Vec<Node>) -> Self {
//         let mut guard = Self {
//             values: Vec::with_capacity(node_array.len()),
//             keys: Vec::with_capacity(0),
//         };

//         for v in node_array {
//             let node = unsafe { *Box::from_raw(<*mut mpv_node>::from(v)) };
//             guard.values.push(node);
//         }

//         guard
//     }
// }

// impl From<HashMap<String, Node>> for MpvNodeListGuard {
//     fn from(node_map: HashMap<String, Node>) -> Self {
//         let mut guard = Self {
//             values: Vec::with_capacity(node_map.len()),
//             keys: Vec::with_capacity(node_map.len()),
//         };

//         for (k, v) in node_map {
//             let cstring = CString::new(k.as_str()).expect("CString::new failed");
//             let node = unsafe { *Box::from_raw(<*mut mpv_node>::from(v)) };
//             guard.keys.push(cstring.into_raw());
//             guard.values.push(node);
//         }

//         guard
//     }
// }

// impl Drop for MpvNodeListGuard {
//     fn drop(&mut self) {
//         for child in &mut self.values {
//             drop_mpv_node_contents(child);
//         }

//         for &key in &self.keys {
//             if !key.is_null() {
//                 unsafe { drop(CString::from_raw(key)) };
//             }
//         }
//     }
// }

/// Shared methods
pub trait MpvNode {
    fn as_ref(&self) -> BorrowedMpvNode<'_>;
    fn as_mut_ptr(&mut self) -> *mut mpv_node;
}

/// Cleaned by libmpv.
/// Created by libmpv.
#[repr(transparent)]
#[derive(Clone, Copy)]
pub struct BorrowedMpvNode<'a>(pub &'a mpv_node);

impl<'a> Deref for BorrowedMpvNode<'a> {
    type Target = &'a mpv_node;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<'a> MpvNode for BorrowedMpvNode<'a> {
    fn as_ref(&self) -> BorrowedMpvNode<'a> {
        self.clone()
    }

    fn as_mut_ptr(&mut self) -> *mut mpv_node {
        (&raw const (*self.0)).cast_mut()
    }
}

/// Must be cleaned on drop via [`free_node_contents()`](Handle::free_node_contents()).
/// Created by libmpv.
#[repr(transparent)]
pub struct ClonedMpvNode(pub mpv_node);

impl Default for ClonedMpvNode {
    fn default() -> Self {
        Self(mpv_node {
            format: FormatType::None as u32,
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
    pub fn new(node: Node) -> Self {
        let mut mpv_node = mpv_node {
            format: 0,
            u: mpv_node__bindgen_ty_1 { int64: 0 },
        };

        match node {
            Node::None => {
                mpv_node.format = mpv_format_MPV_FORMAT_NONE;
            }
            Node::String(s) => {
                mpv_node.format = mpv_format_MPV_FORMAT_STRING;
                mpv_node.u.string = CString::new(s).expect("CString::new failed").into_raw();
            }
            Node::Int(int64) => {
                mpv_node.format = mpv_format_MPV_FORMAT_INT64;
                mpv_node.u.int64 = int64;
            }
            Node::Double(f) => {
                mpv_node.format = mpv_format_MPV_FORMAT_DOUBLE;
                mpv_node.u.double_ = f;
            }
            Node::Bool(bool) => {
                mpv_node.format = mpv_format_MPV_FORMAT_FLAG;
                mpv_node.u.flag = i32::from(bool);
            }
            Node::Array(node_array) => {
                mpv_node.format = mpv_format_MPV_FORMAT_NODE_ARRAY;
                mpv_node.u.list = mpv_node_list_from_node_array(node_array);
            }
            Node::Map(node_map) => {
                mpv_node.format = mpv_format_MPV_FORMAT_NODE_MAP;
                mpv_node.u.list = mpv_node_list_from_node_map(node_map);
            }
            Node::ByteArray(vec) => {
                mpv_node.format = mpv_format_MPV_FORMAT_BYTE_ARRAY;
                mpv_node.u.ba = mpv_byte_array_from_byte_array(vec);
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
        fn drop_mpv_node_contents(node: &mut mpv_node) {
            unsafe {
                match node.format {
                    mpv_format_MPV_FORMAT_STRING => {
                        if !node.u.string.is_null() {
                            drop(CString::from_raw(node.u.string));
                        }
                    }
                    mpv_format_MPV_FORMAT_NODE_ARRAY => {
                        if node.u.list.is_null() {
                            return;
                        }

                        let list = Box::from_raw(node.u.list);
                        let len = usize::try_from(list.num).unwrap_or(0);

                        if !list.values.is_null() {
                            for child in slice::from_raw_parts_mut(list.values, len) {
                                drop_mpv_node_contents(child);
                            }

                            drop(Box::from_raw(ptr::slice_from_raw_parts_mut(list.values, len)));
                        }
                    }
                    mpv_format_MPV_FORMAT_NODE_MAP => {
                        if node.u.list.is_null() {
                            return;
                        }

                        let list = Box::from_raw(node.u.list);
                        let len = usize::try_from(list.num).unwrap_or(0);

                        if !list.values.is_null() {
                            for child in slice::from_raw_parts_mut(list.values, len) {
                                drop_mpv_node_contents(child);
                            }

                            drop(Box::from_raw(ptr::slice_from_raw_parts_mut(list.values, len)));
                        }

                        if list.keys.is_null() {
                            return;
                        }

                        for &key in slice::from_raw_parts(list.keys, len) {
                            if !key.is_null() {
                                drop(CString::from_raw(key));
                            }
                        }

                        drop(Box::from_raw(ptr::slice_from_raw_parts_mut(list.keys, len)));
                    }
                    mpv_format_MPV_FORMAT_BYTE_ARRAY => {
                        if node.u.ba.is_null() {
                            return;
                        }

                        let ba = Box::from_raw(node.u.ba);
                        let data = ba.data;
                        if data.is_null() {
                            return;
                        }

                        let byte_array = ptr::slice_from_raw_parts_mut(data.cast::<u8>(), ba.size);
                        drop(Box::from_raw(byte_array));
                    }
                    _ => {}
                }
            }
        }

        let ptr = &raw mut self.0;
        unsafe { drop_mpv_node_contents(&mut *ptr) };
        unsafe { drop(Box::from_raw(ptr)) }
    }
}

impl<T: MpvNode> From<T> for Node {
    fn from(mpv_node_wrapper: T) -> Self {
        let node = mpv_node_wrapper.as_ref();
        unsafe {
            match node.format {
                mpv_format_MPV_FORMAT_STRING => {
                    if node.u.string.is_null() {
                        Self::None
                    } else {
                        Self::String(CStr::from_ptr(node.u.string).to_string_lossy().into_owned())
                    }
                }
                mpv_format_MPV_FORMAT_INT64 => Self::Int(node.u.int64),
                mpv_format_MPV_FORMAT_DOUBLE => Self::Double(node.u.double_),
                mpv_format_MPV_FORMAT_FLAG => Self::Bool(node.u.flag != 0),
                mpv_format_MPV_FORMAT_NODE_ARRAY => {
                    if node.u.list.is_null() {
                        return Self::Array(Vec::new());
                    }

                    let list = &*node.u.list;
                    let len: usize = list.num.try_into().expect("num fits in usize");

                    let values = if len == 0 || list.values.is_null() {
                        &[]
                    } else {
                        slice::from_raw_parts(list.values, len)
                    };

                    Self::Array(
                        values
                            .iter()
                            .map(|raw_node| Self::from(BorrowedMpvNode(raw_node)))
                            .collect(),
                    )
                }
                mpv_format_MPV_FORMAT_NODE_MAP => {
                    if node.u.list.is_null() {
                        return Self::Map(HashMap::new());
                    }

                    let list = &*node.u.list;
                    let len: usize = list.num.try_into().expect("num fits in usize");

                    let values = if len == 0 || list.values.is_null() {
                        &[]
                    } else {
                        slice::from_raw_parts(list.values, len)
                    };

                    let keys = if len == 0 || list.keys.is_null() {
                        &[]
                    } else {
                        slice::from_raw_parts(list.keys, len)
                    };

                    let map = keys
                        .iter()
                        .zip(values.iter())
                        .filter_map(|(&k, v)| {
                            if k.is_null() {
                                None
                            } else {
                                let key = CStr::from_ptr(k).to_string_lossy().into_owned();
                                (key, Self::from(BorrowedMpvNode(v))).into()
                            }
                        })
                        .collect();

                    Self::Map(map)
                }
                mpv_format_MPV_FORMAT_BYTE_ARRAY => {
                    if node.u.ba.is_null() {
                        return Self::ByteArray(Vec::new());
                    }

                    let arr: &mpv_byte_array = &*node.u.ba;

                    let data = if arr.size == 0 || arr.data.is_null() {
                        &[]
                    } else {
                        slice::from_raw_parts(arr.data as *const u8, arr.size)
                    };

                    Self::ByteArray(data.to_vec())
                }
                _ => Self::None,
            }
        }
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
            .map(|node| RawMpvNode::new(node).0)
            .collect::<Vec<mpv_node>>()
            .into_boxed_slice(),
    ) as *mut mpv_node;

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

    let (keys, values): (Vec<*mut i8>, Vec<mpv_node>) = node_map
        .into_iter()
        .take(num as usize)
        .map(|(key, node)| {
            (
                CString::new(key).expect("CString::new() failed").into_raw(),
                RawMpvNode::new(node).0,
            )
        })
        .collect();

    let (keys, values) = (
        Box::into_raw(keys.into_boxed_slice()) as *mut *mut i8,
        Box::into_raw(values.into_boxed_slice()) as *mut mpv_node,
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
