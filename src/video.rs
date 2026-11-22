//! The game's window and GLES 2 context, standing in for `FuseEgl`.

use std::cell::RefCell;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use sdl3::event::Event;
use sdl3::video::{GLContext, GLProfile, SwapInterval, Window};
use sdl3::{Sdl, VideoSubsystem};
use stable_eyre::eyre::{Result, eyre};

fn gl_get_string(name: u32) -> Option<String> {
    type GlGetString = unsafe extern "C" fn(u32) -> *const libc::c_char;
    let get = crate::gl::host_symbol::<GlGetString>(c"glGetString")?;
    // SAFETY: the host GL context is current.
    unsafe {
        let value = get(name);
        (!value.is_null()).then(|| {
            std::ffi::CStr::from_ptr(value)
                .to_string_lossy()
                .into_owned()
        })
    }
}

fn log_gl_info() {
    const GL_VENDOR: u32 = 0x1f00;
    const GL_RENDERER: u32 = 0x1f01;
    const GL_VERSION: u32 = 0x1f02;
    const GL_EXTENSIONS: u32 = 0x1f03;
    tracing::info!(
        vendor = ?gl_get_string(GL_VENDOR),
        renderer = ?gl_get_string(GL_RENDERER),
        version = ?gl_get_string(GL_VERSION),
        "GL context"
    );
    if let Some(extensions) = gl_get_string(GL_EXTENSIONS) {
        let wanted = ["s3tc", "pvrtc", "atitc", "ETC1", "compressed"];
        let found: Vec<&str> = extensions
            .split(' ')
            .filter(|ext| wanted.iter().any(|w| ext.contains(w)))
            .collect();
        tracing::info!(?found, "compressed texture extensions");
    }
}

const DEFAULT_WIDTH: u32 = 960;
const DEFAULT_HEIGHT: u32 = 544;

static FULLSCREEN: AtomicBool = AtomicBool::new(true);
static REQUESTED_WIDTH: AtomicU32 = AtomicU32::new(DEFAULT_WIDTH);
static REQUESTED_HEIGHT: AtomicU32 = AtomicU32::new(DEFAULT_HEIGHT);
static RENDER_WIDTH: AtomicU32 = AtomicU32::new(DEFAULT_WIDTH);
static RENDER_HEIGHT: AtomicU32 = AtomicU32::new(DEFAULT_HEIGHT);

/// Fullscreen unless the caller asks for an explicit window size.
pub(crate) fn set_window(width: Option<u32>, height: Option<u32>) {
    if width.is_none() && height.is_none() {
        return;
    }
    let width = width.unwrap_or(DEFAULT_WIDTH);
    let height = height.unwrap_or(DEFAULT_HEIGHT);
    REQUESTED_WIDTH.store(width, Ordering::Relaxed);
    REQUESTED_HEIGHT.store(height, Ordering::Relaxed);
    RENDER_WIDTH.store(width, Ordering::Relaxed);
    RENDER_HEIGHT.store(height, Ordering::Relaxed);
    FULLSCREEN.store(false, Ordering::Relaxed);
}

/// Resolves the fullscreen drawable size before the game creates its surface.
pub(crate) fn prepare() -> Result<()> {
    if !fullscreen() {
        return Ok(());
    }
    let sdl = sdl3::init()?;
    let video = sdl.video()?;
    let Some(display) = video.displays()?.into_iter().next() else {
        return Ok(());
    };
    let bounds = display.get_bounds()?;
    let (width, height) = (bounds.width(), bounds.height());
    if width > 0 && height > 0 {
        REQUESTED_WIDTH.store(width, Ordering::Relaxed);
        REQUESTED_HEIGHT.store(height, Ordering::Relaxed);
        RENDER_WIDTH.store(width, Ordering::Relaxed);
        RENDER_HEIGHT.store(height, Ordering::Relaxed);
    }
    Ok(())
}

/// Size the game is told about through the display-size event.
pub(crate) fn window_size() -> (u32, u32) {
    (
        RENDER_WIDTH.load(Ordering::Relaxed),
        RENDER_HEIGHT.load(Ordering::Relaxed),
    )
}

pub(crate) fn requested_size() -> (u32, u32) {
    (
        REQUESTED_WIDTH.load(Ordering::Relaxed),
        REQUESTED_HEIGHT.load(Ordering::Relaxed),
    )
}

fn fullscreen() -> bool {
    FULLSCREEN.load(Ordering::Relaxed)
}

pub(crate) struct Video {
    sdl: Sdl,
    window: Window,
    gl: GLContext,
    frame: u64,
    capture_at: Option<u64>,
    captured: Option<(u32, u32, Vec<u8>)>,
}

impl Video {
    pub(crate) fn open(title: &str, width: u32, height: u32) -> Result<Self> {
        let sdl = sdl3::init()?;
        let video = sdl.video()?;
        Self::configure_gl(&video)?;

        let mut builder = video.window(title, width, height);
        builder.opengl().resizable().high_pixel_density();
        if fullscreen() {
            builder.fullscreen();
        }
        let window = builder.build()?;
        if !fullscreen() {
            let (width, height) = window.size_in_pixels();
            if width > 0 && height > 0 {
                RENDER_WIDTH.store(width, Ordering::Relaxed);
                RENDER_HEIGHT.store(height, Ordering::Relaxed);
            }
        }
        let gl = window.gl_create_context()?;
        window.gl_make_current(&gl)?;
        crate::sprites::reset_textures();
        video.gl_set_swap_interval(SwapInterval::VSync)?;
        log_gl_info();

        Ok(Self {
            sdl,
            window,
            gl,
            frame: 0,
            capture_at: None,
            captured: None,
        })
    }

    fn configure_gl(video: &VideoSubsystem) -> Result<()> {
        let attr = video.gl_attr();
        attr.set_context_profile(GLProfile::GLES);
        attr.set_context_major_version(1);
        attr.set_context_minor_version(1);
        attr.set_double_buffer(true);
        attr.set_depth_size(16);
        attr.set_multisample_buffers(1);
        attr.set_multisample_samples(4);
        Ok(())
    }

    pub(crate) fn activate(&self, active: bool) -> Result<()> {
        let video = self.sdl.video()?;
        if active {
            self.window.gl_make_current(&self.gl)?;
        } else {
            video.gl_release_current_context()?;
        }
        Ok(())
    }

    pub(crate) fn swap(&mut self) {
        if self.capture_at == Some(self.frame + 1) {
            self.captured = self.read_pixels().ok();
        }
        self.window.gl_swap_window();
        self.frame += 1;
    }

    /// Reads the back buffer on the given frame, before it is swapped away.
    pub(crate) fn capture_at(&mut self, frame: u64) {
        self.capture_at = Some(frame);
    }

    pub(crate) fn take_capture(&mut self) -> Option<(u32, u32, Vec<u8>)> {
        self.captured.take()
    }

    pub(crate) fn frame(&self) -> u64 {
        self.frame
    }

    /// Reads the back buffer into a top-down RGBA image.
    pub(crate) fn read_pixels(&self) -> Result<(u32, u32, Vec<u8>)> {
        type GlReadPixels = unsafe extern "C" fn(i32, i32, i32, i32, u32, u32, *mut u8);
        const GL_RGBA: u32 = 0x1908;
        const GL_UNSIGNED_BYTE: u32 = 0x1401;

        let (width, height) = self.window.size_in_pixels();
        type GlGetIntegerv = unsafe extern "C" fn(u32, *mut i32);
        const GL_VIEWPORT: u32 = 0x0ba2;
        if let Some(get) = crate::gl::host_symbol::<GlGetIntegerv>(c"glGetIntegerv") {
            let mut viewport = [0i32; 4];
            // SAFETY: the current context writes four viewport integers.
            unsafe { get(GL_VIEWPORT, viewport.as_mut_ptr()) };
            tracing::info!(?viewport, "game viewport");
        }
        let mut pixels = vec![0u8; width as usize * height as usize * 4];
        let read = crate::gl::host_symbol::<GlReadPixels>(c"glReadPixels")
            .ok_or_else(|| eyre!("host GL library has no glReadPixels"))?;
        // SAFETY: the current context writes RGBA8 pixels into the window-sized buffer.
        unsafe {
            read(
                0,
                0,
                width as i32,
                height as i32,
                GL_RGBA,
                GL_UNSIGNED_BYTE,
                pixels.as_mut_ptr(),
            );
        }

        let stride = width as usize * 4;
        let mut flipped = vec![0u8; pixels.len()];
        for row in 0..height as usize {
            let src = &pixels[(height as usize - 1 - row) * stride..][..stride];
            flipped[row * stride..][..stride].copy_from_slice(src);
        }
        Ok((width, height, flipped))
    }

    pub(crate) fn gamepad_subsystem(&self) -> Result<sdl3::GamepadSubsystem> {
        Ok(self.sdl.gamepad()?)
    }

    pub(crate) fn take_events(&mut self) -> Result<Vec<Event>> {
        let mut pump = self.sdl.event_pump()?;
        pump.pump_events();
        Ok(pump.poll_iter().collect())
    }
}

thread_local! {
    static VIDEO: RefCell<Option<Video>> = const { RefCell::new(None) };
}

/// Runs `f` with the open window, failing if `open` has not been called.
pub(crate) fn with<R>(f: impl FnOnce(&mut Video) -> R) -> Result<R> {
    VIDEO.with_borrow_mut(|video| {
        let video = video.as_mut().ok_or_else(|| eyre!("no window is open"))?;
        Ok(f(video))
    })
}

pub(crate) fn open(title: &str, width: u32, height: u32) -> Result<()> {
    let video = Video::open(title, width, height)?;
    VIDEO.with_borrow_mut(|slot| *slot = Some(video));
    Ok(())
}
