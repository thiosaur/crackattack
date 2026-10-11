//! Input decision logic: reads state and produces `AppCommand`s.
//!
//! This module has no side effects. It reads an `InputContext`
//! snapshot and returns commands for the caller to execute.

use crate::app::command::{AppCommand, AppKey, Direction};
use crate::app::meta_state::{FlowState, GameMode};

/// Read-only snapshot of session state needed for input decisions.
pub struct InputContext {
    pub transition_cooldown: i32,
    // Menu-specific context.
    pub can_go_back: bool,
    pub menu_confirm_action: MenuConfirmAction,
    pub game_seed: u32,
    pub ranking_entry_score: i32,
}

/// Pre-computed result of what a confirm press does in the menu.
/// Keeps `menu_state` mutation out of the pure handler.
#[derive(Debug, Clone)]
pub enum MenuConfirmAction {
    /// The selected entry opens the options modal.
    OpenOptions,
    /// The selected entry opens the network connection modal (automatic discovery).
    OpenNetwork,
    /// The selected entry opens the network screen directly on the manual-connect view.
    OpenNetworkManual,
    /// The selected entry opens a section of the Info screen (rules / credits).
    OpenInfo(crate::app::menu_tree::InfoSection),
    /// The selected entry confirms a game mode.
    SelectMode(GameMode),
    /// The selected entry enters a sub-menu or is disabled.
    /// `apply_app_command` calls `menu_state.confirm()` to do the mutation.
    MenuConfirm,
    /// The selected entry quits the game.
    Quit,
    /// The selected entry returns to the parent menu.
    Back,
}

/// Handles gameplay input. Only called when `derive_input` reports gameplay.
/// Pause, game-over confirm and non-gameplay states are handled by
/// `derive_input()` before this function is reached.
pub fn handle_playing_input(
    _ctx: &InputContext,
    key: AppKey,
    pressed: bool,
) -> Vec<AppCommand> {
    use crackattack_core::input::*;

    let action = match key {
        AppKey::Left => INPUT_LEFT,
        AppKey::Right => INPUT_RIGHT,
        AppKey::Up => INPUT_UP,
        AppKey::Down => INPUT_DOWN,
        AppKey::Swap => INPUT_SWAP,
        AppKey::Raise => INPUT_ADVANCE,
        // Pause and cancel are handled by `derive_input` (opens the pause menu).
        AppKey::Pause | AppKey::Cancel => return vec![],
    };
    vec![AppCommand::SetGameplayKey { action, pressed }]
}

/// Handles a UI intent in the main menu (no overlay open).
///
/// Physical keys are already resolved to a `UiIntent` upstream, so `Confirm`
/// needs no alias. Overlays (options, LAN) are routed elsewhere, by
/// `route_options_intent` and `route_lan_intent`.
pub fn handle_menu_input(
    ctx: &InputContext,
    intent: crate::app::ui_intent::UiIntent,
) -> Vec<AppCommand> {
    use crate::app::ui_intent::{UiDir, UiIntent};
    match intent {
        UiIntent::Cancel => {
            if ctx.can_go_back {
                // Sub-menu: go back at once (not destructive).
                vec![AppCommand::MenuBack]
            } else {
                // Root: do not quit directly. Focus the Quit icon instead; a second
                // confirm quits. This avoids accidental exits.
                vec![AppCommand::MenuFocusQuit]
            }
        }
        // Linear cursor: Up and Left go to the previous entry, Down and Right to the next.
        // Left/Right reach the bottom icon bar (options, info, quit), which is laid out
        // horizontally.
        // The runtime plays the navigation sound, only if the selection moved.
        UiIntent::Navigate(UiDir::Up) | UiIntent::Navigate(UiDir::Left) =>
            vec![AppCommand::MenuNavigate(Direction::Up)],
        UiIntent::Navigate(UiDir::Down) | UiIntent::Navigate(UiDir::Right) =>
            vec![AppCommand::MenuNavigate(Direction::Down)],
        UiIntent::Confirm => {
            match &ctx.menu_confirm_action {
                MenuConfirmAction::OpenOptions => vec![
                    AppCommand::OpenOptionsModal { tab: None },
                    AppCommand::PlayUiConfirm,
                ],
                MenuConfirmAction::OpenNetwork => vec![
                    AppCommand::OpenLanScreen { manual: false },
                    AppCommand::PlayUiConfirm,
                ],
                MenuConfirmAction::OpenNetworkManual => vec![
                    // Manual connection right away, without mDNS discovery.
                    AppCommand::OpenLanScreen { manual: true },
                    AppCommand::PlayUiConfirm,
                ],
                MenuConfirmAction::OpenInfo(section) => vec![
                    AppCommand::OpenInfoScreen(*section),
                    AppCommand::PlayUiConfirm,
                ],
                MenuConfirmAction::SelectMode(mode) => {
                    // All modes: create game and transition directly to Playing.
                    // VS modes start in WaitingBothReady — the ReadyPrompt egui
                    // modal handles the "are you ready?" step.
                    vec![
                        AppCommand::PlayUiConfirm,
                        AppCommand::SelectMode(mode.clone()),
                        AppCommand::CreateGame {
                            mode: mode.clone(),
                            seed: ctx.game_seed,
                            ranking_entry_score: ctx.ranking_entry_score,
                        },
                        AppCommand::StartTransition(FlowState::Playing, 0.5),
                    ]
                }
                MenuConfirmAction::MenuConfirm => {
                    // Sub-menu entry or similar — delegate to apply_app_command
                    // which calls menu_state.confirm() for the mutation.
                    vec![AppCommand::PlayUiConfirm, AppCommand::ConfirmMenuSelection]
                }
                MenuConfirmAction::Quit => vec![AppCommand::RequestExit],
                MenuConfirmAction::Back => vec![AppCommand::PlayUiConfirm, AppCommand::MenuBack],
            }
        }
        // Pause, text and backspace intents have no effect in the main menu.
        _ => vec![],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx_with(action: MenuConfirmAction, _overlay_open: bool, can_go_back: bool) -> InputContext {
        InputContext {
            transition_cooldown: 0,
            can_go_back,
            menu_confirm_action: action,
            game_seed: 0,
            ranking_entry_score: 0,
        }
    }

    /// `UiIntent::Confirm` validates according to the menu action.
    #[test]
    fn confirm_validates_menu() {
        use crate::app::ui_intent::UiIntent;
        // (action, name of the command that must be produced)
        let actions = [
            (MenuConfirmAction::OpenOptions, "OpenOptionsModal"),
            (MenuConfirmAction::OpenNetwork, "OpenLanScreen"),
            (MenuConfirmAction::SelectMode(GameMode::Solo), "CreateGame"),
            (MenuConfirmAction::MenuConfirm, "ConfirmMenuSelection"),
        ];
        for (action, expected) in actions {
            let cmds = handle_menu_input(&ctx_with(action.clone(), false, false), UiIntent::Confirm);
            let dump = format!("{cmds:?}");
            assert!(dump.contains(expected), "Confirm on {action:?} must produce {expected}: {dump}");
        }
    }

    /// `UiIntent::Cancel` goes back in a sub-menu and focuses Quit at the root; `Navigate` does not.
    #[test]
    fn cancel_goes_back_navigate_does_not() {
        use crate::app::ui_intent::{UiDir, UiIntent};
        // Sub-menu: Cancel goes back.
        let back = handle_menu_input(&ctx_with(MenuConfirmAction::MenuConfirm, false, true), UiIntent::Cancel);
        assert_eq!(format!("{back:?}"), format!("{:?}", vec![AppCommand::MenuBack]));

        // Root: Cancel does not quit; it focuses the Quit icon.
        let root = handle_menu_input(&ctx_with(MenuConfirmAction::MenuConfirm, false, false), UiIntent::Cancel);
        assert!(root.iter().any(|c| matches!(c, AppCommand::MenuFocusQuit)));
        assert!(!root.iter().any(|c| matches!(c, AppCommand::RequestExit)));

        // Navigate(Up) moves the cursor; it does not go back.
        let nav = handle_menu_input(&ctx_with(MenuConfirmAction::MenuConfirm, false, true), UiIntent::Navigate(UiDir::Up));
        assert!(nav.iter().any(|c| matches!(c, AppCommand::MenuNavigate(_))));
    }
}
