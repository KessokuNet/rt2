//! Maps the game library with jni-host and binds its imports to bionic-shim.

use std::ffi::{CStr, c_char, c_int, c_void};
use std::path::Path;
use std::sync::OnceLock;

use bionic_shim::Shim;
use jni_host::NativeLibrary;
use stable_eyre::eyre::{Result, WrapErr, eyre};

/// The host `dlsym` bionic-shim installs by default.
static HOST_DLSYM: OnceLock<usize> = OnceLock::new();

const BIONIC_EAI_FAIL: c_int = 4;

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
            res.write(std::ptr::null_mut());
        }
    }
    BIONIC_EAI_FAIL
}

unsafe extern "C" fn gai_strerror(_code: c_int) -> *const c_char {
    c"networking is disabled".as_ptr()
}

/// Keeps the GL intercepts in front of the game's `dlsym` lookups.
unsafe extern "C" fn dlsym(handle: *mut c_void, name: *const c_char) -> *mut c_void {
    if name.is_null() {
        return std::ptr::null_mut();
    }
    // SAFETY: the caller passes a NUL-terminated symbol name.
    let name = unsafe { CStr::from_ptr(name) };
    if let Some(replacement) = crate::gl::intercept(name) {
        return replacement;
    }
    let Some(original) = HOST_DLSYM.get().copied() else {
        return std::ptr::null_mut();
    };
    // SAFETY: the stored address is bionic-shim's dlsym with this signature.
    let original: unsafe extern "C" fn(*mut c_void, *const c_char) -> *mut c_void =
        unsafe { std::mem::transmute(original) };
    // SAFETY: forwarded with the caller's arguments.
    unsafe { original(handle, name.as_ptr()) }
}

/// Loads, relocates and initializes the library, then resolves each name with `resolve`.
///
/// The library stays mapped for the rest of the process.
#[tracing::instrument(skip_all, fields(path = %path.display()))]
pub(crate) fn load<T>(
    path: &Path,
    resolve: impl FnOnce(&dyn Fn(&str) -> Result<usize>) -> Result<T>,
) -> Result<(NativeLibrary, T)> {
    let shim = Shim::new();
    let host_dlsym = shim
        .symbol("dlsym")
        .ok_or_else(|| eyre!("bionic-shim does not provide dlsym"))?;
    HOST_DLSYM
        .set(host_dlsym)
        .map_err(|_| eyre!("the GL dlsym hook was already installed"))?;
    let shim = shim
        .with("dlsym", dlsym as *const () as usize)
        .with("socket", socket as *const () as usize)
        .with("getaddrinfo", getaddrinfo as *const () as usize)
        .with("gai_strerror", gai_strerror as *const () as usize);

    let library = NativeLibrary::load(path, &shim).wrap_err("load game library")?;

    let lookup = |name: &str| -> Result<usize> {
        library
            .address(name)
            .wrap_err_with(|| format!("look up {name}"))?
            .ok_or_else(|| eyre!("library does not export {name}"))
    };
    let resolved = resolve(&lookup)?;
    Ok((library, resolved))
}
