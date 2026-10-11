//! Presentation and input policy derivation from SessionSnapshot.
//!
//! Pure functions — no state, no side effects. Called each frame.
//! All rendering and input decisions derive from these functions.

use crackattack_core::frame_data::*;

/// What the renderer should do this frame.
/// Derived each frame from SessionSnapshot + UI state. Never stored.
pub struct StatusPresentation {
    /// Which egui modal to show (None = no modal).
    pub modal: Option<ModalKind>,
    /// Grid visibility policy.
    pub grid: GridVisibility,
}
// The result is not part of this struct: it is drawn through the snapshot ->
// `ResultVisual` path.

/// Kind of egui modal to render.
pub enum ModalKind {
    PauseMenu {
        local_ready: bool,
        remote_ready: bool,
        is_network: bool,
    },
    ReadyPrompt {
        /// Round about to be played (1-based, unbounded — open series).
        round_number: i32,
        /// Opponent already consented (the "X wants a rematch" hint).
        remote_ready: bool,
        /// Opponent still connected. False = offer contracted to [Menu]
        /// only, silently (no departure message).
        opponent_present: bool,
    },
    WaitingBanner {
        text: &'static str,
        quit_timeout_secs: f32,
    },
    /// Terminal disconnect banner — Enter/Escape both return to menu.
    DisconnectedBanner {
        text: &'static str,
    },
}

/// Grid visibility policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GridVisibility {
    /// Normal rendering — blocks and garbage visible.
    Normal,
    /// Clear blocks and garbage (between rounds).
    Cleared,
    /// Game visible but frozen in place.
    Frozen,
    /// Game visible, frozen, with dark overlay.
    FrozenDarkened,
}

/// Derive the full presentation from a session snapshot.
///
/// `pause_menu_open`: whether the egui pause menu is currently visible.
/// This is a UI state, not a session state — it can be true even when
/// the snapshot doesn't have a pause interruption (e.g., user pressed
/// Escape during ReadyPrompt).
pub fn derive_presentation(
    snap: &SessionSnapshot,
    pause_menu_open: bool,
) -> StatusPresentation {
    // Pause menu takes visual priority when open.
    // Grid visibility inherits from the base state — opening the pause
    // menu during WaitingRemoteReady keeps the board cleared, not stale.
    if pause_menu_open {
        let (local_ready, remote_ready, is_network) = match snap.interruption {
            SessionInterruption::Paused { local_ready, remote_ready, is_network } => {
                (local_ready, remote_ready, is_network)
            }
            SessionInterruption::ConnectionLost { .. }
            | SessionInterruption::Disconnected => {
                // Connection issues: always network context
                (false, false, true)
            }
            _ => {
                // Solo/VsComputer or no pause interruption
                (false, false, false)
            }
        };
        // The pause menu darkens the frozen grid.
        let base_grid = GridVisibility::FrozenDarkened;
        return StatusPresentation {
            modal: Some(ModalKind::PauseMenu { local_ready, remote_ready, is_network }),
            grid: base_grid,
        };
    }

    // No pause menu — derive from snapshot.
    let t = crate::app::locale::t();
    match snap.interruption {
        SessionInterruption::Disconnected => StatusPresentation {
            modal: Some(ModalKind::DisconnectedBanner {
                text: t.opponent_disconnected,
            }),
            grid: GridVisibility::Frozen,
        },
        SessionInterruption::ConnectionLost { .. } => StatusPresentation {
            modal: Some(ModalKind::WaitingBanner {
                text: t.connection_lost_banner,
                quit_timeout_secs: 15.0,
            }),
            grid: GridVisibility::Frozen,
        },
        SessionInterruption::Paused { .. } => {
            // Pause without menu open — the auto-open logic in tick_session
            // will open the menu next frame. Show frozen grid in the meantime.
            StatusPresentation {
                modal: None,
                grid: GridVisibility::FrozenDarkened,
            }
        }
        SessionInterruption::ScenePreparing => StatusPresentation {
            modal: None,
            grid: GridVisibility::Normal, // grid visible under opening curtain
        },
        SessionInterruption::ResumeSync { .. } => StatusPresentation {
            modal: None,
            grid: GridVisibility::Frozen,
        },
        SessionInterruption::AwaitingRoundVerdict => StatusPresentation {
            modal: None,
            grid: GridVisibility::Frozen,
        },
        SessionInterruption::ReadyPrompt { round_number, remote_ready, opponent_present } => {
            // Grid always visible during ReadyPrompt (round 1: fresh start,
            // round 2+: curtain already opened, new grid ready).
            StatusPresentation {
                modal: Some(ModalKind::ReadyPrompt { round_number, remote_ready, opponent_present }),
                grid: GridVisibility::Normal,
            }
        }
        SessionInterruption::WaitingRemoteReady { .. } => StatusPresentation {
            modal: Some(ModalKind::WaitingBanner {
                text: t.waiting_for_opponent,
                quit_timeout_secs: 15.0,
            }),
            grid: GridVisibility::Normal,
        },
        SessionInterruption::None => {
            StatusPresentation {
                modal: None,
                grid: GridVisibility::Normal,
            }
        }
    }
}

/// Input policy for the current frame.
/// Derived from SessionSnapshot + UI state.
pub struct InputPolicy {
    /// Allow gameplay keys (arrows, swap, advance).
    pub gameplay: bool,
    /// What Escape does.
    pub escape_action: EscapeAction,
    /// What Enter/Space does (if anything special).
    pub confirm_action: Option<ConfirmAction>,
}

/// What Escape does in the current state.
pub enum EscapeAction {
    /// Open the pause menu (and request pause if game is running).
    OpenPauseMenu,
    /// Resume from pause (close menu). Used for solo/vs-computer.
    Resume,
    /// Signal ready to resume (network). Does not close the menu.
    SignalResumeReady,
    /// Do nothing (input blocked).
    Nothing,
    /// Go back to menu.
    TransitionMenu,
}

/// Special action for Enter/Space.
pub enum ConfirmAction {
    /// Signal ready for next round.
    SignalReady,
    /// Enter post-game flow.
    EnterPostGame,
    /// Advance to next post-game screen (HighScores → Stats).
    AdvancePostGameScreen,
    /// Return to menu from the final post-game screen (Stats).
    ReturnToMenu,
}

/// Derive the input policy from a session snapshot.
///
/// `pause_menu_open`: whether the egui pause menu is currently visible.
/// `post_game_screen`: when in PostGame flow, which sub-screen is active.
pub fn derive_input(
    snap: &SessionSnapshot,
    pause_menu_open: bool,
    post_game_screen: Option<crate::app::post_game::PostGameScreen>,
) -> InputPolicy {
    // Pause menu open — egui handles most input.
    // Escape is handled by the modal itself (Resume or close).
    if pause_menu_open {
        return InputPolicy {
            gameplay: false,
            escape_action: EscapeAction::Nothing, // egui modal handles Escape
            confirm_action: None,
        };
    }

    // Interruption determines input.
    match snap.interruption {
        SessionInterruption::ScenePreparing => InputPolicy {
            gameplay: false,
            escape_action: EscapeAction::Nothing,
            confirm_action: None,
        },
        SessionInterruption::ReadyPrompt { .. } => InputPolicy {
            gameplay: false,
            escape_action: EscapeAction::OpenPauseMenu,
            confirm_action: Some(ConfirmAction::SignalReady),
        },
        SessionInterruption::Paused { local_ready, is_network, .. } => {
            if !is_network {
                // Solo/VsComputer: Escape resumes immediately
                InputPolicy {
                    gameplay: false,
                    escape_action: EscapeAction::Resume,
                    confirm_action: None,
                }
            } else if !local_ready {
                // Network: not yet ready → signal ready
                InputPolicy {
                    gameplay: false,
                    escape_action: EscapeAction::SignalResumeReady,
                    confirm_action: None,
                }
            } else {
                // Network: already ready, waiting for opponent
                InputPolicy {
                    gameplay: false,
                    escape_action: EscapeAction::Nothing,
                    confirm_action: None,
                }
            }
        }
        SessionInterruption::WaitingRemoteReady { .. }
        | SessionInterruption::ConnectionLost { .. } => InputPolicy {
            gameplay: false,
            escape_action: EscapeAction::OpenPauseMenu,
            confirm_action: None,
        },
        SessionInterruption::ResumeSync { .. } => InputPolicy {
            gameplay: false,
            escape_action: EscapeAction::Nothing,
            confirm_action: None,
        },
        SessionInterruption::AwaitingRoundVerdict => InputPolicy {
            gameplay: false,
            escape_action: EscapeAction::Nothing,
            confirm_action: None,
        },
        SessionInterruption::Disconnected => InputPolicy {
            gameplay: false,
            escape_action: EscapeAction::Nothing, // egui modal handles Escape
            confirm_action: None, // egui modal handles Enter
        },
        SessionInterruption::None => {
            // No interruption — depends on lifecycle.
            match snap.lifecycle {
                SessionLifecycle::Countdown => InputPolicy {
                    gameplay: false,
                    escape_action: EscapeAction::OpenPauseMenu,
                    confirm_action: None,
                },
                SessionLifecycle::Playing => InputPolicy {
                    gameplay: true,
                    escape_action: EscapeAction::OpenPauseMenu,
                    confirm_action: None,
                },
                SessionLifecycle::Resolving | SessionLifecycle::ShowResult => InputPolicy {
                    gameplay: false,
                    escape_action: EscapeAction::Nothing,
                    confirm_action: None,
                },
                SessionLifecycle::Ended => {
                    let confirm = match post_game_screen {
                        None => Some(ConfirmAction::EnterPostGame),
                        Some(screen) if screen.is_final() => Some(ConfirmAction::ReturnToMenu),
                        Some(_) => Some(ConfirmAction::AdvancePostGameScreen),
                    };
                    InputPolicy {
                        gameplay: false,
                        escape_action: EscapeAction::TransitionMenu,
                        confirm_action: confirm,
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snap(
        lifecycle: SessionLifecycle,
        interruption: SessionInterruption,
        result: SessionResult,
    ) -> SessionSnapshot {
        SessionSnapshot { lifecycle, interruption, result, vs_display: None, countdown: None, round_engaged: false }
    }

    #[test]
    fn pres_playing_no_interruption() {
        let p = derive_presentation(&snap(
            SessionLifecycle::Playing, SessionInterruption::None, SessionResult::None,
        ), false);
        assert!(p.modal.is_none());
        assert_eq!(p.grid, GridVisibility::Normal);
    }

    /// The pause menu copies the interruption's flags (solo, network, local ready).
    #[test]
    fn pres_paused_menu_open_copies_flags() {
        for (local_ready, remote_ready, is_network) in
            [(false, false, false), (false, false, true), (true, false, true)]
        {
            let p = derive_presentation(&snap(
                SessionLifecycle::Playing,
                SessionInterruption::Paused { local_ready, remote_ready, is_network },
                SessionResult::None,
            ), true);
            let case = format!("local={local_ready} remote={remote_ready} network={is_network}");
            match p.modal {
                Some(ModalKind::PauseMenu { local_ready: l, remote_ready: r, is_network: n, .. }) => {
                    assert_eq!((l, r, n), (local_ready, remote_ready, is_network), "{case}");
                }
                _ => panic!("{case}: expected a PauseMenu modal"),
            }
        }
    }

    /// Round 1 (fresh start) and round 2+ (curtain already open, new grid ready): the
    /// ready prompt keeps the grid visible in both cases.
    #[test]
    fn pres_ready_prompt_keeps_grid() {
        for round_number in [1, 2] {
            let p = derive_presentation(&snap(
                SessionLifecycle::Playing,
                SessionInterruption::ReadyPrompt { round_number, remote_ready: false, opponent_present: true },
                SessionResult::None,
            ), false);
            assert!(
                matches!(p.modal, Some(ModalKind::ReadyPrompt { round_number: n, .. }) if n == round_number),
                "round {round_number}"
            );
            assert_eq!(p.grid, GridVisibility::Normal, "round {round_number}");
        }
    }

    #[test]
    fn pres_waiting_remote_keeps_grid() {
        let p = derive_presentation(&snap(
            SessionLifecycle::Playing,
            SessionInterruption::WaitingRemoteReady { started_tick: 0 },
            SessionResult::None,
        ), false);
        assert!(matches!(p.modal, Some(ModalKind::WaitingBanner { .. })));
        assert_eq!(p.grid, GridVisibility::Normal);
    }

    #[test]
    fn pres_connection_lost_keeps_grid() {
        let p = derive_presentation(&snap(
            SessionLifecycle::Playing,
            SessionInterruption::ConnectionLost { started_tick: 0 },
            SessionResult::None,
        ), false);
        assert!(matches!(p.modal, Some(ModalKind::WaitingBanner { .. })));
        assert_eq!(p.grid, GridVisibility::Frozen);
    }

    #[test]
    fn pres_resume_sync_silent() {
        let p = derive_presentation(&snap(
            SessionLifecycle::Playing,
            SessionInterruption::ResumeSync { started_tick: 0 },
            SessionResult::None,
        ), false);
        assert!(p.modal.is_none());
        assert_eq!(p.grid, GridVisibility::Frozen);
    }

    // The result is not part of `StatusPresentation`: it is drawn through
    // snapshot -> `ResultVisual::from_result`. These two tests check both halves of that
    // contract: no modal covers the result screen, and the visual mapping exists.
    #[test]
    fn pres_show_result_shows_result() {
        use crate::renderer::ui::ui_renderer::ResultVisual;
        let p = derive_presentation(&snap(
            SessionLifecycle::ShowResult, SessionInterruption::None, SessionResult::RoundWin,
        ), false);
        assert!(p.modal.is_none());
        assert_eq!(ResultVisual::from_result(SessionResult::RoundWin), Some(ResultVisual::RoundWin));
    }

    #[test]
    fn pres_ended_shows_result() {
        use crate::renderer::ui::ui_renderer::ResultVisual;
        let p = derive_presentation(&snap(
            SessionLifecycle::Ended, SessionInterruption::None, SessionResult::SessionWin,
        ), false);
        assert!(p.modal.is_none());
        assert_eq!(ResultVisual::from_result(SessionResult::SessionWin), Some(ResultVisual::Winner));
    }

    #[test]
    fn pres_disconnected_shows_banner() {
        let p = derive_presentation(&snap(
            SessionLifecycle::Playing, SessionInterruption::Disconnected, SessionResult::None,
        ), false);
        assert!(matches!(p.modal, Some(ModalKind::DisconnectedBanner { .. })));
    }

    #[test]
    fn pres_pause_during_waiting_keeps_grid_darkened() {
        let p = derive_presentation(&snap(
            SessionLifecycle::Playing,
            SessionInterruption::WaitingRemoteReady { started_tick: 0 },
            SessionResult::None,
        ), true);
        assert!(matches!(p.modal, Some(ModalKind::PauseMenu { .. })));
        assert_eq!(p.grid, GridVisibility::FrozenDarkened);
    }

    #[test]
    fn input_gameplay_during_playing() {
        let i = derive_input(&snap(
            SessionLifecycle::Playing, SessionInterruption::None, SessionResult::None,
        ), false, None);
        assert!(i.gameplay);
        assert!(matches!(i.escape_action, EscapeAction::OpenPauseMenu));
        assert!(i.confirm_action.is_none());
    }

    #[test]
    fn input_blocked_during_pause_menu() {
        let i = derive_input(&snap(
            SessionLifecycle::Playing,
            SessionInterruption::Paused { local_ready: false, remote_ready: false, is_network: true },
            SessionResult::None,
        ), true, None);
        assert!(!i.gameplay);
        assert!(matches!(i.escape_action, EscapeAction::Nothing));
    }

    #[test]
    fn input_resume_solo_pause() {
        // Solo/VsComputer: Escape resumes immediately
        let i = derive_input(&snap(
            SessionLifecycle::Playing,
            SessionInterruption::Paused { local_ready: false, remote_ready: false, is_network: false },
            SessionResult::None,
        ), false, None);
        assert!(matches!(i.escape_action, EscapeAction::Resume));
    }

    #[test]
    fn input_signal_resume_ready_network() {
        // Network: not yet ready → signal ready
        let i = derive_input(&snap(
            SessionLifecycle::Playing,
            SessionInterruption::Paused { local_ready: false, remote_ready: false, is_network: true },
            SessionResult::None,
        ), false, None);
        assert!(matches!(i.escape_action, EscapeAction::SignalResumeReady));
    }

    #[test]
    fn input_nothing_when_already_ready_network() {
        let i = derive_input(&snap(
            SessionLifecycle::Playing,
            SessionInterruption::Paused { local_ready: true, remote_ready: false, is_network: true },
            SessionResult::None,
        ), false, None);
        assert!(matches!(i.escape_action, EscapeAction::Nothing));
    }

    #[test]
    fn input_ready_prompt_confirm() {
        let i = derive_input(&snap(
            SessionLifecycle::Playing,
            SessionInterruption::ReadyPrompt { round_number: 1, remote_ready: false, opponent_present: true },
            SessionResult::None,
        ), false, None);
        assert!(!i.gameplay);
        assert!(matches!(i.confirm_action, Some(ConfirmAction::SignalReady)));
    }

    #[test]
    fn input_nothing_during_show_result() {
        let i = derive_input(&snap(
            SessionLifecycle::ShowResult, SessionInterruption::None, SessionResult::RoundWin,
        ), false, None);
        assert!(!i.gameplay);
        assert!(matches!(i.escape_action, EscapeAction::Nothing));
    }

    #[test]
    fn input_post_game_on_ended() {
        // No post_game_screen (None) → first confirm enters PostGame
        let i = derive_input(&snap(
            SessionLifecycle::Ended, SessionInterruption::None, SessionResult::SessionWin,
        ), false, None);
        assert!(matches!(i.escape_action, EscapeAction::TransitionMenu));
        assert!(matches!(i.confirm_action, Some(ConfirmAction::EnterPostGame)));
    }

    #[test]
    fn input_post_game_advance_screen() {
        use crate::app::post_game::PostGameScreen;
        // PostGameScreen::ResultMessage → advance
        let i = derive_input(&snap(
            SessionLifecycle::Ended, SessionInterruption::None, SessionResult::SessionWin,
        ), false, Some(PostGameScreen::ResultMessage));
        assert!(matches!(i.confirm_action, Some(ConfirmAction::AdvancePostGameScreen)));

        // PostGameScreen::HighScores → advance
        let i = derive_input(&snap(
            SessionLifecycle::Ended, SessionInterruption::None, SessionResult::SessionWin,
        ), false, Some(PostGameScreen::HighScores));
        assert!(matches!(i.confirm_action, Some(ConfirmAction::AdvancePostGameScreen)));
    }

    #[test]
    fn input_post_game_return_to_menu() {
        use crate::app::post_game::PostGameScreen;
        // PostGameScreen::Stats (final) → return to menu
        let i = derive_input(&snap(
            SessionLifecycle::Ended, SessionInterruption::None, SessionResult::SessionWin,
        ), false, Some(PostGameScreen::Stats));
        assert!(matches!(i.confirm_action, Some(ConfirmAction::ReturnToMenu)));
    }

    #[test]
    fn input_blocked_during_resume_sync() {
        let i = derive_input(&snap(
            SessionLifecycle::Playing,
            SessionInterruption::ResumeSync { started_tick: 0 },
            SessionResult::None,
        ), false, None);
        assert!(!i.gameplay);
        assert!(matches!(i.escape_action, EscapeAction::Nothing));
    }
}
