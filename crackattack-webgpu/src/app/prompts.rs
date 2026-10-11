//! Dynamic input prompts.
//!
//! Resolves UI action labels (Confirm, Back, Pause, Navigate) into a **keyboard key**
//! name or a **gamepad button** name, based on the last input source
//! (`input_source::last()`) and the current language.
//!
//! The output is plain text. The sentence templates (`_fmt`) stay in `locale`, so the
//! connecting word ("to") can be translated; this module only injects the key label.
//!
//! The label shown is the **canonical** one for the defaults (Enter/Esc on the keyboard,
//! A/B on the gamepad), not the user's persisted binding. On a gamepad, `Confirm` is `A`
//! and `Back` is `B`, matching the UI invariant (PadSouth confirms, PadEast goes back).

use crate::app::locale::t;
use crate::input_source;

/// Logical UI action that a prompt refers to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Back,
}

/// Short text label of the key or button bound to `action`, for the last input
/// source and the current language.
pub fn key_label(action: Action) -> &'static str {
    let pad = input_source::is_gamepad();
    match action {
        Action::Back if pad => "B",
        // The localized "Esc" label comes from `Locale` (`key_esc`).
        Action::Back => t().key_esc,
    }
}

// --- Composed prompts ---

/// "Cancel capture" hint (Controls tab of the options).
pub fn cancel_capture_hint() -> String {
    t().cancel_capture_hint_fmt.replace("{}", key_label(Action::Back))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::locale::{set_language, Language};
    use crate::input_source::{set, InputSource};

    /// Labels follow the input source (keyboard gives key names, gamepad gives button
    /// names) and the templates are fully filled (no leftover "{}").
    ///
    /// A **single** test: `set_language` and `input_source::set` mutate global state, so
    /// this avoids two parallel tests interleaving. It **restores** the defaults
    /// (English, keyboard) on exit so other tests are not affected.
    #[test]
    fn prompts_adapt_to_input_source() {
        // Same lock as the `locale` wiring test (global `CURRENT_LANGUAGE`).
        let _lang = crate::app::locale::LangGuard::new();
        // Restores the input source even if an assertion fails below.
        struct RestoreSource(InputSource);
        impl Drop for RestoreSource {
            fn drop(&mut self) {
                input_source::set(self.0);
            }
        }
        let _source = RestoreSource(input_source::last());
        set_language(Language::English);

        // Keyboard.
        set(InputSource::Keyboard);
        assert_eq!(key_label(Action::Back), "Esc");
        assert!(cancel_capture_hint().contains("Esc"));

        // Gamepad.
        set(InputSource::Gamepad);
        assert_eq!(key_label(Action::Back), "B");
        assert!(cancel_capture_hint().contains('B'));
    }
}
