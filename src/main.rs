mod audio;
mod bionic;
mod game;
mod gamepad;
mod gl;
mod input;
mod java;
mod jni;
mod loader;
mod paletted;
mod s3tc;
mod sprites;
mod video;

use std::path::{Path, PathBuf};

use stable_eyre::eyre::{Result, WrapErr, bail, eyre};
use tracing_subscriber::EnvFilter;

use crate::game::Game;
use crate::input::Input;

/// Frames to run before taking `RT2_SCREENSHOT`.
const SCREENSHOT_FRAME: u64 = 120;

fn save_dir() -> Result<PathBuf> {
    let base = std::env::var_os("XDG_DATA_HOME")
        .filter(|d| !d.is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| Path::new(&h).join(".local/share")))
        .ok_or_else(|| eyre!("neither XDG_DATA_HOME nor HOME is set"))?;
    let dir = base.join("rt2");
    std::fs::create_dir_all(&dir).wrap_err_with(|| format!("create {}", dir.display()))?;
    std::fs::canonicalize(&dir).wrap_err_with(|| format!("resolve {}", dir.display()))
}

fn write_png(path: &Path, width: u32, height: u32, rgba: &[u8]) -> Result<()> {
    let file =
        std::fs::File::create(path).wrap_err_with(|| format!("create {}", path.display()))?;
    let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header()?;
    writer.write_image_data(rgba)?;
    Ok(())
}

fn run(apk: &Path, library: &Path) -> Result<()> {
    let save = save_dir()?;
    tracing::info!(path = %save.display(), "save directory");
    let mut game = Game::start(library, apk, &save)?;
    tracing::info!("game created");

    let screenshot = std::env::var_os("RT2_SCREENSHOT").map(PathBuf::from);
    let screenshot_frame = std::env::var("RT2_SCREENSHOT_FRAME")
        .ok()
        .and_then(|f| f.parse().ok())
        .unwrap_or(SCREENSHOT_FRAME);
    if screenshot.is_some() {
        video::with(|v| v.capture_at(screenshot_frame))?;
    }

    let mut input = Input::new()?;
    loop {
        let events = video::with(|v| v.take_events())??;
        for event in &events {
            if input.handle(&mut game, event) {
                return Ok(());
            }
        }
        input.update(&mut game);
        if !game.frame() {
            break;
        }
        crate::audio::update()?;
        if screenshot.is_some() && video::with(|v| v.frame())? > screenshot_frame {
            break;
        }
    }

    if let Some(path) = screenshot {
        let Some((width, height, pixels)) = video::with(|v| v.take_capture())? else {
            bail!("no frame was captured");
        };
        write_png(&path, width, height, &pixels)?;
        tracing::info!(path = %path.display(), width, height, "wrote screenshot");
    }
    tracing::info!(frames = video::with(|v| v.frame())?, "game finished");
    Ok(())
}

fn main() -> Result<()> {
    stable_eyre::install()?;
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .with_writer(std::io::stderr)
        .init();

    let args: Vec<PathBuf> = std::env::args_os().skip(1).map(PathBuf::from).collect();
    let [apk, library] = args.as_slice() else {
        bail!("usage: rt2 <game.apk> <librthunder2lite.so>");
    };
    let apk = std::fs::canonicalize(apk).wrap_err_with(|| format!("resolve {}", apk.display()))?;
    jni::init();
    audio::init()?;
    let result = run(&apk, library);
    audio::shutdown();
    result
}
