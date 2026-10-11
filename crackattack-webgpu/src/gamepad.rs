//! Gamepad support through SDL3 (HIDAPI, mapping database and correct GUIDs, so
//! common controllers are recognized without extra setup).
//!
//! The backend polls state. On each poll SDL refreshes the gamepad state, which
//! is read (`button`/`axis`) and diffed against the previous frame. The result is
//! a stream of `PhysicalInput` events, the stable boundary consumed by
//! `dispatch_input`. Stick axes go through a deadzone latch with hysteresis.

use sdl3::gamepad::{Axis, Button, Gamepad};
use crate::keycodes::{PhysicalInput, PadButton, PadAxisDirection};

/// Deadzone: a direction activates above this value (axis normalized to `f32` in [-1, 1]).
const AXIS_ACTIVATE: f32 = 0.5;
/// A direction is released below this value (hysteresis against jitter).
const AXIS_RELEASE: f32 = 0.3;

/// Cardinal direction produced by a pair of axes (horizontal X, vertical Y).
#[derive(Clone, Copy)]
enum Dir { Up, Down, Left, Right }

/// Latch for the 4 stick directions (deadzone plus hysteresis). It turns a
/// continuous analog axis into discrete directions with clean press and release.
#[derive(Default)]
struct AxisLatch {
    left: bool,
    right: bool,
    up: bool,
    down: bool,
}

impl AxisLatch {
    /// Horizontal axis (positive = right). Returns (newly pressed, newly released).
    fn update_x(&mut self, val: f32) -> (Option<Dir>, Option<Dir>) {
        let (mut pressed, mut released) = (None, None);
        if val > AXIS_ACTIVATE && !self.right { self.right = true; pressed = Some(Dir::Right); }
        else if val < AXIS_RELEASE && self.right { self.right = false; released = Some(Dir::Right); }
        if val < -AXIS_ACTIVATE && !self.left { self.left = true; pressed = Some(Dir::Left); }
        else if val > -AXIS_RELEASE && self.left { self.left = false; released = Some(Dir::Left); }
        (pressed, released)
    }

    /// Vertical axis, oriented so that positive = up. The caller negates the raw
    /// SDL value, whose convention is up = negative.
    fn update_y(&mut self, val: f32) -> (Option<Dir>, Option<Dir>) {
        let (mut pressed, mut released) = (None, None);
        if val > AXIS_ACTIVATE && !self.up { self.up = true; pressed = Some(Dir::Up); }
        else if val < AXIS_RELEASE && self.up { self.up = false; released = Some(Dir::Up); }
        if val < -AXIS_ACTIVATE && !self.down { self.down = true; pressed = Some(Dir::Down); }
        else if val > -AXIS_RELEASE && self.down { self.down = false; released = Some(Dir::Down); }
        (pressed, released)
    }

    /// Emits a release for every direction still active, then clears the state.
    /// Call it on disconnect or pad change so no direction stays stuck downstream.
    fn reset_into(&mut self, events: &mut Vec<GamepadEvent>) {
        for (held, dir) in [
            (&mut self.up, Dir::Up), (&mut self.down, Dir::Down),
            (&mut self.left, Dir::Left), (&mut self.right, Dir::Right),
        ] {
            if *held {
                *held = false;
                events.push(GamepadEvent { input: PhysicalInput::PadAxis(stick_dir(dir)), pressed: false });
            }
        }
    }
}

/// Maps an axis direction to the left-stick `PadAxisDirection`.
fn stick_dir(d: Dir) -> PadAxisDirection {
    match d {
        Dir::Up => PadAxisDirection::LeftStickUp,
        Dir::Down => PadAxisDirection::LeftStickDown,
        Dir::Left => PadAxisDirection::LeftStickLeft,
        Dir::Right => PadAxisDirection::LeftStickRight,
    }
}

/// Normalizes an SDL axis value (`i16`, -32768..32767) to `f32` in [-1, 1].
fn norm(v: i16) -> f32 {
    (v as f32 / 32767.0).clamp(-1.0, 1.0)
}

/// SDL3 buttons polled each frame, mapped to `PadButton`. Guide, L3/R3, paddles, misc and touchpad are ignored.
const BUTTONS: [(Button, PadButton); 12] = [
    (Button::South, PadButton::South),
    (Button::East, PadButton::East),
    (Button::West, PadButton::West),
    (Button::North, PadButton::North),
    (Button::Start, PadButton::Start),
    (Button::Back, PadButton::Select),
    (Button::LeftShoulder, PadButton::LeftShoulder),
    (Button::RightShoulder, PadButton::RightShoulder),
    (Button::DPadUp, PadButton::DPadUp),
    (Button::DPadDown, PadButton::DPadDown),
    (Button::DPadLeft, PadButton::DPadLeft),
    (Button::DPadRight, PadButton::DPadRight),
];

/// Gamepad event: press or release of a `PhysicalInput`.
#[derive(Debug, Clone, Copy)]
pub struct GamepadEvent {
    pub input: PhysicalInput,
    pub pressed: bool,
}

/// Core of gamepad polling: hot-plug, button diff and axis latch.
/// Used by `Sdl3Platform::poll_events`, whose event pump already updates the
/// joystick state, so no separate `update()` call is needed.
#[derive(Default)]
pub(crate) struct PadPoller {
    active: Option<Gamepad>,
    latch: AxisLatch,
    prev: [bool; BUTTONS.len()],
}

impl PadPoller {
    pub(crate) fn poll_step(&mut self, gamepads: &sdl3::GamepadSubsystem, out: &mut Vec<GamepadEvent>) {
        // Active pad disconnected: release all held state (no stuck input) and close it.
        if self.active.as_ref().map_or(false, |g| !g.connected()) {
            for (i, (_, pad_b)) in BUTTONS.iter().enumerate() {
                if self.prev[i] {
                    self.prev[i] = false;
                    out.push(GamepadEvent { input: PhysicalInput::PadButton(*pad_b), pressed: false });
                }
            }
            self.latch.reset_into(out);
            self.active = None;
        }
        // Hot-plug: open the first available pad if none is active.
        if self.active.is_none() {
            if let Ok(ids) = gamepads.gamepads() {
                for id in ids {
                    if let Ok(gp) = gamepads.open(id) {
                        let name = gp.name().unwrap_or_else(|| "(unnamed)".into());
                        let recognized = gp.mapping().map_or(false, |m| !m.is_empty());
                        log::info!("Gamepad opened: \"{name}\" (recognized by SDL: {recognized})");
                        self.active = Some(gp);
                        self.prev = [false; BUTTONS.len()];
                        self.latch = AxisLatch::default();
                        break;
                    }
                }
            }
        }

        if let Some(gp) = self.active.as_ref() {
            // Buttons: diff current and previous state to get edges.
            for (i, (sdl_b, pad_b)) in BUTTONS.iter().enumerate() {
                let now = gp.button(*sdl_b);
                if now != self.prev[i] {
                    self.prev[i] = now;
                    out.push(GamepadEvent { input: PhysicalInput::PadButton(*pad_b), pressed: now });
                }
            }
            // Left stick goes through the latch. SDL reports up as negative, so Y is negated (positive = up).
            let x = norm(gp.axis(Axis::LeftX));
            let y = -norm(gp.axis(Axis::LeftY));
            for (pressed, released) in [self.latch.update_x(x), self.latch.update_y(y)] {
                if let Some(d) = released {
                    out.push(GamepadEvent { input: PhysicalInput::PadAxis(stick_dir(d)), pressed: false });
                }
                if let Some(d) = pressed {
                    out.push(GamepadEvent { input: PhysicalInput::PadAxis(stick_dir(d)), pressed: true });
                }
            }
        }
    }
}
