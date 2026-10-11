//! Last input source used (keyboard or gamepad).
//!
//! Lets the UI pick matching prompts and glyphs. Only `Keyboard` and `Gamepad`
//! are tracked on purpose: the mouse is not a tracked input flow.

use std::sync::atomic::{AtomicU8, Ordering};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputSource {
    Keyboard,
    Gamepad,
}

// 0 = Keyboard (default), 1 = Gamepad.
static LAST: AtomicU8 = AtomicU8::new(0);

pub fn set(src: InputSource) {
    let v = match src {
        InputSource::Keyboard => 0,
        InputSource::Gamepad => 1,
    };
    LAST.store(v, Ordering::Relaxed);
}

pub fn last() -> InputSource {
    match LAST.load(Ordering::Relaxed) {
        1 => InputSource::Gamepad,
        _ => InputSource::Keyboard,
    }
}

pub fn is_gamepad() -> bool {
    last() == InputSource::Gamepad
}
