//! GameSession trait — unified session interface for all game modes.
//!
//! The runtime manipulates `dyn GameSession` without knowing whether
//! it's Solo, VS Computer, or VS Network. Mode-specific behavior
//! (lag pause, disconnect, pause sync) lives in the implementations,
//! not in the runtime.

use crate::frame_data::FrameData;
use crate::input::InputSnapshot;

/// Unified game session interface.
///
/// The runtime calls these methods without knowing the concrete
/// session type. All mode-specific logic (network disconnect,
/// lag pause, sync compensation) is encapsulated in the impls.
///
/// There are no default methods: every method is required. A default could
/// hide a mode that forgot to implement it (for example a fake `snapshot()` or a
/// `drain_events()` that drops lifecycle events). A mode to which a method does
/// not apply implements it explicitly and says why in a comment.
pub trait GameSession: Send {
    /// Advance one tick. Returns frame data for rendering.
    fn tick(&mut self, input: &InputSnapshot) -> FrameData;

    /// Current frame data (without stepping).
    fn frame_data(&self) -> FrameData;

    /// Signal that the local player is ready for the next round.
    fn signal_local_ready(&mut self);

    /// Structured snapshot of the full session state and must reflect the real state (see the `contract_*` tests).
    fn snapshot(&self) -> crate::frame_data::SessionSnapshot;

    /// Drain one-shot lifecycle events emitted since the last call.
    fn drain_events(&mut self) -> Vec<crate::frame_data::SessionEvent>;

    /// Request a local pause. Returns true if accepted.
    fn request_pause(&mut self) -> bool;

    /// Request a local resume. Returns true if accepted.
    fn request_resume(&mut self) -> bool;

    /// Abandon the game (forfeit in network).
    fn abandon(&mut self);

    /// Access the underlying GameState for autoplay/dev tools.
    /// `None` is valid for a mode without a local board, but the impl must return it explicitly.
    fn game_state_mut(&mut self) -> Option<&mut crate::game_state::GameState>;
}

use crate::game_state::{GameState, GameStatus};
use crate::frame_data::{CountdownDigit, CountdownInfo};

/// Countdown constants for session-managed countdowns (3-2-1-GO).
pub(crate) const COUNTDOWN_TOTAL_TICKS: i32 = 200;
pub(crate) const TICKS_PER_DIGIT: i32 = 50;

/// Solo game session — no opponent.
pub struct SoloSession {
    pub game: GameState,
    /// Local pause state (the only cause of freeze in solo).
    frozen: bool,
    /// Session-managed countdown timer (200 → 0).
    countdown_remaining: i32,
    /// Lifecycle events emitted during this tick, drained by the runtime.
    pending_events: Vec<crate::frame_data::SessionEvent>,
}

impl SoloSession {
    pub fn new(mut game: GameState) -> Self {
        // The displayed clock starts at GO and shows 0:00 during the countdown.
        // Matches the original 1.1.14: time_step runs during the countdown, the clock does not.
        game.clock_base = game.time_step + COUNTDOWN_TOTAL_TICKS;
        Self { game, frozen: false, countdown_remaining: COUNTDOWN_TOTAL_TICKS, pending_events: Vec::new() }
    }

    fn countdown_digit(&self) -> Option<CountdownDigit> {
        let r = self.countdown_remaining;
        if r > 150 { Some(CountdownDigit::Three) }
        else if r > 100 { Some(CountdownDigit::Two) }
        else if r > 50 { Some(CountdownDigit::One) }
        else if r > 0 { Some(CountdownDigit::Go) }
        else { None }
    }

    fn countdown_info(&self) -> Option<CountdownInfo> {
        let digit = self.countdown_digit()?;
        let r = self.countdown_remaining;
        let segment_start = match digit {
            CountdownDigit::Three => 200,
            CountdownDigit::Two => 150,
            CountdownDigit::One => 100,
            CountdownDigit::Go => 50,
        };
        Some(CountdownInfo {
            digit,
            ticks_into_digit: segment_start - r,
            ticks_per_digit: TICKS_PER_DIGIT,
        })
    }
}

impl GameSession for SoloSession {
    fn tick(&mut self, input: &InputSnapshot) -> FrameData {
        if self.frozen {
            return self.game.frame_data();
        }
        if self.countdown_remaining > 0 {
            // Emit event on first tick of each digit (including very first tick)
            let remaining_before = self.countdown_remaining;
            self.countdown_remaining -= 1;

            let boundaries = [200, 150, 100, 50];
            if boundaries.contains(&remaining_before) {
                if let Some(d) = self.countdown_digit() {
                    self.pending_events.push(crate::frame_data::SessionEvent::CountdownDigit { digit: d });
                }
            }

            // Tick creep acceleration during countdown
            self.game.creep.tick_acceleration_only();
            self.game.time_step += 1;
            return self.game.frame_data();
        }
        self.game.step(input)
    }

    fn frame_data(&self) -> FrameData {
        self.game.frame_data()
    }

    fn signal_local_ready(&mut self) {
        // Solo has no rounds and no opponent: intentionally a no-op.
    }

    fn abandon(&mut self) {
        // Leaving is enough, nobody to notify: intentionally a no-op.
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
        use crate::vs::round::RESOLVING_DURATION_TICKS;

        let ticks_finished = self.game.finished_tick
            .map(|ft| self.game.time_step - ft)
            .unwrap_or(0);

        let lifecycle = match self.game.status {
            GameStatus::Playing => {
                if self.countdown_remaining > 0 {
                    SessionLifecycle::Countdown
                } else {
                    SessionLifecycle::Playing
                }
            }
            GameStatus::Won | GameStatus::Lost => {
                if ticks_finished < RESOLVING_DURATION_TICKS {
                    SessionLifecycle::Resolving
                } else {
                    SessionLifecycle::Ended
                }
            }
        };
        let interruption = if self.frozen {
            SessionInterruption::Paused { local_ready: false, remote_ready: false, is_network: false }
        } else {
            SessionInterruption::None
        };
        let result = match self.game.status {
            GameStatus::Won => SessionResult::SessionWin,
            GameStatus::Lost => SessionResult::SessionLose,
            GameStatus::Playing => SessionResult::None,
        };
        SessionSnapshot {
            lifecycle, interruption, result, vs_display: None,
            // Solo: the game is "engaged" while it runs (leaving = the score
            // dies) — the pause quit keeps its confirmation.
            round_engaged: self.game.status == GameStatus::Playing,
            countdown: self.countdown_info(),
        }
    }

    fn drain_events(&mut self) -> Vec<crate::frame_data::SessionEvent> {
        std::mem::take(&mut self.pending_events)
    }

    fn game_state_mut(&mut self) -> Option<&mut GameState> {
        Some(&mut self.game)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frame_data::{
        CountdownDigit, SessionEvent, SessionInterruption, SessionLifecycle, SessionResult,
    };
    use crate::input::InputSnapshot;

    // Contract tests: the snapshot and drained events reflect the real session
    // state, never a fabricated one.

    fn solo() -> SoloSession {
        SoloSession::new(GameState::new(42))
    }

    #[test]
    fn contract_solo_snapshot_starts_in_countdown() {
        let s = solo();
        let snap = s.snapshot();
        // A fresh solo session starts in countdown with no interruption.
        assert_eq!(snap.lifecycle, SessionLifecycle::Countdown);
        assert!(snap.countdown.is_some());
        assert_eq!(snap.interruption, SessionInterruption::None);
    }

    #[test]
    fn contract_solo_pause_resume_roundtrip() {
        let mut s = solo();
        assert!(s.request_pause());
        assert!(matches!(
            s.snapshot().interruption,
            SessionInterruption::Paused { is_network: false, .. }
        ));
        assert!(s.request_resume());
        assert_eq!(s.snapshot().interruption, SessionInterruption::None);
    }

    #[test]
    fn contract_solo_result_follows_game_status() {
        let mut s = solo();
        assert_eq!(s.snapshot().result, SessionResult::None);
        s.game.status = GameStatus::Lost;
        assert_eq!(s.snapshot().result, SessionResult::SessionLose);
    }

    #[test]
    fn contract_solo_drain_events_delivers_then_empties() {
        let mut s = solo();
        let _ = s.tick(&InputSnapshot::default());
        let evs = s.drain_events();
        assert!(evs.iter().any(|e| matches!(
            e,
            SessionEvent::CountdownDigit { digit: CountdownDigit::Three }
        )));
        assert!(s.drain_events().is_empty());
    }

    #[test]
    fn contract_solo_exposes_game_state() {
        let mut s = solo();
        assert!(s.game_state_mut().is_some());
    }
}
