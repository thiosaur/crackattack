//! Row-based navigation of the options screens (gamepad and touch).
//!
//! It is specific to the options screens and is not a generic UI framework.
//!
//! Model:
//! - each screen is a list of rows of widgets;
//! - the focus is a 2D position `(row, col)`;
//! - Up/Down changes row; Left/Right adjusts a value widget, or moves within a row of
//!   buttons; `A` activates.
//!
//! Every options tab can be navigated with keyboard and gamepad. The `BindingSlot`,
//! `DirMode` and `ResetBindings` widgets model the binding slots, the gamepad direction
//! mode and the reset button.

use crate::app::meta_state::OptionsTab;
use crate::platform::persist::BindingAction;

/// A focusable widget of an options screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OptWidget {
    EditName,
    Language,
    Fullscreen,
    SfxEnabled,
    SfxVolume,
    MusicEnabled,
    MusicVolume,
    /// Gamepad direction mode (D-pad, stick or both). A value widget.
    DirMode,
    /// Binding slot (action, slot index). `A` starts the key capture.
    BindingSlot(BindingAction, usize),
    /// Button that resets the controls.
    ResetBindings,
    /// Back: leaves the options. Changes apply live, so this saves and closes.
    Back,
}

impl OptWidget {
    /// Returns whether this is a value widget: Left/Right adjusts it instead of changing column.
    pub fn is_value(self) -> bool {
        matches!(
            self,
            OptWidget::Language
                | OptWidget::Fullscreen
                | OptWidget::SfxEnabled
                | OptWidget::SfxVolume
                | OptWidget::MusicEnabled
                | OptWidget::MusicVolume
                | OptWidget::DirMode
        )
    }
}

/// Returns the declarative layout (rows of widgets) of an options screen.
///
/// A value widget is alone on its row, so Left/Right always means "adjust". Only rows of
/// buttons have several columns, where Left/Right means "move".
pub fn layout(tab: OptionsTab) -> Vec<Vec<OptWidget>> {
    use OptWidget::*;
    match tab {
        OptionsTab::Player => vec![
            vec![EditName],
            vec![Language],
            vec![Fullscreen],
            vec![Back],
        ],
        OptionsTab::Sound => vec![
            vec![SfxEnabled],
            vec![SfxVolume],
            vec![MusicEnabled],
            vec![MusicVolume],
            vec![Back],
        ],
        // Keyboard tab: per-key bindings.
        OptionsTab::Keyboard => {
            use BindingAction::*;
            vec![
                vec![BindingSlot(Up, 0)],
                vec![BindingSlot(Down, 0)],
                vec![BindingSlot(Left, 0)],
                vec![BindingSlot(Right, 0)],
                vec![BindingSlot(Swap, 0)],
                vec![BindingSlot(Raise, 0)],
                vec![BindingSlot(Pause, 0)],
                vec![Back, ResetBindings], // footer: Back + Reset
            ]
        }
        // Gamepad tab: direction mode and the gamepad slots of Swap and Raise.
        OptionsTab::Gamepad => {
            use BindingAction::*;
            vec![
                vec![DirMode],
                vec![BindingSlot(Swap, 1)],
                vec![BindingSlot(Raise, 1)],
                vec![Back, ResetBindings], // footer: Back + Reset
            ]
        }
    }
}

/// Returns the navigable layout: `layout` without the volume rows whose switch is off.
///
/// The volume of a disabled sound cannot be focused or adjusted, so navigation skips it.
/// The slider is still drawn (greyed out) but leaves the keyboard and gamepad path.
pub fn nav_layout(tab: OptionsTab, sfx_enabled: bool, music_enabled: bool) -> Vec<Vec<OptWidget>> {
    let mut rows = layout(tab);
    if tab == OptionsTab::Sound {
        rows.retain(|r| match r.as_slice() {
            [OptWidget::SfxVolume] => sfx_enabled,
            [OptWidget::MusicVolume] => music_enabled,
            _ => true,
        });
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flat(rows: &[Vec<OptWidget>]) -> Vec<OptWidget> {
        rows.iter().flatten().copied().collect()
    }

    #[test]
    fn sound_nav_skips_disabled_volumes() {
        // Everything on: both volumes are navigable.
        let both = flat(&nav_layout(OptionsTab::Sound, true, true));
        assert!(both.contains(&OptWidget::SfxVolume));
        assert!(both.contains(&OptWidget::MusicVolume));

        // SFX off: its volume leaves the path, the rest stays.
        let no_sfx = flat(&nav_layout(OptionsTab::Sound, false, true));
        assert!(!no_sfx.contains(&OptWidget::SfxVolume));
        assert!(no_sfx.contains(&OptWidget::SfxEnabled));
        assert!(no_sfx.contains(&OptWidget::MusicVolume));

        // Everything off: no volume, but the switches and Back stay.
        let none = flat(&nav_layout(OptionsTab::Sound, false, false));
        assert!(!none.contains(&OptWidget::SfxVolume));
        assert!(!none.contains(&OptWidget::MusicVolume));
        assert!(none.contains(&OptWidget::SfxEnabled));
        assert!(none.contains(&OptWidget::MusicEnabled));
        assert!(none.contains(&OptWidget::Back));
    }

    #[test]
    fn nav_layout_untouched_for_non_sound_tabs() {
        // The sound flags do not affect the other tabs.
        assert_eq!(
            nav_layout(OptionsTab::Player, false, false),
            layout(OptionsTab::Player)
        );
    }
}
