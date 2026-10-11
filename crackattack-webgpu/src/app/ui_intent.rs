//! Vocabulary of **UI intents**.
//!
//! Pure (no windowing dependency), so the UI routers in the library can use it. The
//! **physical input to intent** resolution lives in the **binary** (`ui_conventions`),
//! where `PhysicalInput` and `KeyCode` live. `input_source` does NOT drive this resolution
//! (it only serves the prompts): the current physical input and the active context decide.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UiDir {
    Up,
    Down,
    Left,
    Right,
}

/// UI intent resolved by convention (never rebindable).
/// Every intent comes from a physical input. Typed characters do not go through intents: they
/// reach egui as text events.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UiIntent {
    Confirm,
    Cancel,
    Pause,
    /// Raw direction. The consumer decides between *navigate* and *adjust* from the focused widget.
    Navigate(UiDir),
    Backspace,
}

/// Active UI context, chosen by the top-level router. It determines the resolution.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UiContext {
    /// In game: **only `Pause` is accepted** (the rest is gameplay bindings). `B` does not pause.
    Gameplay,
    /// UI navigation (menus, overlays, pause): Confirm, Cancel, Navigate, Pause.
    Navigation,
    /// Active text editing: Backspace, Confirm, Cancel and Navigate (on-screen keyboard grid).
    /// Characters arrive through text events, not through this resolution.
    TextEdit,
}
