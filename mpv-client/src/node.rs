use mpv_client_sys::{
    mpv_byte_array, mpv_format_MPV_FORMAT_BYTE_ARRAY, mpv_format_MPV_FORMAT_DOUBLE, mpv_format_MPV_FORMAT_FLAG,
    mpv_format_MPV_FORMAT_INT64, mpv_format_MPV_FORMAT_NODE_ARRAY, mpv_format_MPV_FORMAT_NODE_MAP,
    mpv_format_MPV_FORMAT_NONE, mpv_format_MPV_FORMAT_STRING, mpv_node, mpv_node__bindgen_ty_1, mpv_node_list,
};
use std::{
    collections::HashMap,
    ffi::{CStr, CString, c_char, c_void},
    ptr, slice,
};

use crate::Handle;

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

impl From<&mpv_node> for Node {
    /// # Panics
    /// If the mpv node contains a negative count (invalid for the C API).
    fn from(node: &mpv_node) -> Self {
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

                    Self::Array(values.iter().map(Self::from).collect())
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
                                (key, Self::from(v)).into()
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

impl From<&Node> for *mut mpv_node {
    /// # Panics
    /// If a node string contains a null byte or a length overflows [`i32`].
    fn from(node: &Node) -> Self {
        let mut mpv_node = Box::new(mpv_node {
            format: 0,
            u: mpv_node__bindgen_ty_1 { int64: 0 },
        });

        match node {
            Node::None => {
                mpv_node.format = mpv_format_MPV_FORMAT_NONE;
            }
            Node::String(s) => {
                mpv_node.format = mpv_format_MPV_FORMAT_STRING;
                let cstr = CString::new(s.as_str()).expect("CString::new failed");
                mpv_node.u.string = cstr.into_raw();
            }
            Node::Int(i) => {
                mpv_node.format = mpv_format_MPV_FORMAT_INT64;
                mpv_node.u.int64 = *i;
            }
            Node::Double(f) => {
                mpv_node.format = mpv_format_MPV_FORMAT_DOUBLE;
                mpv_node.u.double_ = *f;
            }
            Node::Bool(b) => {
                mpv_node.format = mpv_format_MPV_FORMAT_FLAG;
                mpv_node.u.flag = i32::from(*b);
            }
            Node::Array(arr) => {
                mpv_node.format = mpv_format_MPV_FORMAT_NODE_ARRAY;

                let mut guard = MapBuilderGuard {
                    values: Vec::with_capacity(arr.len()),
                    keys: Vec::with_capacity(0),
                };

                for v in arr {
                    let node = unsafe { *Box::from_raw(Self::from(v)) };
                    guard.values.push(node);
                }

                let values = std::mem::take(&mut guard.values);
                std::mem::forget(guard);

                let values_ptr = if values.is_empty() {
                    ptr::null_mut()
                } else {
                    Box::into_raw(values.into_boxed_slice()).cast::<mpv_node>()
                };

                let list = Box::new(mpv_node_list {
                    num: arr.len().try_into().expect("len fits in i32"),
                    values: values_ptr,
                    keys: std::ptr::null_mut(),
                });

                mpv_node.u.list = Box::into_raw(list);
            }
            Node::Map(map) => {
                mpv_node.format = mpv_format_MPV_FORMAT_NODE_MAP;

                let mut guard = MapBuilderGuard {
                    values: Vec::with_capacity(map.len()),
                    keys: Vec::with_capacity(map.len()),
                };

                for (k, v) in map {
                    let cstring = CString::new(k.as_str()).expect("CString::new failed");
                    let node = unsafe { *Box::from_raw(Self::from(v)) };
                    guard.keys.push(cstring.into_raw());
                    guard.values.push(node);
                }

                let values = std::mem::take(&mut guard.values);
                let keys = std::mem::take(&mut guard.keys);
                std::mem::forget(guard);

                let values_ptr = if values.is_empty() {
                    ptr::null_mut()
                } else {
                    Box::into_raw(values.into_boxed_slice()).cast::<mpv_node>()
                };

                let keys_ptr = if keys.is_empty() {
                    ptr::null_mut()
                } else {
                    Box::into_raw(keys.into_boxed_slice()).cast::<*mut c_char>()
                };

                let list = Box::new(mpv_node_list {
                    num: map.len().try_into().expect("len fits in i32"),
                    values: values_ptr,
                    keys: keys_ptr,
                });

                mpv_node.u.list = Box::into_raw(list);
            }
            Node::ByteArray(vec) => {
                mpv_node.format = mpv_format_MPV_FORMAT_BYTE_ARRAY;

                let data = if vec.is_empty() {
                    std::ptr::null_mut()
                } else {
                    let boxed_slice = vec.clone().into_boxed_slice();
                    Box::into_raw(boxed_slice).cast::<c_void>()
                };

                let ba = Box::new(mpv_byte_array { data, size: vec.len() });
                mpv_node.u.ba = Box::into_raw(ba);
            }
        }

        Box::into_raw(mpv_node)
    }
}

pub struct MpvNodeGuard(*mut mpv_node);
impl From<&Node> for MpvNodeGuard {
    fn from(node: &Node) -> Self {
        Self(<*mut mpv_node>::from(node))
    }
}

impl MpvNodeGuard {
    #[must_use]
    pub const fn as_ptr(&self) -> *mut mpv_node {
        self.0
    }
}

impl Drop for MpvNodeGuard {
    fn drop(&mut self) {
        let ptr = self.0;
        if ptr.is_null() {
            return;
        }

        unsafe { drop_mpv_node_contents(&mut *ptr) };
        unsafe { drop(Box::from_raw(ptr)) }
    }
}

pub struct MpvNodeContentsGuard(pub *mut mpv_node);
impl Drop for MpvNodeContentsGuard {
    fn drop(&mut self) {
        Handle::free_node_contents(self.0);
    }
}

struct MapBuilderGuard {
    values: Vec<mpv_node>,
    keys: Vec<*mut c_char>,
}

impl Drop for MapBuilderGuard {
    fn drop(&mut self) {
        for child in &mut self.values {
            drop_mpv_node_contents(child);
        }

        for &key in &self.keys {
            if !key.is_null() {
                unsafe { drop(CString::from_raw(key)) };
            }
        }
    }
}

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

                if !list.keys.is_null() {
                    for &key in slice::from_raw_parts(list.keys, len) {
                        if !key.is_null() {
                            drop(CString::from_raw(key));
                        }
                    }

                    drop(Box::from_raw(ptr::slice_from_raw_parts_mut(list.keys, len)));
                }
            }
            mpv_format_MPV_FORMAT_BYTE_ARRAY => {
                if node.u.ba.is_null() {
                    return;
                }

                let ba = Box::from_raw(node.u.ba);
                if !ba.data.is_null() {
                    let slice_ptr = ptr::slice_from_raw_parts_mut(ba.data.cast::<u8>(), ba.size);
                    drop(Box::from_raw(slice_ptr));
                }
            }
            _ => {}
        }
    }
}
