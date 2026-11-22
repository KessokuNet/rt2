//! Android libc (bionic) imports for the game library, backed by glibc.

use std::ffi::{CStr, c_char, c_int, c_void};
use std::ptr;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicI32, Ordering};

/// Bionic's static initializer values for recursive and error-checking mutexes.
const BIONIC_MUTEX_RECURSIVE_INIT: i32 = 0x4000;
const BIONIC_MUTEX_ERRORCHECK_INIT: i32 = 0x8000;
/// Values below this are bionic mutex initializers; anything above is our heap pointer.
const MUTEX_POINTER_MIN: i32 = 0x10000;
/// Bionic's `EAI_FAIL`; bionic uses positive `EAI_*` codes, glibc negative ones.
const BIONIC_EAI_FAIL: c_int = 4;

unsafe extern "C" {
    fn __cxa_atexit(func: extern "C" fn(*mut c_void), arg: *mut c_void, dso: *mut c_void) -> c_int;
    fn rt2_android_log_print(prio: c_int, tag: *const c_char, fmt: *const c_char, ...) -> c_int;
}

/// Bionic's i386 `struct stat`, which is the kernel's `stat64` layout.
#[repr(C, packed(4))]
struct BionicStat {
    st_dev: u64,
    pad0: [u8; 4],
    st_ino_lo: u32,
    st_mode: u32,
    st_nlink: u32,
    st_uid: u32,
    st_gid: u32,
    st_rdev: u64,
    pad3: [u8; 4],
    st_size: i64,
    st_blksize: u32,
    st_blocks: u64,
    st_atime: [u32; 2],
    st_mtime: [u32; 2],
    st_ctime: [u32; 2],
    st_ino: u64,
}

const _: () = assert!(size_of::<BionicStat>() == 96);
const _: () = assert!(std::mem::offset_of!(BionicStat, st_size) == 44);

#[repr(C)]
struct BionicTimeval {
    tv_sec: i32,
    tv_usec: i32,
}

#[unsafe(no_mangle)]
unsafe extern "C" fn rt2_android_log(prio: c_int, tag: *const c_char, msg: *const c_char) {
    // SAFETY: the C shim passes NUL-terminated strings; tag may be null.
    let (tag, msg) = unsafe {
        let tag = if tag.is_null() {
            "".into()
        } else {
            CStr::from_ptr(tag).to_string_lossy()
        };
        (tag, CStr::from_ptr(msg).to_string_lossy())
    };
    let msg = msg.trim_end();
    match prio {
        ..=2 => tracing::trace!(target: "game", %tag, "{msg}"),
        3 => tracing::debug!(target: "game", %tag, "{msg}"),
        4 => tracing::info!(target: "game", %tag, "{msg}"),
        5 => tracing::warn!(target: "game", %tag, "{msg}"),
        _ => tracing::error!(target: "game", %tag, "{msg}"),
    }
}

unsafe extern "C" fn fstat(fd: c_int, out: *mut BionicStat) -> c_int {
    // SAFETY: zeroed stat64 is valid; the caller passes a bionic stat buffer.
    unsafe {
        let mut st: libc::stat64 = std::mem::zeroed();
        if libc::fstat64(fd, &mut st) != 0 {
            return -1;
        }
        out.write(BionicStat {
            st_dev: st.st_dev,
            pad0: [0; 4],
            st_ino_lo: st.st_ino as u32,
            st_mode: st.st_mode,
            st_nlink: st.st_nlink,
            st_uid: st.st_uid,
            st_gid: st.st_gid,
            st_rdev: st.st_rdev,
            pad3: [0; 4],
            st_size: st.st_size,
            st_blksize: st.st_blksize as u32,
            st_blocks: st.st_blocks as u64,
            st_atime: [st.st_atime as u32, st.st_atime_nsec as u32],
            st_mtime: [st.st_mtime as u32, st.st_mtime_nsec as u32],
            st_ctime: [st.st_ctime as u32, st.st_ctime_nsec as u32],
            st_ino: st.st_ino,
        });
    }
    0
}

unsafe extern "C" fn lseek(fd: c_int, offset: i32, whence: c_int) -> i32 {
    // SAFETY: plain syscall wrapper.
    let pos = unsafe { libc::lseek64(fd, i64::from(offset), whence) };
    i32::try_from(pos).unwrap_or_else(|_| {
        // SAFETY: errno is thread local.
        unsafe { *libc::__errno_location() = libc::EOVERFLOW };
        -1
    })
}

unsafe extern "C" fn time(out: *mut i32) -> i32 {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i32);
    if !out.is_null() {
        // SAFETY: the caller passes a valid pointer or null.
        unsafe { out.write(now) };
    }
    now
}

unsafe extern "C" fn gettimeofday(tv: *mut BionicTimeval, _tz: *mut c_void) -> c_int {
    if !tv.is_null() {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default();
        // SAFETY: the caller passes a valid timeval.
        unsafe {
            tv.write(BionicTimeval {
                tv_sec: now.as_secs() as i32,
                tv_usec: now.subsec_micros() as i32,
            });
        }
    }
    0
}

unsafe extern "C" fn select(
    n: c_int,
    read: *mut libc::fd_set,
    write: *mut libc::fd_set,
    except: *mut libc::fd_set,
    timeout: *mut BionicTimeval,
) -> c_int {
    // SAFETY: fd_set layouts match; the timeval is converted.
    unsafe {
        let mut host = timeout.as_ref().map(|t| libc::timeval {
            tv_sec: t.tv_sec,
            tv_usec: t.tv_usec,
        });
        let host_ptr = host.as_mut().map_or(ptr::null_mut(), ptr::from_mut);
        libc::select(n, read, write, except, host_ptr)
    }
}

fn new_host_mutex(initializer: i32) -> *mut libc::pthread_mutex_t {
    let kind = match initializer {
        BIONIC_MUTEX_RECURSIVE_INIT => libc::PTHREAD_MUTEX_RECURSIVE,
        BIONIC_MUTEX_ERRORCHECK_INIT => libc::PTHREAD_MUTEX_ERRORCHECK,
        _ => libc::PTHREAD_MUTEX_NORMAL,
    };
    let mutex = Box::into_raw(Box::new(libc::PTHREAD_MUTEX_INITIALIZER));
    // SAFETY: attr and mutex are valid for the duration of the calls.
    unsafe {
        let mut attr: libc::pthread_mutexattr_t = std::mem::zeroed();
        libc::pthread_mutexattr_init(&mut attr);
        libc::pthread_mutexattr_settype(&mut attr, kind);
        libc::pthread_mutex_init(mutex, &attr);
        libc::pthread_mutexattr_destroy(&mut attr);
    }
    mutex
}

/// Returns the glibc mutex behind a 4-byte bionic mutex, creating it on first use.
unsafe fn host_mutex(slot: *mut i32) -> *mut libc::pthread_mutex_t {
    // SAFETY: the slot is an aligned, live bionic mutex owned by the game.
    let slot = unsafe { AtomicI32::from_ptr(slot) };
    let current = slot.load(Ordering::Acquire);
    if !(0..MUTEX_POINTER_MIN).contains(&current) {
        return current as usize as *mut _;
    }
    let mutex = new_host_mutex(current);
    match slot.compare_exchange(
        current,
        mutex as usize as i32,
        Ordering::AcqRel,
        Ordering::Acquire,
    ) {
        Ok(_) => mutex,
        Err(winner) => {
            // SAFETY: we created this mutex and nobody else has seen it.
            drop(unsafe { Box::from_raw(mutex) });
            winner as usize as *mut _
        }
    }
}

unsafe extern "C" fn mutex_init(slot: *mut i32, attr: *const i32) -> c_int {
    // SAFETY: bionic's pthread_mutexattr_t is a single int holding the type.
    let kind = unsafe { attr.as_ref() }.map_or(0, |a| a & 3);
    let initializer = match kind {
        1 => BIONIC_MUTEX_RECURSIVE_INIT,
        2 => BIONIC_MUTEX_ERRORCHECK_INIT,
        _ => 0,
    };
    // SAFETY: the game passes a valid mutex slot.
    unsafe {
        slot.write(initializer);
        host_mutex(slot);
    }
    0
}

unsafe extern "C" fn mutex_destroy(slot: *mut i32) -> c_int {
    // SAFETY: the game passes a valid mutex slot that it no longer uses.
    unsafe {
        let current = slot.read();
        if !(0..MUTEX_POINTER_MIN).contains(&current) {
            let mutex = current as usize as *mut libc::pthread_mutex_t;
            libc::pthread_mutex_destroy(mutex);
            drop(Box::from_raw(mutex));
        }
        slot.write(0);
    }
    0
}

unsafe extern "C" fn mutex_lock(slot: *mut i32) -> c_int {
    // SAFETY: the game passes a valid mutex slot.
    unsafe { libc::pthread_mutex_lock(host_mutex(slot)) }
}

unsafe extern "C" fn mutex_unlock(slot: *mut i32) -> c_int {
    // SAFETY: the game passes a valid mutex slot.
    unsafe { libc::pthread_mutex_unlock(host_mutex(slot)) }
}

unsafe extern "C" fn socket(domain: c_int, kind: c_int, protocol: c_int) -> c_int {
    tracing::info!(domain, kind, protocol, "refused socket");
    // SAFETY: errno is thread local.
    unsafe { *libc::__errno_location() = libc::EACCES };
    -1
}

unsafe extern "C" fn getaddrinfo(
    node: *const c_char,
    _service: *const c_char,
    _hints: *const c_void,
    res: *mut *mut c_void,
) -> c_int {
    // SAFETY: node is null or a C string; res is null or writable.
    unsafe {
        if !node.is_null() {
            tracing::info!(node = %CStr::from_ptr(node).to_string_lossy(), "refused getaddrinfo");
        }
        if !res.is_null() {
            res.write(ptr::null_mut());
        }
    }
    BIONIC_EAI_FAIL
}

unsafe extern "C" fn gai_strerror(_code: c_int) -> *const c_char {
    c"networking is disabled".as_ptr()
}

/// Libraries the game opens with `dlopen` that we forward to the host.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
enum HostLib {
    Gles1,
    Gles2,
    Egl,
}

impl HostLib {
    const ALL: [Self; 3] = [Self::Gles1, Self::Gles2, Self::Egl];

    const fn android_name(self) -> &'static CStr {
        match self {
            Self::Gles1 => c"libGLESv1_CM.so",
            Self::Gles2 => c"libGLESv2.so",
            Self::Egl => c"libEGL.so",
        }
    }

    const fn host_name(self) -> &'static CStr {
        match self {
            Self::Gles1 => c"libGLESv1_CM.so.1",
            Self::Gles2 => c"libGLESv2.so.2",
            Self::Egl => c"libEGL.so.1",
        }
    }

    fn handle(self) -> *mut c_void {
        static HANDLES: [HostLib; 3] = HostLib::ALL;
        ptr::from_ref(&HANDLES[self as usize]).cast_mut().cast()
    }

    fn from_handle(handle: *mut c_void) -> Option<Self> {
        Self::ALL.into_iter().find(|lib| lib.handle() == handle)
    }

    fn host_handle(self) -> Option<usize> {
        static OPENED: [OnceLock<Option<usize>>; 3] =
            [OnceLock::new(), OnceLock::new(), OnceLock::new()];
        *OPENED[self as usize].get_or_init(|| {
            // SAFETY: dlopen with a static C string.
            let handle = unsafe {
                libc::dlopen(self.host_name().as_ptr(), libc::RTLD_NOW | libc::RTLD_LOCAL)
            };
            if handle.is_null() {
                tracing::error!(lib = ?self.host_name(), "cannot open host library");
                None
            } else {
                Some(handle as usize)
            }
        })
    }
}

unsafe extern "C" fn dlopen(name: *const c_char, _flags: c_int) -> *mut c_void {
    // SAFETY: name is null or a C string.
    let name = unsafe { name.as_ref().map(|n| CStr::from_ptr(n)) };
    let lib = name.and_then(|n| HostLib::ALL.into_iter().find(|lib| lib.android_name() == n));
    tracing::info!(?name, forwarded = lib.is_some(), "dlopen");
    lib.map_or(ptr::null_mut(), HostLib::handle)
}

unsafe extern "C" fn dlsym(handle: *mut c_void, name: *const c_char) -> *mut c_void {
    let Some(lib) = HostLib::from_handle(handle) else {
        return ptr::null_mut();
    };
    let Some(host) = lib.host_handle() else {
        return ptr::null_mut();
    };
    // SAFETY: name remains valid for this lookup.
    if let Some(replacement) = crate::gl::intercept(unsafe { CStr::from_ptr(name) }) {
        return replacement;
    }
    // SAFETY: host is a live dlopen handle and name is a C string.
    let mut sym = unsafe { libc::dlsym(host as *mut c_void, name) };
    if sym.is_null() {
        // SAFETY: name is a C string owned by the caller.
        sym = unsafe { egl_proc_address(name) };
    }
    if sym.is_null() {
        // SAFETY: name is a C string.
        tracing::warn!(name = ?unsafe { CStr::from_ptr(name) }, "dlsym missing");
    }
    sym
}

/// Resolves a symbol in the host GLES 1 library, bypassing our own interception.
pub(crate) fn host_gles_proc(name: &CStr) -> Option<usize> {
    let host = HostLib::Gles1.host_handle()?;
    // SAFETY: host is a live dlopen handle and name is a C string.
    let sym = unsafe { libc::dlsym(host as *mut c_void, name.as_ptr()) };
    (!sym.is_null()).then_some(sym as usize)
}

/// Android exports the `_OES`/`_EXT` GL entry points from the GLES libraries; on the host they
/// are only reachable through `eglGetProcAddress`.
unsafe fn egl_proc_address(name: *const c_char) -> *mut c_void {
    type EglGetProcAddress = unsafe extern "C" fn(*const c_char) -> *mut c_void;
    let Some(egl) = HostLib::Egl.host_handle() else {
        return ptr::null_mut();
    };
    // SAFETY: the EGL library exports eglGetProcAddress.
    let getter = unsafe { libc::dlsym(egl as *mut c_void, c"eglGetProcAddress".as_ptr()) };
    if getter.is_null() {
        return ptr::null_mut();
    }
    // SAFETY: the pointer came from dlsym with the name above; the C string is the caller's.
    let getter: EglGetProcAddress = unsafe { std::mem::transmute(getter) };
    unsafe { getter(name) }
}

unsafe extern "C" fn dlclose(_handle: *mut c_void) -> c_int {
    0
}

macro_rules! imports {
    ($($name:literal => $addr:expr),* $(,)?) => {
        vec![$((
            CStr::from_bytes_with_nul(concat!($name, "\0").as_bytes()).expect("import names have no NUL"),
            $addr as *const (),
        )),*]
    };
}

/// Every symbol the game library imports, mapped to a host function or bionic shim.
pub(crate) fn imports() -> Vec<(&'static CStr, *const ())> {
    imports! {
        "__android_log_print" => rt2_android_log_print,
        "__errno" => libc::__errno_location,
        "__cxa_atexit" => __cxa_atexit,
        "fstat" => fstat,
        "lseek" => lseek,
        "time" => time,
        "gettimeofday" => gettimeofday,
        "select" => select,
        "pthread_mutex_init" => mutex_init,
        "pthread_mutex_destroy" => mutex_destroy,
        "pthread_mutex_lock" => mutex_lock,
        "pthread_mutex_unlock" => mutex_unlock,
        "socket" => socket,
        "getaddrinfo" => getaddrinfo,
        "gai_strerror" => gai_strerror,
        "dlopen" => dlopen,
        "dlsym" => dlsym,
        "dlclose" => dlclose,
        "accept" => libc::accept,
        "bind" => libc::bind,
        "bsearch" => libc::bsearch,
        "close" => libc::close,
        "connect" => libc::connect,
        "fclose" => libc::fclose,
        "fcntl" => libc::fcntl,
        "fflush" => libc::fflush,
        "fopen" => libc::fopen,
        "free" => libc::free,
        "fwrite" => libc::fwrite,
        "getenv" => libc::getenv,
        "getpeername" => libc::getpeername,
        "getsockname" => libc::getsockname,
        "getsockopt" => libc::getsockopt,
        "ioctl" => libc::ioctl,
        "listen" => libc::listen,
        "malloc" => libc::malloc,
        "memcpy" => libc::memcpy,
        "memset" => libc::memset,
        "mkdir" => libc::mkdir,
        "open" => libc::open,
        "read" => libc::read,
        "realloc" => libc::realloc,
        "recv" => libc::recv,
        "recvfrom" => libc::recvfrom,
        "remove" => libc::remove,
        "rename" => libc::rename,
        "rmdir" => libc::rmdir,
        "send" => libc::send,
        "sendto" => libc::sendto,
        "setenv" => libc::setenv,
        "setsockopt" => libc::setsockopt,
        "shutdown" => libc::shutdown,
        "sprintf" => libc::sprintf,
        "strcpy" => libc::strcpy,
        "write" => libc::write,
    }
}
