//! Replacements for the game's Java shell (`com.polarbit.fuse.*`).

use jni_sys::{jobject, jvalue};

use crate::jni::object;

/// A Java method implementation: receiver (or class for static methods) and decoded arguments.
pub(crate) type Handler = fn(this: jobject, args: &[jvalue]) -> jvalue;

fn string(s: &str) -> jvalue {
    jvalue {
        l: object::new_string(s),
    }
}

fn empty_string(_this: jobject, _args: &[jvalue]) -> jvalue {
    string("")
}

fn country_code(_this: jobject, _args: &[jvalue]) -> jvalue {
    string("us")
}

fn model_name(_this: jobject, _args: &[jvalue]) -> jvalue {
    string("PC")
}

fn platform_version(_this: jobject, _args: &[jvalue]) -> jvalue {
    string("4.4.4")
}

fn launch_url(_this: jobject, args: &[jvalue]) -> jvalue {
    // SAFETY: LaunchUrl's only argument is a String handle.
    let url = unsafe { args.first().and_then(|arg| object::string(arg.l)) };
    tracing::info!(?url, "ignored LaunchUrl");
    crate::jni::VOID
}

fn nothing(_this: jobject, _args: &[jvalue]) -> jvalue {
    crate::jni::VOID
}

fn no(_this: jobject, _args: &[jvalue]) -> jvalue {
    jvalue { z: false }
}

const WINDOW_TITLE: &str = "Raging Thunder 2";

fn egl_create(_this: jobject, args: &[jvalue]) -> jvalue {
    // SAFETY: the game passes an int array of requested config attributes, terminated by EGL_NONE.
    let requested =
        unsafe { args.first().and_then(|arg| object::array_bytes(arg.l)) }.map(|(bytes, _)| {
            bytes
                .as_chunks::<4>()
                .0
                .iter()
                .map(|w| i32::from_le_bytes(*w))
                .collect::<Vec<_>>()
        });
    tracing::info!(?requested, "creating window and GLES 2 context");
    let ok = crate::video::open(
        WINDOW_TITLE,
        crate::video::WINDOW_SIZE.0,
        crate::video::WINDOW_SIZE.1,
    )
    .map_err(|error| tracing::error!(%error, "cannot create context"))
    .is_ok();
    jvalue { z: ok }
}

fn egl_activate(_this: jobject, args: &[jvalue]) -> jvalue {
    // SAFETY: Activate takes a single boolean.
    let active = unsafe { args.first().is_some_and(|arg| arg.z) };
    let ok = crate::video::with(|video| video.activate(active))
        .map_err(|error| tracing::error!(%error, "activate failed"))
        .is_ok();
    jvalue { z: ok }
}

fn egl_update_display(_this: jobject, _args: &[jvalue]) -> jvalue {
    let ok = crate::video::with(|video| video.swap()).is_ok();
    jvalue { z: ok }
}

pub(crate) fn lookup(class: &str, name: &str) -> Option<Handler> {
    let class = class.strip_prefix("com/polarbit/fuse/")?;
    let handler: Handler = match (class, name) {
        ("FuseSystem", "GetIMEI" | "GetPhoneNr" | "GetOperatorName") => empty_string,
        ("FuseSystem", "GetCountryCode") => country_code,
        ("FuseSystem", "GetModelName") => model_name,
        ("FuseSystem", "GetPlatformVersion") => platform_version,
        ("FuseSystem", "LaunchUrl") => launch_url,
        ("FuseSensor", "ActivateAccelerometer") => no,
        ("FuseEgl", "EglCreate") => egl_create,
        ("FuseEgl", "Activate") => egl_activate,
        ("FuseEgl", "EglUpdateDisplay") => egl_update_display,
        ("Fuse", "TelemetrySubmit") => nothing,
        _ => return None,
    };
    Some(handler)
}
