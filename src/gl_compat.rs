//! GLES 1.1 fixed-point and ES-only float calls translated for desktop OpenGL.
//! Conversion rules: GLES 1.1 §6.1.2; parameter counts: Khronos es1.1 refpages.

use std::ffi::{CStr, c_void};

use crate::gl::host_symbol;

fn fixed(value: i32) -> f32 {
    value as f32 / 65536.0
}

fn fixed64(value: i32) -> f64 {
    f64::from(value) / 65536.0
}

#[derive(Clone, Copy, Debug)]
enum Parameter {
    Fog,
    LightModel,
    Light,
    Material,
    TexEnv,
    TexParameter,
    Point,
}

impl Parameter {
    fn len(self, pname: u32) -> usize {
        match (self, pname) {
            (Self::Fog, 0x0b66) | (Self::LightModel, 0x0b53)
            | (Self::Light, 0x1200..=0x1203)
            | (Self::Material, 0x1200..=0x1202 | 0x1600 | 0x1602)
            | (Self::TexEnv, 0x2201) | (Self::TexParameter, 0x8b9d) => 4,
            (Self::Light, 0x1204) | (Self::Point, 0x8129) => 3,
            _ => 1,
        }
    }

    fn is_enum(self, pname: u32) -> bool {
        match self {
            Self::Fog => pname == 0x0b65, // GL_FOG_MODE
            Self::TexEnv => matches!(pname, 0x2200 | 0x8571..=0x8572
                | 0x8580..=0x8582 | 0x8588..=0x858a
                | 0x8590..=0x8592 | 0x8598..=0x859a),
            Self::TexParameter => matches!(pname, 0x2800..=0x2803 | 0x8072),
            _ => false,
        }
    }

    fn to_float(self, pname: u32, value: i32) -> f32 {
        if self.is_enum(pname) { value as f32 } else { fixed(value) }
    }

    fn from_float(self, pname: u32, value: f32) -> i32 {
        if self.is_enum(pname) { value as i32 } else { (value * 65536.0) as i32 }
    }
}

macro_rules! scalar {
    ($($name:ident => $native:literal),* $(,)?) => {$(
        unsafe extern "C" fn $name(value: i32) {
            type Fn = unsafe extern "C" fn(f32);
            if let Some(host) = host_symbol::<Fn>($native) {
                // SAFETY: the host takes exactly one float.
                unsafe { host(fixed(value)) };
            }
        }
    )*};
}

scalar! {
    point_size_x => c"glPointSize",
    line_width_x => c"glLineWidth",
}

macro_rules! scalar1 {
    ($($name:ident => $native:literal, $kind:ident),* $(,)?) => {$(
        unsafe extern "C" fn $name(pname: u32, value: i32) {
            type Fn = unsafe extern "C" fn(u32, f32);
            if let Some(host) = host_symbol::<Fn>($native) {
                // SAFETY: the host takes one enum and one float.
                unsafe { host(pname, Parameter::$kind.to_float(pname, value)) };
            }
        }
    )*};
}

scalar1! {
    fog_x => c"glFogf", Fog,
    light_model_x => c"glLightModelf", LightModel,
    point_parameter_x => c"glPointParameterf", Point,
}

unsafe extern "C" fn alpha_func_x(func: u32, value: i32) {
    type Fn = unsafe extern "C" fn(u32, f32);
    if let Some(host) = host_symbol::<Fn>(c"glAlphaFunc") {
        // SAFETY: the host takes the comparison enum and a float reference.
        unsafe { host(func, fixed(value)) };
    }
}

macro_rules! scalar2 {
    ($($name:ident => $native:literal, $kind:ident),* $(,)?) => {$(
        unsafe extern "C" fn $name(first: u32, pname: u32, value: i32) {
            type Fn = unsafe extern "C" fn(u32, u32, f32);
            if let Some(host) = host_symbol::<Fn>($native) {
                // SAFETY: the host takes two enums and one float.
                unsafe { host(first, pname, Parameter::$kind.to_float(pname, value)) };
            }
        }
    )*};
}

scalar2! {
    light_x => c"glLightf", Light,
    material_x => c"glMaterialf", Material,
    tex_env_x => c"glTexEnvf", TexEnv,
}

macro_rules! color4 {
    ($($name:ident => $native:literal),* $(,)?) => {$(
        unsafe extern "C" fn $name(r: i32, g: i32, b: i32, a: i32) {
            type Fn = unsafe extern "C" fn(f32, f32, f32, f32);
            if let Some(host) = host_symbol::<Fn>($native) {
                // SAFETY: native entry points converted below use fixed fn signatures.
                unsafe { host(fixed(r), fixed(g), fixed(b), fixed(a)) };
            }
        }
    )*};
}

color4! {
    clear_color_x => c"glClearColor",
    color4_x => c"glColor4f",
}

unsafe extern "C" fn normal3_x(x: i32, y: i32, z: i32) {
    type Fn = unsafe extern "C" fn(f32, f32, f32);
    if let Some(host) = host_symbol::<Fn>(c"glNormal3f") {
        // SAFETY: matching three-float normal vector on desktop.
        unsafe { host(fixed(x), fixed(y), fixed(z)) };
    }
}

unsafe extern "C" fn sample_coverage_x(value: i32, invert: u8) {
    type Fn = unsafe extern "C" fn(f32, u8);
    if let Some(host) = host_symbol::<Fn>(c"glSampleCoverage") {
        // SAFETY: the boolean is one ABI byte, as on GLES.
        unsafe { host(fixed(value), invert) };
    }
}

unsafe extern "C" fn clear_depth_x(depth: i32) {
    type Fn = unsafe extern "C" fn(f64);
    if let Some(host) = host_symbol::<Fn>(c"glClearDepth") {
        // SAFETY: glClearDepth takes exactly one double.
        unsafe { host(fixed64(depth)) };
    }
}

unsafe extern "C" fn depth_range_x(near: i32, far: i32) {
    type Fn = unsafe extern "C" fn(f64, f64);
    if let Some(host) = host_symbol::<Fn>(c"glDepthRange") {
        // SAFETY: same near and far values as doubles on desktop.
        unsafe { host(fixed64(near), fixed64(far)) };
    }
}

unsafe extern "C" fn polygon_offset_x(factor: i32, units: i32) {
    type Fn = unsafe extern "C" fn(f32, f32);
    if let Some(host) = host_symbol::<Fn>(c"glPolygonOffset") {
        // SAFETY: matching pair of floats on desktop.
        unsafe { host(fixed(factor), fixed(units)) };
    }
}

unsafe extern "C" fn clip_plane_x(plane: u32, equation: *const i32) {
    type Fn = unsafe extern "C" fn(u32, *const f64);
    // SAFETY: GLES requires four readable equation coefficients.
    let input = unsafe { std::slice::from_raw_parts(equation, 4) };
    let values: [f64; 4] = core::array::from_fn(|i| fixed64(input[i]));
    if let Some(host) = host_symbol::<Fn>(c"glClipPlane") {
        // SAFETY: values holds the four coefficients during the call.
        unsafe { host(plane, values.as_ptr()) };
    }
}

unsafe extern "C" fn get_clip_plane_x(plane: u32, equation: *mut i32) {
    type Fn = unsafe extern "C" fn(u32, *mut f64);
    if let Some(host) = host_symbol::<Fn>(c"glGetClipPlane") {
        let mut values = [0.0f64; 4];
        // SAFETY: values holds four doubles for the host.
        unsafe { host(plane, values.as_mut_ptr()) };
        for i in 0..4 {
            // SAFETY: the caller supplied the same four-value destination.
            unsafe { *equation.add(i) = (values[i] * 65536.0) as i32 };
        }
    }
}

macro_rules! matrix6 {
    ($($name:ident => $native:literal),* $(,)?) => {$(
        unsafe extern "C" fn $name(a: i32, b: i32, c: i32, d: i32, e: i32, f: i32) {
            type Fn = unsafe extern "C" fn(f64, f64, f64, f64, f64, f64);
            if let Some(host) = host_symbol::<Fn>($native) {
                // SAFETY: native entry points converted below use fixed fn signatures.
                unsafe { host(fixed64(a), fixed64(b), fixed64(c), fixed64(d), fixed64(e), fixed64(f)) };
            }
        }
    )*};
}

matrix6! {
    frustum_x => c"glFrustum",
    ortho_x => c"glOrtho",
}

macro_rules! transform3 {
    ($($name:ident => $native:literal),* $(,)?) => {$(
        unsafe extern "C" fn $name(x: i32, y: i32, z: i32) {
            type Fn = unsafe extern "C" fn(f64, f64, f64);
            if let Some(host) = host_symbol::<Fn>($native) {
                // SAFETY: native entry points converted below use fixed fn signatures.
                unsafe { host(fixed64(x), fixed64(y), fixed64(z)) };
            }
        }
    )*};
}

transform3! {
    translate_x => c"glTranslated",
    scale_x => c"glScaled",
}

unsafe extern "C" fn rotate_x(angle: i32, x: i32, y: i32, z: i32) {
    type Fn = unsafe extern "C" fn(f64, f64, f64, f64);
    if let Some(host) = host_symbol::<Fn>(c"glRotated") {
        // SAFETY: matching angle and normalized axis values on desktop.
        unsafe { host(fixed64(angle), fixed64(x), fixed64(y), fixed64(z)) };
    }
}

macro_rules! matrix16 {
    ($($name:ident => $native:literal),* $(,)?) => {$(
        unsafe extern "C" fn $name(input: *const i32) {
            type Fn = unsafe extern "C" fn(*const f32);
            // SAFETY: GLES requires sixteen readable matrix elements.
            let input = unsafe { std::slice::from_raw_parts(input, 16) };
            let values: [f32; 16] = core::array::from_fn(|i| fixed(input[i]));
            if let Some(host) = host_symbol::<Fn>($native) {
                // SAFETY: values contains the sixteen matrix elements during the call.
                unsafe { host(values.as_ptr()) };
            }
        }
    )*};
}

matrix16! {
    load_matrix_x => c"glLoadMatrixf",
    mult_matrix_x => c"glMultMatrixf",
}

unsafe fn parameter_values(kind: Parameter, pname: u32, input: *const i32) -> [f32; 4] {
    let len = kind.len(pname);
    // SAFETY: the GLES caller supplies the pname-specific number of readable values.
    let input = unsafe { std::slice::from_raw_parts(input, len) };
    let mut values = [0.0; 4];
    for (dst, &src) in values.iter_mut().zip(input) {
        *dst = kind.to_float(pname, src);
    }
    values
}

macro_rules! vector1 {
    ($($name:ident => $native:literal, $kind:ident),* $(,)?) => {$(
        unsafe extern "C" fn $name(pname: u32, input: *const i32) {
            type Fn = unsafe extern "C" fn(u32, *const f32);
            if let Some(host) = host_symbol::<Fn>($native) {
                // SAFETY: input contains the GLES vector required by pname.
                let values = unsafe { parameter_values(Parameter::$kind, pname, input) };
                // SAFETY: the host always takes an enum and a pointer, even for one value.
                unsafe { host(pname, values.as_ptr()) };
            }
        }
    )*};
}

vector1! {
    fog_xv => c"glFogfv", Fog,
    light_model_xv => c"glLightModelfv", LightModel,
    point_parameter_xv => c"glPointParameterfv", Point,
}

macro_rules! vector2 {
    ($($name:ident => $native:literal, $kind:ident),* $(,)?) => {$(
        unsafe extern "C" fn $name(first: u32, pname: u32, input: *const i32) {
            type Fn = unsafe extern "C" fn(u32, u32, *const f32);
            if let Some(host) = host_symbol::<Fn>($native) {
                // SAFETY: input contains the GLES vector required by pname.
                let values = unsafe { parameter_values(Parameter::$kind, pname, input) };
                // SAFETY: the host always takes two enums and a pointer, even for one value.
                unsafe { host(first, pname, values.as_ptr()) };
            }
        }
    )*};
}

vector2! {
    light_xv => c"glLightfv", Light,
    material_xv => c"glMaterialfv", Material,
    tex_env_xv => c"glTexEnvfv", TexEnv,
    tex_parameter_xv => c"glTexParameterfv", TexParameter,
}

macro_rules! get_vector {
    ($($name:ident => $native:literal, $kind:ident),* $(,)?) => {$(
        unsafe extern "C" fn $name(first: u32, pname: u32, output: *mut i32) {
            type Fn = unsafe extern "C" fn(u32, u32, *mut f32);
            if let Some(host) = host_symbol::<Fn>($native) {
                let mut values = [0.0f32; 4];
                // SAFETY: valid GLES pnames return at most four floats.
                unsafe { host(first, pname, values.as_mut_ptr()) };
                let kind = Parameter::$kind;
                for i in 0..kind.len(pname) {
                    // SAFETY: output has exactly the number of elements required by pname.
                    unsafe { *output.add(i) = kind.from_float(pname, values[i]) };
                }
            }
        }
    )*};
}

get_vector! {
    get_light_xv => c"glGetLightfv", Light,
    get_material_xv => c"glGetMaterialfv", Material,
    get_tex_env_xv => c"glGetTexEnvfv", TexEnv,
    get_tex_parameter_xv => c"glGetTexParameterfv", TexParameter,
}

unsafe extern "C" fn multi_texcoord_x(texture: u32, s: i32, t: i32, r: i32, q: i32) {
    type Fn = unsafe extern "C" fn(u32, f32, f32, f32, f32);
    if let Some(host) = host_symbol::<Fn>(c"glMultiTexCoord4f") {
        // SAFETY: same texture unit enum and four floats on desktop.
        unsafe { host(texture, fixed(s), fixed(t), fixed(r), fixed(q)) };
    }
}

macro_rules! matrix6_f {
    ($($name:ident => $native:literal),* $(,)?) => {$(
        unsafe extern "C" fn $name(a: f32, b: f32, c: f32, d: f32, e: f32, f: f32) {
            type Fn = unsafe extern "C" fn(f64, f64, f64, f64, f64, f64);
            if let Some(host) = host_symbol::<Fn>($native) {
                // SAFETY: desktop projection entry points take six doubles.
                unsafe { host(a.into(), b.into(), c.into(), d.into(), e.into(), f.into()) };
            }
        }
    )*};
}

matrix6_f! {
    ortho_f => c"glOrtho",
    frustum_f => c"glFrustum",
}

unsafe extern "C" fn clear_depth_f(depth: f32) {
    type Fn = unsafe extern "C" fn(f64);
    if let Some(host) = host_symbol::<Fn>(c"glClearDepth") {
        // SAFETY: the desktop entry point takes one double.
        unsafe { host(depth.into()) };
    }
}

unsafe extern "C" fn depth_range_f(near: f32, far: f32) {
    type Fn = unsafe extern "C" fn(f64, f64);
    if let Some(host) = host_symbol::<Fn>(c"glDepthRange") {
        // SAFETY: the desktop entry point takes two doubles.
        unsafe { host(near.into(), far.into()) };
    }
}

unsafe extern "C" fn clip_plane_f(plane: u32, equation: *const f32) {
    type Fn = unsafe extern "C" fn(u32, *const f64);
    if let Some(host) = host_symbol::<Fn>(c"glClipPlane") {
        // SAFETY: GLES requires four readable float coefficients.
        let input = unsafe { std::slice::from_raw_parts(equation, 4) };
        let values: [f64; 4] = core::array::from_fn(|i| input[i].into());
        // SAFETY: values holds four doubles throughout the call.
        unsafe { host(plane, values.as_ptr()) };
    }
}

unsafe extern "C" fn get_clip_plane_f(plane: u32, equation: *mut f32) {
    type Fn = unsafe extern "C" fn(u32, *mut f64);
    if let Some(host) = host_symbol::<Fn>(c"glGetClipPlane") {
        let mut values = [0.0f64; 4];
        // SAFETY: the host writes four doubles into values, not the float destination.
        unsafe { host(plane, values.as_mut_ptr()) };
        for i in 0..4 {
            // SAFETY: GLES requires four writable float coefficients.
            unsafe { *equation.add(i) = values[i] as f32 };
        }
    }
}

// GLES 1.1 glGet: all non-scalar state, including the variable-length format list.
fn state_len(pname: u32, compressed_formats: usize) -> usize {
    match pname {
        0x0ba6..=0x0ba8 => 16, // modelview, projection, texture matrices
        0x0b00 | 0x0b03 | 0x0b53 | 0x0b66 | 0x0ba2 | 0x0c10 | 0x0c22 | 0x0c23 => 4,
        0x0b02 | 0x8129 => 3, // current normal, point attenuation
        0x0b12 | 0x0b22 | 0x0b70 | 0x0d3a | 0x846d | 0x846e => 2,
        0x86a3 => compressed_formats, // GL_COMPRESSED_TEXTURE_FORMATS
        _ => 1,
    }
}

fn state_is_enum(pname: u32) -> bool {
    matches!(pname,
        0x0b45 | 0x0b46 | 0x0b54 | 0x0b65 | 0x0b74 | 0x0b92
        | 0x0b94..=0x0b96 | 0x0ba0 | 0x0bc1 | 0x0be0..=0x0be1 | 0x0bf0
        | 0x0c50..=0x0c54 | 0x807b | 0x807e | 0x8082 | 0x8089
        | 0x84e0..=0x84e1 | 0x8b9a..=0x8b9b // implementation read type/format
        | 0x8847 | 0x86a3 | 0x86a9 | 0x898a) // OES array types, compressed enums
}

unsafe extern "C" fn get_fixed_v(pname: u32, output: *mut i32) {
    type GetInt = unsafe extern "C" fn(u32, *mut i32);
    type GetFloat = unsafe extern "C" fn(u32, *mut f32);
    let formats = if pname == 0x86a3 {
        let Some(host) = host_symbol::<GetInt>(c"glGetIntegerv") else { return };
        let mut count = 0;
        // SAFETY: GL_NUM_COMPRESSED_TEXTURE_FORMATS returns exactly one integer.
        unsafe { host(0x86a2, &mut count) };
        count.max(0) as usize
    } else {
        0
    };
    let len = state_len(pname, formats);
    if len == 0 { return; }
    if state_is_enum(pname) {
        if let Some(host) = host_symbol::<GetInt>(c"glGetIntegerv") {
            // SAFETY: output has the pname-specific length; enum values remain unscaled.
            unsafe { host(pname, output) };
        }
    } else if let Some(host) = host_symbol::<GetFloat>(c"glGetFloatv") {
        let mut values = vec![0.0f32; len];
        // SAFETY: values has the GLES pname-specific result length.
        unsafe { host(pname, values.as_mut_ptr()) };
        for i in 0..len {
            // SAFETY: output has the same pname-specific length as values.
            unsafe { *output.add(i) = (values[i] * 65536.0) as i32 };
        }
    }
}

/// Returns our GLES-to-desktop translation, avoiding desktop ES-only dispatch stubs.
pub(crate) fn intercept(name: &CStr) -> Option<*mut c_void> {
    let call = match name.to_bytes() {
        b"glOrthof" => ortho_f as *const (),
        b"glFrustumf" => frustum_f as *const (),
        b"glClearDepthf" => clear_depth_f as *const (),
        b"glDepthRangef" => depth_range_f as *const (),
        b"glClipPlanef" => clip_plane_f as *const (),
        b"glGetClipPlanef" => get_clip_plane_f as *const (),
        b"glGetFixedv" => get_fixed_v as *const (),
        b"glAlphaFuncx" => alpha_func_x as *const (),
        b"glFogx" => fog_x as *const (),
        b"glLightModelx" => light_model_x as *const (),
        b"glLightx" => light_x as *const (),
        b"glMaterialx" => material_x as *const (),
        b"glPointSizex" => point_size_x as *const (),
        b"glLineWidthx" => line_width_x as *const (),
        b"glPointParameterx" => point_parameter_x as *const (),
        b"glClearColorx" => clear_color_x as *const (),
        b"glColor4x" => color4_x as *const (),
        b"glNormal3x" => normal3_x as *const (),
        b"glSampleCoveragex" => sample_coverage_x as *const (),
        b"glTexEnvx" => tex_env_x as *const (),
        b"glTexParameterxv" => tex_parameter_xv as *const (),
        b"glClearDepthx" => clear_depth_x as *const (),
        b"glDepthRangex" => depth_range_x as *const (),
        b"glPolygonOffsetx" => polygon_offset_x as *const (),
        b"glClipPlanex" => clip_plane_x as *const (),
        b"glGetClipPlanex" => get_clip_plane_x as *const (),
        b"glFrustumx" => frustum_x as *const (),
        b"glOrthox" => ortho_x as *const (),
        b"glTranslatex" => translate_x as *const (),
        b"glScalex" => scale_x as *const (),
        b"glRotatex" => rotate_x as *const (),
        b"glLoadMatrixx" => load_matrix_x as *const (),
        b"glMultMatrixx" => mult_matrix_x as *const (),
        b"glFogxv" => fog_xv as *const (),
        b"glLightModelxv" => light_model_xv as *const (),
        b"glLightxv" => light_xv as *const (),
        b"glMaterialxv" => material_xv as *const (),
        b"glTexEnvxv" => tex_env_xv as *const (),
        b"glGetLightxv" => get_light_xv as *const (),
        b"glGetMaterialxv" => get_material_xv as *const (),
        b"glGetTexEnvxv" => get_tex_env_xv as *const (),
        b"glGetTexParameterxv" => get_tex_parameter_xv as *const (),
        b"glPointParameterxv" => point_parameter_xv as *const (),
        b"glMultiTexCoord4x" => multi_texcoord_x as *const (),
        _ => return None,
    };
    Some(call as *mut c_void)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixed_conversion_preserves_fraction_and_sign() {
        assert_eq!(fixed(65536), 1.0);
        assert_eq!(fixed(-32768), -0.5);
        assert_eq!(fixed64(i32::MIN), -32768.0);
        assert_eq!(fixed64(i32::MAX), 32768.0 - 1.0 / 65536.0);
    }

    #[test]
    fn parameter_lengths_are_family_specific() {
        for (kind, pnames, len) in [
            (Parameter::Fog, & [0x0b66][..], 4),
            (Parameter::Fog, & [0x0b65, 0x0b62, 0x0b63, 0x0b64][..], 1),
            (Parameter::LightModel, & [0x0b53][..], 4),
            (Parameter::LightModel, & [0x0b52][..], 1),
            (Parameter::Light, & [0x1200, 0x1201, 0x1202, 0x1203][..], 4),
            (Parameter::Light, & [0x1204][..], 3),
            (Parameter::Light, & [0x1205, 0x1206, 0x1207, 0x1208, 0x1209][..], 1),
            (Parameter::Material, & [0x1200, 0x1201, 0x1202, 0x1600, 0x1602][..], 4),
            (Parameter::Material, & [0x1601][..], 1),
            (Parameter::TexEnv, & [0x2201][..], 4),
            (Parameter::TexEnv, & [0x2200, 0x8573, 0x0d1c, 0x8862][..], 1),
            (Parameter::TexParameter, & [0x2800, 0x2801, 0x2802, 0x2803, 0x8191][..], 1),
            (Parameter::TexParameter, & [0x8b9d][..], 4),
            (Parameter::Point, & [0x8129][..], 3),
            (Parameter::Point, & [0x8126, 0x8127, 0x8128][..], 1),
        ] {
            for &pname in pnames {
                assert_eq!(kind.len(pname), len, "{kind:?}: {pname:#x}");
            }
        }
    }

    #[test]
    fn enum_values_are_unscaled_but_numeric_values_are_fixed() {
        for (kind, pname, value) in [
            (Parameter::Fog, 0x0b65, 0x0800), // GL_EXP
            (Parameter::TexEnv, 0x2200, 0x2100), // GL_MODULATE
            (Parameter::TexEnv, 0x8571, 0x0104), // combine: GL_ADD
            (Parameter::TexEnv, 0x8580, 0x1702), // source: GL_TEXTURE
            (Parameter::TexEnv, 0x8598, 0x0302), // operand: GL_SRC_ALPHA
            (Parameter::TexParameter, 0x2801, 0x2601), // GL_LINEAR
            (Parameter::TexParameter, 0x2802, 0x812f), // GL_CLAMP_TO_EDGE
        ] {
            assert_eq!(kind.to_float(pname, value), value as f32);
            assert_eq!(kind.from_float(pname, value as f32), value);
        }
        for (kind, pname) in [
            (Parameter::Fog, 0x0b62),
            (Parameter::Light, 0x1205),
            (Parameter::Material, 0x1601),
            (Parameter::TexEnv, 0x8573), // GL_RGB_SCALE
            (Parameter::TexEnv, 0x0d1c), // GL_ALPHA_SCALE
            (Parameter::TexEnv, 0x8862), // GL_COORD_REPLACE_OES boolean
            (Parameter::TexParameter, 0x8191), // GL_GENERATE_MIPMAP boolean
        ] {
            assert_eq!(kind.to_float(pname, 2 * 65536), 2.0);
            assert_eq!(kind.from_float(pname, 2.0), 2 * 65536);
        }
    }

    #[test]
    fn vector_conversion_reads_only_required_input() {
        let scalar = [65536];
        // SAFETY: shininess requires exactly one readable value.
        let values = unsafe { parameter_values(Parameter::Material, 0x1601, scalar.as_ptr()) };
        assert_eq!(values, [1.0, 0.0, 0.0, 0.0]);
        let direction = [65536, -65536, 32768];
        // SAFETY: spot direction requires exactly three readable values.
        let values = unsafe { parameter_values(Parameter::Light, 0x1204, direction.as_ptr()) };
        assert_eq!(values, [1.0, -1.0, 0.5, 0.0]);
        let ambient = [65536, 32768, 16384, 65536];
        // SAFETY: light model ambient requires exactly four readable values.
        let values = unsafe { parameter_values(Parameter::LightModel, 0x0b53, ambient.as_ptr()) };
        assert_eq!(values, [1.0, 0.5, 0.25, 1.0]);
    }

    #[test]
    fn get_state_lengths_and_enum_conversion() {
        assert_eq!(state_len(0x0b21, 0), 1); // line width
        assert_eq!(state_len(0x0b70, 0), 2); // depth range
        assert_eq!(state_len(0x0b02, 0), 3); // normal
        assert_eq!(state_len(0x0ba2, 0), 4); // viewport
        for pname in 0x0ba6..=0x0ba8 {
            assert_eq!(state_len(pname, 0), 16);
        }
        assert_eq!(state_len(0x86a3, 37), 37);
        assert_eq!(state_len(0x86a3, 0), 0);
        assert!(state_is_enum(0x0b65));
        assert!(state_is_enum(0x84e0));
        assert!(state_is_enum(0x898a));
        assert!(!state_is_enum(0x898b)); // point size array stride is integer
        assert!(!state_is_enum(0x0b52)); // boolean
        assert!(!state_is_enum(0x0ba2)); // integer viewport
    }

    #[test]
    fn abi_shapes_and_dispatch_are_exact() {
        // These assignments check the guest ABI at compile time, without invoking GL.
        let _: unsafe extern "C" fn(i32) = point_size_x;
        let _: unsafe extern "C" fn(i32) = line_width_x;
        let _: unsafe extern "C" fn(i32) = clear_depth_x;
        let _: unsafe extern "C" fn(i32, i32) = depth_range_x;
        let _: unsafe extern "C" fn(u32, i32) = fog_x;
        let _: unsafe extern "C" fn(u32, i32) = light_model_x;
        let _: unsafe extern "C" fn(u32, u32, i32) = light_x;
        let _: unsafe extern "C" fn(u32, u32, i32) = material_x;
        let _: unsafe extern "C" fn(u32, *const i32) = fog_xv;
        let _: unsafe extern "C" fn(u32, *const i32) = light_model_xv;
        let _: unsafe extern "C" fn(u32, *const i32) = point_parameter_xv;
        let _: unsafe extern "C" fn(u32, u32, *const i32) = light_xv;
        let _: unsafe extern "C" fn(u32, u32, *const i32) = material_xv;
        let _: unsafe extern "C" fn(u32, u32, *const i32) = tex_env_xv;
        let _: unsafe extern "C" fn(i32, u8) = sample_coverage_x;
        let _: unsafe extern "C" fn(f32) = clear_depth_f;
        let _: unsafe extern "C" fn(f32, f32) = depth_range_f;
        let _: unsafe extern "C" fn(f32, f32, f32, f32, f32, f32) = ortho_f;
        let _: unsafe extern "C" fn(f32, f32, f32, f32, f32, f32) = frustum_f;
        let _: unsafe extern "C" fn(u32, *const f32) = clip_plane_f;
        let _: unsafe extern "C" fn(u32, *mut f32) = get_clip_plane_f;
        let _: unsafe extern "C" fn(u32, *mut i32) = get_fixed_v;
        for (name, call) in [
            (c"glClearDepthx", clear_depth_x as *const ()),
            (c"glDepthRangex", depth_range_x as *const ()),
            (c"glOrthof", ortho_f as *const ()),
            (c"glFrustumf", frustum_f as *const ()),
            (c"glClearDepthf", clear_depth_f as *const ()),
            (c"glDepthRangef", depth_range_f as *const ()),
            (c"glClipPlanef", clip_plane_f as *const ()),
            (c"glGetClipPlanef", get_clip_plane_f as *const ()),
            (c"glGetFixedv", get_fixed_v as *const ()),
        ] {
            assert_eq!(intercept(name), Some(call as *mut c_void));
        }
        assert_eq!(intercept(c"glNotAFunction"), None);
    }
}
