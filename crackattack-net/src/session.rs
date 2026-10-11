//! VsNetworkGameSession — wraps VsSession<NetworkOpponent>.
//!
//! All network-specific behavior lives here:
//! - Grid level propagation
//! - Disconnect mid-round: forfeit. Disconnect post-round: the "play again"
//!   offer silently shrinks (no departure message is sent).
//! - Pause state machine (local, remote, lag, sync wait)
//!
//! The pause state machine is the single source of truth for whether
//! the game is frozen. The session's `tick()` calls `step()` or `frame_data()`
//! based on `is_frozen()`. Neither `main.rs` nor `NetworkOpponent`
//! decides the pause state; they only provide inputs.

use crackattack_core::frame_data::FrameData;
use crackattack_core::game_state::GameState;
use crackattack_core::input::InputSnapshot;
use crackattack_core::vs::{
    GameSession, VsSession, Opponent,
};

use crate::opponent::NetworkOpponent;
use crate::protocol::{ControlMessage, SyncContext, RoundLifeStatus, PeerRole, VerdictReason};

/// Safety margin added to the measured RTT when computing `target_start_tick`.
const SYNC_SAFETY_TICKS: i32 = 2;

/// Centralized pause state for network VS mode (symmetric model).
///
/// Three independent causes of game freeze:
/// - `paused`: either player pressed P (either side can trigger it)
/// - `lag_paused`: no data received for 2+ communication cycles
/// - `target_start_tick > 0`: SyncControl convergence in progress
///
/// Resume is consensual: both players must signal "ready to resume"
/// before the SyncControl handshake begins.
///
/// The game is frozen when ANY of these is active.
#[derive(Debug, Clone)]
struct NetworkPauseState {
    /// Game is paused (triggered by either side).
    paused: bool,
    /// Local player pressed "Ready to resume".
    local_resume_ready: bool,
    /// Remote player pressed "Ready to resume".
    remote_resume_ready: bool,
    /// Lag pause: no data received for 2+ communication cycles.
    lag_paused: bool,
    /// Remote pause received during a lag pause. Replayed when the lag ends.
    queued_remote_pause: bool,
    /// Remote resume-ready received during a lag pause. Replayed when the lag ends.
    queued_remote_resume_ready: bool,
    /// SyncControl: target tick for synchronized resume. 0 = inactive.
    target_start_tick: i32,
    /// SyncControl: pause epoch counter. Incremented on each resume proposal.
    pause_seq: u16,
}

impl NetworkPauseState {
    fn new() -> Self {
        Self {
            paused: false,
            local_resume_ready: false,
            remote_resume_ready: false,
            lag_paused: false,
            queued_remote_pause: false,
            queued_remote_resume_ready: false,
            target_start_tick: 0,
            pause_seq: 0,
        }
    }

    /// Is the game effectively frozen?
    fn is_frozen(&self) -> bool {
        self.paused
            || self.lag_paused
            || self.target_start_tick > 0
    }

    /// Pauses the game (either side can trigger it).
    /// Returns true if the pause was accepted (the game was not already frozen).
    fn try_pause(&mut self) -> bool {
        if self.is_frozen() { return false; }
        self.paused = true;
        self.local_resume_ready = false;
        self.remote_resume_ready = false;
        true
    }

    /// Signals that the local player is ready to resume.
    /// Returns true if the state changed. When both are ready,
    /// the caller starts SyncControl (`propose_sync`).
    fn signal_resume_ready(&mut self) -> bool {
        if !self.paused { return false; }
        if self.local_resume_ready { return false; }
        if self.target_start_tick > 0 { return false; } // Sync already in progress.
        self.local_resume_ready = true;
        true
    }

    /// Are both players ready to resume?
    fn both_ready(&self) -> bool {
        self.paused && self.local_resume_ready && self.remote_resume_ready
    }

    /// Proposes a `target_start_tick` for a synchronized resume.
    fn propose_sync(&mut self, local_tick: i32, remote_tick: i32, margin: i32) {
        self.pause_seq = self.pause_seq.wrapping_add(1);
        self.target_start_tick = local_tick.max(remote_tick) + margin;
    }

    /// Adopts or converges with the remote's `target_start_tick`.
    /// Only applies within the same `pause_seq` epoch.
    fn adopt_remote_target(&mut self, remote_target: i32, remote_seq: u16) {
        if remote_target <= 0 { return; }
        if self.target_start_tick <= 0 {
            // No local proposal: adopt the remote's target and epoch.
            self.target_start_tick = remote_target;
            self.pause_seq = remote_seq;
        } else if remote_seq == self.pause_seq {
            // Same epoch: converge on the max (the target never decreases).
            self.target_start_tick = self.target_start_tick.max(remote_target);
        }
        // Different epoch and we have our own proposal: ignore the stale remote.
    }

    /// Checks whether sync has converged (both sides agree on the same target).
    fn is_sync_converged(&self, remote_target: i32) -> bool {
        self.target_start_tick > 0 && self.target_start_tick == remote_target
    }

    /// Clears the sync state once the resume is complete.
    fn clear_sync(&mut self) {
        self.target_start_tick = 0;
        self.paused = false;
        self.local_resume_ready = false;
        self.remote_resume_ready = false;
    }

    fn apply_remote_pause(&mut self) {
        if self.lag_paused {
            self.queued_remote_pause = true;
            self.queued_remote_resume_ready = false;
            return;
        }
        self.paused = true;
        self.local_resume_ready = false;
        self.remote_resume_ready = false;
    }

    fn apply_remote_resume_ready(&mut self) {
        if self.lag_paused {
            self.queued_remote_resume_ready = true;
            self.queued_remote_pause = false;
            return;
        }
        self.remote_resume_ready = true;
    }

    fn apply_lag_pause(&mut self) {
        self.lag_paused = true;
    }

    fn apply_lag_resume(&mut self) {
        self.lag_paused = false;
        if self.queued_remote_pause {
            self.queued_remote_pause = false;
            self.apply_remote_pause();
        } else if self.queued_remote_resume_ready {
            self.queued_remote_resume_ready = false;
            self.apply_remote_resume_ready();
        }
    }
}

/// A disconnect during an ENGAGED round is a forfeit (the opponent
/// walked out on a live round). In the post-round phases nothing is
/// engaged: a departure is a **silent shrink of the post-round offer**.
/// No message is sent; the screen simply stops offering "play again".
/// Single source of truth: `core::vs::round`.
use crackattack_core::vs::round::is_round_engaged as round_engaged;

/// State for an in-progress round arbitration.
struct RoundArbitration {
    round_id: u16,
    local_report: Option<(RoundLifeStatus, u32)>,
    remote_report: Option<(RoundLifeStatus, u32)>,
}

/// Determines the winner and the reason from both players' reports.
/// `srv_*` is the server's data and `cli_*` is the client's.
fn arbitrate(
    srv_status: RoundLifeStatus, srv_swaps: u32,
    cli_status: RoundLifeStatus, cli_swaps: u32,
) -> (PeerRole, VerdictReason) {
    let srv_lost = matches!(srv_status, RoundLifeStatus::Lost { .. });
    let cli_lost = matches!(cli_status, RoundLifeStatus::Lost { .. });

    if srv_lost && !cli_lost {
        return (PeerRole::Client, VerdictReason::OnlyOneLost);
    }
    if cli_lost && !srv_lost {
        return (PeerRole::Server, VerdictReason::OnlyOneLost);
    }

    // Both lost: compare `loss_tick` (higher = survived longer = winner).
    let srv_tick = match srv_status { RoundLifeStatus::Lost { loss_tick } => loss_tick, _ => 0 };
    let cli_tick = match cli_status { RoundLifeStatus::Lost { loss_tick } => loss_tick, _ => 0 };

    if srv_tick > cli_tick {
        return (PeerRole::Server, VerdictReason::HigherLossTick);
    }
    if cli_tick > srv_tick {
        return (PeerRole::Client, VerdictReason::HigherLossTick);
    }

    // Equal `loss_tick`: fewer swaps wins.
    if srv_swaps < cli_swaps {
        return (PeerRole::Server, VerdictReason::FewerSwaps);
    }
    if cli_swaps < srv_swaps {
        return (PeerRole::Client, VerdictReason::FewerSwaps);
    }

    // Total tie: the server wins.
    (PeerRole::Server, VerdictReason::ServerTieBreak)
}

/// Maximum time in `AwaitingVerdict` before escalating to a disconnect (~10 s at 50 Hz).
/// Covers an opponent that keeps sending traffic but never sends its `RoundLifeReport`.
/// Lag detection does not fire in that case, so the session would block forever.
const VERDICT_TIMEOUT_TICKS: i32 = 500;

/// VS Network game session.
pub struct VsNetworkGameSession {
    pub(crate) inner: VsSession<NetworkOpponent>,
    /// Set to true on disconnect forfeit to prevent repeated transitions.
    forfeit_signaled: bool,
    /// Opponent left during the post-round (a clean quit or a crash; the two
    /// cannot be told apart). NOT an error: the offer shrinks silently and the
    /// session keeps running its post-round.
    opponent_departed: bool,
    /// Centralized pause state machine.
    pause: NetworkPauseState,
    /// SyncControl: target tick for round start sync. 0 = no sync active.
    round_sync_target: i32,
    /// Tick when lag pause started (for SuspensionState::ConnectionLost).
    lag_started_tick: i32,
    /// Tick when WaitingRemote started (for SuspensionState::WaitingRemoteReady).
    waiting_started_tick: i32,
    /// Whether the OpponentDisconnected event has been sent (one-shot guard).
    disconnect_event_sent: bool,
    /// Whether local stats have been sent to the opponent this round.
    stats_sent: bool,
    /// In-progress round arbitration (network-only).
    arbitration: Option<RoundArbitration>,
    /// Tick at which `AwaitingVerdict` was entered (0 = not waiting). Used for the arbitration timeout.
    awaiting_verdict_since: i32,
}

impl VsNetworkGameSession {
    /// Forces the "ready?" prompt to show on resume (reconnection). This is the
    /// only external access to the inner session.
    pub fn set_needs_ready_prompt(&mut self, v: bool) {
        self.inner.set_needs_ready_prompt(v);
    }

    pub fn new(mut session: VsSession<NetworkOpponent>) -> Self {
        // Network mode: ReadyPrompt between rounds, SyncControl for round start.
        session.set_auto_start_round(false);
        session.set_needs_ready_prompt(true);
        // Network mode: defer round outcome to the arbitration system.
        session.defer_round_outcome = true;
        Self {
            inner: session,
            forfeit_signaled: false,
            opponent_departed: false,
            pause: NetworkPauseState::new(),
            round_sync_target: 0,
            lag_started_tick: 0,
            waiting_started_tick: 0,
            disconnect_event_sent: false,
            stats_sent: false,
            arbitration: None,
            awaiting_verdict_since: 0,
        }
    }

    /// Handles an opponent disconnect, if there is one.
    fn check_disconnect(&mut self) {
        if !self.inner.opponent.is_connected() {
            self.trigger_disconnect();
        }
    }

    /// Handles a disconnect. Called from `check_disconnect` and from the
    /// timeout handlers. Two very different situations:
    ///
    /// - **Round engaged** (Countdown/Playing/AwaitingVerdict): forfeit.
    ///   Terminal: lifecycle `Ended`, Disconnected banner. The forfeited
    ///   round is NOT recorded in the ladder (no `RoundEnded`).
    /// - **Post-round**: silent departure. Not an event. The post-round
    ///   keeps running and the offer shrinks to [Menu]
    ///   (`opponent_present: false` in the ReadyPrompt snapshot). If we
    ///   were waiting for the opponent's "play again", the wait ends and
    ///   we return to the shrunken prompt. No message either way.
    fn trigger_disconnect(&mut self) {
        if self.forfeit_signaled || self.opponent_departed { return; }
        use crackattack_core::vs::SessionPhase;

        if round_engaged(self.inner.phase) {
            log::warn!("Network opponent disconnected mid-round — forfeit (phase={:?}, outcome={:?}, tally={}-{})",
                self.inner.phase, self.inner.round_outcome,
                self.inner.session_wins, self.inner.session_losses);
            self.inner.player_game.status = crackattack_core::game_state::GameStatus::Won;
            self.forfeit_signaled = true;
            return;
        }

        log::info!("Opponent departed post-round (phase={:?}) — silent offer contraction",
            self.inner.phase);
        self.opponent_departed = true;
        // Waiting for their "play again": the wait ends and we return to the
        // prompt, which now only offers [Menu].
        if self.inner.phase == SessionPhase::WaitingRemote {
            self.inner.phase = SessionPhase::WaitingBothReady;
        }
        // Clear any lag/pause/sync freeze so that no ConnectionLost banner
        // stays over the post-round screen.
        self.pause = NetworkPauseState::new();
        self.round_sync_target = 0;
    }

    /// Processes incoming pause/lag events and the SyncControl negotiation.
    fn process_pause_events(&mut self) {
        let mut _changed = false;

        if self.inner.opponent.take_opponent_paused() {
            self.pause.apply_remote_pause();
            _changed = true;
            log::info!("Pause: remote pause");
        }
        if self.inner.opponent.take_opponent_unpaused() {
            self.pause.apply_remote_resume_ready();
            _changed = true;
            log::info!("Pause: remote resume ready");
            // If both are ready, start SyncControl.
            if self.pause.both_ready() && self.pause.target_start_tick == 0 {
                let local_tick = self.inner.opponent.current_tick();
                let remote_tick = self.inner.opponent.last_recv_sync();
                let margin = self.inner.opponent.rtt_ticks() + SYNC_SAFETY_TICKS;
                self.pause.propose_sync(local_tick, remote_tick, margin);
                self.inner.opponent.send_control(ControlMessage::StartAt {
                    context: SyncContext::PauseResume,
                    epoch: self.pause.pause_seq,
                    tick: self.pause.target_start_tick,
                });
                log::info!("Pause: both ready, proposed sync target={} seq={}",
                    self.pause.target_start_tick, self.pause.pause_seq);
            }
        }
        if self.inner.opponent.take_lag_pause() {
            self.pause.apply_lag_pause();
            self.lag_started_tick = self.inner.opponent.current_tick();
            _changed = true;
            log::info!("Pause: lag pause");
        }
        if self.inner.opponent.take_lag_resume() {
            self.pause.apply_lag_resume();
            _changed = true;
            log::info!("Pause: lag resume");
        }

        // Pause SyncControl: adopt or converge on the remote's pause target.
        let remote_target = self.inner.opponent.remote_pause_target();
        let remote_seq = self.inner.opponent.remote_pause_seq();
        if remote_target > 0 && self.pause.target_start_tick == 0 {
            self.pause.adopt_remote_target(remote_target, remote_seq);
            self.inner.opponent.send_control(ControlMessage::StartAt {
                context: SyncContext::PauseResume,
                epoch: self.pause.pause_seq,
                tick: self.pause.target_start_tick,
            });
        } else if remote_target > 0 && self.pause.target_start_tick > 0 {
            let old = self.pause.target_start_tick;
            self.pause.adopt_remote_target(remote_target, remote_seq);
            if self.pause.target_start_tick != old {
                self.inner.opponent.send_control(ControlMessage::StartAt {
                    context: SyncContext::PauseResume,
                    epoch: self.pause.pause_seq,
                    tick: self.pause.target_start_tick,
                });
            }
        }

        // Pause convergence check.
        if self.pause.target_start_tick > 0
            && self.pause.is_sync_converged(remote_target)
            && self.inner.opponent.current_tick() >= self.pause.target_start_tick
        {
            log::info!("SyncControl: pause converged at tick {}", self.pause.target_start_tick);
            self.pause.clear_sync();
            self.inner.opponent.clear_remote_pause_sync();
            _changed = true;
        }
    }
}

impl GameSession for VsNetworkGameSession {
    fn tick(&mut self, input: &InputSnapshot) -> FrameData {
        // Update the local grid level for the next communication cycle.
        self.inner.opponent.set_local_grid_level(
            self.inner.player_game.grid.top_effective_row,
        );
        // Cache the opponent level for `snapshot()` (`inner.time_step` is not always called).
        self.inner.cached_opponent_level = self.inner.opponent.grid_level();

        // `trigger_disconnect()` handles everything.
        self.check_disconnect();

        // Terminal forfeit: freeze the simulation. The Disconnected banner
        // drives the exit (Enter/Escape to the menu).
        if self.forfeit_signaled {
            return self.inner.player_game.frame_data();
        }

        // Process pause/lag events from the network.
        self.process_pause_events();

        // Timeouts on blocking states (using the network tick, which always advances).
        // There is deliberately no timeout on the post-round wait for the
        // opponent's "play again" (`WaitingRemote`): we wait, like a lobby. A real
        // departure closes the socket and ends the wait via `trigger_disconnect`.
        let net_tick = self.inner.opponent.current_tick();
        // ConnectionLost for more than 30 s (1500 ticks): treat as a disconnect.
        if self.pause.lag_paused
            && !self.forfeit_signaled
            && self.lag_started_tick > 0
            && net_tick - self.lag_started_tick > 1500
        {
            log::warn!("Timeout: ConnectionLost exceeded 30s — treating as disconnect");
            self.trigger_disconnect();
        }
        // ResumeSync for more than 10 s: escalate to ConnectionLost.
        if self.pause.target_start_tick > 0
            && !self.pause.paused
            && !self.pause.lag_paused
            && !self.forfeit_signaled
        {
            // ResumeSync has no start tick, so use the target as the reference.
            // If the current tick is far past the target, the sync failed.
            if net_tick > self.pause.target_start_tick + 500 { // 10 s past the target
                log::warn!("Timeout: ResumeSync exceeded 10s past target — escalating to lag");
                self.pause.apply_lag_pause();
                self.lag_started_tick = net_tick;
            }
        }
        // Arbitration timeout. If the opponent keeps sending traffic but never sends
        // its `RoundLifeReport`, we would stay in `AwaitingVerdict` (lag detection does
        // not fire). Escalate to a disconnect (engaged round: forfeit).
        if self.inner.phase == SessionPhase::AwaitingVerdict {
            if self.awaiting_verdict_since == 0 {
                self.awaiting_verdict_since = net_tick.max(1);
            } else if !self.forfeit_signaled
                && net_tick - self.awaiting_verdict_since > VERDICT_TIMEOUT_TICKS
            {
                log::warn!("Timeout: AwaitingVerdict exceeded ~10s — treating as disconnect");
                self.trigger_disconnect();
            }
        } else {
            self.awaiting_verdict_since = 0;
        }

        // If frozen by pause/lag/sync, do not advance the simulation.
        // Keep the network communication alive.
        if self.pause.is_frozen() {
            self.inner.opponent.time_step();
            return self.inner.player_game.frame_data();
        }

        // Round start sync: intercept `WaitingRemote` ONLY when both sides
        // are ready and a sync negotiation is active or needed.
        use crackattack_core::vs::SessionPhase;
        let in_waiting_remote = self.inner.phase() == SessionPhase::WaitingRemote;
        let target_round = self.inner.round_number as u32;
        let both_ready = in_waiting_remote
            && self.inner.opponent.opponent_round_ready() == target_round;

        if both_ready {
            // Start the sync if it has not started yet.
            if self.round_sync_target == 0 {
                let local_tick = self.inner.opponent.current_tick();
                let remote_tick = self.inner.opponent.last_recv_sync();
                let margin = self.inner.opponent.rtt_ticks() + SYNC_SAFETY_TICKS;
                self.round_sync_target = local_tick.max(remote_tick) + margin;
                self.inner.opponent.send_control(ControlMessage::StartAt {
                    context: SyncContext::RoundStart,
                    epoch: target_round as u16,
                    tick: self.round_sync_target,
                });
                log::info!("RoundSync: proposed target={} (local={}, remote={})",
                    self.round_sync_target, local_tick, remote_tick);
            }

            // Adopt the remote's round proposal, if any.
            let remote_target = self.inner.opponent.remote_round_target();
            if remote_target > 0 {
                let old = self.round_sync_target;
                self.round_sync_target = self.round_sync_target.max(remote_target);
                if self.round_sync_target != old {
                    self.inner.opponent.send_control(ControlMessage::StartAt {
                        context: SyncContext::RoundStart,
                        epoch: target_round as u16,
                        tick: self.round_sync_target,
                    });
                }
            }

            // Check convergence.
            let remote_target = self.inner.opponent.remote_round_target();
            if self.round_sync_target > 0
                && self.round_sync_target == remote_target
                && self.inner.opponent.current_tick() >= self.round_sync_target
            {
                log::info!("RoundSync: converged at tick {} — starting round",
                    self.round_sync_target);
                self.round_sync_target = 0;
                self.inner.opponent.clear_remote_round_sync();
                // Reset the pause state for the new round.
                // `forfeit_signaled` and `opponent_departed` are NOT reset: both are
                // unreachable here (a forfeit freezes the session, and a departed
                // opponent can never converge a round sync).
                self.pause = NetworkPauseState::new();
                self.stats_sent = false;
                self.arbitration = None;
                self.inner.do_start_new_round();
                // `do_start_new_round` sets the phase to `Countdown` (via `start_round`).
                // Do not override it to `Playing`: let the countdown run.
                return self.inner.time_step(input);
            }

            // Not converged yet: keep the communication alive and do not let
            // `VsSession` auto-start the round.
            self.inner.opponent.time_step();
            return self.inner.player_game.frame_data();
        }

        // Send the local stats to the opponent once, when the round ends (Resolving or ShowResult).
        if !self.stats_sent && matches!(
            self.inner.phase(),
            SessionPhase::Resolving | SessionPhase::ShowResult
        ) {
            let stats = &self.inner.player_game.frame_data().stats;
            self.inner.opponent.send_control(ControlMessage::Stats {
                blocks_eliminated: stats.blocks_eliminated,
                chains_count: stats.chains_count,
                max_chain_level: stats.max_chain_level,
                garbage_sent: stats.garbage_sent,
                garbage_received: stats.garbage_received,
                garbage_shattered: stats.garbage_shattered,
                swaps_performed: stats.swaps_performed,
                duration_ticks: stats.duration_ticks,
            });
            self.stats_sent = true;
            log::info!("Sent local stats to opponent");
        }

        let frame = self.inner.time_step(input);

        // Did we just enter `AwaitingVerdict` (the local player lost)?
        if self.inner.phase == SessionPhase::AwaitingVerdict && self.arbitration.is_none() {
            let round_id = self.inner.round_number as u16;
            let loss_tick = self.inner.player_game.time_step;
            let swaps = self.inner.player_game.stats.swaps_performed;

            self.inner.opponent.send_life_report(round_id,
                RoundLifeStatus::Lost { loss_tick }, swaps);

            self.arbitration = Some(RoundArbitration {
                round_id,
                local_report: Some((RoundLifeStatus::Lost { loss_tick }, swaps)),
                remote_report: None,
            });
            // Signal the visual fade (before the verdict: the player sees the grid topped out).
            self.inner.pending_events.push(
                crackattack_core::frame_data::SessionEvent::LocalLoss);
            log::info!("Arbitration: local loss reported (round={} tick={} swaps={})",
                round_id, loss_tick, swaps);
        }

        if let Some(ref mut arb) = self.arbitration {
            // Check for the remote life report.
            if let Some((status, swaps)) = self.inner.opponent.take_life_report() {
                arb.remote_report = Some((status, swaps));
                log::info!("Arbitration: received remote report: {:?} swaps={}", status, swaps);

                // If WE are alive and the opponent reported Lost, send our Alive report.
                if arb.local_report.is_none() {
                    let our_swaps = self.inner.player_game.stats.swaps_performed;
                    arb.local_report = Some((RoundLifeStatus::Alive, our_swaps));
                    self.inner.opponent.send_life_report(arb.round_id,
                        RoundLifeStatus::Alive, our_swaps);
                    // Enter `AwaitingVerdict` (we are alive but wait for the verdict).
                    self.inner.phase = SessionPhase::AwaitingVerdict;
                    log::info!("Arbitration: we are alive, sent Alive report (swaps={})", our_swaps);
                }
            }

            // Check for the verdict (client side).
            if let Some((winner, reason)) = self.inner.opponent.take_verdict() {
                let is_server = self.inner.win_ties();
                let won = match winner {
                    PeerRole::Server => is_server,
                    PeerRole::Client => !is_server,
                };
                log::info!("Arbitration: received verdict winner={:?} reason={:?} => won={}",
                    winner, reason, won);
                self.inner.apply_round_verdict(won);
                self.arbitration = None;
                return frame;
            }

            // Server: arbitrate once we have both reports.
            let is_server = self.inner.win_ties();
            if is_server {
                if let (Some((srv_status, srv_swaps)), Some((cli_status, cli_swaps))) =
                    (arb.local_report, arb.remote_report)
                {
                    let (winner, reason) = arbitrate(srv_status, srv_swaps, cli_status, cli_swaps);

                    self.inner.opponent.send_verdict(arb.round_id, winner, reason);

                    let won = winner == PeerRole::Server;
                    log::info!("Arbitration: server verdict winner={:?} reason={:?} => won={}",
                        winner, reason, won);
                    self.inner.apply_round_verdict(won);
                    self.arbitration = None;
                    return frame;
                }
            }
        } else {
            // No active arbitration: check whether the opponent sent a life report
            // (the opponent lost while we are alive and arbitration has not started).
            if let Some((status, swaps)) = self.inner.opponent.take_life_report() {
                let round_id = self.inner.round_number as u16;
                let our_swaps = self.inner.player_game.stats.swaps_performed;

                // Send our Alive report.
                self.inner.opponent.send_life_report(round_id,
                    RoundLifeStatus::Alive, our_swaps);

                // Enter `AwaitingVerdict`.
                self.inner.phase = SessionPhase::AwaitingVerdict;

                let arb = RoundArbitration {
                    round_id,
                    local_report: Some((RoundLifeStatus::Alive, our_swaps)),
                    remote_report: Some((status, swaps)),
                    };

                log::info!("Arbitration: opponent reported {:?} while we are alive (round={})",
                    status, round_id);

                // Server: arbitrate immediately.
                let is_server = self.inner.win_ties();
                if is_server {
                    let (srv_status, srv_swaps) = arb.local_report.unwrap();
                    let (cli_status, cli_swaps) = arb.remote_report.unwrap();
                    let (winner, reason) = arbitrate(srv_status, srv_swaps, cli_status, cli_swaps);

                    self.inner.opponent.send_verdict(arb.round_id, winner, reason);

                    let won = winner == PeerRole::Server;
                    log::info!("Arbitration: server immediate verdict winner={:?} reason={:?} => won={}",
                        winner, reason, won);
                    self.inner.apply_round_verdict(won);
                    // `arb` is not stored: the round is already resolved.
                } else {
                    // Client: wait for the verdict.
                    self.arbitration = Some(arb);
                }
            }
        }

        frame
    }

    fn frame_data(&self) -> FrameData {
        self.inner.player_game.frame_data()
    }

    fn signal_local_ready(&mut self) {
        // Opponent gone: nobody to play with. The shrunken offer only leads to
        // the menu ("play again" is no longer offered).
        if self.opponent_departed { return; }
        // Network mode: signal ready but NEVER auto-start the round.
        // The `tick()` interceptor handles sync convergence before starting.
        use crackattack_core::vs::SessionPhase;
        if self.inner.phase == SessionPhase::WaitingBothReady {
            let target = self.inner.round_number as u32;
            self.inner.opponent.set_local_round_ready(target);
            self.inner.phase = SessionPhase::WaitingRemote;
            self.waiting_started_tick = self.inner.opponent.current_tick();
            // Do not call `do_start_new_round()`: `tick()` calls it after
            // `target_start_tick` converges.
        }
    }

    fn game_state_mut(&mut self) -> Option<&mut GameState> {
        Some(&mut self.inner.player_game)
    }

    fn request_pause(&mut self) -> bool {
        if self.pause.try_pause() {
            self.inner.opponent.signal_paused();
            log::info!("Pause: pause accepted (via request_pause)");
            true
        } else {
            false
        }
    }

    fn request_resume(&mut self) -> bool {
        if self.pause.signal_resume_ready() {
            // `Unpause` means "I am ready to resume".
            self.inner.opponent.send_control(ControlMessage::Unpause {
                epoch: self.pause.pause_seq,
            });
            log::info!("Pause: local resume ready signaled");

            // If both sides are now ready, start SyncControl.
            if self.pause.both_ready() && self.pause.target_start_tick == 0 {
                let local_tick = self.inner.opponent.current_tick();
                let remote_tick = self.inner.opponent.last_recv_sync();
                let margin = self.inner.opponent.rtt_ticks() + SYNC_SAFETY_TICKS;
                self.pause.propose_sync(local_tick, remote_tick, margin);
                self.inner.opponent.send_control(ControlMessage::StartAt {
                    context: SyncContext::PauseResume,
                    epoch: self.pause.pause_seq,
                    tick: self.pause.target_start_tick,
                });
                log::info!("Pause: both ready, proposed sync target={} seq={}",
                    self.pause.target_start_tick, self.pause.pause_seq);
            }
            true
        } else {
            false
        }
    }

    fn abandon(&mut self) {
        self.inner.opponent.signal_forfeit();
        log::info!("Network session abandoned (forfeit signaled)");
    }

    fn snapshot(&self) -> crackattack_core::frame_data::SessionSnapshot {
        use crackattack_core::frame_data::*;
        use crackattack_core::vs::SessionPhase;

        let lifecycle = if self.forfeit_signaled {
            SessionLifecycle::Ended
        } else {
            match self.inner.phase() {
                SessionPhase::Countdown => SessionLifecycle::Countdown,
                SessionPhase::Playing => SessionLifecycle::Playing,
                SessionPhase::Resolving => SessionLifecycle::Resolving,
                SessionPhase::ShowResult => SessionLifecycle::ShowResult,
                SessionPhase::Prepared
                | SessionPhase::WaitingBothReady | SessionPhase::WaitingRemote
                | SessionPhase::AwaitingVerdict => {
                    SessionLifecycle::Playing
                }
            }
        };

        // Interruption priority: disconnect > lag > pause > sync > phase suspension.
        let interruption = if self.forfeit_signaled {
            SessionInterruption::Disconnected
        } else if self.pause.lag_paused {
            SessionInterruption::ConnectionLost { started_tick: self.lag_started_tick }
        } else if self.pause.paused {
            SessionInterruption::Paused {
                local_ready: self.pause.local_resume_ready,
                remote_ready: self.pause.remote_resume_ready,
                is_network: true,
            }
        } else if self.pause.target_start_tick > 0 {
            SessionInterruption::ResumeSync {
                started_tick: self.inner.opponent.current_tick(),
            }
        } else {
            match self.inner.phase() {
                SessionPhase::Prepared => SessionInterruption::ScenePreparing,
                SessionPhase::WaitingBothReady => {
                    let present = !self.opponent_departed
                        && self.inner.opponent.is_connected();
                    SessionInterruption::ReadyPrompt {
                        round_number: self.inner.round_number,
                        // "X wants to play again": the opponent already agreed to
                        // this round. Never shown for a departed opponent (a Ready
                        // received before the departure is void).
                        remote_ready: present
                            && self.inner.opponent.opponent_round_ready()
                                == self.inner.round_number as u32,
                        opponent_present: present,
                    }
                }
                SessionPhase::WaitingRemote => SessionInterruption::WaitingRemoteReady {
                    started_tick: self.waiting_started_tick,
                },
                SessionPhase::AwaitingVerdict => SessionInterruption::AwaitingRoundVerdict,
                _ => SessionInterruption::None,
            }
        };

        let result = if self.forfeit_signaled {
            SessionResult::SessionWin  // The opponent forfeited.
        } else {
            match self.inner.phase() {
                SessionPhase::Resolving | SessionPhase::ShowResult => match self.inner.round_outcome {
                    Some(crackattack_core::vs::RoundOutcome::PlayerWon) => SessionResult::RoundWin,
                    Some(crackattack_core::vs::RoundOutcome::PlayerLost) => SessionResult::RoundLose,
                    None => SessionResult::None,
                },
                _ => SessionResult::None,
            }
        };

        let vs_display = Some(VsDisplay {
            opponent_name: self.inner.opponent_name().to_string(),
            opponent_level: self.inner.cached_opponent_level,
            round_number: self.inner.round_number,
            session_wins: self.inner.session_wins,
            session_losses: self.inner.session_losses,
            opponent_stats: self.inner.opponent.opponent_stats().cloned(),
            opponent_uuid: Some(self.inner.opponent.opponent_uuid()),
        });

        SessionSnapshot {
            lifecycle, interruption, result, vs_display,
            countdown: self.inner.countdown_info(),
            round_engaged: !self.forfeit_signaled && round_engaged(self.inner.phase()),
        }
    }

    fn drain_events(&mut self) -> Vec<crackattack_core::frame_data::SessionEvent> {
        let mut events = std::mem::take(&mut self.inner.pending_events);
        // Add the disconnect event if it was just signaled.
        if self.forfeit_signaled && !self.disconnect_event_sent {
            events.push(crackattack_core::frame_data::SessionEvent::OpponentDisconnected);
            self.disconnect_event_sent = true;
        }
        events
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Contract test: a network session built on a loopback TCP pair exposes its
    // real state (`vs_display` is set and pausing is accepted).
    #[test]
    fn contract_net_snapshot_reflects_real_session() {
        use crate::connection::{NetConnection, Role};
        use crate::opponent::NetworkOpponent;
        use crackattack_core::frame_data::SessionInterruption;
        use crackattack_core::vs::session::VsSession;
        use std::net::{TcpListener, TcpStream};

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let client = TcpStream::connect(addr).unwrap();
        let (_server, _sa) = listener.accept().unwrap(); // Keeps the peer alive.

        let conn = NetConnection::from_stream(client, addr);
        let opp = NetworkOpponent::new(conn, Role::Client, "PAIR".into(), [7u8; 16]);
        let mut s = VsNetworkGameSession::new(VsSession::new(42, opp));

        let snap = s.snapshot();
        assert!(snap.vs_display.is_some());
        // A new network session waits for the "ready?" prompt of round 1. This
        // state belongs to the round; it is not a placeholder `None`.
        assert!(matches!(
            snap.interruption,
            SessionInterruption::ReadyPrompt { round_number: 1, .. }
        ));
        // `drain_events` returns the real events, then is empty.
        let _premiers = s.drain_events();
        assert!(s.drain_events().is_empty());
        assert!(s.game_state_mut().is_some());

        assert!(s.request_pause());
        assert!(matches!(
            s.snapshot().interruption,
            SessionInterruption::Paused { is_network: true, .. }
        ));
    }

    fn fresh() -> NetworkPauseState {
        NetworkPauseState::new()
    }

    #[test]
    fn pause_then_both_ready_and_sync() {
        let mut s = fresh();
        assert!(!s.is_frozen());
        assert!(s.try_pause());
        assert!(s.is_frozen());
        assert!(s.paused);

        // Local signals ready.
        assert!(s.signal_resume_ready());
        assert!(s.local_resume_ready);
        assert!(!s.both_ready()); // The remote is not ready yet.
        assert!(s.is_frozen()); // Still paused.

        // Remote signals ready.
        s.apply_remote_resume_ready();
        assert!(s.remote_resume_ready);
        assert!(s.both_ready());

        // Both ready: propose sync, then clear.
        s.propose_sync(100, 80, 5);
        assert_eq!(s.target_start_tick, 105);
        assert!(s.is_frozen()); // Frozen by sync target and pause.

        // Convergence: `clear_sync` resets everything.
        s.clear_sync();
        assert!(!s.is_frozen());
        assert!(!s.paused);
        assert!(!s.local_resume_ready);
        assert!(!s.remote_resume_ready);
    }

    #[test]
    fn remote_pause_then_both_ready() {
        let mut s = fresh();
        s.apply_remote_pause();
        assert!(s.is_frozen());
        assert!(s.paused);
        assert!(!s.local_resume_ready);
        assert!(!s.remote_resume_ready);

        // Both signal ready.
        assert!(s.signal_resume_ready());
        s.apply_remote_resume_ready();
        assert!(s.both_ready());
    }

    #[test]
    fn pause_blocked_when_already_frozen() {
        let mut s = fresh();
        s.apply_remote_pause(); // Already frozen (`paused` is true).
        assert!(!s.try_pause()); // Cannot pause twice.
    }

    #[test]
    fn pause_blocked_during_lag() {
        let mut s = fresh();
        s.apply_lag_pause();
        assert!(!s.try_pause());
    }

    #[test]
    fn pause_blocked_during_sync() {
        let mut s = fresh();
        s.target_start_tick = 500;
        assert!(!s.try_pause());
    }

    #[test]
    fn resume_ready_blocked_during_sync() {
        let mut s = fresh();
        s.paused = true;
        s.target_start_tick = 500;
        assert!(!s.signal_resume_ready());
    }

    #[test]
    fn resume_ready_when_not_paused_is_noop() {
        let mut s = fresh();
        assert!(!s.signal_resume_ready());
    }

    #[test]
    fn resume_ready_idempotent() {
        let mut s = fresh();
        s.paused = true;
        assert!(s.signal_resume_ready());
        assert!(!s.signal_resume_ready()); // Already ready.
    }

    #[test]
    fn propose_sync_calculates_target() {
        let mut s = fresh();
        s.propose_sync(100, 120, 5); // Margin = 5.
        // Target = max(100, 120) + 5 = 125.
        assert_eq!(s.target_start_tick, 125);
        assert_eq!(s.pause_seq, 1);
        assert!(s.is_frozen()); // Frozen by `target_start_tick > 0`.
    }

    #[test]
    fn adopt_remote_target_no_local_adopts_any_epoch() {
        // With no local proposal, adopt the remote regardless of epoch.
        let mut s = fresh();
        s.pause_seq = 0;
        s.adopt_remote_target(200, 5);
        assert_eq!(s.target_start_tick, 200);
        assert_eq!(s.pause_seq, 5); // Adopted the remote's epoch.
    }

    #[test]
    fn adopt_remote_target_wrong_epoch_ignored_when_local_set() {
        // With our own proposal, ignore a remote with a different epoch.
        let mut s = fresh();
        s.pause_seq = 3;
        s.target_start_tick = 100;
        s.adopt_remote_target(200, 2); // Different epoch.
        assert_eq!(s.target_start_tick, 100); // Unchanged.
    }

    #[test]
    fn converge_takes_max() {
        let mut s = fresh();
        s.pause_seq = 5;
        s.target_start_tick = 100;
        s.adopt_remote_target(150, 5);
        assert_eq!(s.target_start_tick, 150); // Max of 100 and 150.

        s.adopt_remote_target(120, 5);
        assert_eq!(s.target_start_tick, 150); // Monotone: never decreases.
    }

    #[test]
    fn sync_convergence_check() {
        let mut s = fresh();
        s.target_start_tick = 200;
        assert!(!s.is_sync_converged(0));
        assert!(!s.is_sync_converged(100));
        assert!(s.is_sync_converged(200));
    }

    #[test]
    fn clear_sync_unfreezes_and_resets_pause() {
        let mut s = fresh();
        s.paused = true;
        s.local_resume_ready = true;
        s.remote_resume_ready = true;
        s.target_start_tick = 200;
        assert!(s.is_frozen());
        s.clear_sync();
        assert_eq!(s.target_start_tick, 0);
        assert!(!s.paused);
        assert!(!s.local_resume_ready);
        assert!(!s.remote_resume_ready);
        assert!(!s.is_frozen());
    }

    #[test]
    fn both_ready_then_sync_then_clear() {
        let mut s = fresh();
        s.paused = true;
        s.local_resume_ready = true;
        s.remote_resume_ready = true;
        assert!(s.both_ready());

        s.propose_sync(100, 80, 5); // Margin = 5.
        // Target = max(100, 80) + 5 = 105.
        assert_eq!(s.target_start_tick, 105);
        assert!(s.is_frozen()); // Frozen by sync target and pause.

        // Simulate convergence.
        assert!(s.is_sync_converged(105));
        s.clear_sync();
        assert!(!s.is_frozen());
    }

    #[test]
    fn lag_during_pause_stays_paused_after_lag_resume() {
        let mut s = fresh();
        assert!(s.try_pause());
        s.apply_lag_pause();

        s.apply_lag_resume();
        assert!(s.is_frozen()); // Still paused.
        assert!(s.paused);
        assert!(!s.lag_paused);
    }

    #[test]
    fn remote_pause_queued_during_lag() {
        let mut s = fresh();
        s.apply_lag_pause();
        s.apply_remote_pause();
        assert!(!s.paused); // Not applied yet.
        assert!(s.queued_remote_pause);

        s.apply_lag_resume();
        assert!(s.paused);
        assert!(!s.queued_remote_pause);
    }

    #[test]
    fn remote_resume_ready_queued_during_lag() {
        let mut s = fresh();
        s.paused = true;
        s.apply_lag_pause();
        s.apply_remote_resume_ready();
        assert!(!s.remote_resume_ready); // Not applied yet.
        assert!(s.queued_remote_resume_ready);

        s.apply_lag_resume();
        assert!(s.remote_resume_ready);
        assert!(!s.queued_remote_resume_ready);
    }

    #[test]
    fn queued_signals_are_mutually_exclusive() {
        let mut s = fresh();
        s.apply_lag_pause();
        s.apply_remote_pause();
        assert!(s.queued_remote_pause);

        s.apply_remote_resume_ready();
        assert!(!s.queued_remote_pause);
        assert!(s.queued_remote_resume_ready);
    }

    #[test]
    fn pause_resets_ready_flags() {
        let mut s = fresh();
        // Stale "ready" from a previous pause: a new pause must clear it, otherwise the
        // game would resume at once.
        s.local_resume_ready = true;
        s.remote_resume_ready = true;
        assert!(s.try_pause());
        assert!(!s.local_resume_ready);
        assert!(!s.remote_resume_ready);
    }

    #[test]
    fn remote_pause_resets_ready_flags() {
        let mut s = fresh();
        s.local_resume_ready = true;
        s.remote_resume_ready = true;
        s.apply_remote_pause();
        assert!(!s.local_resume_ready);
        assert!(!s.remote_resume_ready);
    }

    #[test]
    fn freeze_cause_priority() {
        let mut s = fresh();
        assert!(!s.is_frozen());

        // Pause: frozen.
        s.paused = true;
        assert!(s.is_frozen());

        // Lag on top of pause: still frozen.
        s.lag_paused = true;
        assert!(s.is_frozen());

        // Lag cleared, sync active: still frozen.
        s.lag_paused = false;
        s.target_start_tick = 100;
        assert!(s.is_frozen());

        // Sync cleared, still paused: frozen.
        s.target_start_tick = 0;
        assert!(s.is_frozen());
        assert!(s.paused);

        // Pause cleared: not frozen.
        s.paused = false;
        assert!(!s.is_frozen());
    }

    #[test]
    fn arbitrate_only_server_lost() {
        let (winner, reason) = arbitrate(
            RoundLifeStatus::Lost { loss_tick: 500 }, 10,
            RoundLifeStatus::Alive, 20,
        );
        assert_eq!(winner, PeerRole::Client);
        assert_eq!(reason, VerdictReason::OnlyOneLost);
    }

    #[test]
    fn arbitrate_only_client_lost() {
        let (winner, reason) = arbitrate(
            RoundLifeStatus::Alive, 10,
            RoundLifeStatus::Lost { loss_tick: 500 }, 20,
        );
        assert_eq!(winner, PeerRole::Server);
        assert_eq!(reason, VerdictReason::OnlyOneLost);
    }

    #[test]
    fn arbitrate_both_lost_server_survived_longer() {
        let (winner, reason) = arbitrate(
            RoundLifeStatus::Lost { loss_tick: 600 }, 10,
            RoundLifeStatus::Lost { loss_tick: 500 }, 10,
        );
        assert_eq!(winner, PeerRole::Server);
        assert_eq!(reason, VerdictReason::HigherLossTick);
    }

    #[test]
    fn arbitrate_both_lost_client_survived_longer() {
        let (winner, reason) = arbitrate(
            RoundLifeStatus::Lost { loss_tick: 500 }, 10,
            RoundLifeStatus::Lost { loss_tick: 600 }, 10,
        );
        assert_eq!(winner, PeerRole::Client);
        assert_eq!(reason, VerdictReason::HigherLossTick);
    }

    #[test]
    fn arbitrate_same_tick_server_fewer_swaps() {
        let (winner, reason) = arbitrate(
            RoundLifeStatus::Lost { loss_tick: 500 }, 10,
            RoundLifeStatus::Lost { loss_tick: 500 }, 20,
        );
        assert_eq!(winner, PeerRole::Server);
        assert_eq!(reason, VerdictReason::FewerSwaps);
    }

    #[test]
    fn arbitrate_same_tick_client_fewer_swaps() {
        let (winner, reason) = arbitrate(
            RoundLifeStatus::Lost { loss_tick: 500 }, 20,
            RoundLifeStatus::Lost { loss_tick: 500 }, 10,
        );
        assert_eq!(winner, PeerRole::Client);
        assert_eq!(reason, VerdictReason::FewerSwaps);
    }

    #[test]
    fn arbitrate_total_tie_server_wins() {
        let (winner, reason) = arbitrate(
            RoundLifeStatus::Lost { loss_tick: 500 }, 10,
            RoundLifeStatus::Lost { loss_tick: 500 }, 10,
        );
        assert_eq!(winner, PeerRole::Server);
        assert_eq!(reason, VerdictReason::ServerTieBreak);
    }

    /// Forfeit only applies to ENGAGED rounds. Every post-round phase is a
    /// silent departure.
    #[test]
    fn round_engaged_classification() {
        use crackattack_core::vs::SessionPhase as P;
        for p in [P::Countdown, P::Playing, P::AwaitingVerdict] {
            assert!(round_engaged(p), "{:?} must be a committed round (forfeit)", p);
        }
        for p in [P::Resolving, P::ShowResult, P::Prepared,
                  P::WaitingBothReady, P::WaitingRemote] {
            assert!(!round_engaged(p), "{:?} must be post-round (silent departure)", p);
        }
    }
}
