//! SDL controls translated into the game's `Jni.OnEvent` protocol.

use std::collections::{BTreeSet, HashSet};

use sdl3::event::{Event, WindowEvent};
use sdl3::keyboard::Scancode;
use sdl3::mouse::MouseButton;
use stable_eyre::eyre::Result;

use crate::game::Game;
use crate::gamepad::{Gamepads, PadInput};

pub(crate) mod key {
    pub(crate) const BACK: i32 = 4;
    pub(crate) const HOME: i32 = 3;
    pub(crate) const DPAD_UP: i32 = 19;
    pub(crate) const DPAD_DOWN: i32 = 20;
    pub(crate) const DPAD_LEFT: i32 = 21;
    pub(crate) const DPAD_RIGHT: i32 = 22;
    pub(crate) const DPAD_CENTER: i32 = 23;
    pub(crate) const DEL: i32 = 67;
    pub(crate) const MENU: i32 = 82;
    pub(crate) const TAB: i32 = 61;
    pub(crate) const A: i32 = 29;
    pub(crate) const ZERO: i32 = 7;
    pub(crate) const BUTTON_X: i32 = 99;
    pub(crate) const BUTTON_Y: i32 = 100;
    pub(crate) const BUTTON_L1: i32 = 102;
    pub(crate) const BUTTON_R1: i32 = 103;
}

mod action {
    pub(super) const DOWN: i32 = 0;
    pub(super) const UP: i32 = 1;
    pub(super) const MOVE: i32 = 2;
}

pub(crate) fn android_keycode(scancode: Scancode) -> Option<i32> {
    let code = match scancode {
        Scancode::Return | Scancode::KpEnter | Scancode::Space => key::DPAD_CENTER,
        Scancode::Escape | Scancode::AcBack => key::BACK,
        Scancode::Backspace => key::DEL,
        Scancode::Tab => key::TAB,
        Scancode::Up | Scancode::W => key::DPAD_UP,
        Scancode::Down | Scancode::S => key::DPAD_DOWN,
        Scancode::Left | Scancode::A => key::DPAD_LEFT,
        Scancode::Right | Scancode::D => key::DPAD_RIGHT,
        Scancode::LCtrl | Scancode::RCtrl => key::BUTTON_X,
        Scancode::LShift | Scancode::RShift => key::BUTTON_Y,
        Scancode::Application => key::MENU,
        Scancode::AcHome => key::HOME,
        Scancode::_0 => key::ZERO,
        _ => {
            let raw = scancode as i32;
            if (Scancode::A as i32..=Scancode::Z as i32).contains(&raw) {
                key::A + raw - Scancode::A as i32
            } else if (Scancode::_1 as i32..=Scancode::_9 as i32).contains(&raw) {
                key::ZERO + 1 + raw - Scancode::_1 as i32
            } else {
                return None;
            }
        }
    };
    Some(code)
}

#[derive(Default)]
struct Keys {
    keyboard: HashSet<Scancode>,
    sent: BTreeSet<i32>,
}

impl Keys {
    fn keyboard(&mut self, scancode: Scancode, down: bool, repeat: bool) {
        if repeat {
            return;
        }
        if down {
            self.keyboard.insert(scancode);
        } else {
            self.keyboard.remove(&scancode);
        }
    }

    fn steering(&self) -> Option<f32> {
        let left = self.keyboard.contains(&Scancode::Left) || self.keyboard.contains(&Scancode::A);
        let right =
            self.keyboard.contains(&Scancode::Right) || self.keyboard.contains(&Scancode::D);
        (left || right).then_some(i32::from(right) as f32 - i32::from(left) as f32)
    }

    fn transitions(&mut self, pad: &BTreeSet<i32>) -> Vec<(i32, bool)> {
        let current: BTreeSet<i32> = self
            .keyboard
            .iter()
            .filter_map(|&s| android_keycode(s))
            .chain(pad.iter().copied())
            .collect();
        let released: BTreeSet<i32> = self.sent.difference(&current).copied().collect();
        let mut events: Vec<(i32, bool)> = released.iter().map(|&key| (key, false)).collect();
        events.extend(current.difference(&self.sent).map(|&key| (key, true)));
        // Xperia face and shoulder keys share engine action bits.
        for aliases in [
            [key::DPAD_CENTER, key::BUTTON_R1],
            [key::BUTTON_X, key::BUTTON_L1],
        ] {
            if aliases.iter().any(|key| released.contains(key)) {
                events.extend(
                    aliases
                        .into_iter()
                        .filter(|key| current.contains(key) && self.sent.contains(key))
                        .map(|key| (key, true)),
                );
            }
        }
        self.sent = current;
        events
    }
}

fn acceleration(steer: f32) -> [i32; 3] {
    const GRAVITY: f32 = 9.80665;
    const SENSOR_SCALE: f32 = 6553.0;
    let lean = 50.0f32.to_radians();
    let angle = steer.clamp(-1.0, 1.0) * 35.0f32.to_radians();
    [
        (GRAVITY * lean.sin() * angle.cos() * SENSOR_SCALE) as i32,
        (GRAVITY * lean.sin() * angle.sin() * SENSOR_SCALE) as i32,
        (GRAVITY * lean.cos() * SENSOR_SCALE) as i32,
    ]
}

pub(crate) struct Input {
    pressed: bool,
    pointer: (i32, i32),
    focused: bool,
    keys: Keys,
    pads: Gamepads,
    steering_mode: Option<(bool, bool)>,
}

impl Input {
    pub(crate) fn new() -> Result<Self> {
        let subsystem = crate::video::with(|video| video.gamepad_subsystem())??;
        Ok(Self {
            pressed: false,
            pointer: (0, 0),
            focused: true,
            keys: Keys::default(),
            pads: Gamepads::new(subsystem)?,
            steering_mode: None,
        })
    }

    pub(crate) fn update(&mut self, game: &mut Game) {
        let tilt = game.uses_tilt_steering();
        let sensor_active = crate::java::accelerometer_enabled();
        if self.steering_mode != Some((tilt, sensor_active)) {
            tracing::debug!(tilt, sensor_active, "steering mode");
            self.steering_mode = Some((tilt, sensor_active));
        }
        let pad = if self.focused {
            self.pads.poll(tilt)
        } else {
            PadInput::default()
        };
        for (code, down) in self.keys.transitions(&pad.keys) {
            game.key(code, down);
        }
        if sensor_active {
            game.accelerometer(acceleration(self.keys.steering().unwrap_or(pad.steer)));
        }
    }

    pub(crate) fn handle(&mut self, game: &mut Game, event: &Event) -> bool {
        self.pads.handle_event(event);
        match *event {
            Event::Quit { .. }
            | Event::Window {
                win_event: WindowEvent::CloseRequested,
                ..
            } => return true,
            Event::KeyDown {
                scancode: Some(scancode),
                repeat,
                ..
            } if self.focused => {
                self.keys.keyboard(scancode, true, repeat);
            }
            Event::KeyUp {
                scancode: Some(scancode),
                ..
            } => {
                self.keys.keyboard(scancode, false, false);
            }
            Event::Window {
                win_event: WindowEvent::FocusLost,
                ..
            } => {
                self.focused = false;
                self.keys.keyboard.clear();
                if self.pressed {
                    game.touch(self.pointer.0, self.pointer.1, action::UP);
                    self.pressed = false;
                }
            }
            Event::Window {
                win_event: WindowEvent::FocusGained,
                ..
            } => self.focused = true,
            Event::MouseButtonDown {
                mouse_btn: MouseButton::Left,
                x,
                y,
                ..
            } if self.focused => {
                self.pointer = (x as i32, y as i32);
                self.pressed = true;
                game.touch(self.pointer.0, self.pointer.1, action::DOWN);
            }
            Event::MouseButtonUp {
                mouse_btn: MouseButton::Left,
                x,
                y,
                ..
            } if self.pressed => {
                self.pointer = (x as i32, y as i32);
                self.pressed = false;
                game.touch(self.pointer.0, self.pointer.1, action::UP);
            }
            Event::MouseMotion { x, y, .. } if self.pressed => {
                self.pointer = (x as i32, y as i32);
                game.touch(self.pointer.0, self.pointer.1, action::MOVE);
            }
            _ => {}
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::{Keys, acceleration, android_keycode, key};
    use sdl3::keyboard::Scancode;
    use std::collections::BTreeSet;

    #[test]
    fn held_arrows_ignore_auto_repeat_but_manual_taps_still_transition() {
        let mut keys = Keys::default();
        let pad = BTreeSet::new();
        keys.keyboard(Scancode::Left, true, false);
        assert_eq!(keys.transitions(&pad), [(key::DPAD_LEFT, true)]);
        for _ in 0..5 {
            keys.keyboard(Scancode::Left, true, true);
            assert!(keys.transitions(&pad).is_empty());
            assert_eq!(keys.steering(), Some(-1.0));
        }
        keys.keyboard(Scancode::Left, false, false);
        assert_eq!(keys.transitions(&pad), [(key::DPAD_LEFT, false)]);
        keys.keyboard(Scancode::Left, true, false);
        assert_eq!(keys.transitions(&pad), [(key::DPAD_LEFT, true)]);
    }

    #[test]
    fn releasing_one_binding_does_not_release_another_held_binding() {
        let mut keys = Keys::default();
        keys.keyboard(Scancode::Left, true, false);
        keys.keyboard(Scancode::A, true, false);
        assert_eq!(keys.transitions(&BTreeSet::new()), [(key::DPAD_LEFT, true)]);
        keys.keyboard(Scancode::Left, false, false);
        assert!(keys.transitions(&BTreeSet::new()).is_empty());
        keys.keyboard(Scancode::A, false, false);
        assert!(
            keys.transitions(&BTreeSet::from([key::DPAD_LEFT]))
                .is_empty()
        );
        assert_eq!(
            keys.transitions(&BTreeSet::new()),
            [(key::DPAD_LEFT, false)]
        );
    }

    #[test]
    fn releasing_an_alias_restores_a_held_shared_action() {
        let mut keys = Keys::default();
        keys.keyboard(Scancode::Space, true, false);
        let pad = BTreeSet::from([key::BUTTON_R1]);
        keys.transitions(&pad);
        keys.keyboard(Scancode::Space, false, false);
        assert_eq!(
            keys.transitions(&pad),
            [(key::DPAD_CENTER, false), (key::BUTTON_R1, true)]
        );
    }

    #[test]
    fn virtual_accelerometer_preserves_gravity_and_steering_direction() {
        let left = acceleration(-1.0);
        let right = acceleration(1.0);
        let center = acceleration(0.0);
        assert_eq!(left[0], right[0]);
        assert_eq!(left[1], -right[1]);
        assert!(left[1] < 0 && right[1] > 0);
        assert_eq!(center[1], 0);
        assert_eq!(left[2], center[2]);
        let norm = center
            .iter()
            .map(|&v| f64::from(v).powi(2))
            .sum::<f64>()
            .sqrt()
            / 6553.0;
        assert!((norm - 9.80665).abs() < 0.001);
    }

    #[test]
    fn keyboard_select_pause_and_digit_codes_match_the_android_table() {
        assert_eq!(android_keycode(Scancode::Escape), Some(key::BACK));
        assert_eq!(android_keycode(Scancode::Return), Some(key::DPAD_CENTER));
        assert_eq!(android_keycode(Scancode::_0), Some(7));
        assert_eq!(android_keycode(Scancode::_1), Some(8));
        assert_eq!(android_keycode(Scancode::_9), Some(16));
    }
}
