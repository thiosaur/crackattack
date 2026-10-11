//! Maps a physical input to a UI intent by fixed convention.
//!
//! There is no rebinding here, and the result does not depend on `input_source`. The menu
//! router (`route_menu_input`) calls this layer.

use crate::keycodes::{Key, PadAxisDirection, PadButton, PhysicalInput};
use crate::app::ui_intent::{UiContext, UiDir, UiIntent};

/// Resolves a UI intent from the physical input and the active context.
///
/// Never produces `Text(char)`: text comes from the platform's text events, not from a [`Key`].
pub fn resolve_ui_intent(input: PhysicalInput, ctx: UiContext) -> Option<UiIntent> {
    match ctx {
        // In game, Pause is the only UI intent. The fixed conventions are Start (gamepad)
        // and Escape (keyboard). The P key, like any other keyboard key, is a configurable
        // binding resolved upstream (`app_key == Pause`), not a convention. `B`/East does
        // not pause (console convention).
        UiContext::Gameplay => match input {
            PhysicalInput::PadButton(PadButton::Start) => Some(UiIntent::Pause),
            PhysicalInput::Key(Key::Escape) => Some(UiIntent::Pause),
            _ => None,
        },
        UiContext::Navigation => resolve_navigation(input),
        UiContext::TextEdit => resolve_text_edit(input),
    }
}

fn resolve_navigation(input: PhysicalInput) -> Option<UiIntent> {
    use UiIntent::*;
    match input {
        PhysicalInput::Key(Key::Enter) | PhysicalInput::Key(Key::Space) => Some(Confirm),
        PhysicalInput::Key(Key::Escape) => Some(Cancel),
        // Backspace goes back in navigation, like Escape and B.
        PhysicalInput::Key(Key::Backspace) => Some(Cancel),
        PhysicalInput::Key(Key::KeyP) => Some(Pause),
        PhysicalInput::PadButton(PadButton::South) => Some(Confirm),
        PhysicalInput::PadButton(PadButton::East) => Some(Cancel),
        PhysicalInput::PadButton(PadButton::Start) => Some(Pause),
        // Keyboard navigation uses the arrow keys. WASD is gameplay, not UI navigation.
        PhysicalInput::Key(k) => keyboard_dir(k).map(Navigate),
        PhysicalInput::PadButton(b) => pad_button_dir(b).map(Navigate),
        PhysicalInput::PadAxis(a) => Some(Navigate(axis_dir(a))),
    }
}

fn resolve_text_edit(input: PhysicalInput) -> Option<UiIntent> {
    use UiIntent::*;
    match input {
        PhysicalInput::Key(Key::Backspace) => Some(Backspace),
        PhysicalInput::Key(Key::Enter) => Some(Confirm),
        PhysicalInput::Key(Key::Escape) => Some(Cancel),
        PhysicalInput::PadButton(PadButton::South) => Some(Confirm),
        PhysicalInput::PadButton(PadButton::East) => Some(Cancel),
        // Navigation of the on-screen keyboard grid. `Space` gives `None` here: it is a
        // character produced by text events, not a confirmation.
        PhysicalInput::Key(k) => keyboard_dir(k).map(Navigate),
        PhysicalInput::PadButton(b) => pad_button_dir(b).map(Navigate),
        PhysicalInput::PadAxis(a) => Some(Navigate(axis_dir(a))),
    }
}

/// Arrow keys only. WASD is gameplay, not UI navigation.
fn keyboard_dir(k: Key) -> Option<UiDir> {
    match k {
        Key::ArrowUp => Some(UiDir::Up),
        Key::ArrowDown => Some(UiDir::Down),
        Key::ArrowLeft => Some(UiDir::Left),
        Key::ArrowRight => Some(UiDir::Right),
        _ => None,
    }
}

fn pad_button_dir(b: PadButton) -> Option<UiDir> {
    match b {
        PadButton::DPadUp => Some(UiDir::Up),
        PadButton::DPadDown => Some(UiDir::Down),
        PadButton::DPadLeft => Some(UiDir::Left),
        PadButton::DPadRight => Some(UiDir::Right),
        _ => None,
    }
}

fn axis_dir(a: PadAxisDirection) -> UiDir {
    match a {
        PadAxisDirection::LeftStickUp => UiDir::Up,
        PadAxisDirection::LeftStickDown => UiDir::Down,
        PadAxisDirection::LeftStickLeft => UiDir::Left,
        PadAxisDirection::LeftStickRight => UiDir::Right,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gameplay_only_pause_and_b_does_not_pause() {
        let g = UiContext::Gameplay;
        // The fixed pause conventions are Start (gamepad) and Escape (keyboard).
        assert_eq!(resolve_ui_intent(PhysicalInput::PadButton(PadButton::Start), g), Some(UiIntent::Pause));
        assert_eq!(resolve_ui_intent(PhysicalInput::Key(Key::Escape), g), Some(UiIntent::Pause));
        // P is not a convention: it is a keyboard binding, resolved upstream via `app_key`.
        assert_eq!(resolve_ui_intent(PhysicalInput::Key(Key::KeyP), g), None);
        // B (East) does not pause in game, and neither do A or the directions.
        assert_eq!(resolve_ui_intent(PhysicalInput::PadButton(PadButton::East), g), None);
        assert_eq!(resolve_ui_intent(PhysicalInput::PadButton(PadButton::South), g), None);
        assert_eq!(resolve_ui_intent(PhysicalInput::Key(Key::ArrowUp), g), None);
    }

    #[test]
    fn navigation_conventions() {
        let n = UiContext::Navigation;
        assert_eq!(resolve_ui_intent(PhysicalInput::Key(Key::Enter), n), Some(UiIntent::Confirm));
        assert_eq!(resolve_ui_intent(PhysicalInput::Key(Key::Space), n), Some(UiIntent::Confirm));
        assert_eq!(resolve_ui_intent(PhysicalInput::Key(Key::Escape), n), Some(UiIntent::Cancel));
        // Backspace goes back, like Escape and B.
        assert_eq!(resolve_ui_intent(PhysicalInput::Key(Key::Backspace), n), Some(UiIntent::Cancel));
        assert_eq!(resolve_ui_intent(PhysicalInput::PadButton(PadButton::South), n), Some(UiIntent::Confirm));
        assert_eq!(resolve_ui_intent(PhysicalInput::PadButton(PadButton::East), n), Some(UiIntent::Cancel));
        assert_eq!(resolve_ui_intent(PhysicalInput::PadButton(PadButton::Start), n), Some(UiIntent::Pause));
        assert_eq!(resolve_ui_intent(PhysicalInput::Key(Key::ArrowDown), n), Some(UiIntent::Navigate(UiDir::Down)));
        assert_eq!(resolve_ui_intent(PhysicalInput::PadButton(PadButton::DPadLeft), n), Some(UiIntent::Navigate(UiDir::Left)));
        assert_eq!(resolve_ui_intent(PhysicalInput::PadAxis(PadAxisDirection::LeftStickRight), n), Some(UiIntent::Navigate(UiDir::Right)));
    }

    #[test]
    fn text_edit_space_is_not_confirm() {
        let t = UiContext::TextEdit;
        // Space is not Confirm while editing text: it is a character (text events).
        assert_eq!(resolve_ui_intent(PhysicalInput::Key(Key::Space), t), None);
        assert_eq!(resolve_ui_intent(PhysicalInput::Key(Key::Backspace), t), Some(UiIntent::Backspace));
        assert_eq!(resolve_ui_intent(PhysicalInput::Key(Key::Enter), t), Some(UiIntent::Confirm));
        assert_eq!(resolve_ui_intent(PhysicalInput::Key(Key::Escape), t), Some(UiIntent::Cancel));
        // The on-screen keyboard grid is navigable.
        assert_eq!(resolve_ui_intent(PhysicalInput::Key(Key::ArrowUp), t), Some(UiIntent::Navigate(UiDir::Up)));
    }
}
