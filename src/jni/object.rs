//! Java objects handed to the game; every `jobject` is a leaked `Box<JObject>`.

use std::ffi::{CStr, CString, c_void};
use std::sync::Mutex;

use jni_sys::jobject;

pub(crate) enum JObject {
    Class(String),
    Instance {
        class: jobject,
    },
    String(CString),
    Array {
        elem_size: usize,
        len: usize,
        data: Box<[u64]>,
    },
    ObjectArray(Vec<jobject>),
    Buffer {
        addr: *mut c_void,
        capacity: i64,
    },
}

fn leak(obj: JObject) -> jobject {
    Box::into_raw(Box::new(obj)).cast()
}

pub(crate) fn class(name: &str) -> jobject {
    static CLASSES: Mutex<Vec<(String, usize)>> = Mutex::new(Vec::new());
    let mut classes = CLASSES
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Some(&(_, handle)) = classes.iter().find(|(n, _)| n == name) {
        return handle as jobject;
    }
    let handle = leak(JObject::Class(name.to_owned()));
    classes.push((name.to_owned(), handle as usize));
    handle
}

pub(crate) fn new_instance(class_name: &str) -> jobject {
    leak(JObject::Instance {
        class: class(class_name),
    })
}

pub(crate) fn new_string(bytes: impl Into<Vec<u8>>) -> jobject {
    let mut bytes = bytes.into();
    bytes.retain(|&b| b != 0);
    leak(JObject::String(CString::new(bytes).unwrap_or_default()))
}

pub(crate) fn new_array(elem_size: usize, len: i32) -> jobject {
    let len = usize::try_from(len).unwrap_or(0);
    let words = (len * elem_size).div_ceil(size_of::<u64>()).max(1);
    leak(JObject::Array {
        elem_size,
        len,
        data: vec![0; words].into_boxed_slice(),
    })
}

pub(crate) fn new_object_array(len: i32, init: jobject) -> jobject {
    leak(JObject::ObjectArray(vec![
        init;
        usize::try_from(len).unwrap_or(0)
    ]))
}

pub(crate) fn new_buffer(addr: *mut c_void, capacity: i64) -> jobject {
    leak(JObject::Buffer { addr, capacity })
}

/// Borrows an object the game passed back to us.
pub(crate) unsafe fn get<'a>(obj: jobject) -> Option<&'a mut JObject> {
    // SAFETY: handles are leaked boxes, so any non-null handle is live.
    unsafe { obj.cast::<JObject>().as_mut() }
}

pub(crate) unsafe fn string<'a>(obj: jobject) -> Option<&'a CStr> {
    // SAFETY: forwarded from the caller.
    match unsafe { get(obj) } {
        Some(JObject::String(s)) => Some(s.as_c_str()),
        _ => None,
    }
}

pub(crate) fn class_name(class: jobject) -> String {
    // SAFETY: class handles come from `class`.
    match unsafe { get(class) } {
        Some(JObject::Class(name)) => name.clone(),
        _ => "<not a class>".to_owned(),
    }
}

pub(crate) unsafe fn class_of(obj: jobject) -> jobject {
    // SAFETY: forwarded from the caller.
    match unsafe { get(obj) } {
        None => std::ptr::null_mut(),
        Some(JObject::Class(_)) => class("java/lang/Class"),
        Some(JObject::Instance { class: c }) => *c,
        Some(JObject::String(_)) => class("java/lang/String"),
        Some(JObject::Array { .. } | JObject::ObjectArray(_)) => class("java/lang/Object[]"),
        Some(JObject::Buffer { .. }) => class("java/nio/ByteBuffer"),
    }
}

/// Returns the bytes of a primitive array and its element size.
pub(crate) unsafe fn array_bytes<'a>(arr: jobject) -> Option<(&'a mut [u8], usize)> {
    // SAFETY: forwarded from the caller.
    match unsafe { get(arr) } {
        Some(JObject::Array {
            elem_size,
            len,
            data,
        }) => {
            // SAFETY: data holds at least len * elem_size bytes.
            let bytes = unsafe {
                std::slice::from_raw_parts_mut(data.as_mut_ptr().cast::<u8>(), *len * *elem_size)
            };
            Some((bytes, *elem_size))
        }
        _ => None,
    }
}
