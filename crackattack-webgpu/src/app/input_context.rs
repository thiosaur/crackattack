//! Active input context: **exactly one at a time**.
//!
//! Pure, testable decision logic that picks how a physical input is resolved into an
//! intent, based on the session state. The wiring lives in `dispatch_input` (`main.rs`).

use crate::platform::persist::BindingAction;

/// The active input context. It decides how the current physical event is resolved
/// (raw capture, gameplay bindings or UI conventions).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Context {
    /// Binding capture armed (options): the next **raw** input is assigned.
    /// Takes priority over everything else.
    Capture { action: BindingAction, slot: usize },
    /// Bare main menu (no overlay).
    MenuRoot,
    /// Options overlay open, row-based navigation.
    MenuOptions,
    /// Options overlay with the on-screen keyboard open: text entry.
    MenuOptionsTextEdit,
    /// LAN screen open.
    MenuLan,
    /// Game active: gameplay bindings plus the conventional pause.
    Gameplay,
    /// In-game UI sub-state (ready, waiting, post-game): UI conventions, no gameplay.
    InGameUi,
    /// Pause menu open.
    PauseMenu,
}

/// Family of `flow_state`, reduced to what the context decision needs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContextFlow {
    /// `FlowState::Menu`.
    Menu,
    /// `FlowState::Playing` ou `FlowState::PostGame`.
    InGame,
}

/// Minimal inputs needed to decide the active context.
/// Deliberately flat (booleans and options) so they can be tested without `AppState`.
#[derive(Debug, Clone, Copy)]
pub struct ContextInputs {
    /// Binding capture armed in the options, if any.
    pub capture: Option<(BindingAction, usize)>,
    pub flow: ContextFlow,
    /// Options overlay open.
    pub options_open: bool,
    /// On-screen keyboard open in the options.
    pub options_osk_open: bool,
    /// LAN screen open.
    pub lan_open: bool,
    /// Pause menu open.
    pub pause_menu_open: bool,
    /// Game actually playable this frame (`derive_input(...).gameplay`).
    pub gameplay_active: bool,
}

/// Decides the active context. **Pure**: same input, same output, no side effects.
///
/// Priority order:
/// 1. armed capture, which wins everywhere;
/// 2. by flow: menu (plus overlays) or in-game (pause menu, gameplay or UI sub-state).
pub fn resolve_context(inp: &ContextInputs) -> Context {
    // 1. Capture sits on top and short-circuits everything.
    if let Some((action, slot)) = inp.capture {
        return Context::Capture { action, slot };
    }
    match inp.flow {
        ContextFlow::Menu => {
            // Overlays win over the bare menu (LAN first, then options, as in `route_menu_input`).
            if inp.lan_open {
                Context::MenuLan
            } else if inp.options_open {
                if inp.options_osk_open {
                    Context::MenuOptionsTextEdit
                } else {
                    Context::MenuOptions
                }
            } else {
                Context::MenuRoot
            }
        }
        ContextFlow::InGame => {
            if inp.pause_menu_open {
                Context::PauseMenu
            } else if inp.gameplay_active {
                Context::Gameplay
            } else {
                Context::InGameUi
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base() -> ContextInputs {
        ContextInputs {
            capture: None,
            flow: ContextFlow::Menu,
            options_open: false,
            options_osk_open: false,
            lan_open: false,
            pause_menu_open: false,
            gameplay_active: false,
        }
    }

    #[test]
    fn menu_root_when_no_overlay() {
        assert_eq!(resolve_context(&base()), Context::MenuRoot);
    }

    #[test]
    fn menu_overlays() {
        let mut i = base();
        i.options_open = true;
        assert_eq!(resolve_context(&i), Context::MenuOptions);
        i.options_osk_open = true;
        assert_eq!(resolve_context(&i), Context::MenuOptionsTextEdit);
        // LAN wins over options (same order as `route_menu_input`).
        i.lan_open = true;
        assert_eq!(resolve_context(&i), Context::MenuLan);
    }

    #[test]
    fn capture_wins_over_everything() {
        let mut i = base();
        i.options_open = true;
        i.options_osk_open = true;
        i.capture = Some((BindingAction::Up, 1));
        assert_eq!(resolve_context(&i), Context::Capture { action: BindingAction::Up, slot: 1 });
    }

    #[test]
    fn ingame_gameplay_vs_substate_vs_pause() {
        let mut i = base();
        i.flow = ContextFlow::InGame;
        i.gameplay_active = true;
        assert_eq!(resolve_context(&i), Context::Gameplay);
        // UI sub-state (ready, post-game): no gameplay.
        i.gameplay_active = false;
        assert_eq!(resolve_context(&i), Context::InGameUi);
        // The pause menu wins over both.
        i.pause_menu_open = true;
        i.gameplay_active = true;
        assert_eq!(resolve_context(&i), Context::PauseMenu);
    }
}
