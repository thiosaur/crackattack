//! Input handling with edge detection.
//!
//! Platform events arrive asynchronously. A quick press and release can both
//! arrive before the game tick runs:
//! 1. key down: state = SWAP
//! 2. key up: state = 0
//! 3. tick: sees state = 0 and misses the press
//!
//! To avoid this, rising edges are captured the moment they arrive in
//! `key_down()` and accumulated in `pressed_this_frame` until the next
//! `end_frame()`.

pub const INPUT_LEFT: u8     = 1 << 0;
pub const INPUT_RIGHT: u8    = 1 << 1;
pub const INPUT_UP: u8       = 1 << 2;
pub const INPUT_DOWN: u8     = 1 << 3;
pub const INPUT_SWAP: u8     = 1 << 4;
pub const INPUT_ADVANCE: u8  = 1 << 5;

pub(crate) const INPUT_DIRECTIONS: u8 = INPUT_LEFT | INPUT_RIGHT | INPUT_UP | INPUT_DOWN;

/// Input state manager with edge detection for asynchronous event loops.
///
/// Key down and key up can both arrive before the game tick runs. Rising
/// edges are captured in `pressed_this_frame` when `key_down()` is called,
/// so short presses are never lost. As in the original game, the movement code
/// only sees the level state at the tick: a release followed by a press between
/// two ticks leaves the button held, and does not count as a new press.
///
/// Usage:
/// ```rust
/// use crackattack_core::input::{Input, INPUT_LEFT, INPUT_SWAP};
///
/// let mut input = Input::new();
///
/// // Event handlers (called by the platform layer):
/// input.key_down(INPUT_SWAP);  // Captures rising edge immediately
/// input.key_up(INPUT_SWAP);    // state=0, but pressed_this_frame still has SWAP
///
/// // Game tick (the game logic reads an immutable snapshot):
/// if input.snapshot().was_pressed(INPUT_SWAP) { /* swap - works! */ }
///
/// // After all ticks:
/// input.end_frame();  // Clears pressed_this_frame for next frame
/// ```
#[derive(Debug, Clone, Default)]
pub struct Input {
    /// Current state bitmask (updated immediately by events)
    state: u8,
    /// State at end of previous frame
    prev_state: u8,
    /// Keys that had a rising edge since last end_frame() - captures quick presses
    pressed_this_frame: u8,
}

impl Input {
    /// Create new input state
    pub fn new() -> Self {
        Self {
            state: 0,
            prev_state: 0,
            pressed_this_frame: 0,
        }
    }

    /// Reset all state
    pub fn reset(&mut self) {
        self.state = 0;
        self.prev_state = 0;
        self.pressed_this_frame = 0;
    }

    /// Call AFTER all game ticks to prepare for next frame.
    /// Clears pressed_this_frame and saves state for next frame's edge detection.
    pub fn end_frame(&mut self) {
        self.prev_state = self.state;
        self.pressed_this_frame = 0;
    }

    /// Process key down event and capture the rising edge immediately.
    ///
    /// The edge is recorded now, not when the game tick runs. Even if
    /// `key_up()` is called before the tick, `pressed_this_frame` remembers
    /// the press.
    #[inline]
    pub fn key_down(&mut self, action: u8) {
        // Capture rising edge: if not currently pressed, mark as pressed this frame
        if (self.state & action) == 0 {
            self.pressed_this_frame |= action;
        }
        self.state |= action;
    }

    /// Process key up event
    #[inline]
    pub fn key_up(&mut self, action: u8) {
        self.state &= !action;
    }

    /// Process key event (combined down/up)
    #[inline]
    pub fn set_key(&mut self, action: u8, pressed: bool) {
        if pressed {
            self.key_down(action);
        } else {
            self.key_up(action);
        }
    }

    /// Create an immutable snapshot for passing to game logic
    pub fn snapshot(&self) -> InputSnapshot {
        InputSnapshot {
            state: self.state,
            prev_state: self.prev_state,
            pressed_this_frame: self.pressed_this_frame,
        }
    }
}

/// Immutable snapshot of input state for passing to game logic
///
/// This lets the renderer own the [`Input`] while passing snapshots to the
/// game logic.
///
/// `pressed_this_frame` captures rising edges at key_down() time,
/// ensuring quick presses are never lost even if key_up() arrives
/// before the game tick runs.
#[derive(Debug, Clone, Copy, Default)]
pub struct InputSnapshot {
    pub state: u8,
    pub prev_state: u8,
    pub pressed_this_frame: u8,
}

impl InputSnapshot {
    /// Create empty snapshot
    pub fn new() -> Self {
        InputSnapshot { state: 0, prev_state: 0, pressed_this_frame: 0 }
    }

    /// Check if action is currently pressed
    #[inline]
    pub(crate) fn is_down(&self, action: u8) -> bool {
        (self.state & action) != 0
    }

    /// Check if action was just pressed (rising edge captured at key_down time)
    #[inline]
    pub fn was_pressed(&self, action: u8) -> bool {
        (self.pressed_this_frame & action) != 0
    }

    /// Get direction input bitmask
    #[inline]
    pub(crate) fn move_command(&self) -> u8 {
        self.state & INPUT_DIRECTIONS
    }

    /// Check if advance is pressed
    #[inline]
    pub fn advance(&self) -> bool {
        self.is_down(INPUT_ADVANCE)
    }

    /// Get move command as bitmask
    pub(crate) fn move_cmd(&self) -> i32 {
        self.move_command() as i32
    }
}
