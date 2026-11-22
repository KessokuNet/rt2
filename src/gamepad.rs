//! Held-state SDL gamepad input using the Vita/Xperia Play key mappings.

use std::collections::{BTreeMap, BTreeSet};

use sdl3::GamepadSubsystem;
use sdl3::event::Event;
use sdl3::gamepad::{Axis, Button, Gamepad};
use sdl3::joystick::JoystickId;
use stable_eyre::eyre::{Result, WrapErr};

use crate::input::key;

const DEADZONE: f32 = 0.15;
const MENU_THRESHOLD: f32 = 0.5;
const TRIGGER_THRESHOLD: f32 = 0.5;

#[derive(Debug, Default, PartialEq)]
pub(crate) struct PadInput {
    pub(crate) keys: BTreeSet<i32>,
    /// Normalized steering in [-1, 1], negative left; zero outside tilt mode.
    pub(crate) steer: f32,
}

pub(crate) struct Gamepads {
    // Drop handles before our subsystem reference (handles also retain theirs).
    pads: BTreeMap<JoystickId, Gamepad>,
    subsystem: GamepadSubsystem,
}

impl Gamepads {
    pub(crate) fn new(subsystem: GamepadSubsystem) -> Result<Self> {
        let attached = subsystem.gamepads().wrap_err("enumerating SDL gamepads")?;
        let mut manager = Self {
            pads: BTreeMap::new(),
            subsystem,
        };
        for id in attached {
            manager.open(id);
        }
        Ok(manager)
    }

    /// Device events only: button and axis states are sampled by `poll`.
    pub(crate) fn handle_event(&mut self, event: &Event) {
        match event {
            Event::GamepadAdded { which, .. } => self.open(*which),
            Event::GamepadRemoved { which, .. } => {
                self.pads.remove(which);
                tracing::info!(%which, "gamepad disconnected");
            }
            _ => {}
        }
    }

    fn open(&mut self, id: JoystickId) {
        // Initial enumeration and queued Added events can describe the same pad.
        if self.pads.contains_key(&id) {
            return;
        }
        match self.subsystem.open(id) {
            Ok(pad) => {
                tracing::info!(%id, name = ?pad.name(), "gamepad connected");
                self.pads.insert(id, pad);
            }
            Err(error) => {
                tracing::warn!(%id, %error, "could not open SDL gamepad");
            }
        }
    }

    pub(crate) fn poll(&mut self, tilt_mode: bool) -> PadInput {
        self.subsystem.update();
        // Also discard disconnected handles if a Removed event was not forwarded.
        self.pads.retain(|_, pad| pad.connected());
        combine_inputs(
            self.pads.values().map(|pad| {
                map_input(
                    pad.axis(Axis::LeftX),
                    pad.axis(Axis::LeftY),
                    pad.axis(Axis::TriggerLeft),
                    pad.axis(Axis::TriggerRight),
                    |button| pad.button(button),
                    tilt_mode,
                )
            }),
            tilt_mode,
        )
    }
}

/// Normalize both i16 endpoints exactly, then rescale outside the deadzone.
fn stick_axis(raw: i16) -> f32 {
    let normalized = if raw < 0 {
        f32::from(raw) / 32768.0
    } else {
        f32::from(raw) / 32767.0
    };
    if normalized.abs() <= DEADZONE {
        0.0
    } else {
        normalized.signum() * (normalized.abs() - DEADZONE) / (1.0 - DEADZONE)
    }
}

fn trigger_pressed(raw: i16) -> bool {
    // SDL gamepad triggers are unipolar: 0 released, 32767 fully pressed.
    f32::from(raw) / 32767.0 > TRIGGER_THRESHOLD
}

/// Pure mapping so tests do not need an SDL subsystem or physical controller.
fn map_input(
    left_x: i16,
    left_y: i16,
    trigger_left: i16,
    trigger_right: i16,
    held: impl Fn(Button) -> bool,
    tilt_mode: bool,
) -> PadInput {
    let mut input = PadInput::default();
    for (button, code) in [
        (Button::South, key::DPAD_CENTER),
        (Button::West, key::BUTTON_X),
        (Button::North, key::BUTTON_Y),
        (Button::East, key::BACK),
        (Button::Start, key::BACK),
        (Button::Back, key::BACK),
        (Button::LeftShoulder, key::BUTTON_L1),
        (Button::RightShoulder, key::BUTTON_R1),
        (Button::DPadUp, key::DPAD_UP),
        (Button::DPadDown, key::DPAD_DOWN),
        (Button::DPadLeft, key::DPAD_LEFT),
        (Button::DPadRight, key::DPAD_RIGHT),
    ] {
        if held(button) {
            input.keys.insert(code);
        }
    }
    if trigger_pressed(trigger_left) {
        input.keys.insert(key::BUTTON_X);
    }
    if trigger_pressed(trigger_right) {
        input.keys.insert(key::DPAD_CENTER);
    }

    let x = stick_axis(left_x);
    if tilt_mode {
        input.steer = x;
    } else {
        let y = stick_axis(left_y);
        for (pressed, code) in [
            (x < -MENU_THRESHOLD, key::DPAD_LEFT),
            (x > MENU_THRESHOLD, key::DPAD_RIGHT),
            (y < -MENU_THRESHOLD, key::DPAD_UP),
            (y > MENU_THRESHOLD, key::DPAD_DOWN),
        ] {
            if pressed {
                input.keys.insert(code);
            }
        }
    }
    input
}

fn combine_inputs(inputs: impl IntoIterator<Item = PadInput>, tilt_mode: bool) -> PadInput {
    let mut combined = PadInput::default();
    for input in inputs {
        combined.keys.extend(input.keys);
        // Idle controllers must not center another controller's active stick.
        if tilt_mode && input.steer.abs() > combined.steer.abs() {
            combined.steer = input.steer;
        }
    }
    if tilt_mode {
        // Physical DPAD steering takes priority over analog steering.
        match (
            combined.keys.contains(&key::DPAD_LEFT),
            combined.keys.contains(&key::DPAD_RIGHT),
        ) {
            (true, false) => combined.steer = -1.0,
            (false, true) => combined.steer = 1.0,
            _ => {}
        }
    }
    combined
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(x: i16, y: i16, buttons: &[Button], tilt: bool) -> PadInput {
        map_input(x, y, 0, 0, |button| buttons.contains(&button), tilt)
    }

    #[test]
    fn deadzone_is_rescaled_and_endpoints_are_exact() {
        assert_eq!(stick_axis(i16::MIN), -1.0);
        assert_eq!(stick_axis(i16::MAX), 1.0);
        for raw in [-4915, -1, 0, 1, 4915] {
            assert_eq!(stick_axis(raw), 0.0);
        }
        assert!(stick_axis(4916) > 0.0);
        assert!(stick_axis(-4916) < 0.0);
        let expected = (0.5 - DEADZONE) / (1.0 - DEADZONE);
        assert!((stick_axis(16384) - expected).abs() < 0.0001);
        assert!((stick_axis(-16384) + expected).abs() < 0.0001);
    }

    #[test]
    fn vita_buttons_map_to_android_keys() {
        for (button, code) in [
            (Button::South, 23),
            (Button::West, 99),
            (Button::North, 100),
            (Button::East, 4),
            (Button::Start, 4),
            (Button::Back, 4),
            (Button::LeftShoulder, 102),
            (Button::RightShoulder, 103),
            (Button::DPadUp, 19),
            (Button::DPadDown, 20),
            (Button::DPadLeft, 21),
            (Button::DPadRight, 22),
        ] {
            for tilt in [false, true] {
                assert_eq!(sample(0, 0, &[button], tilt).keys, BTreeSet::from([code]));
            }
        }
        assert_eq!(sample(0, 0, &[Button::Guide], false), PadInput::default());
    }

    #[test]
    fn triggers_alias_buttons_without_duplicate_keys() {
        assert!(!trigger_pressed(i16::MIN));
        assert!(!trigger_pressed(0));
        assert!(!trigger_pressed(16383));
        assert!(trigger_pressed(16384));
        let input = map_input(0, 0, i16::MAX, i16::MAX, |_| true, false);
        assert!(input.keys.contains(&key::BUTTON_X));
        assert!(input.keys.contains(&key::DPAD_CENTER));
        let aliases = map_input(
            0,
            0,
            i16::MAX,
            i16::MAX,
            |b| matches!(b, Button::South | Button::West),
            false,
        );
        assert_eq!(
            aliases.keys,
            BTreeSet::from([key::BUTTON_X, key::DPAD_CENTER])
        );
    }

    #[test]
    fn menu_threshold_uses_rescaled_stick_and_is_disabled_in_tilt() {
        // 50% raw deflection is below 50% of the rescaled output.
        assert!(sample(16384, -16384, &[], false).keys.is_empty());
        assert!(sample(18841, 0, &[], false).keys.is_empty());
        assert_eq!(
            sample(18842, 0, &[], false).keys,
            BTreeSet::from([key::DPAD_RIGHT])
        );
        assert_eq!(
            sample(i16::MIN, i16::MAX, &[], false).keys,
            BTreeSet::from([key::DPAD_LEFT, key::DPAD_DOWN]),
        );
        assert_eq!(
            sample(i16::MAX, i16::MIN, &[], false).keys,
            BTreeSet::from([key::DPAD_RIGHT, key::DPAD_UP]),
        );
        assert_eq!(sample(i16::MAX, 0, &[], false).steer, 0.0);
        let tilt = sample(i16::MAX, i16::MIN, &[], true);
        assert!(tilt.keys.is_empty());
        assert_eq!(tilt.steer, 1.0);
    }

    #[test]
    fn strongest_stick_wins_and_idle_pads_do_not_release_keys() {
        let input = combine_inputs(
            [
                sample(-22000, 0, &[Button::South], true),
                sample(28000, 0, &[Button::West], true),
                sample(0, 0, &[], true),
            ],
            true,
        );
        assert_eq!(input.steer, stick_axis(28000));
        assert_eq!(
            input.keys,
            BTreeSet::from([key::DPAD_CENTER, key::BUTTON_X])
        );
        let tied = combine_inputs(
            [
                sample(i16::MIN, 0, &[], true),
                sample(i16::MAX, 0, &[], true),
            ],
            true,
        );
        assert_eq!(tied.steer, -1.0);
    }

    #[test]
    fn physical_dpad_overrides_analog_across_controllers() {
        let input = combine_inputs(
            [
                sample(i16::MAX, 0, &[], true),
                sample(0, 0, &[Button::DPadLeft], true),
            ],
            true,
        );
        assert_eq!(input.steer, -1.0);
        let right = combine_inputs([sample(i16::MIN, 0, &[Button::DPadRight], true)], true);
        assert_eq!(right.steer, 1.0);
        let opposed = combine_inputs(
            [
                sample(22000, 0, &[Button::DPadLeft], true),
                sample(0, 0, &[Button::DPadRight], true),
            ],
            true,
        );
        assert_eq!(opposed.steer, stick_axis(22000));
    }

    #[test]
    fn dropping_contributions_releases_only_their_keys() {
        let both = combine_inputs(
            [
                sample(i16::MAX, 0, &[Button::South, Button::West], true),
                sample(0, 0, &[Button::South], true),
            ],
            true,
        );
        assert_eq!(both.keys, BTreeSet::from([key::DPAD_CENTER, key::BUTTON_X]));
        let remaining = combine_inputs([sample(0, 0, &[Button::South], true)], true);
        assert_eq!(remaining.keys, BTreeSet::from([key::DPAD_CENTER]));
        assert_eq!(remaining.steer, 0.0);
        assert_eq!(combine_inputs([], true), PadInput::default());
    }
}
