//! VsComputerGameSession — wraps VsSession<GridOpponent>.
//!
//! All VS Computer-specific behavior lives here, not in the runtime.

use crate::frame_data::FrameData;
use crate::game_state::GameState;
use crate::input::InputSnapshot;
use super::grid_opponent::GridOpponent;
use super::opponent::Opponent;
use super::session::VsSession;
use super::round::SessionPhase;
use super::game_session::GameSession;

/// VS Computer game session.
pub struct VsComputerGameSession {
    pub inner: VsSession<GridOpponent>,
    /// Local pause state (the only cause of freeze in VS Computer).
    frozen: bool,
}

impl VsComputerGameSession {
    /// Forces the "Ready?" prompt. The runtime calls this right after creation,
    /// as it does for network sessions.
    pub fn set_needs_ready_prompt(&mut self, v: bool) {
        self.inner.set_needs_ready_prompt(v);
    }

    pub fn new(seed: u32, opponent: GridOpponent) -> Self {
        let mut inner = VsSession::new(seed, opponent);
        // Open series, same model as network mode: the player consents ("Play again?")
        // between rounds. In production, `session_runtime` then calls
        // `set_needs_ready_prompt(true)`, which also puts round 1 in `WaitingBothReady`,
        // so the "Ready?" prompt shows before round 1 too. This call only sets the
        // base between-round behavior (round 1 starts directly), which tests that
        // build the session without `session_runtime` rely on.
        inner.set_ready_prompt_between_rounds(true);
        Self {
            inner,
            frozen: false,
        }
    }
}

impl GameSession for VsComputerGameSession {
    fn tick(&mut self, input: &InputSnapshot) -> FrameData {
        // Freeze check first — nothing advances while paused.
        if self.frozen {
            self.inner.opponent.time_step();
            return self.inner.player_game.frame_data();
        }
        self.inner.time_step(input)
    }

    fn frame_data(&self) -> FrameData {
        self.inner.player_game.frame_data()
    }

    fn signal_local_ready(&mut self) {
        self.inner.signal_local_ready();
    }

    fn abandon(&mut self) {
        // The opponent is local, so there is nobody to notify. Leaving the session
        // is enough (network sessions send a forfeit instead).
    }

    fn request_pause(&mut self) -> bool {
        self.frozen = true;
        true
    }

    fn request_resume(&mut self) -> bool {
        self.frozen = false;
        true
    }

    fn snapshot(&self) -> crate::frame_data::SessionSnapshot {
        use crate::frame_data::*;

        let lifecycle = match self.inner.phase() {
            SessionPhase::Countdown => SessionLifecycle::Countdown,
            SessionPhase::Playing => SessionLifecycle::Playing,
            SessionPhase::Resolving => SessionLifecycle::Resolving,
            SessionPhase::ShowResult => SessionLifecycle::ShowResult,
            SessionPhase::Prepared
            | SessionPhase::WaitingBothReady | SessionPhase::WaitingRemote
            | SessionPhase::AwaitingVerdict => {
                SessionLifecycle::Playing
            }
        };

        let interruption = if self.frozen {
            SessionInterruption::Paused { local_ready: false, remote_ready: false, is_network: false }
        } else {
            match self.inner.phase() {
                SessionPhase::Prepared => SessionInterruption::ScenePreparing,
                SessionPhase::WaitingBothReady => SessionInterruption::ReadyPrompt {
                    round_number: self.inner.round_number,
                    // The AI never announces first and never leaves.
                    remote_ready: false,
                    opponent_present: true,
                },
                // WaitingRemote never visible in VS Computer
                _ => SessionInterruption::None,
            }
        };

        let result = match self.inner.phase() {
            SessionPhase::Resolving | SessionPhase::ShowResult => match self.inner.round_outcome {
                Some(super::round::RoundOutcome::PlayerWon) => SessionResult::RoundWin,
                Some(super::round::RoundOutcome::PlayerLost) => SessionResult::RoundLose,
                None => SessionResult::None,
            },
            _ => SessionResult::None,
        };

        let vs_display = Some(VsDisplay {
            opponent_name: self.inner.opponent_name().to_string(),
            // opponent_level is read via opponent_grid_level() (mut) each tick.
            // In snapshot() (immutable), we report the last known value.
            opponent_level: self.inner.cached_opponent_level,
            round_number: self.inner.round_number,
            session_wins: self.inner.session_wins,
            session_losses: self.inner.session_losses,
            // Handed over once the round is over, as the network reports it: not while it is played.
            opponent_stats: match self.inner.phase() {
                SessionPhase::Resolving | SessionPhase::ShowResult => self.inner.opponent.final_stats().cloned(),
                _ => None,
            },
            opponent_uuid: None, // Local AI: no persistent identity
            opponent_board: self.inner.opponent.board_view().cloned(),
        });

        SessionSnapshot {
            lifecycle, interruption, result, vs_display,
            countdown: self.inner.countdown_info(),
            round_engaged: super::round::is_round_engaged(self.inner.phase()),
        }
    }

    fn drain_events(&mut self) -> Vec<crate::frame_data::SessionEvent> {
        std::mem::take(&mut self.inner.pending_events)
    }

    fn game_state_mut(&mut self) -> Option<&mut GameState> {
        Some(&mut self.inner.player_game)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frame_data::{SessionInterruption, SessionLifecycle};
    use crate::ai_difficulty::AIDifficulty;

    // Contract tests: the snapshot and drained events reflect the real session state.

    fn vs_ia() -> VsComputerGameSession {
        VsComputerGameSession::new(42, GridOpponent::new(AIDifficulty::Medium, 42))
    }

    /// The opponent's stats reach the snapshot once the round is over, and not while it is played.
    #[test]
    fn the_opponents_stats_are_handed_over_at_the_end_of_the_round() {
        use crate::input::{Input, INPUT_ADVANCE, INPUT_DOWN, INPUT_LEFT, INPUT_RIGHT, INPUT_SWAP, INPUT_UP};
        let mut s = VsComputerGameSession::new(42, GridOpponent::new(AIDifficulty::Hard, 42));
        let mut human = crate::vs::GridAutoplay::new(AIDifficulty::Hard, 42);
        let mut input = Input::new();
        let mut during = None;
        for tick in 0..(50 * 60 * 6) {
            let flags = human.update(&s.inner.player_game);
            for bit in [INPUT_LEFT, INPUT_RIGHT, INPUT_UP, INPUT_DOWN, INPUT_SWAP, INPUT_ADVANCE] {
                input.set_key(bit, flags & bit != 0);
            }
            s.inner.time_step(&input.snapshot());
            input.end_frame();
            if tick == 1000 {
                during = Some(s.snapshot().vs_display.unwrap().opponent_stats);
            }
            if matches!(s.inner.phase(), SessionPhase::Resolving | SessionPhase::ShowResult) {
                break;
            }
        }
        assert!(matches!(s.inner.phase(), SessionPhase::Resolving | SessionPhase::ShowResult), "the round did not end");
        assert!(matches!(during, Some(None)), "stats shown while the round is played");
        let shown = s.snapshot().vs_display.unwrap().opponent_stats.expect("no stats at the end of the round");
        assert!(shown.duration_ticks > 0 && shown.swaps_performed > 0);
        assert_eq!(shown.swaps_performed, s.inner.opponent.game().stats.swaps_performed);
    }

    #[test]
    fn contract_vs_ia_snapshot_reflects_real_session() {
        let s = vs_ia();
        let snap = s.snapshot();
        // The real session exposes the opponent, and round 1 starts in countdown.
        assert!(snap.vs_display.is_some());
        assert_eq!(snap.lifecycle, SessionLifecycle::Countdown);
    }

    #[test]
    fn contract_vs_ia_pause_resume_roundtrip() {
        let mut s = vs_ia();
        assert!(s.request_pause());
        assert!(matches!(
            s.snapshot().interruption,
            SessionInterruption::Paused { is_network: false, .. }
        ));
        assert!(s.request_resume());
        assert_eq!(s.snapshot().interruption, SessionInterruption::None);
    }

    #[test]
    fn contract_vs_ia_exposes_game_state() {
        let mut s = vs_ia();
        assert!(s.game_state_mut().is_some());
    }
}
