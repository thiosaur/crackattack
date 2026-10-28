//! Generic VS session — coordinates player GameState vs any Opponent.
//!
//! The session is generic over `O: Opponent`. The game loop, garbage
//! routing and the round lifecycle are identical regardless of whether
//! the opponent is an AI or a network player.
//!
//! A session is an OPEN series of rounds: round → celebration → consent
//! ("Play again?") → next round, indefinitely. The session ends by an
//! explicit player action (quit), never by the session itself.

use crate::game_state::{GameState, GameStatus};
use crate::frame_data::{FrameData, CountdownDigit, CountdownInfo};
use crate::input::InputSnapshot;
use crate::score::ScoringMode;
use super::board_view::BoardViewPublisher;
use super::opponent::Opponent;
use super::round::{RoundOutcome, SessionPhase, RESOLVING_DURATION_TICKS, CELEBRATING_ROUND_TICKS};
use super::game_session::{COUNTDOWN_TOTAL_TICKS, TICKS_PER_DIGIT};

/// Generic VS session: player GameState vs any Opponent.
pub struct VsSession<O: Opponent> {
    pub player_game: GameState,
    pub opponent: O,
    /// Current round number (1-based, unbounded — open series).
    pub round_number: i32,
    /// Session tally: rounds won during this connection.
    pub session_wins: i32,
    /// Session tally: rounds lost during this connection.
    pub session_losses: i32,
    pub phase: SessionPhase,
    pub round_outcome: Option<RoundOutcome>,
    seed_base: u32,
    /// Server wins ties (true for server, false for client/AI).
    win_ties: bool,
    /// When false, WaitingRemote does NOT auto-start the round.
    /// Used by VsNetworkGameSession to control the start via SyncControl.
    auto_start_round: bool,
    /// When false, rounds chain without consent (no ReadyPrompt).
    /// VsComputer/VsNetwork = true (post-round consent), tests = false.
    needs_ready_prompt: bool,
    /// Cached opponent grid level (updated each tick, readable from snapshot).
    pub cached_opponent_level: i32,
    /// Tick at which Resolving started (for phase transition timing).
    pub(crate) resolving_start_tick: Option<i32>,
    /// True when prepare_round was called during ShowResult
    /// (inter-round prep). signal_local_ready skips creating the round.
    round_prepared: bool,
    /// Lifecycle events emitted during this tick, drained by the wrapper session.
    pub pending_events: Vec<crate::frame_data::SessionEvent>,
    /// Session-managed countdown timer (200 → 0).
    countdown_remaining: i32,
    /// When true, the session does NOT determine round outcome on loss.
    /// Instead it enters AwaitingVerdict and the network wrapper handles it.
    pub defer_round_outcome: bool,
    /// Whether the local player's board is published to the opponent (network preview). Off by
    /// default: the computer opponent has no use for it.
    share_local_board: bool,
    /// Decides when a new picture of the local player's board goes out.
    local_board_publisher: BoardViewPublisher,
}

impl<O: Opponent> VsSession<O> {
    /// Create a new VS session with the given opponent.
    pub fn new(seed: u32, opponent: O) -> Self {
        // Placeholder game — immediately replaced by prepare_round() below.
        let game = GameState::with_scoring(seed, ScoringMode::Classic);

        let mut s = Self {
            player_game: game,
            opponent,
            round_number: 1,
            session_wins: 0,
            session_losses: 0,
            phase: SessionPhase::Countdown, // countdown at start; overridden by set_needs_ready_prompt
            round_outcome: None,
            seed_base: seed,
            win_ties: false,
            auto_start_round: true,
            needs_ready_prompt: false,
            cached_opponent_level: -1,
            resolving_start_tick: None,
            round_prepared: false,
            pending_events: Vec::new(),
            countdown_remaining: COUNTDOWN_TOTAL_TICKS,
            defer_round_outcome: false,
            share_local_board: false,
            local_board_publisher: BoardViewPublisher::new(),
        };
        // Prepare round 1 immediately so the grid visible during
        // ReadyPrompt is the real grid (same seed as will be played).
        s.prepare_round();
        s.round_prepared = true;
        // The displayed clock starts at GO (0:00 during the countdown). Matches the
        // original 1.1.14: time_step runs during the countdown (creep schedule,
        // timestamps), the clock does not. See `GameState::clock_base`.
        s.player_game.clock_base = s.player_game.time_step + COUNTDOWN_TOTAL_TICKS;
        s
    }

    /// Share the local player's board with the opponent (network preview). The choice is
    /// announced at once and again at each round, followed by the starting board when shared.
    pub fn set_share_local_board(&mut self, share: bool) {
        self.share_local_board = share;
        self.announce_local_board();
    }

    /// Suspends publishing the local board while the link is in trouble. Nothing is queued;
    /// the first played tick after resuming sends a fresh picture.
    pub fn set_local_board_suspended(&mut self, suspended: bool) {
        self.local_board_publisher.set_suspended(suspended);
    }

    /// Announces the sharing choice, then the starting board of the round when shared.
    fn announce_local_board(&mut self) {
        self.opponent.announce_board_sharing(self.share_local_board);
        if self.share_local_board {
            let view = self.local_board_publisher.start(&self.player_game);
            if !self.local_board_publisher.is_suspended() {
                self.opponent.publish_local_board(&view);
            }
        }
    }

    /// Disable auto-start in WaitingRemote (for network sync).
    pub fn set_auto_start_round(&mut self, enabled: bool) {
        self.auto_start_round = enabled;
    }

    /// Set whether this side wins ties (server = true, client = false).
    pub fn set_win_ties(&mut self, win: bool) {
        self.win_ties = win;
    }

    /// Whether this side wins ties (server = true, client = false).
    pub fn win_ties(&self) -> bool {
        self.win_ties
    }

    /// Enable the post-round consent WITHOUT interrupting the current round.
    /// VS Computer: round 1 starts directly (launching from the menu IS the initial
    /// consent), subsequent rounds go through the post-round screen ("Play again").
    pub(crate) fn set_ready_prompt_between_rounds(&mut self, needs: bool) {
        self.needs_ready_prompt = needs;
    }

    /// Enable consent between rounds AND the initial "Ready?" prompt.
    /// Network: both sides must explicitly sync before round 1 too.
    pub fn set_needs_ready_prompt(&mut self, needs: bool) {
        self.needs_ready_prompt = needs;
        if needs && matches!(self.phase, SessionPhase::Playing | SessionPhase::Countdown) {
            // Switch to WaitingBothReady (round hasn't started yet).
            self.phase = SessionPhase::WaitingBothReady;
            self.countdown_remaining = 0;
        }
    }

    /// Main tick: step player game, route garbage, step opponent, check outcomes.
    pub fn time_step(&mut self, input: &InputSnapshot) -> FrameData {
        // Cache opponent level for snapshot() (immutable reads).
        self.cached_opponent_level = self.opponent.grid_level();

        match self.phase {
            SessionPhase::Countdown => {
                self.opponent.countdown_step();

                // Emit event on first tick of each digit (including very first tick)
                let remaining_before = self.countdown_remaining;
                self.countdown_remaining -= 1;

                // Check if we just entered a digit boundary (200, 150, 100, 50)
                // or if this is the very first tick (remaining was at the boundary)
                let boundaries = [200, 150, 100, 50];
                if boundaries.contains(&remaining_before) {
                    if let Some(d) = self.countdown_digit() {
                        self.pending_events.push(crate::frame_data::SessionEvent::CountdownDigit { digit: d });
                    }
                }

                // Tick creep acceleration during countdown
                self.player_game.creep.tick_acceleration_only();
                self.player_game.time_step += 1;

                if self.countdown_remaining <= 0 {
                    self.phase = SessionPhase::Playing;
                }

                self.player_game.frame_data()
            }
            SessionPhase::Playing => {
                // 1. Step the player game
                let frame = self.player_game.step(input);

                // Picture of the player's board for the opponent's preview, when it is due.
                if self.share_local_board {
                    if let Some(view) = self.local_board_publisher.step(&self.player_game) {
                        self.opponent.publish_local_board(&view);
                    }
                }

                // 2. Route player's outgoing garbage to opponent
                for (h, w, f) in self.player_game.outgoing_garbage.drain(..) {
                    self.opponent.send_garbage(h, w, f);
                }

                // 3. Step the opponent
                self.opponent.play_step();

                // 4. Route opponent's garbage to player
                for (h, w, f) in self.opponent.recv_garbage() {
                    self.player_game.stats.record_garbage_received(h as u32);
                    // Initial drop delay of received garbage: average delay plus a random
                    // spread, drawn from the simulation RNG so it stays deterministic.
                    // Matches the original 1.1.14.
                    self.player_game.garbage_generator.add_to_queue(
                        h, w, f, self.player_game.time_step, Some(&mut self.player_game.rng),
                    );
                }

                // 5. Notify opponent if we lost (for timestamp-based tie-breaking)
                //    Only when defer_round_outcome is false (solo/AI).
                //    Network mode uses send_life_report() instead.
                let player_lost = self.player_game.status == GameStatus::Lost;
                if player_lost && !self.defer_round_outcome {
                    self.opponent.notify_local_loss(self.player_game.time_step);
                }

                // 6. Check win/loss conditions
                let opponent_lost = self.opponent.has_opponent_lost();

                if player_lost || opponent_lost {
                    if self.defer_round_outcome {
                        // Network mode: don't determine outcome — enter AwaitingVerdict.
                        // The network wrapper (VsNetworkGameSession) will handle arbitration.
                        self.round_outcome = None;
                        self.phase = SessionPhase::AwaitingVerdict;
                    } else {
                        // Solo/AI mode: determine outcome locally.
                        let player_ts = self.player_game.time_step;
                        let opponent_ts = self.opponent.opponent_loss_timestamp();

                        let outcome = if opponent_lost && !player_lost {
                            RoundOutcome::PlayerWon
                        } else if player_lost && !opponent_lost {
                            RoundOutcome::PlayerLost
                        } else {
                            // Both lost — compare timestamps (earlier loss = loser)
                            let opp_ts = opponent_ts.unwrap_or(player_ts);
                            if opp_ts < player_ts {
                                RoundOutcome::PlayerWon
                            } else if opp_ts > player_ts {
                                RoundOutcome::PlayerLost
                            } else {
                                // Exact tie — server wins
                                if self.win_ties { RoundOutcome::PlayerWon } else { RoundOutcome::PlayerLost }
                            }
                        };

                        if outcome == RoundOutcome::PlayerWon {
                            self.player_game.status = GameStatus::Won;
                            self.player_game.finished_tick = Some(self.player_game.time_step);
                        }
                        self.round_outcome = Some(outcome);
                        self.phase = SessionPhase::Resolving;
                        self.resolving_start_tick = Some(self.player_game.time_step);
                        self.pending_events.push(crate::frame_data::SessionEvent::RoundEnded {
                            won: outcome == RoundOutcome::PlayerWon,
                        });
                    }
                }

                frame
            }
            SessionPhase::Resolving => {
                // Game zone fading out — keep opponent alive, step for timer.
                self.opponent.time_step();
                let frame = self.player_game.step(input);

                let elapsed = self.player_game.time_step
                    - self.resolving_start_tick.unwrap_or(self.player_game.time_step);
                if elapsed >= RESOLVING_DURATION_TICKS {
                    self.phase = SessionPhase::ShowResult;
                }

                frame
            }
            SessionPhase::ShowResult => {
                // Keep opponent communication alive during result display
                self.opponent.time_step();
                let frame = self.player_game.step(input);

                if frame.ui.ticks_since_finished >= CELEBRATING_ROUND_TICKS + RESOLVING_DURATION_TICKS {
                    // Record the round in the session tally, then move to the
                    // next round of the open series.
                    match self.round_outcome {
                        Some(RoundOutcome::PlayerWon) => self.session_wins += 1,
                        Some(RoundOutcome::PlayerLost) => self.session_losses += 1,
                        None => {}
                    }
                    self.round_number += 1;

                    // Prepare the new round (grid ready behind curtain)
                    self.prepare_round();
                    self.round_prepared = true;

                    if self.needs_ready_prompt {
                        // Post-round consent: snapshot shows ReadyPrompt.
                        // The curtain opens as a visual consequence.
                        self.phase = SessionPhase::WaitingBothReady;
                    } else {
                        // No consent required: chain directly (countdown
                        // behind opening curtain).
                        self.start_round();
                        self.round_prepared = false;
                    }
                }

                frame
            }
            SessionPhase::Prepared => {
                // Round prepared, curtain opening — keep opponent alive.
                self.opponent.time_step();
                self.player_game.frame_data()
            }
            SessionPhase::WaitingBothReady => {
                // Gameplay suspended, network continues.
                // Transition happens when local consents (signal_local_ready).
                self.opponent.time_step();
                self.player_game.frame_data()
            }
            SessionPhase::AwaitingVerdict => {
                // Waiting for server verdict — keep opponent communication alive.
                self.opponent.time_step();
                self.player_game.frame_data()
            }
            SessionPhase::WaitingRemote => {
                // Local consented, waiting for opponent
                self.opponent.time_step();
                if self.auto_start_round {
                    let target = self.round_number as u32;
                    if self.opponent.opponent_round_ready() == target {
                        self.do_start_new_round();
                        self.phase = SessionPhase::Playing;
                    }
                }
                self.player_game.frame_data()
            }
        }
    }

    /// Signal that the local player consents to (re)play.
    /// Called when the player presses Ready/Play again during WaitingBothReady.
    pub fn signal_local_ready(&mut self) {
        if self.phase == SessionPhase::WaitingBothReady {
            let target = self.round_number as u32;
            self.opponent.set_local_round_ready(target);
            // Check if opponent is already ready
            if self.opponent.opponent_round_ready() == target {
                if self.round_prepared {
                    self.start_round();
                    self.round_prepared = false;
                } else {
                    self.do_start_new_round();
                }
            } else {
                self.phase = SessionPhase::WaitingRemote;
            }
        }
    }

    /// Prepare the next round: create fresh GameState, reset opponent.
    /// The grid exists but the countdown is frozen (not Playing yet).
    pub(crate) fn prepare_round(&mut self) {
        let new_seed = self.seed_base.wrapping_add(self.round_number as u32 * 1000);
        self.player_game = GameState::with_scoring(new_seed, ScoringMode::Classic);
        self.player_game.vs_mode = true;
        self.opponent.game_start();
        if self.share_local_board {
            self.announce_local_board();
        }
        self.round_outcome = None;
        self.resolving_start_tick = None;
        self.pending_events.push(crate::frame_data::SessionEvent::RoundPrepared {
            round_number: self.round_number,
        });
    }

    /// Start the round: phase = Countdown, emit RoundStarted.
    /// Call after prepare_round() or as part of do_start_new_round().
    pub fn start_round(&mut self) {
        self.phase = SessionPhase::Countdown;
        self.countdown_remaining = COUNTDOWN_TOTAL_TICKS;
        // The displayed clock shows 0:00 during the countdown and starts at GO (matches 1.1.14).
        self.player_game.clock_base = self.player_game.time_step + COUNTDOWN_TOTAL_TICKS;
        self.pending_events.push(crate::frame_data::SessionEvent::RoundStarted {
            round_number: self.round_number,
        });
        // Initial CountdownDigit::Three event is emitted on the first tick
        // of the Countdown phase (boundary 200).
    }

    /// Start the next round — called after both sides are ready.
    /// Public so VsNetworkGameSession can call it after sync convergence.
    /// Idempotent: if prepare_round() was already called, only starts.
    pub fn do_start_new_round(&mut self) {
        if !self.round_prepared {
            self.prepare_round();
        }
        self.start_round();
        self.round_prepared = false;
    }

    /// Get opponent's grid level for level lights display.
    pub fn opponent_grid_level(&mut self) -> i32 {
        self.opponent.grid_level()
    }

    /// Apply a round verdict from the network arbitration system.
    /// Called by VsNetworkGameSession when the server verdict is received or computed.
    pub fn apply_round_verdict(&mut self, won: bool) {
        let outcome = if won { RoundOutcome::PlayerWon } else { RoundOutcome::PlayerLost };
        self.round_outcome = Some(outcome);
        if won {
            self.player_game.status = GameStatus::Won;
            self.player_game.finished_tick = Some(self.player_game.time_step);
        }
        self.phase = SessionPhase::Resolving;
        self.resolving_start_tick = Some(self.player_game.time_step);
        self.pending_events.push(crate::frame_data::SessionEvent::RoundEnded { won });
    }

    pub fn opponent_name(&self) -> &str {
        self.opponent.opponent_name()
    }

    pub fn phase(&self) -> SessionPhase {
        self.phase
    }

    fn countdown_digit(&self) -> Option<CountdownDigit> {
        let r = self.countdown_remaining;
        if r > 150 { Some(CountdownDigit::Three) }
        else if r > 100 { Some(CountdownDigit::Two) }
        else if r > 50 { Some(CountdownDigit::One) }
        else if r > 0 { Some(CountdownDigit::Go) }
        else { None }
    }

    pub fn countdown_info(&self) -> Option<CountdownInfo> {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai_difficulty::AIDifficulty;
    use crate::input::Input;

    /// An opponent that records what the session publishes of the player's board.
    struct RecordingOpponent {
        inner: crate::vs::grid_opponent::GridOpponent,
        /// `(session tick, view)` for each published picture.
        views: Vec<(i32, super::super::board_view::OpponentBoardView)>,
        announcements: Vec<bool>,
        tick: i32,
    }

    impl RecordingOpponent {
        fn new() -> Self {
            Self { inner: crate::vs::grid_opponent::GridOpponent::new(AIDifficulty::Easy, 42), views: Vec::new(), announcements: Vec::new(), tick: 0 }
        }
    }

    impl Opponent for RecordingOpponent {
        fn game_start(&mut self) { self.inner.game_start() }
        fn time_step(&mut self) { self.tick += 1; self.inner.time_step() }
        fn send_garbage(&mut self, h: i32, w: i32, f: i32) { self.inner.send_garbage(h, w, f) }
        // No garbage back: the player's board only changes by its own play.
        fn recv_garbage(&mut self) -> Vec<(i32, i32, i32)> { Vec::new() }
        fn grid_level(&mut self) -> i32 { 0 }
        fn has_opponent_lost(&self) -> bool { false }
        fn opponent_name(&self) -> &str { "Recorder" }
        fn publish_local_board(&mut self, view: &super::super::board_view::OpponentBoardView) {
            self.views.push((self.tick, view.clone()));
        }
        fn announce_board_sharing(&mut self, shares: bool) { self.announcements.push(shares) }
    }

    /// Input that raises the stack on the first ticks of every 100, so the board keeps changing.
    fn raising_input(input: &mut Input, tick: i32) -> crate::input::InputSnapshot {
        input.set_key(crate::input::INPUT_ADVANCE, tick % 100 < 20);
        let snap = input.snapshot();
        input.end_frame();
        snap
    }

    #[test]
    fn local_board_is_not_published_unless_shared() {
        let mut session = VsSession::new(42, RecordingOpponent::new());
        let snap = Input::new().snapshot();
        for _ in 0..800 {
            session.time_step(&snap);
        }
        assert!(session.opponent.views.is_empty());
        assert!(session.opponent.announcements.is_empty());
    }

    #[test]
    fn shared_local_board_follows_the_publishing_rule() {
        use super::super::board_view::{BOARD_VIEW_MIN_GAP, BOARD_VIEW_PERIOD, OpponentBoardView};
        let mut session = VsSession::new(42, RecordingOpponent::new());
        session.set_share_local_board(true);
        // Announced at once, followed by the starting board.
        assert_eq!(session.opponent.announcements, vec![true]);
        assert_eq!(session.opponent.views.len(), 1);
        assert_eq!(session.opponent.views[0].1, OpponentBoardView::from_game(&session.player_game));

        // The countdown publishes nothing: the board does not move.
        let snap = Input::new().snapshot();
        for _ in 0..200 {
            session.time_step(&snap);
        }
        assert_eq!(session.phase(), SessionPhase::Playing);
        assert_eq!(session.opponent.views.len(), 1);

        // Playing: never more than one view per gap, never more than a period without one while
        // the board changes (the creep raises it), each view the board of its tick.
        let start = session.opponent.tick;
        let mut input = Input::new();
        for tick in 0..600 {
            session.time_step(&raising_input(&mut input, tick));
            if let Some((t, view)) = session.opponent.views.last() {
                if *t == session.opponent.tick - 1 {
                    assert_eq!(view, &OpponentBoardView::from_game(&session.player_game));
                }
            }
        }
        let ticks: Vec<i32> = session.opponent.views[1..].iter().map(|(t, _)| *t).collect();
        assert!(!ticks.is_empty(), "nothing published while playing");
        assert!(ticks.windows(2).all(|w| w[1] - w[0] >= BOARD_VIEW_MIN_GAP));
        assert!(ticks[0] - start <= BOARD_VIEW_PERIOD);
    }

    #[test]
    fn suspended_local_board_resumes_with_a_fresh_view() {
        let mut session = VsSession::new(42, RecordingOpponent::new());
        session.set_share_local_board(true);
        let snap = Input::new().snapshot();
        for _ in 0..200 {
            session.time_step(&snap);
        }
        session.set_local_board_suspended(true);
        let before = session.opponent.views.len();
        let mut input = Input::new();
        for tick in 0..200 {
            session.time_step(&raising_input(&mut input, tick));
        }
        assert_eq!(session.opponent.views.len(), before, "published while suspended");
        // The board changed meanwhile: the first tick after resuming sends it.
        session.set_local_board_suspended(false);
        session.time_step(&snap);
        assert_eq!(session.opponent.views.len(), before + 1, "no fresh view after resuming");
    }

    #[test]
    fn session_creation_no_ready_prompt() {
        // Default: needs_ready_prompt = false → starts in Countdown.
        let session = VsSession::new(42, crate::vs::grid_opponent::GridOpponent::new(AIDifficulty::Medium, 42));
        assert!(session.player_game.vs_mode);
        assert_eq!(session.opponent_name(), "Sigma");
        assert_eq!(session.phase(), SessionPhase::Countdown);
        assert_eq!(session.round_number, 1);
        assert_eq!((session.session_wins, session.session_losses), (0, 0));
    }

    #[test]
    fn session_creation_with_ready_prompt() {
        // Network-style: needs_ready_prompt = true → starts in WaitingBothReady.
        let mut session = VsSession::new(42, crate::vs::grid_opponent::GridOpponent::new(AIDifficulty::Medium, 42));
        session.set_needs_ready_prompt(true);
        assert_eq!(session.phase(), SessionPhase::WaitingBothReady);
    }

    #[test]
    fn ready_prompt_between_rounds_does_not_interrupt_round_one() {
        // `set_ready_prompt_between_rounds` alone enables the between-round prompt without
        // changing the phase, so round 1 starts directly (Countdown). Production VS Computer
        // also calls `set_needs_ready_prompt(true)`, which adds a prompt before round 1.
        let mut session = VsSession::new(42, crate::vs::grid_opponent::GridOpponent::new(AIDifficulty::Medium, 42));
        session.set_ready_prompt_between_rounds(true);
        assert_eq!(session.phase(), SessionPhase::Countdown);
    }

    #[test]
    fn session_step_runs() {
        let mut session = VsSession::new(42, crate::vs::grid_opponent::GridOpponent::new(AIDifficulty::Easy, 42));
        let input = Input::new();
        let snap = input.snapshot();

        // Starts in Countdown (no ready prompt needed).
        assert_eq!(session.phase(), SessionPhase::Countdown);

        // Run through countdown (200 ticks)
        for _ in 0..200 {
            let _frame = session.time_step(&snap);
        }
        // After countdown, should be in Playing
        assert_eq!(session.phase(), SessionPhase::Playing);

        for _ in 0..100 {
            let _frame = session.time_step(&snap);
        }
        assert_eq!(session.phase(), SessionPhase::Playing);
    }

    #[test]
    fn round_transition_with_signal_local_ready() {
        // With needs_ready_prompt, test the WaitingBothReady → Countdown transition.
        let mut session = VsSession::new(42, crate::vs::grid_opponent::GridOpponent::new(AIDifficulty::Easy, 42));
        session.set_needs_ready_prompt(true);

        assert_eq!(session.phase(), SessionPhase::WaitingBothReady);

        // Signal local ready — AI is always ready, so should go to Countdown
        session.signal_local_ready();
        assert_eq!(session.phase(), SessionPhase::Countdown);

        let input = Input::new();
        let snap = input.snapshot();
        // Run through countdown (200 ticks)
        for _ in 0..200 {
            let _frame = session.time_step(&snap);
        }
        assert_eq!(session.phase(), SessionPhase::Playing);
    }

    #[test]
    fn snapshot_playing_after_countdown() {
        use crate::vs::computer_session::VsComputerGameSession;
        use crate::vs::game_session::GameSession;
        use crate::frame_data::*;

        let mut session = VsComputerGameSession::new(42, crate::vs::grid_opponent::GridOpponent::new(AIDifficulty::Easy, 42));
        // Run through countdown (200 ticks)
        let input = Input::new();
        let snap_input = input.snapshot();
        for _ in 0..200 {
            session.tick(&snap_input);
        }

        let snap = session.snapshot();
        // After countdown: Playing, no interruption
        assert_eq!(snap.lifecycle, SessionLifecycle::Playing);
        assert_eq!(snap.interruption, SessionInterruption::None);
        assert_eq!(snap.result, SessionResult::None);
    }

    #[test]
    fn snapshot_paused() {
        use crate::vs::computer_session::VsComputerGameSession;
        use crate::vs::game_session::GameSession;
        use crate::frame_data::*;

        let mut session = VsComputerGameSession::new(42, crate::vs::grid_opponent::GridOpponent::new(AIDifficulty::Easy, 42));
        // No signal_local_ready needed — VsComputer starts in Playing.
        session.request_pause();

        let snap = session.snapshot();
        assert_eq!(snap.interruption, SessionInterruption::Paused { local_ready: false, remote_ready: false, is_network: false });
        assert!(snap.interruption != SessionInterruption::None);
    }

    #[test]
    fn snapshot_round_started_event() {
        // VsComputer sessions start round 1 in Countdown directly. RoundStarted
        // is emitted by VsSession when signal_local_ready fires from a prompt.
        use crate::frame_data::*;

        let mut session = VsSession::new(42, crate::vs::grid_opponent::GridOpponent::new(AIDifficulty::Easy, 42));
        session.set_needs_ready_prompt(true);
        session.signal_local_ready();

        assert!(session.pending_events.iter().any(|e| matches!(e, SessionEvent::RoundStarted { round_number: 1 })));
    }

    /// Helper: force a VsComputer session into ShowResult by setting status to Lost.
    fn run_until_show_result(session: &mut crate::vs::computer_session::VsComputerGameSession) {
        use crate::vs::game_session::GameSession;
        let input = Input::new();
        let snap = input.snapshot();
        // Run through countdown (200 ticks)
        for _ in 0..200 {
            session.tick(&snap);
        }
        session.drain_events(); // clear any accumulated events

        // Force player loss
        session.inner.player_game.status = crate::game_state::GameStatus::Lost;
        // Tick to trigger the Playing→Resolving transition
        session.tick(&snap);
    }

    #[test]
    fn event_round_ended_on_show_result() {
        use crate::vs::computer_session::VsComputerGameSession;
        use crate::vs::game_session::GameSession;
        use crate::frame_data::*;

        let mut session = VsComputerGameSession::new(42, crate::vs::grid_opponent::GridOpponent::new(AIDifficulty::Easy, 42));
        run_until_show_result(&mut session);

        let events = session.drain_events();
        assert!(events.iter().any(|e| matches!(e, SessionEvent::RoundEnded { won: false })),
            "Expected RoundEnded {{ won: false }}, got {:?}", events);
    }

    #[test]
    fn open_cycle_tallies_and_prepares_next_round() {
        // The heart of the open series: after ShowResult completes, the
        // session records the tally, increments round_number, prepares the
        // next round and waits for consent — it never "finishes".
        let mut vs = VsSession::new(42, crate::vs::grid_opponent::GridOpponent::new(AIDifficulty::Easy, 42));
        vs.set_ready_prompt_between_rounds(true);
        let input = Input::new();
        let snap = input.snapshot();

        vs.pending_events.clear();

        // Force into ShowResult with a round loss outcome.
        vs.phase = SessionPhase::ShowResult;
        vs.round_outcome = Some(RoundOutcome::PlayerLost);
        vs.player_game.status = crate::game_state::GameStatus::Lost;
        vs.player_game.finished_tick = Some(0);
        // Advance past resolving + celebration from finished_tick.
        for _ in 0..400 {
            vs.time_step(&snap);
        }

        assert_eq!(vs.phase, SessionPhase::WaitingBothReady,
            "Expected WaitingBothReady (consent for next round), phase is {:?}", vs.phase);
        assert_eq!((vs.session_wins, vs.session_losses), (0, 1));
        assert_eq!(vs.round_number, 2);
        assert!(vs.pending_events.iter().any(|e| matches!(e,
            crate::frame_data::SessionEvent::RoundPrepared { round_number: 2 })),
            "Expected RoundPrepared(2), got {:?}", vs.pending_events);
    }

    #[test]
    fn open_cycle_chains_many_rounds() {
        // No upper bound: the tally accumulates and round_number keeps growing.
        let mut vs = VsSession::new(42, crate::vs::grid_opponent::GridOpponent::new(AIDifficulty::Easy, 42));
        vs.set_ready_prompt_between_rounds(true);
        let input = Input::new();
        let snap = input.snapshot();

        for expected_round in 2..=5 {
            vs.phase = SessionPhase::ShowResult;
            vs.round_outcome = Some(RoundOutcome::PlayerWon);
            vs.player_game.status = crate::game_state::GameStatus::Won;
            vs.player_game.finished_tick = Some(vs.player_game.time_step);
            for _ in 0..400 {
                vs.time_step(&snap);
            }
            assert_eq!(vs.round_number, expected_round);
            assert_eq!(vs.phase, SessionPhase::WaitingBothReady);
            // Consent → next round starts (AI is always ready).
            vs.signal_local_ready();
            assert_eq!(vs.phase, SessionPhase::Countdown);
        }
        assert_eq!(vs.session_wins, 4);
    }

    /// Matches the original 1.1.14: the displayed clock stays at 0:00 during the
    /// whole 3-2-1-GO. Internally time_step runs (creep schedule, timestamps),
    /// but the clock is decoupled and only starts at GO.
    #[test]
    fn clock_starts_at_go_not_during_countdown() {
        let mut session = VsSession::new(42, crate::vs::grid_opponent::GridOpponent::new(AIDifficulty::Easy, 42));
        let input = Input::new();
        let snap = input.snapshot();

        for _ in 0..200 {
            let f = session.time_step(&snap);
            assert_eq!(f.ui.clock_time, 0, "clock != 0 during the countdown");
        }
        assert_eq!(session.phase(), SessionPhase::Playing);
        // 50 game ticks = 1 s on the clock.
        let mut last = 0;
        for _ in 0..50 {
            last = session.time_step(&snap).ui.clock_time;
        }
        assert_eq!(last, 50, "the clock must start at 0 on GO");
        // Internal time_step: 1 (start, as in the original) + 200 (countdown) + 50 (play).
        assert_eq!(session.player_game.time_step, 251);
    }

    #[test]
    fn event_round_started_emitted_once() {
        use crate::frame_data::*;

        let mut session = VsSession::new(42, crate::vs::grid_opponent::GridOpponent::new(AIDifficulty::Easy, 42));
        session.set_needs_ready_prompt(true);

        // signal_local_ready → RoundStarted
        session.signal_local_ready();
        let round_started_count = session.pending_events.iter()
            .filter(|e| matches!(e, SessionEvent::RoundStarted { .. }))
            .count();
        assert_eq!(round_started_count, 1, "Expected exactly 1 RoundStarted");
        session.pending_events.clear();

        // Subsequent ticks should not re-emit RoundStarted
        let input = Input::new();
        let snap = input.snapshot();
        for _ in 0..10 {
            session.time_step(&snap);
        }
        let events = &session.pending_events;
        let round_started_count = events.iter()
            .filter(|e| matches!(e, SessionEvent::RoundStarted { .. }))
            .count();
        assert_eq!(round_started_count, 0, "RoundStarted should not repeat");
    }
}
