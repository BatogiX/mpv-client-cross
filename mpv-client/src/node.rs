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

/// Shared methods
pub trait MpvNode: Sized {
    fn as_ref(&self) -> BorrowedMpvNode<'_>;
    fn as_mut_ptr(&mut self) -> *mut mpv_node;
    fn to_node(self) -> Node {
        let mpv_node = self.as_ref();
        unsafe {
            match mpv_node.format {
                mpv_format_MPV_FORMAT_STRING => {
                    if mpv_node.u.string.is_null() {
                        Node::None
                    } else {
                        Node::String(CStr::from_ptr(mpv_node.u.string).to_string_lossy().into_owned())
                    }
                }
                mpv_format_MPV_FORMAT_INT64 => Node::Int(mpv_node.u.int64),
                mpv_format_MPV_FORMAT_DOUBLE => Node::Double(mpv_node.u.double_),
                mpv_format_MPV_FORMAT_FLAG => Node::Bool(mpv_node.u.flag != 0),
                mpv_format_MPV_FORMAT_NODE_ARRAY => {
                    if mpv_node.u.list.is_null() {
                        return Node::Array(Vec::new());
                    }

                    let list = &*mpv_node.u.list;
                    let len: usize = list.num.try_into().expect("num fits in usize");

                    let values = if len == 0 || list.values.is_null() {
                        &[]
                    } else {
                        slice::from_raw_parts(list.values, len)
                    };

                    Node::Array(
                        values
                            .iter()
                            .map(|raw_node| BorrowedMpvNode(raw_node).to_node())
                            .collect(),
                    )
                }
                mpv_format_MPV_FORMAT_NODE_MAP => {
                    if mpv_node.u.list.is_null() {
                        return Node::Map(HashMap::new());
                    }

                    let list = &*mpv_node.u.list;
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
                                (key, BorrowedMpvNode(v).to_node()).into()
                            }
                        })
                        .collect();

                    Node::Map(map)
                }
                mpv_format_MPV_FORMAT_BYTE_ARRAY => {
                    if mpv_node.u.ba.is_null() {
                        return Node::ByteArray(Vec::new());
                    }

                    let arr: &mpv_byte_array = &*mpv_node.u.ba;

                    let data = if arr.size == 0 || arr.data.is_null() {
                        &[]
                    } else {
                        slice::from_raw_parts(arr.data as *const u8, arr.size)
                    };

                    Node::ByteArray(data.to_vec())
                }
                _ => Node::None,
            }
        }
    }

    fn to_node_array(self) -> Vec<Node> {
        let mpv_node = self.as_ref();
        let list = unsafe { mpv_node.u.list };
        if list.is_null() {
            return vec![];
        }

        let list = unsafe { &*list };
        let num = list.num;
        let values = list.values;
        if num <= 0 || values.is_null() {
            return vec![];
        }

        #[allow(clippy::cast_sign_loss)]
        let num = num as usize;
        let values = unsafe { slice::from_raw_parts(values, num) };

        values
            .iter()
            .map(|mpv_node| BorrowedMpvNode(mpv_node).to_node())
            .collect()
    }

    fn to_node_map(self) -> HashMap<String, Node> {
        let mpv_node = self.as_ref();
        let list = unsafe { mpv_node.u.list };
        if list.is_null() {
            return HashMap::new();
        }

        let list = unsafe { &*list };
        let num = list.num;
        let keys = list.keys;
        let values = list.values;
        if num <= 0 || values.is_null() || keys.is_null() {
            log::error!("num <= 0 || values.is_null() || keys.is_null()");
            log::error!("num: {num}, values: {values:#?}, keys: {keys:#?}");
            return HashMap::new();
        }

        #[allow(clippy::cast_sign_loss)]
        let num = num as usize;
        let keys = unsafe { slice::from_raw_parts(keys, num) };
        let values = unsafe { slice::from_raw_parts(values, num) };

        let mut node_map = HashMap::with_capacity(num);
        for (key, value) in keys.iter().zip(values) {
            if key.is_null() {
                log::error!("key.is_null()");
                return HashMap::new();
            }

            let key = unsafe { CStr::from_ptr(*key) }.to_string_lossy().into_owned();
            let value = BorrowedMpvNode(value).to_node();
            node_map.insert(key, value);
        }

        node_map
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
    pub fn from_node(node: Node) -> Self {
        let mut mpv_node = mpv_node {
            format: 0,
            u: mpv_node__bindgen_ty_1 { int64: 0 },
        };

        match node {
            Node::None => {
                mpv_node.format = mpv_format_MPV_FORMAT_NONE;
            }
            Node::String(string) => {
                mpv_node.format = mpv_format_MPV_FORMAT_STRING;
                mpv_node.u.string = CString::new(string).expect("CString::new failed").into_raw();
            }
            Node::Int(int64) => {
                mpv_node.format = mpv_format_MPV_FORMAT_INT64;
                mpv_node.u.int64 = int64;
            }
            Node::Double(float64) => {
                mpv_node.format = mpv_format_MPV_FORMAT_DOUBLE;
                mpv_node.u.double_ = float64;
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
            Node::ByteArray(byte_array) => {
                mpv_node.format = mpv_format_MPV_FORMAT_BYTE_ARRAY;
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

        log::error!(
            "dropping RawMpvNode: format: {:#?}, ba: {:#?}, double_: {:#?}, flag: {:#?}, int64: {:#?}, list: {:#?}, string: {:#?},",
            self.0.format,
            unsafe { self.0.u.ba },
            unsafe { self.0.u.double_ },
            unsafe { self.0.u.flag },
            unsafe { self.0.u.int64 },
            unsafe { self.0.u.list },
            unsafe { self.0.u.string },
        );

        let ptr = &raw mut self.0;
        unsafe { drop_mpv_node_contents(&mut *ptr) };
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
            .map(|node| RawMpvNode::from_node(node).0)
            .collect::<Vec<mpv_node>>()
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

    let (keys, values): (Vec<*mut i8>, Vec<mpv_node>) = node_map
        .into_iter()
        .take(num as usize)
        .map(|(key, node)| {
            (
                CString::new(key).expect("CString::new() failed").into_raw(),
                RawMpvNode::from_node(node).0,
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
