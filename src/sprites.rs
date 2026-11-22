//! Atlas sampling corrections for the engine's interleaved 2D sprite batches.

use std::cell::RefCell;
use std::collections::HashMap;
use std::ffi::c_void;
use std::sync::OnceLock;

use crate::gl::host_symbol;

const GL_TEXTURE_2D: u32 = 0x0de1;
const GL_TEXTURE0: i32 = 0x84c0;
const GL_ACTIVE_TEXTURE: u32 = 0x84e0;
const GL_CLIENT_ACTIVE_TEXTURE: u32 = 0x84e1;
const GL_TEXTURE_BINDING_2D: u32 = 0x8069;
const GL_TEXTURE_COORD_ARRAY: u32 = 0x8078;
const GL_TEXTURE_COORD_ARRAY_SIZE: u32 = 0x8088;
const GL_TEXTURE_COORD_ARRAY_TYPE: u32 = 0x8089;
const GL_TEXTURE_COORD_ARRAY_STRIDE: u32 = 0x808a;
const GL_TEXTURE_COORD_ARRAY_POINTER: u32 = 0x8092;
const GL_VERTEX_ARRAY_SIZE: u32 = 0x807a;
const GL_VERTEX_ARRAY_TYPE: u32 = 0x807b;
const GL_VERTEX_ARRAY_STRIDE: u32 = 0x807c;
const GL_ARRAY_BUFFER_BINDING: u32 = 0x8894;
const GL_ELEMENT_ARRAY_BUFFER_BINDING: u32 = 0x8895;
const GL_VERTEX_ARRAY_BUFFER_BINDING: u32 = 0x8896;
const GL_TEXTURE_COORD_ARRAY_BUFFER_BINDING: u32 = 0x889a;
const GL_TRIANGLES: u32 = 0x0004;
const GL_UNSIGNED_SHORT: u32 = 0x1403;
const GL_FIXED: i32 = 0x140c;

/// Two fixed-point positions, one RGBA8 color and two fixed-point UVs.
const SPRITE_VERTEX_STRIDE: i32 = 20;
const FIXED_ONE: i32 = 65536;
const FIXED_HALF: i32 = FIXED_ONE / 2;

type GetInteger = unsafe extern "C" fn(u32, *mut i32);
type GetPointer = unsafe extern "C" fn(u32, *mut *mut c_void);
type IsEnabled = unsafe extern "C" fn(u32) -> u8;
type TexCoordPointer = unsafe extern "C" fn(i32, u32, i32, *const c_void);
type DrawElements = unsafe extern "C" fn(u32, i32, u32, *const c_void);
type DeleteTextures = unsafe extern "C" fn(i32, *const u32);

struct Gl {
    get_integer: GetInteger,
    get_pointer: GetPointer,
    is_enabled: IsEnabled,
    texcoord_pointer: TexCoordPointer,
    draw_elements: DrawElements,
    delete_textures: DeleteTextures,
}

impl Gl {
    fn integer(&self, pname: u32) -> i32 {
        let mut value = 0;
        // SAFETY: every caller queries a scalar GLES integer state.
        unsafe { (self.get_integer)(pname, &mut value) };
        value
    }

    fn uses_sprite_arrays(&self) -> bool {
        self.integer(GL_VERTEX_ARRAY_SIZE) == 2
            && self.integer(GL_VERTEX_ARRAY_TYPE) == GL_FIXED
            && self.integer(GL_VERTEX_ARRAY_STRIDE) == SPRITE_VERTEX_STRIDE
            && self.integer(GL_TEXTURE_COORD_ARRAY_SIZE) == 2
            && self.integer(GL_TEXTURE_COORD_ARRAY_TYPE) == GL_FIXED
            && self.integer(GL_TEXTURE_COORD_ARRAY_STRIDE) == SPRITE_VERTEX_STRIDE
            && self.integer(GL_ACTIVE_TEXTURE) == GL_TEXTURE0
            && self.integer(GL_CLIENT_ACTIVE_TEXTURE) == GL_TEXTURE0
            && self.integer(GL_ARRAY_BUFFER_BINDING) == 0
            && self.integer(GL_ELEMENT_ARRAY_BUFFER_BINDING) == 0
            && self.integer(GL_VERTEX_ARRAY_BUFFER_BINDING) == 0
            && self.integer(GL_TEXTURE_COORD_ARRAY_BUFFER_BINDING) == 0
    }
}

fn gl() -> Option<&'static Gl> {
    static GL: OnceLock<Option<Gl>> = OnceLock::new();
    GL.get_or_init(|| {
        Some(Gl {
            get_integer: host_symbol(c"glGetIntegerv")?,
            get_pointer: host_symbol(c"glGetPointerv")?,
            is_enabled: host_symbol(c"glIsEnabled")?,
            texcoord_pointer: host_symbol(c"glTexCoordPointer")?,
            draw_elements: host_symbol(c"glDrawElements")?,
            delete_textures: host_symbol(c"glDeleteTextures")?,
        })
    })
    .as_ref()
}

thread_local! {
    static TEXTURE_SIZES: RefCell<HashMap<i32, (u32, u32)>> = RefCell::new(HashMap::new());
}

pub(crate) fn reset_textures() {
    TEXTURE_SIZES.with_borrow_mut(HashMap::clear);
}

pub(crate) fn record_texture(target: u32, level: i32, width: u32, height: u32) {
    if target == GL_TEXTURE_2D
        && level == 0
        && let Some(gl) = gl()
    {
        let texture = gl.integer(GL_TEXTURE_BINDING_2D);
        TEXTURE_SIZES.with_borrow_mut(|sizes| {
            if width == 0 || height == 0 {
                sizes.remove(&texture);
            } else {
                sizes.insert(texture, (width, height));
            }
        });
    }
}

pub(crate) unsafe extern "C" fn delete_textures(count: i32, textures: *const u32) {
    let Some(gl) = gl() else { return };
    if count > 0 && !textures.is_null() {
        TEXTURE_SIZES.with_borrow_mut(|sizes| {
            for i in 0..count as usize {
                // SAFETY: glDeleteTextures receives count readable texture names.
                let texture = unsafe { textures.add(i).read_unaligned() };
                sizes.remove(&(texture as i32));
            }
        });
    }
    // SAFETY: forward the caller's texture-name array unchanged.
    unsafe { (gl.delete_textures)(count, textures) };
}

fn inset_quad(uvs: &mut [[i32; 2]; 4], width: u32, height: u32) -> bool {
    if width == 0
        || height == 0
        || uvs.iter().flatten().any(|&v| !(0..=FIXED_ONE).contains(&v))
        || uvs[0][0] != uvs[3][0]
        || uvs[1][0] != uvs[2][0]
        || uvs[0][1] != uvs[1][1]
        || uvs[2][1] != uvs[3][1]
    {
        return false;
    }
    let mut changed = false;
    for (axis, size) in [width, height].into_iter().enumerate() {
        let low = uvs.iter().map(|uv| uv[axis]).min().unwrap();
        let high = uvs.iter().map(|uv| uv[axis]).max().unwrap();
        if [low, high]
            .into_iter()
            .any(|v| (i64::from(v) * i64::from(size)) % i64::from(FIXED_ONE) != 0)
        {
            continue;
        }
        let inset = ((FIXED_HALF as u32 / size) as i32).min((high - low) / 2);
        for uv in uvs.iter_mut() {
            let value = uv[axis];
            if value == low && low > 0 {
                uv[axis] += inset;
            } else if value == high && high < FIXED_ONE {
                uv[axis] -= inset;
            }
            changed |= uv[axis] != value;
        }
    }
    changed
}

fn quad_indices(indices: &[u16]) -> bool {
    !indices.is_empty()
        && indices.len().is_multiple_of(6)
        && indices
            .as_chunks::<6>()
            .0
            .iter()
            .all(|&[a, b, c, d, e, f]| {
                a.checked_add(1) == Some(b)
                    && a.checked_add(2) == Some(c)
                    && d == a
                    && e == c
                    && a.checked_add(3) == Some(f)
            })
}

unsafe fn adjusted_uvs(
    gl: &Gl,
    count: i32,
    indices: *const c_void,
) -> Option<(*mut c_void, Vec<[i32; 2]>)> {
    if !gl.uses_sprite_arrays() {
        return None;
    }
    // SAFETY: the context is current and this queries the active client UV array.
    if unsafe { (gl.is_enabled)(GL_TEXTURE_COORD_ARRAY) } == 0 {
        return None;
    }
    let texture = gl.integer(GL_TEXTURE_BINDING_2D);
    let (width, height) = TEXTURE_SIZES.with_borrow(|sizes| sizes.get(&texture).copied())?;
    let indices: Vec<u16> = (0..count as usize)
        .map(|i| {
            // SAFETY: client index storage contains count unsigned shorts, as required by the draw.
            unsafe { indices.cast::<u16>().add(i).read_unaligned() }
        })
        .collect();
    if !quad_indices(&indices) {
        return None;
    }
    let mut pointer = std::ptr::null_mut();
    // SAFETY: pointer is one writable address slot; buffer bindings were checked above.
    unsafe { (gl.get_pointer)(GL_TEXTURE_COORD_ARRAY_POINTER, &mut pointer) };
    if pointer.is_null() {
        return None;
    }
    let vertices = usize::from(*indices.iter().max()?) + 1;
    let mut adjusted = vec![[0i32; 2]; vertices];
    let mut changed = false;
    for &[base, _, _, _, _, _] in indices.as_chunks::<6>().0 {
        let base = usize::from(base);
        let mut quad: [[i32; 2]; 4] = std::array::from_fn(|i| {
            let offset = (base + i) * SPRITE_VERTEX_STRIDE as usize;
            // SAFETY: each quad vertex is referenced by the draw and has two fixed-point UVs.
            unsafe {
                pointer
                    .cast::<u8>()
                    .add(offset)
                    .cast::<[i32; 2]>()
                    .read_unaligned()
            }
        });
        changed |= inset_quad(&mut quad, width, height);
        adjusted[base..base + 4].copy_from_slice(&quad);
    }
    changed.then_some((pointer, adjusted))
}

pub(crate) unsafe extern "C" fn draw_elements(
    mode: u32,
    count: i32,
    kind: u32,
    indices: *const c_void,
) {
    let Some(gl) = gl() else { return };
    let adjusted = if mode == GL_TRIANGLES
        && count > 0
        && count % 6 == 0
        && kind == GL_UNSIGNED_SHORT
        && !indices.is_null()
    {
        // SAFETY: adjusted_uvs checks client-array bindings before reading draw storage.
        unsafe { adjusted_uvs(gl, count, indices) }
    } else {
        None
    };
    if let Some((original, uvs)) = adjusted {
        // SAFETY: uvs covers every referenced vertex and stays alive until the host draw returns.
        unsafe {
            (gl.texcoord_pointer)(2, GL_FIXED as u32, 0, uvs.as_ptr().cast());
            (gl.draw_elements)(mode, count, kind, indices);
            (gl.texcoord_pointer)(2, GL_FIXED as u32, SPRITE_VERTEX_STRIDE, original);
        }
    } else {
        // SAFETY: the caller's draw is forwarded without changing arrays or buffer bindings.
        unsafe { (gl.draw_elements)(mode, count, kind, indices) };
    }
}

#[cfg(test)]
mod tests {
    use super::{inset_quad, quad_indices};

    #[test]
    fn insets_sprite_edges_by_half_a_texel() {
        let mut uvs = [[640, 8192], [12800, 8192], [12800, 1024], [640, 1024]];
        assert!(inset_quad(&mut uvs, 1024, 512));
        assert_eq!(
            uvs,
            [[672, 8128], [12768, 8128], [12768, 1088], [672, 1088]]
        );
    }

    #[test]
    fn preserves_mirrored_sprites_and_collapses_single_texel_regions_to_their_centers() {
        let mut uvs = [[128, 256], [64, 256], [64, 128], [128, 128]];
        assert!(inset_quad(&mut uvs, 1024, 512));
        assert_eq!(uvs, [[96, 192]; 4]);
    }

    #[test]
    fn leaves_repeating_and_nonrectangular_uvs_unchanged() {
        for original in [
            [[0, 0], [131072, 0], [131072, 65536], [0, 65536]],
            [[0, 0], [1000, 0], [2000, 1000], [0, 1000]],
        ] {
            let mut uvs = original;
            assert!(!inset_quad(&mut uvs, 1024, 512));
            assert_eq!(uvs, original);
        }
    }

    #[test]
    fn leaves_full_textures_and_existing_texel_insets_unchanged() {
        for original in [
            [[0, 0], [65536, 0], [65536, 65536], [0, 65536]],
            [[672, 8128], [12768, 8128], [12768, 1088], [672, 1088]],
        ] {
            let mut uvs = original;
            assert!(!inset_quad(&mut uvs, 1024, 512));
            assert_eq!(uvs, original);
        }
    }

    #[test]
    fn accepts_only_complete_sprite_quad_index_patterns() {
        assert!(quad_indices(&[0, 1, 2, 0, 2, 3, 4, 5, 6, 4, 6, 7]));
        for indices in [
            &[][..],
            &[0, 1, 2][..],
            &[0, 1, 2, 2, 3, 0][..],
            &[65535, 0, 1, 65535, 1, 2][..],
        ] {
            assert!(!quad_indices(indices));
        }
    }
}
