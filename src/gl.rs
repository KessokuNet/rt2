//! GL entry points we intercept between the game and the host GLES library.

use std::ffi::{CStr, c_char, c_void};
use std::sync::OnceLock;

use crate::bionic;

/// EXT_texture_compression_s3tc formats, which Mesa does not offer in an ES1 context.
const DXT1_RGB: u32 = 0x83f0;
const DXT1_RGBA: u32 = 0x83f1;
const DXT3: u32 = 0x83f2;
const DXT5: u32 = 0x83f3;

const GL_EXTENSIONS: u32 = 0x1f03;

type GlGetString = unsafe extern "C" fn(u32) -> *const c_char;
type GlCompressedTexImage2D =
    unsafe extern "C" fn(u32, i32, u32, i32, i32, i32, i32, *const c_void);
type GlTexImage2D = unsafe extern "C" fn(u32, i32, i32, i32, i32, i32, u32, u32, *const c_void);
type GlCompressedTexSubImage2D =
    unsafe extern "C" fn(u32, i32, i32, i32, i32, i32, u32, i32, *const c_void);
type GlGetError = unsafe extern "C" fn() -> u32;

const GL_RGBA: i32 = 0x1908;
const GL_UNSIGNED_BYTE: u32 = 0x1401;

pub(crate) fn host_symbol<T: Copy>(name: &CStr) -> Option<T> {
    let address = bionic::host_gles_proc(name)?;
    // SAFETY: callers name the host GLES signature for the symbol.
    Some(unsafe { std::mem::transmute_copy(&address) })
}

unsafe extern "C" fn get_error() -> u32 {
    let code = host_symbol::<GlGetError>(c"glGetError").map_or(0, |get| {
        // SAFETY: the host exports glGetError with this signature.
        unsafe { get() }
    });
    if code != 0 {
        tracing::debug!(code, "GL error");
    }
    code
}

fn format_name(format: u32) -> &'static str {
    match format {
        DXT1_RGB => "DXT1_RGB",
        DXT1_RGBA => "DXT1_RGBA",
        DXT3 => "DXT3",
        DXT5 => "DXT5",
        0x8d64 => "ETC1",
        _ => "unknown",
    }
}

unsafe extern "C" fn get_string(name: u32) -> *const c_char {
    let Some(host) = host_symbol::<GlGetString>(c"glGetString") else {
        return std::ptr::null();
    };
    if name != GL_EXTENSIONS {
        // SAFETY: forwarded to the host with the caller's argument.
        return unsafe { host(name) };
    }
    static EXTENSIONS: OnceLock<String> = OnceLock::new();
    let extensions = EXTENSIONS.get_or_init(|| {
        // SAFETY: the context is current, so the host returns its extension list.
        let list = unsafe { host(GL_EXTENSIONS) };
        let list = if list.is_null() {
            String::new()
        } else {
            // SAFETY: GL_EXTENSIONS returns a NUL-terminated string.
            unsafe { CStr::from_ptr(list) }
                .to_string_lossy()
                .into_owned()
        };
        format!("{list} GL_EXT_texture_compression_s3tc GL_AMD_compressed_ATC_texture\0")
    });
    extensions.as_ptr().cast()
}

unsafe extern "C" fn compressed_tex_image(
    target: u32,
    level: i32,
    format: u32,
    width: i32,
    height: i32,
    border: i32,
    size: i32,
    data: *const c_void,
) {
    tracing::debug!(
        target,
        level,
        format = format_name(format),
        format,
        width,
        height,
        size,
        "compressed upload"
    );
    if width < 0 || height < 0 || size < 0 {
        return;
    }
    // SAFETY: glCompressedTexImage2D promised `size` readable bytes at `data`.
    let compressed = if data.is_null() {
        &[]
    } else {
        unsafe { std::slice::from_raw_parts(data.cast::<u8>(), size as usize) }
    };
    if let Some(rgba) = crate::s3tc::decode(format, compressed, width as u32, height as u32)
        .or_else(|| crate::paletted::decode(format, compressed, width as u32, height as u32))
        && let Some(host) = host_symbol::<GlTexImage2D>(c"glTexImage2D")
    {
        // SAFETY: rgba is a tight width * height RGBA8 buffer that outlives the upload.
        unsafe {
            host(
                target,
                level,
                GL_RGBA,
                width,
                height,
                border,
                GL_RGBA as u32,
                GL_UNSIGNED_BYTE,
                rgba.as_ptr().cast(),
            );
        }
        crate::sprites::record_texture(target, level, width as u32, height as u32);
        if level == 0
            && let Some(dir) = std::env::var_os("RT2_TEXTURE_DUMP")
        {
            static INDEX: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
            let index = INDEX.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let _ = crate::write_png(
                &std::path::Path::new(&dir)
                    .join(format!("{index:03}-{width}x{height}-{format:04x}.png")),
                width as u32,
                height as u32,
                &rgba,
            )
            .map_err(|error| tracing::warn!(%error, "texture dump failed"));
        }
        return;
    }
    if let Some(host) = host_symbol::<GlCompressedTexImage2D>(c"glCompressedTexImage2D") {
        if let Some(dir) = std::env::var_os("RT2_UNKNOWN_TEXTURE_DUMP") {
            let path = std::path::Path::new(&dir)
                .join(format!("palette-{width}x{height}-{format:04x}.bin"));
            if let Err(error) = std::fs::write(&path, compressed) {
                tracing::warn!(%error, path = %path.display(), "palette texture dump failed");
            }
        }
        // SAFETY: forwarded to the host with the caller's arguments.
        unsafe { host(target, level, format, width, height, border, size, data) };
        crate::sprites::record_texture(target, level, width as u32, height as u32);
    }
}

unsafe extern "C" fn tex_parameter(target: u32, pname: u32, value: i32) {
    const MIN_FILTER: u32 = 0x2801;
    const MAG_FILTER: u32 = 0x2800;
    if matches!(pname, MIN_FILTER | MAG_FILTER) {
        tracing::debug!(target, pname, value, "texture filter");
    }
    type GlTexParameter = unsafe extern "C" fn(u32, u32, i32);
    if let Some(host) = host_symbol::<GlTexParameter>(c"glTexParameterx") {
        // SAFETY: signature matches the game's call.
        unsafe { host(target, pname, value) };
    }
}

unsafe extern "C" fn compressed_tex_sub_image(
    target: u32,
    level: i32,
    x: i32,
    y: i32,
    width: i32,
    height: i32,
    format: u32,
    size: i32,
    data: *const c_void,
) {
    tracing::debug!(
        target,
        level,
        x,
        y,
        width,
        height,
        format,
        size,
        "compressed sub-upload"
    );
    if let Some(host) = host_symbol::<GlCompressedTexSubImage2D>(c"glCompressedTexSubImage2D") {
        // SAFETY: forwarded to the host with the game's arguments.
        unsafe { host(target, level, x, y, width, height, format, size, data) };
    }
}

unsafe extern "C" fn tex_image(
    target: u32,
    level: i32,
    internal: i32,
    width: i32,
    height: i32,
    border: i32,
    format: u32,
    kind: u32,
    data: *const c_void,
) {
    tracing::debug!(
        target,
        level,
        internal,
        width,
        height,
        format,
        kind,
        "texture upload"
    );
    if let Some(host) = host_symbol::<GlTexImage2D>(c"glTexImage2D") {
        // SAFETY: the caller supplied a correctly sized texture buffer for its format.
        unsafe {
            host(
                target, level, internal, width, height, border, format, kind, data,
            )
        };
        if width >= 0 && height >= 0 {
            crate::sprites::record_texture(target, level, width as u32, height as u32);
        }
    }
}

/// Returns our replacement for a GL entry point, or None to use the host's.
pub(crate) fn intercept(name: &CStr) -> Option<*mut c_void> {
    match name.to_bytes() {
        b"glGetString" => Some(get_string as *const () as *mut c_void),
        b"glCompressedTexImage2D" => Some(compressed_tex_image as *const () as *mut c_void),
        b"glTexImage2D" => Some(tex_image as *const () as *mut c_void),
        b"glTexParameterx" => Some(tex_parameter as *const () as *mut c_void),
        b"glTexParameteri" => Some(tex_parameter as *const () as *mut c_void),
        b"glCompressedTexSubImage2D" => Some(compressed_tex_sub_image as *const () as *mut c_void),
        b"glGetError" => Some(get_error as *const () as *mut c_void),
        b"glDrawElements" => Some(crate::sprites::draw_elements as *const () as *mut c_void),
        b"glDeleteTextures" => Some(crate::sprites::delete_textures as *const () as *mut c_void),
        _ => None,
    }
}
