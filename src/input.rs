//! Keyboard and pointer translation into the game's `Jni.OnEvent` protocol.

use sdl3::event::Event;
use sdl3::keyboard::Scancode;
use sdl3::mouse::MouseButton;

use crate::game::Game;

/// Android key codes the game's key table understands.
mod key {
    pub(super) const BACK: i32 = 4;
    pub(super) const HOME: i32 = 3;
    pub(super) const DPAD_UP: i32 = 19;
    pub(super) const DPAD_DOWN: i32 = 20;
    pub(super) const DPAD_LEFT: i32 = 21;
    pub(super) const DPAD_RIGHT: i32 = 22;
    pub(super) const ENTER: i32 = 66;
    pub(super) const DEL: i32 = 67;
    pub(super) const MENU: i32 = 82;
    pub(super) const ESCAPE: i32 = 111;
    pub(super) const SPACE: i32 = 62;
    pub(super) const TAB: i32 = 61;
    pub(super) const A: i32 = 29;
    pub(super) const ZERO: i32 = 7;
}

/// Android `MotionEvent` actions, which the game's touch decoder expects.
mod action {
    pub(super) const DOWN: i32 = 0;
    pub(super) const UP: i32 = 1;
    pub(super) const MOVE: i32 = 2;
}

pub(crate) fn android_keycode(scancode: Scancode) -> Option<i32> {
    let raw = scancode as i32;
    if (Scancode::A as i32..=Scancode::Z as i32).contains(&raw) {
        return Some(key::A + raw - Scancode::A as i32);
    }
    if (Scancode::_0 as i32..=Scancode::_9 as i32).contains(&raw) {
        return Some(key::ZERO + raw - Scancode::_0 as i32);
    }
    let code = match scancode {
        Scancode::Return | Scancode::KpEnter => key::ENTER,
        Scancode::Escape => key::ESCAPE,
        Scancode::Backspace => key::DEL,
        Scancode::Tab => key::TAB,
        Scancode::Space => key::SPACE,
        Scancode::Up => key::DPAD_UP,
        Scancode::Down => key::DPAD_DOWN,
        Scancode::Left => key::DPAD_LEFT,
        Scancode::Right => key::DPAD_RIGHT,
        Scancode::Application => key::MENU,
        Scancode::AcBack => key::BACK,
        Scancode::AcHome => key::HOME,
        _ => return None,
    };
    Some(code)
}

pub(crate) struct Input {
    pressed: bool,
}

impl Input {
    pub(crate) const fn new() -> Self {
        Self { pressed: false }
    }

    /// Forwards one SDL event; returns true when the game window should close.
    pub(crate) fn handle(&mut self, game: &mut Game, event: &Event) -> bool {
        match *event {
            Event::Quit { .. } => return true,
            Event::KeyDown {
                scancode: Some(scancode),
                repeat,
                ..
            } => {
                if let Some(code) = android_keycode(scancode) {
                    game.key(code, !repeat);
                }
            }
            Event::KeyUp {
                scancode: Some(scancode),
                ..
            } => {
                if let Some(code) = android_keycode(scancode) {
                    game.key(code, false);
                }
            }
            Event::MouseButtonDown {
                mouse_btn: MouseButton::Left,
                x,
                y,
                ..
            } => {
                self.pressed = true;
                game.touch(x as i32, y as i32, action::DOWN);
            }
            Event::MouseButtonUp {
                mouse_btn: MouseButton::Left,
                x,
                y,
                ..
            } => {
                self.pressed = false;
                game.touch(x as i32, y as i32, action::UP);
            }
            Event::MouseMotion { x, y, .. } if self.pressed => {
                game.touch(x as i32, y as i32, action::MOVE);
            }
            _ => {}
        }
        false
    }
}
