# rt2

A Linux compatibility layer for the original Android x86 library of **Raging Thunder 2**. The loader is written in Rust, with a small C shim for variadic JNI calls.

You must supply a legally obtained APK containing `lib/x86/librthunder2lite.so`. This repository does not include or redistribute the game library, APK, or assets. The wrapper's MIT license does not apply to the original game.

## Status

Menus and races render through GLES 1.1. Keyboard, mouse, SDL3 gamepads, audio playback and persistent profiles are implemented. Audio uses the game's own PCM mixer.

Ads, Facebook integration, billing and networking remain disabled. This is an unofficial port, unaffiliated with the game's developers.

## Build

The build targets `i686-unknown-linux-gnu`. The Fedora 44 container supplies the Rust toolchain and 32-bit development libraries; run the resulting executable on the host desktop.

You need Podman and Just. The host also needs 32-bit SDL3, EGL, GLES and graphics-driver libraries. On Fedora, the runtime packages are `SDL3.i686`, `mesa-libEGL.i686`, `libglvnd-gles.i686` and `mesa-dri-drivers.i686`.

```sh
podman build -t rt3-build -f .devcontainer/Containerfile .
podman run --rm \
  -v "$PWD:/workspaces/rt2:Z" -v rt3-cargo:/root/.cargo \
  -w /workspaces/rt2 rt3-build just build
```

If the project's devcontainer is already running, `just build` uses it directly.

## Supply the APK and run

```sh
just --set apk /path/to/game.apk extract
./target/i686-unknown-linux-gnu/debug/rt2 [--width W] [--height H] /path/to/game.apk game/librthunder2lite.so
```

The game runs fullscreen by default. `--width` and `--height` open a window at that size; the unspecified dimension falls back to 960x544.

The APK must contain the x86 library, not just an ARM library. The free 1.0.16 and 1.0.17 builds have been exercised during development.

## Controls

Gamepad bindings use SDL's physical button positions, independent of printed button labels.

| Gamepad input | Action |
| --- | --- |
| Left stick | Steering; menu navigation when using button controls |
| D-pad | Steering and menu navigation |
| South / bottom face button | Accelerate / confirm |
| West / left face button | Brake / reverse |
| North / top face button | Boost |
| East / right face button, Start or Back | Back / pause |
| Left shoulder or trigger | Brake; the shoulder also handles menu paging |
| Right shoulder or trigger | Accelerate; the shoulder also handles menu paging |

Controllers support hot-plugging. The left stick follows the game's steering mode: virtual tilt input for tilt controls, or directional keys for button controls.

Keyboard bindings are arrows or WASD for directions, Space or Enter for accelerate/confirm, Ctrl for brake, Shift for boost, and Escape for back/pause. Mouse clicks and drags act as touch input.

## Saves

Profiles, career progress and settings are stored in `$XDG_DATA_HOME/rt2`, or `~/.local/share/rt2` when that variable is unset. The main profile is `rt2save.dat`.

The game loads its native save format and flushes it during normal shutdown, including closing the window. These are profile saves, not mid-race snapshots. Killing the process forcibly bypasses shutdown.

## Checks

```sh
podman run --rm \
  -v "$PWD:/workspaces/rt2:Z" -v rt3-cargo:/root/.cargo \
  -w /workspaces/rt2 rt3-build just check

# After building and extracting the supplied APK:
just --no-deps test-saves
```

The save check runs two game processes with an isolated temporary data directory and verifies the saved profile survives a restart. It does not touch your normal profile.

## Third-party code

`vendor/texture2ddecoder` contains the upstream 0.1.2 decoder with a fix for DXT5 alpha-index truncation on 32-bit targets. Its copyright and MIT/Apache-2.0 license notices are retained.

The control mappings and virtual steering approach follow the [Raging Thunder 2 Vita port](https://github.com/MetalSyntax/Raging-Thunder-2-vita).
