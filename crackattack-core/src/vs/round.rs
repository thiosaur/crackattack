//! Round lifecycle: SessionPhase, RoundOutcome, durations.
//!
//! A VS session is an OPEN series of rounds over one connection — there is
//! no "match" layer above the round cycle. Session tallying (wins/losses)
//! lives in `VsSession` as plain counters.

/// Round outcome (determined after celebration).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RoundOutcome {
    PlayerWon,
    PlayerLost,
}

/// Duration of the Resolving phase (game zone fade-out) in ticks.
pub(crate) const RESOLVING_DURATION_TICKS: i32 = 150;

/// ShowResult duration after a round (1 second) — the single, uniform
/// celebration: every round ends the same way (no long "match end" variant).
pub(crate) const CELEBRATING_ROUND_TICKS: i32 = 50;

/// OutQuad easing: fast start, slow finish.
/// Is a round ENGAGED in this phase? A round is engaged from its countdown
/// until its outcome is determined — leaving then is destructive (rated
/// loss, exit confirmation). The post-round phases engage nothing: leaving
/// is a normal, direct exit. Invariant: an exit needs confirmation if and only
/// if it costs a rated round.
pub fn is_round_engaged(phase: SessionPhase) -> bool {
    matches!(phase,
        SessionPhase::Countdown | SessionPhase::Playing | SessionPhase::AwaitingVerdict)
}

/// Session lifecycle phase.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionPhase {
    /// Countdown before round start (3-2-1-GO).
    Countdown,
    /// A round is in progress.
    Playing,
    /// Round ended, game zone fading out (RESOLVING_DURATION_TICKS = 150).
    Resolving,
    /// Show result phase after resolving (celebration).
    ShowResult,
    /// Round prepared, curtain opening, not yet interactive.
    Prepared,
    /// ShowResult done. Both sides waiting for consent to (re)play.
    WaitingBothReady,
    /// Local player consented. Waiting for opponent to be ready.
    WaitingRemote,
    /// Waiting for server verdict on round outcome (network only).
    AwaitingVerdict,
}
