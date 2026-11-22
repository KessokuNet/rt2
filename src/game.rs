//! The game's native entry points and the events it understands.

use std::ffi::c_void;
use std::os::unix::ffi::OsStrExt;
use std::path::Path;

use jni_sys::{JNIEnv, JavaVM, jclass, jobject, jstring};
use stable_eyre::eyre::Result;

use crate::jni::object;
use crate::loader;

pub(crate) type JniOnLoad = unsafe extern "C" fn(vm: *mut JavaVM, reserved: *mut c_void) -> i32;
pub(crate) type FuseOnInit = unsafe extern "C" fn(
    env: *mut JNIEnv,
    class: jclass,
    activity: jobject,
    task: jobject,
    sensor: jobject,
    utils: jobject,
    audio: jobject,
    egl: jobject,
);
pub(crate) type OnCreate = unsafe extern "C" fn(
    env: *mut JNIEnv,
    class: jclass,
    apk_path: jstring,
    save_path: jstring,
) -> i32;
pub(crate) type OnEvent =
    unsafe extern "C" fn(env: *mut JNIEnv, class: jclass, i32, i32, i32, i32, i32) -> i32;

/// Event families of `Jni.OnEvent`; the second argument selects the handler.
pub(crate) mod event {
    /// Lifecycle: frame tick, pause, resume.
    pub(crate) const FRAME: i32 = 0;
    /// Input: keys, touch, joystick.
    pub(crate) const INPUT: i32 = 1;
    /// Display size in pixels.
    pub(crate) const DISPLAY: i32 = 3;
}

pub(crate) struct Game {
    env: *mut JNIEnv,
    class: jclass,
    on_load: JniOnLoad,
    on_init: FuseOnInit,
    on_create: OnCreate,
    on_event: OnEvent,
}

impl Game {
    /// Resolves the exports and runs `JNI_OnLoad` and `FuseOnInit`.
    pub(crate) fn start(library: &Path, apk: &Path, save: &Path) -> Result<Self> {
        let game = loader::load(library, |lookup| {
            fn cast<F: Copy>(addr: usize) -> F {
                // SAFETY: the caller names a signature that matches the export.
                unsafe { std::mem::transmute_copy(&addr) }
            }
            let event: OnEvent = cast(lookup("Java_com_polarbit_fuse_Jni_OnEvent")?);
            let create: OnCreate = cast(lookup("Java_com_polarbit_fuse_Jni_OnCreate")?);
            let init: FuseOnInit = cast(lookup("Java_com_polarbit_fuse_MainTask_FuseOnInit")?);
            let load: JniOnLoad = cast(lookup("JNI_OnLoad")?);
            Ok((load, init, create, event))
        })?;
        let (on_load, on_init, on_create, on_event) = game;

        let mut game = Self {
            env: crate::jni::env(),
            class: object::class("com/polarbit/fuse/Jni"),
            on_load,
            on_init,
            on_create,
            on_event,
        };

        // SAFETY: these calls follow the order MainTask uses on Android.
        unsafe {
            let version = (game.on_load)(crate::jni::vm(), std::ptr::null_mut());
            tracing::info!(version = format_args!("{version:#x}"), "JNI_OnLoad");

            (game.on_init)(
                game.env,
                object::class("com/polarbit/fuse/MainTask"),
                object::new_instance("android/app/Activity"),
                object::new_instance("com/polarbit/fuse/MainTask"),
                object::new_instance("com/polarbit/fuse/FuseSensor"),
                object::new_instance("com/polarbit/fuse/FuseSystem"),
                object::new_instance("com/polarbit/fuse/FuseAudio"),
                object::new_instance("com/polarbit/fuse/FuseEgl"),
            );
        }

        let (width, height) = crate::video::WINDOW_SIZE;
        game.event(event::DISPLAY, 0, width as i32, height as i32, 0);

        let apk_path = object::new_string(apk.as_os_str().as_bytes());
        let save_path = {
            let mut bytes = save.as_os_str().as_bytes().to_vec();
            bytes.push(b'/');
            object::new_string(bytes)
        };
        // SAFETY: the game created both objects through the fake JNI.
        if unsafe { (game.on_create)(game.env, game.class, apk_path, save_path) } == 0 {
            stable_eyre::eyre::bail!("OnCreate failed");
        }
        game.keydown(1);
        Ok(game)
    }

    /// Sends one raw event.
    pub(crate) fn event(&mut self, kind: i32, sub: i32, a: i32, b: i32, c: i32) -> i32 {
        // SAFETY: the game interprets the five integers as documented by PEventQueue::OnEvent.
        unsafe { (self.on_event)(self.env, self.class, kind, sub, a, b, c) }
    }

    /// Runs one frame; returns false when the game wants to quit.
    pub(crate) fn frame(&mut self) -> bool {
        self.event(event::FRAME, 1, 0, 0, 0) != 0
    }

    pub(crate) fn key(&mut self, code: i32, down: bool) {
        self.event(event::INPUT, 0, code, 0, i32::from(down));
    }

    fn keydown(&mut self, code: i32) {
        self.key(code, true);
    }

    pub(crate) fn touch(&mut self, x: i32, y: i32, action: i32) {
        self.event(event::INPUT, 1, x, y, action);
    }
}
