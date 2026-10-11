//! Physical input ↔ stable string conversion and compiled bindings.
//!
//! Backend-agnostic: a physical key is the project's own [`Key`] type, so the platform
//! layer can be swapped. Conversion from the platform's scancodes to `Key` happens at the
//! boundary (the binary, `main.rs`). Persistence uses strings; this module bridges the two.
//! Gamepad buttons and axes are also [`PhysicalInput`]s.

use std::collections::HashMap;
use crate::app::command::AppKey;
use crate::platform::persist::{BindingAction, KeyBindings};

/// A physical keyboard key, independent of the window/event backend.
///
/// Variant names follow the W3C key code names (`KeyA`, `Digit0`, ...). They match the stable
/// names stored in persisted bindings, so do not rename them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Key {
    ArrowUp, ArrowDown, ArrowLeft, ArrowRight,
    KeyA, KeyB, KeyC, KeyD, KeyE, KeyF, KeyG, KeyH, KeyI, KeyJ, KeyK, KeyL, KeyM,
    KeyN, KeyO, KeyP, KeyQ, KeyR, KeyS, KeyT, KeyU, KeyV, KeyW, KeyX, KeyY, KeyZ,
    Digit0, Digit1, Digit2, Digit3, Digit4, Digit5, Digit6, Digit7, Digit8, Digit9,
    Space, Enter, Escape, Tab, Backspace,
    ShiftLeft, ShiftRight, ControlLeft, ControlRight, AltLeft, AltRight,
    Comma, Period, Slash, Semicolon, BracketLeft, BracketRight, Minus, Equal,
    Backquote, F1, F10, F11, F12,
}

/// A physical input source: keyboard key, gamepad button, or gamepad axis direction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PhysicalInput {
    Key(Key),
    PadButton(PadButton),
    PadAxis(PadAxisDirection),
}

/// Gamepad buttons, independent of any gamepad backend.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PadButton {
    South,       // A (Xbox) / Cross (PS)
    East,        // B (Xbox) / Circle (PS)
    West,        // X (Xbox) / Square (PS)
    North,       // Y (Xbox) / Triangle (PS)
    Start,
    Select,
    DPadUp,
    DPadDown,
    DPadLeft,
    DPadRight,
    LeftShoulder,
    RightShoulder,
}

/// Stick axis quantized to cardinal directions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PadAxisDirection {
    LeftStickUp,
    LeftStickDown,
    LeftStickLeft,
    LeftStickRight,
}

/// (stable_name, display_name, Key)
const KEY_TABLE: &[(&str, &str, Key)] = &[
    // Arrows
    // Thin arrows (Unicode Arrows block, U+219x), drawn through the Noto Sans Symbols fallback font.
    ("ArrowUp", "\u{2191}", Key::ArrowUp),
    ("ArrowDown", "\u{2193}", Key::ArrowDown),
    ("ArrowLeft", "\u{2190}", Key::ArrowLeft),
    ("ArrowRight", "\u{2192}", Key::ArrowRight),
    // Letters
    ("KeyA", "A", Key::KeyA),
    ("KeyB", "B", Key::KeyB),
    ("KeyC", "C", Key::KeyC),
    ("KeyD", "D", Key::KeyD),
    ("KeyE", "E", Key::KeyE),
    ("KeyF", "F", Key::KeyF),
    ("KeyG", "G", Key::KeyG),
    ("KeyH", "H", Key::KeyH),
    ("KeyI", "I", Key::KeyI),
    ("KeyJ", "J", Key::KeyJ),
    ("KeyK", "K", Key::KeyK),
    ("KeyL", "L", Key::KeyL),
    ("KeyM", "M", Key::KeyM),
    ("KeyN", "N", Key::KeyN),
    ("KeyO", "O", Key::KeyO),
    ("KeyP", "P", Key::KeyP),
    ("KeyQ", "Q", Key::KeyQ),
    ("KeyR", "R", Key::KeyR),
    ("KeyS", "S", Key::KeyS),
    ("KeyT", "T", Key::KeyT),
    ("KeyU", "U", Key::KeyU),
    ("KeyV", "V", Key::KeyV),
    ("KeyW", "W", Key::KeyW),
    ("KeyX", "X", Key::KeyX),
    ("KeyY", "Y", Key::KeyY),
    ("KeyZ", "Z", Key::KeyZ),
    // Numbers
    ("Digit0", "0", Key::Digit0),
    ("Digit1", "1", Key::Digit1),
    ("Digit2", "2", Key::Digit2),
    ("Digit3", "3", Key::Digit3),
    ("Digit4", "4", Key::Digit4),
    ("Digit5", "5", Key::Digit5),
    ("Digit6", "6", Key::Digit6),
    ("Digit7", "7", Key::Digit7),
    ("Digit8", "8", Key::Digit8),
    ("Digit9", "9", Key::Digit9),
    // Special
    ("Space", "Space", Key::Space),
    ("Enter", "Enter", Key::Enter),
    ("Escape", "Esc", Key::Escape),
    ("Tab", "Tab", Key::Tab),
    ("Backspace", "Backspace", Key::Backspace),
    // Debug hotkey F10 (diagnostic snapshot, see `log_debug_snapshot`). It has a name so
    // that test miniscripts can inject and replay it.
    ("F10", "F10", Key::F10),
    // Modifiers
    ("ShiftLeft", "L-Shift", Key::ShiftLeft),
    ("ShiftRight", "R-Shift", Key::ShiftRight),
    ("ControlLeft", "L-Ctrl", Key::ControlLeft),
    ("ControlRight", "R-Ctrl", Key::ControlRight),
    ("AltLeft", "L-Alt", Key::AltLeft),
    ("AltRight", "R-Alt", Key::AltRight),
    // Punctuation
    ("Comma", ",", Key::Comma),
    ("Period", ".", Key::Period),
    ("Slash", "/", Key::Slash),
    ("Semicolon", ";", Key::Semicolon),
    ("BracketLeft", "[", Key::BracketLeft),
    ("BracketRight", "]", Key::BracketRight),
    ("Minus", "-", Key::Minus),
    ("Equal", "=", Key::Equal),
];

/// Resolves a key name to a [`Key`].
///
/// The name is either a stable name (`KeyP`, `Escape`, `ArrowLeft`, ...) or a display label
/// (`P`, `;`). Matching is case-insensitive. `Left`, `Right`, `Up` and `Down` are accepted as
/// aliases for the arrow keys. Used by the test miniscript (`script.rs`).
pub fn key_from_name(name: &str) -> Option<Key> {
    let name = match name.to_ascii_lowercase().as_str() {
        "left" => "ArrowLeft",
        "right" => "ArrowRight",
        "up" => "ArrowUp",
        "down" => "ArrowDown",
        _ => name,
    };
    KEY_TABLE.iter()
        .find(|(stable, label, _)| stable.eq_ignore_ascii_case(name) || *label == name)
        .map(|(_, _, k)| *k)
}

/// Returns the stable name of a [`Key`] (the inverse of [`key_from_name`]).
///
/// Used to serialize a recording (`script::Recorder`). Returns `None` if the key is not in the
/// table, which means it cannot be replayed.
pub fn name_from_key(key: Key) -> Option<&'static str> {
    KEY_TABLE.iter().find(|(_, _, k)| *k == key).map(|(stable, _, _)| *stable)
}

/// (stable_name, display_name, PadButton)
const PAD_BUTTON_TABLE: &[(&str, &str, PadButton)] = &[
    ("PadSouth", "A/\u{2715}", PadButton::South),
    ("PadEast", "B/\u{25cb}", PadButton::East),
    ("PadWest", "X/\u{25a1}", PadButton::West),
    ("PadNorth", "Y/\u{25b3}", PadButton::North),
    ("PadStart", "Start", PadButton::Start),
    ("PadSelect", "Select", PadButton::Select),
    ("PadDPadUp", "D-Up", PadButton::DPadUp),
    ("PadDPadDown", "D-Down", PadButton::DPadDown),
    ("PadDPadLeft", "D-Left", PadButton::DPadLeft),
    ("PadDPadRight", "D-Right", PadButton::DPadRight),
    ("PadLShoulder", "LB", PadButton::LeftShoulder),
    ("PadRShoulder", "RB", PadButton::RightShoulder),
];

/// (stable_name, display_name, PadAxisDirection)
const PAD_AXIS_TABLE: &[(&str, &str, PadAxisDirection)] = &[
    ("PadLeftStickUp", "Stick \u{2191}", PadAxisDirection::LeftStickUp),
    ("PadLeftStickDown", "Stick \u{2193}", PadAxisDirection::LeftStickDown),
    ("PadLeftStickLeft", "Stick \u{2190}", PadAxisDirection::LeftStickLeft),
    ("PadLeftStickRight", "Stick \u{2192}", PadAxisDirection::LeftStickRight),
];

/// Convert a PhysicalInput to its stable persistence name.
pub fn physical_input_to_name(input: PhysicalInput) -> Option<&'static str> {
    match input {
        PhysicalInput::Key(k) => name_from_key(k),
        PhysicalInput::PadButton(b) => {
            PAD_BUTTON_TABLE.iter().find(|(_, _, pb)| *pb == b).map(|(n, _, _)| *n)
        }
        PhysicalInput::PadAxis(a) => {
            PAD_AXIS_TABLE.iter().find(|(_, _, pa)| *pa == a).map(|(n, _, _)| *n)
        }
    }
}

/// Returns whether a physical input is valid for the options tab being captured.
///
/// The Keyboard tab accepts only keys. The Gamepad tab accepts only buttons and axes. This
/// prevents binding a key to a gamepad action, and the reverse. Tabs without bindings
/// (Player, Sound) accept everything.
pub fn input_matches_options_tab(
    tab: crate::app::meta_state::OptionsTab,
    physical: PhysicalInput,
) -> bool {
    use crate::app::meta_state::OptionsTab;
    match tab {
        OptionsTab::Keyboard => matches!(physical, PhysicalInput::Key(_)),
        OptionsTab::Gamepad => {
            matches!(physical, PhysicalInput::PadButton(_) | PhysicalInput::PadAxis(_))
        }
        OptionsTab::Player | OptionsTab::Sound => true,
    }
}

/// Convert a stable name to a PhysicalInput. Tries keyboard, then pad buttons, then axes.
pub fn name_to_physical_input(name: &str) -> Option<PhysicalInput> {
    if let Some(kc) = KEY_TABLE.iter().find(|(n, _, _)| *n == name).map(|(_, _, k)| *k) {
        return Some(PhysicalInput::Key(kc));
    }
    if let Some(pb) = PAD_BUTTON_TABLE.iter().find(|(n, _, _)| *n == name).map(|(_, _, b)| *b) {
        return Some(PhysicalInput::PadButton(pb));
    }
    if let Some(pa) = PAD_AXIS_TABLE.iter().find(|(n, _, _)| *n == name).map(|(_, _, a)| *a) {
        return Some(PhysicalInput::PadAxis(pa));
    }
    None
}

/// Display label overrides for the current keyboard layout.
///
/// The platform sets them once after SDL init (see [`set_layout_labels`]). Only the display
/// changes. The persisted stable names (`KeyA`, ...) stay physical scancodes and are never
/// localized.
static LAYOUT_LABELS: std::sync::OnceLock<HashMap<&'static str, String>> =
    std::sync::OnceLock::new();

/// Installs the labels of the current layout. The platform calls this once at boot.
pub fn set_layout_labels(labels: HashMap<&'static str, String>) {
    let _ = LAYOUT_LABELS.set(labels);
}

/// Human-readable display name for any input (for UI).
/// The current layout wins (on AZERTY, `KeyA` shows "Q"); otherwise a curated label is used.
pub fn key_display_name(name: &str) -> &str {
    if let Some(label) = LAYOUT_LABELS.get().and_then(|m| m.get(name)) {
        return label;
    }
    // Keys named by a full word are localized. Short codes (LB, D-Up, L-Shift, ...) are
    // a convention and stay untranslated.
    let l = crate::app::locale::t();
    match name {
        "Space" => return l.key_space,
        "Enter" => return l.key_enter,
        "Escape" => return l.key_esc,
        "Backspace" => return l.key_backspace,
        _ => {}
    }
    if let Some((_, d, _)) = KEY_TABLE.iter().find(|(n, _, _)| *n == name) {
        return d;
    }
    if let Some((_, d, _)) = PAD_BUTTON_TABLE.iter().find(|(n, _, _)| *n == name) {
        return d;
    }
    if let Some((_, d, _)) = PAD_AXIS_TABLE.iter().find(|(n, _, _)| *n == name) {
        return d;
    }
    name
}

/// Runtime-compiled input mapping: PhysicalInput → AppKey.
/// Built from `KeyBindings` (persistent strings) at startup and on config change.
pub struct CompiledBindings {
    map: HashMap<PhysicalInput, AppKey>,
}

fn binding_to_app_key(action: BindingAction) -> AppKey {
    match action {
        BindingAction::Up => AppKey::Up,
        BindingAction::Down => AppKey::Down,
        BindingAction::Left => AppKey::Left,
        BindingAction::Right => AppKey::Right,
        BindingAction::Swap => AppKey::Swap,
        BindingAction::Raise => AppKey::Raise,
        BindingAction::Pause => AppKey::Pause,
    }
}

impl CompiledBindings {
    /// Compile a `KeyBindings` (strings) into a runtime lookup map.
    /// Unknown input names are silently skipped.
    pub fn compile(bindings: &KeyBindings) -> Self {
        use crate::platform::persist::DirMode;
        let mut map = HashMap::new();
        let dir_mode = bindings.dir_mode;
        for action in BindingAction::ALL {
            let app_key = binding_to_app_key(action);
            for name in bindings.get(action) {
                if let Some(pi) = name_to_physical_input(name) {
                    // Gamepad direction filter: depending on `dir_mode`, skip D-pad or stick
                    // inputs. Keyboard directions are never filtered.
                    let skip_dir = match dir_mode {
                        DirMode::DPad => matches!(pi, PhysicalInput::PadAxis(_)),
                        DirMode::Stick => matches!(
                            pi,
                            PhysicalInput::PadButton(
                                PadButton::DPadUp | PadButton::DPadDown
                                | PadButton::DPadLeft | PadButton::DPadRight
                            )
                        ),
                        DirMode::Both => false,
                    };
                    // Gamepad pause is a fixed convention (Start), never a binding. Skip any
                    // gamepad input bound to `Pause`, in case `state.json` was edited by hand.
                    let skip_pad_pause = action == BindingAction::Pause
                        && matches!(pi, PhysicalInput::PadButton(_) | PhysicalInput::PadAxis(_));
                    if skip_dir || skip_pad_pause {
                        continue;
                    }
                    map.insert(pi, app_key);
                }
            }
        }
        Self { map }
    }

    /// Look up a PhysicalInput → AppKey.
    pub fn get(&self, input: PhysicalInput) -> Option<AppKey> {
        self.map.get(&input).copied()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // The layout override changes only the display. Persisted stable names and their
    // resolution do not change.
    #[test]
    fn layout_labels_change_display_only() {
        let mut m = HashMap::new();
        m.insert("KeyA", "Q".to_string());
        set_layout_labels(m);
        // `LAYOUT_LABELS` is a `OnceLock`: it cannot be reset, and a second `set` is silently
        // ignored. This must remain the ONLY test of the binary that installs labels (no
        // other does today); otherwise fail here with a clear cause, not on the next line.
        assert!(
            LAYOUT_LABELS.get().and_then(|m| m.get("KeyA")).map(String::as_str) == Some("Q"),
            "layout labels were already installed by another test (OnceLock)"
        );
        assert_eq!(key_display_name("KeyA"), "Q"); // localized display
        assert_eq!(name_from_key(Key::KeyA), Some("KeyA")); // persistence unchanged
        assert!(matches!(key_from_name("KeyA"), Some(Key::KeyA))); // resolution unchanged
        assert_eq!(key_display_name("ArrowUp"), "\u{2191}"); // not covered: curated label
    }
    use crate::platform::persist::KeyBindings;

    #[test]
    fn compile_default_bindings_maps_arrows() {
        let compiled = CompiledBindings::compile(&KeyBindings::default());
        assert_eq!(compiled.get(PhysicalInput::Key(Key::ArrowUp)), Some(AppKey::Up));
        assert_eq!(compiled.get(PhysicalInput::Key(Key::ArrowDown)), Some(AppKey::Down));
        assert_eq!(compiled.get(PhysicalInput::Key(Key::ArrowLeft)), Some(AppKey::Left));
        assert_eq!(compiled.get(PhysicalInput::Key(Key::ArrowRight)), Some(AppKey::Right));
    }

    #[test]
    fn compile_default_bindings_maps_actions() {
        let compiled = CompiledBindings::compile(&KeyBindings::default());
        assert_eq!(compiled.get(PhysicalInput::Key(Key::Space)), Some(AppKey::Swap));
        assert_eq!(compiled.get(PhysicalInput::Key(Key::Enter)), Some(AppKey::Raise));
        assert_eq!(compiled.get(PhysicalInput::Key(Key::KeyP)), Some(AppKey::Pause));
        // Escape is not a gameplay binding. Pausing with Escape (and cancelling in
        // navigation) are fixed conventions.
        assert_eq!(compiled.get(PhysicalInput::Key(Key::Escape)), None);
    }

    #[test]
    fn capture_filters_input_type_by_tab() {
        use crate::app::meta_state::OptionsTab;
        let key = PhysicalInput::Key(Key::Space);
        let pad = PhysicalInput::PadButton(PadButton::South);
        // Keyboard tab: keys yes, gamepad no.
        assert!(input_matches_options_tab(OptionsTab::Keyboard, key));
        assert!(!input_matches_options_tab(OptionsTab::Keyboard, pad));
        // Gamepad tab: buttons yes, keys no.
        assert!(input_matches_options_tab(OptionsTab::Gamepad, pad));
        assert!(!input_matches_options_tab(OptionsTab::Gamepad, key));
        // Gamepad axes are accepted in the Gamepad tab.
        let axis = PhysicalInput::PadAxis(PadAxisDirection::LeftStickUp);
        assert!(input_matches_options_tab(OptionsTab::Gamepad, axis));
        assert!(!input_matches_options_tab(OptionsTab::Keyboard, axis));
    }

    #[test]
    fn keys_without_default_binding_return_none() {
        // WASD is not a default direction alias (rebind it if wanted); F1/Tab are unused.
        // The arrow keys stay bound: see `compile_default_bindings_maps_arrows`.
        let compiled = CompiledBindings::compile(&KeyBindings::default());
        for key in [Key::KeyW, Key::KeyA, Key::KeyD, Key::F1, Key::Tab] {
            assert_eq!(compiled.get(PhysicalInput::Key(key)), None, "{key:?}");
        }
    }

    #[test]
    fn compile_maps_gamepad_buttons() {
        let compiled = CompiledBindings::compile(&KeyBindings::default());
        assert_eq!(compiled.get(PhysicalInput::PadButton(PadButton::South)), Some(AppKey::Swap));
        assert_eq!(compiled.get(PhysicalInput::PadButton(PadButton::DPadUp)), Some(AppKey::Up));
        assert_eq!(compiled.get(PhysicalInput::PadAxis(PadAxisDirection::LeftStickLeft)), Some(AppKey::Left));
    }

    #[test]
    fn dir_mode_filters_gamepad_directions() {
        use crate::platform::persist::DirMode;
        let dpad = PhysicalInput::PadButton(PadButton::DPadUp);
        let stick = PhysicalInput::PadAxis(PadAxisDirection::LeftStickUp);
        let kbd = Key::ArrowUp; // keyboard direction: never filtered

        // DPad: D-pad active, stick ignored.
        let mut kb = KeyBindings::default();
        kb.dir_mode = DirMode::DPad;
        let c = CompiledBindings::compile(&kb);
        assert_eq!(c.get(dpad), Some(AppKey::Up));
        assert_eq!(c.get(stick), None);
        assert_eq!(c.get(PhysicalInput::Key(kbd)), Some(AppKey::Up));

        // Stick: stick active, D-pad ignored.
        kb.dir_mode = DirMode::Stick;
        let c = CompiledBindings::compile(&kb);
        assert_eq!(c.get(stick), Some(AppKey::Up));
        assert_eq!(c.get(dpad), None);
        assert_eq!(c.get(PhysicalInput::Key(kbd)), Some(AppKey::Up));

        // Both (default): both active.
        kb.dir_mode = DirMode::Both;
        let c = CompiledBindings::compile(&kb);
        assert_eq!(c.get(dpad), Some(AppKey::Up));
        assert_eq!(c.get(stick), Some(AppKey::Up));
    }

    #[test]
    fn pause_is_keyboard_rebindable() {
        // Keyboard pause is configurable: rebinding `pause` to another key compiles it to
        // `AppKey::Pause`. Gameplay resolution then adds the fixed Start/Escape conventions.
        let mut kb = KeyBindings::default();
        kb.pause = vec!["KeyG".into()];
        let c = CompiledBindings::compile(&kb);
        assert_eq!(c.get(PhysicalInput::Key(Key::KeyG)), Some(AppKey::Pause));
        assert_eq!(c.get(PhysicalInput::Key(Key::KeyP)), None);
    }

    #[test]
    fn gamepad_pause_binding_is_ignored() {
        // Gamepad pause is a fixed convention (Start), not a binding. A gamepad input
        // placed on `pause` (for example `PadStart` in an old `state.json`) is ignored.
        let mut kb = KeyBindings::default();
        kb.pause = vec!["KeyP".into(), "PadStart".into(), "PadEast".into()];
        let c = CompiledBindings::compile(&kb);
        assert_eq!(c.get(PhysicalInput::Key(Key::KeyP)), Some(AppKey::Pause)); // keyboard kept
        assert_eq!(c.get(PhysicalInput::PadButton(PadButton::Start)), None); // gamepad ignored
        assert_eq!(c.get(PhysicalInput::PadButton(PadButton::East)), None);
    }

    #[test]
    fn unknown_key_name_is_ignored() {
        let mut bindings = KeyBindings::default();
        // Replace first swap slot with a bogus name.
        bindings.swap = vec!["NonExistentKey".into(), "Space".into()];
        let compiled = CompiledBindings::compile(&bindings);
        // Space (second slot) still works.
        assert_eq!(compiled.get(PhysicalInput::Key(Key::Space)), Some(AppKey::Swap));
    }
}
