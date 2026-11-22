//! A fake JNI environment that answers the game's calls into its Java shell.

pub(crate) mod object;

use std::ffi::{CStr, CString, c_char, c_void};
use std::ptr;
use std::sync::OnceLock;

use jni_sys::{
    JNIEnv, JNIInvokeInterface__1_4, JNINativeInterface__1_6, JavaVM, jarray, jboolean, jclass,
    jint, jlong, jmethodID, jobject, jobjectArray, jobjectRefType, jsize, jstring, jvalue,
};

use crate::java::{self, Handler};

/// Entries in the C trap table; `JNINativeInterface__1_6` uses the first 233.
const TRAP_SLOTS: usize = 240;
/// Upper bound on Java method parameters the C shim decodes.
const MAX_ARGS: usize = 16;

const ENV_WORDS: usize = size_of::<JNINativeInterface__1_6>() / size_of::<usize>();
const _: () = assert!(ENV_WORDS <= TRAP_SLOTS);

unsafe extern "C" {
    static rt2_jni_traps: [usize; TRAP_SLOTS];
    /// Plain, V and A entry points per return type, Object through Void.
    static rt2_jni_call_families: [[usize; 3]; 10];
}

// The C shim defines these; jni-sys declares the variadic entry with the `C` ABI, the rest `system`.
unsafe extern "C" {
    fn rt2_jni_new_object(env: *mut JNIEnv, clazz: jclass, method: jmethodID, ...) -> jobject;
}
unsafe extern "system" {
    fn rt2_jni_new_object_v(
        env: *mut JNIEnv,
        clazz: jclass,
        method: jmethodID,
        args: *mut c_void,
    ) -> jobject;
    fn rt2_jni_new_object_a(
        env: *mut JNIEnv,
        clazz: jclass,
        method: jmethodID,
        args: *const jvalue,
    ) -> jobject;
}

/// A method ID handed to the game.
struct JMethod {
    class: String,
    name: String,
    sig: CString,
    arg_count: usize,
    handler: Option<Handler>,
}

struct Handles {
    env: usize,
    vm: usize,
}

static HANDLES: OnceLock<Handles> = OnceLock::new();

pub(crate) fn env() -> *mut JNIEnv {
    HANDLES.get().expect("jni::init was not called").env as *mut JNIEnv
}

pub(crate) fn vm() -> *mut JavaVM {
    HANDLES.get().expect("jni::init was not called").vm as *mut JavaVM
}

pub(crate) const VOID: jvalue = jvalue { j: 0 };

/// Counts the parameters in a JNI method descriptor such as `(I[BLjava/lang/String;)V`.
fn arg_count(sig: &str) -> usize {
    let params = sig
        .strip_prefix('(')
        .and_then(|s| s.split(')').next())
        .unwrap_or("");
    let mut chars = params.chars();
    let mut count = 0;
    while let Some(mut c) = chars.next() {
        while c == '[' {
            c = chars.next().unwrap_or(';');
        }
        if c == 'L' {
            chars.by_ref().find(|&c| c == ';');
        }
        count += 1;
    }
    count
}

#[unsafe(no_mangle)]
unsafe extern "C" fn rt2_jni_new_instance(class: jclass) -> jobject {
    let class = object::class_name(class);
    tracing::debug!(%class, "constructed Java object");
    object::new_instance(&class)
}

#[unsafe(no_mangle)]
unsafe extern "C" fn rt2_jni_trap(slot: i32) -> ! {
    tracing::error!(slot, "unimplemented JNIEnv function");
    std::process::abort();
}

#[unsafe(no_mangle)]
unsafe extern "C" fn rt2_jni_method_sig(method: jmethodID) -> *const c_char {
    // SAFETY: method IDs come from get_method_id.
    unsafe { (*method.cast::<JMethod>()).sig.as_ptr() }
}

#[unsafe(no_mangle)]
unsafe extern "C" fn rt2_jni_invoke(
    this: jobject,
    method: jmethodID,
    args: *const jvalue,
    out: *mut jvalue,
) {
    // SAFETY: method IDs come from get_method_id; argument arrays hold one value per parameter.
    unsafe {
        let method = &*method.cast::<JMethod>();
        let args = if args.is_null() {
            &[][..]
        } else {
            std::slice::from_raw_parts(args, method.arg_count)
        };
        let result = match method.handler {
            Some(handler) => handler(this, args),
            None => {
                tracing::warn!(class = %method.class, name = %method.name, sig = ?method.sig, "unhandled Java call");
                VOID
            }
        };
        out.write(result);
    }
}

unsafe extern "system" fn get_version(_env: *mut JNIEnv) -> jint {
    jni_sys::JNI_VERSION_1_6
}

unsafe extern "system" fn find_class(_env: *mut JNIEnv, name: *const c_char) -> jclass {
    // SAFETY: the game passes a C string.
    object::class(&unsafe { CStr::from_ptr(name) }.to_string_lossy())
}

unsafe extern "system" fn exception_occurred(_env: *mut JNIEnv) -> jobject {
    ptr::null_mut()
}

unsafe extern "system" fn exception_check(_env: *mut JNIEnv) -> jboolean {
    false
}

unsafe extern "system" fn nothing(_env: *mut JNIEnv) {}

unsafe extern "system" fn capacity_ok(_env: *mut JNIEnv, _capacity: jint) -> jint {
    0
}

unsafe extern "system" fn same_ref(_env: *mut JNIEnv, obj: jobject) -> jobject {
    obj
}

unsafe extern "system" fn drop_ref(_env: *mut JNIEnv, _obj: jobject) {}

unsafe extern "system" fn is_same_object(_env: *mut JNIEnv, a: jobject, b: jobject) -> jboolean {
    a == b
}

unsafe extern "system" fn get_object_class(_env: *mut JNIEnv, obj: jobject) -> jclass {
    // SAFETY: the game passes a handle we created.
    unsafe { object::class_of(obj) }
}

unsafe extern "system" fn is_instance_of(
    _env: *mut JNIEnv,
    obj: jobject,
    class: jclass,
) -> jboolean {
    // SAFETY: the game passes a handle we created.
    unsafe { object::class_of(obj) == class }
}

unsafe extern "system" fn get_method_id(
    _env: *mut JNIEnv,
    class: jclass,
    name: *const c_char,
    sig: *const c_char,
) -> jmethodID {
    let class = object::class_name(class);
    // SAFETY: the game passes C strings.
    let (name, sig) = unsafe { (CStr::from_ptr(name), CStr::from_ptr(sig)) };
    let name = name.to_string_lossy().into_owned();
    let handler = java::lookup(&class, &name);
    if handler.is_none() {
        tracing::debug!(%class, %name, ?sig, "no handler for Java method");
    }
    let method = JMethod {
        arg_count: arg_count(&sig.to_string_lossy()).min(MAX_ARGS),
        class,
        name,
        sig: sig.to_owned(),
        handler,
    };
    Box::into_raw(Box::new(method)).cast()
}

unsafe extern "system" fn new_string_utf(_env: *mut JNIEnv, utf8: *const c_char) -> jstring {
    // SAFETY: the game passes a C string.
    object::new_string(unsafe { CStr::from_ptr(utf8) }.to_bytes())
}

unsafe extern "system" fn get_string_utf_length(_env: *mut JNIEnv, s: jstring) -> jsize {
    // SAFETY: the game passes a handle we created.
    unsafe { object::string(s) }.map_or(0, |s| s.to_bytes().len() as jsize)
}

unsafe extern "system" fn get_string_utf_chars(
    _env: *mut JNIEnv,
    s: jstring,
    is_copy: *mut jboolean,
) -> *const c_char {
    // SAFETY: is_copy is null or writable; s is a handle we created.
    unsafe {
        if !is_copy.is_null() {
            is_copy.write(false);
        }
        object::string(s).map_or(ptr::null(), CStr::as_ptr)
    }
}

unsafe extern "system" fn release_string_utf_chars(
    _env: *mut JNIEnv,
    _s: jstring,
    _chars: *const c_char,
) {
}

unsafe extern "system" fn get_array_length(_env: *mut JNIEnv, arr: jarray) -> jsize {
    // SAFETY: the game passes a handle we created.
    match unsafe { object::get(arr) } {
        Some(object::JObject::Array { len, .. }) => *len as jsize,
        Some(object::JObject::ObjectArray(items)) => items.len() as jsize,
        _ => 0,
    }
}

unsafe extern "system" fn new_object_array(
    _env: *mut JNIEnv,
    len: jsize,
    _class: jclass,
    init: jobject,
) -> jobjectArray {
    object::new_object_array(len, init)
}

unsafe extern "system" fn get_object_array_element(
    _env: *mut JNIEnv,
    arr: jobjectArray,
    index: jsize,
) -> jobject {
    // SAFETY: the game passes a handle we created.
    match unsafe { object::get(arr) } {
        Some(object::JObject::ObjectArray(items)) => items
            .get(index as usize)
            .copied()
            .unwrap_or(ptr::null_mut()),
        _ => ptr::null_mut(),
    }
}

unsafe extern "system" fn set_object_array_element(
    _env: *mut JNIEnv,
    arr: jobjectArray,
    index: jsize,
    value: jobject,
) {
    // SAFETY: the game passes a handle we created.
    if let Some(object::JObject::ObjectArray(items)) = unsafe { object::get(arr) }
        && let Some(slot) = items.get_mut(index as usize)
    {
        *slot = value;
    }
}

unsafe fn array_elements(arr: jarray, is_copy: *mut jboolean) -> *mut c_void {
    // SAFETY: is_copy is null or writable; arr is a handle we created.
    unsafe {
        if !is_copy.is_null() {
            is_copy.write(false);
        }
        object::array_bytes(arr).map_or(ptr::null_mut(), |(bytes, _)| bytes.as_mut_ptr().cast())
    }
}

/// Byte range of `len` elements starting at `start`, if it lies inside the array.
unsafe fn array_region<'a>(arr: jarray, start: jsize, len: jsize) -> Option<&'a mut [u8]> {
    // SAFETY: arr is a handle we created.
    let (bytes, size) = unsafe { object::array_bytes(arr) }?;
    let start = usize::try_from(start).ok()? * size;
    let end = start + usize::try_from(len).ok()? * size;
    bytes.get_mut(start..end)
}

macro_rules! array_fns {
    ($($ty:ty: $new:ident $get:ident $release:ident $get_region:ident $set_region:ident),* $(,)?) => {$(
        unsafe extern "system" fn $new(_env: *mut JNIEnv, len: jsize) -> jarray {
            object::new_array(size_of::<$ty>(), len)
        }

        unsafe extern "system" fn $get(_env: *mut JNIEnv, arr: jarray, is_copy: *mut jboolean) -> *mut $ty {
            // SAFETY: forwarded from the game.
            unsafe { array_elements(arr, is_copy).cast() }
        }

        unsafe extern "system" fn $release(_env: *mut JNIEnv, _arr: jarray, _elems: *mut $ty, _mode: jint) {}

        unsafe extern "system" fn $get_region(_env: *mut JNIEnv, arr: jarray, start: jsize, len: jsize, buf: *mut $ty) {
            // SAFETY: the game passes a buffer of len elements.
            if let Some(src) = unsafe { array_region(arr, start, len) } {
                unsafe { ptr::copy_nonoverlapping(src.as_ptr(), buf.cast(), src.len()) };
            }
        }

        unsafe extern "system" fn $set_region(_env: *mut JNIEnv, arr: jarray, start: jsize, len: jsize, buf: *const $ty) {
            // SAFETY: the game passes a buffer of len elements.
            if let Some(dst) = unsafe { array_region(arr, start, len) } {
                unsafe { ptr::copy_nonoverlapping(buf.cast(), dst.as_mut_ptr(), dst.len()) };
            }
        }
    )*};
}

array_fns! {
    jboolean: new_boolean_array get_boolean_elements release_boolean_elements get_boolean_region set_boolean_region,
    i8: new_byte_array get_byte_elements release_byte_elements get_byte_region set_byte_region,
    u16: new_char_array get_char_elements release_char_elements get_char_region set_char_region,
    i16: new_short_array get_short_elements release_short_elements get_short_region set_short_region,
    i32: new_int_array get_int_elements release_int_elements get_int_region set_int_region,
    i64: new_long_array get_long_elements release_long_elements get_long_region set_long_region,
    f32: new_float_array get_float_elements release_float_elements get_float_region set_float_region,
    f64: new_double_array get_double_elements release_double_elements get_double_region set_double_region,
}

unsafe extern "system" fn get_primitive_array_critical(
    _env: *mut JNIEnv,
    arr: jarray,
    is_copy: *mut jboolean,
) -> *mut c_void {
    // SAFETY: forwarded from the game.
    unsafe { array_elements(arr, is_copy) }
}

unsafe extern "system" fn release_primitive_array_critical(
    _env: *mut JNIEnv,
    _arr: jarray,
    _elems: *mut c_void,
    _mode: jint,
) {
}

unsafe extern "system" fn get_java_vm(_env: *mut JNIEnv, out: *mut *mut JavaVM) -> jint {
    // SAFETY: the game passes a writable pointer.
    unsafe { out.write(vm()) };
    jni_sys::JNI_OK
}

unsafe extern "system" fn new_direct_byte_buffer(
    _env: *mut JNIEnv,
    addr: *mut c_void,
    capacity: jlong,
) -> jobject {
    object::new_buffer(addr, capacity)
}

unsafe extern "system" fn get_direct_buffer_address(
    _env: *mut JNIEnv,
    buf: jobject,
) -> *mut c_void {
    // SAFETY: the game passes a handle we created.
    match unsafe { object::get(buf) } {
        Some(object::JObject::Buffer { addr, .. }) => *addr,
        _ => ptr::null_mut(),
    }
}

unsafe extern "system" fn get_direct_buffer_capacity(_env: *mut JNIEnv, buf: jobject) -> jlong {
    // SAFETY: the game passes a handle we created.
    match unsafe { object::get(buf) } {
        Some(object::JObject::Buffer { capacity, .. }) => *capacity,
        _ => -1,
    }
}

unsafe extern "system" fn get_object_ref_type(_env: *mut JNIEnv, _obj: jobject) -> jobjectRefType {
    jobjectRefType::JNIGlobalRefType
}

unsafe extern "system" fn vm_ok(_vm: *mut JavaVM) -> jint {
    jni_sys::JNI_OK
}

unsafe extern "system" fn vm_get_env(
    _vm: *mut JavaVM,
    out: *mut *mut c_void,
    _version: jint,
) -> jint {
    // SAFETY: the game passes a writable pointer.
    unsafe { out.write(env().cast()) };
    jni_sys::JNI_OK
}

unsafe extern "system" fn vm_attach(
    _vm: *mut JavaVM,
    out: *mut *mut c_void,
    _args: *mut c_void,
) -> jint {
    // SAFETY: the game passes a writable pointer.
    unsafe { out.write(env().cast()) };
    jni_sys::JNI_OK
}

fn env_table() -> &'static JNINativeInterface__1_6 {
    let mut words = Box::new([0usize; ENV_WORDS]);
    // SAFETY: the C shim defines both tables as constant data.
    let (traps, families) = unsafe { (&rt2_jni_traps, &rt2_jni_call_families) };
    words.copy_from_slice(&traps[..ENV_WORDS]);

    for (first, family) in [
        std::mem::offset_of!(JNINativeInterface__1_6, CallObjectMethod),
        std::mem::offset_of!(JNINativeInterface__1_6, CallStaticObjectMethod),
    ]
    .map(|offset| offset / size_of::<usize>())
    .into_iter()
    .flat_map(|first| (0..families.len()).map(move |i| (first + i * 3, i)))
    {
        words[first..first + 3].copy_from_slice(&families[family]);
    }

    // SAFETY: the table is ENV_WORDS pointer-sized words, every one a valid function address.
    let t = unsafe { &mut *Box::into_raw(words).cast::<JNINativeInterface__1_6>() };
    t.GetVersion = get_version;
    t.FindClass = find_class;
    t.ExceptionOccurred = exception_occurred;
    t.ExceptionDescribe = nothing;
    t.ExceptionClear = nothing;
    t.PushLocalFrame = capacity_ok;
    t.PopLocalFrame = same_ref;
    t.NewGlobalRef = same_ref;
    t.DeleteGlobalRef = drop_ref;
    t.DeleteLocalRef = drop_ref;
    t.IsSameObject = is_same_object;
    t.NewLocalRef = same_ref;
    t.EnsureLocalCapacity = capacity_ok;
    t.GetObjectClass = get_object_class;
    t.IsInstanceOf = is_instance_of;
    t.GetMethodID = get_method_id;
    t.GetStaticMethodID = get_method_id;
    t.NewObject = rt2_jni_new_object;
    t.NewObjectV = rt2_jni_new_object_v;
    t.NewObjectA = rt2_jni_new_object_a;
    t.NewStringUTF = new_string_utf;
    t.GetStringUTFLength = get_string_utf_length;
    t.GetStringUTFChars = get_string_utf_chars;
    t.ReleaseStringUTFChars = release_string_utf_chars;
    t.GetArrayLength = get_array_length;
    t.NewObjectArray = new_object_array;
    t.GetObjectArrayElement = get_object_array_element;
    t.SetObjectArrayElement = set_object_array_element;

    macro_rules! arrays {
        ($($new:ident $get:ident $release:ident $get_region:ident $set_region:ident
            => $f_new:ident $f_get:ident $f_release:ident $f_get_region:ident $f_set_region:ident;)*) => {$(
            t.$new = $f_new;
            t.$get = $f_get;
            t.$release = $f_release;
            t.$get_region = $f_get_region;
            t.$set_region = $f_set_region;
        )*};
    }
    arrays! {
        NewBooleanArray GetBooleanArrayElements ReleaseBooleanArrayElements GetBooleanArrayRegion SetBooleanArrayRegion
            => new_boolean_array get_boolean_elements release_boolean_elements get_boolean_region set_boolean_region;
        NewByteArray GetByteArrayElements ReleaseByteArrayElements GetByteArrayRegion SetByteArrayRegion
            => new_byte_array get_byte_elements release_byte_elements get_byte_region set_byte_region;
        NewCharArray GetCharArrayElements ReleaseCharArrayElements GetCharArrayRegion SetCharArrayRegion
            => new_char_array get_char_elements release_char_elements get_char_region set_char_region;
        NewShortArray GetShortArrayElements ReleaseShortArrayElements GetShortArrayRegion SetShortArrayRegion
            => new_short_array get_short_elements release_short_elements get_short_region set_short_region;
        NewIntArray GetIntArrayElements ReleaseIntArrayElements GetIntArrayRegion SetIntArrayRegion
            => new_int_array get_int_elements release_int_elements get_int_region set_int_region;
        NewLongArray GetLongArrayElements ReleaseLongArrayElements GetLongArrayRegion SetLongArrayRegion
            => new_long_array get_long_elements release_long_elements get_long_region set_long_region;
        NewFloatArray GetFloatArrayElements ReleaseFloatArrayElements GetFloatArrayRegion SetFloatArrayRegion
            => new_float_array get_float_elements release_float_elements get_float_region set_float_region;
        NewDoubleArray GetDoubleArrayElements ReleaseDoubleArrayElements GetDoubleArrayRegion SetDoubleArrayRegion
            => new_double_array get_double_elements release_double_elements get_double_region set_double_region;
    }

    t.GetJavaVM = get_java_vm;
    t.GetPrimitiveArrayCritical = get_primitive_array_critical;
    t.ReleasePrimitiveArrayCritical = release_primitive_array_critical;
    t.NewWeakGlobalRef = same_ref;
    t.DeleteWeakGlobalRef = drop_ref;
    t.ExceptionCheck = exception_check;
    t.NewDirectByteBuffer = new_direct_byte_buffer;
    t.GetDirectBufferAddress = get_direct_buffer_address;
    t.GetDirectBufferCapacity = get_direct_buffer_capacity;
    t.GetObjectRefType = get_object_ref_type;
    t
}

fn vm_table() -> &'static JNIInvokeInterface__1_4 {
    const WORDS: usize = size_of::<JNIInvokeInterface__1_4>() / size_of::<usize>();
    const _: () = assert!(WORDS == 8);
    // Three reserved words, then the functions, laid out by the same repr(C) as the interface.
    let mut words = [0usize; WORDS];
    words[3] = vm_ok as *const () as usize;
    words[4] = vm_attach as *const () as usize;
    words[5] = vm_ok as *const () as usize;
    words[6] = vm_get_env as *const () as usize;
    words[7] = vm_attach as *const () as usize;
    let table = Box::leak(Box::new(words));
    // SAFETY: the interface is eight pointer-sized fields in this order.
    unsafe { &*table.as_ptr().cast::<JNIInvokeInterface__1_4>() }
}

/// A `JNIEnv*` or `JavaVM*` points at a word holding the function table's address.
fn leak_interface<T>(table: &'static T) -> usize {
    ptr::from_ref(Box::leak(Box::new(ptr::from_ref(table)))) as usize
}

pub(crate) fn init() {
    HANDLES.get_or_init(|| Handles {
        env: leak_interface(env_table()),
        vm: leak_interface(vm_table()),
    });
}

#[cfg(test)]
mod tests {
    use super::arg_count;

    #[test]
    fn arg_count_skips_object_and_array_descriptors() {
        assert_eq!(arg_count("()V"), 0);
        assert_eq!(arg_count("([I)Z"), 1);
        assert_eq!(arg_count("(Landroid/app/Activity;ILjava/lang/String;)V"), 3);
        assert_eq!(
            arg_count("(Landroid/content/Context;Ljava/lang/String;III[I)V"),
            6
        );
        assert_eq!(arg_count("([[Ljava/lang/String;JD)V"), 3);
    }
}
