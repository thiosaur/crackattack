//! Opponent trait — unified interface for AI and network opponents.
//!
//! Both ComputerOpponent and NetworkOpponent implement this trait.
//! The VS session code never knows which kind of opponent it is
//! talking to.
//!
//! Methods with default implementations are no-ops for AI opponents
//! but carry real logic for network opponents.

/// Unified opponent interface for VS mode.
///
/// Implementations must handle their own timing, garbage queuing,
/// and loss detection. The session coordinator only calls these
/// methods — it never inspects the opponent's internals.
pub trait Opponent {
    /// Reset for a new round.
    fn game_start(&mut self);

    /// Advance one tick of opponent logic.
    fn time_step(&mut self);

    /// Queue garbage from the local player to the opponent.
    fn send_garbage(&mut self, height: i32, width: i32, flavor: i32);

    /// Drain garbage from the opponent destined for the local player.
    /// Returns (height, width, flavor) tuples.
    fn recv_garbage(&mut self) -> Vec<(i32, i32, i32)>;

    /// Opponent's grid level for level lights display (0-12).
    fn grid_level(&mut self) -> i32;

    /// Notify the opponent that the local player may have lost.
    /// `time_step` is the game tick when loss was detected.
    fn notify_local_loss(&mut self, _time_step: i32) {}

    /// Has the opponent lost?
    fn has_opponent_lost(&self) -> bool;

    /// Timestamp when the opponent lost (valid only if has_opponent_lost() is true).
    /// Used for tie-breaking simultaneous losses.
    fn opponent_loss_timestamp(&self) -> Option<i32> { None }

    /// Signal that the local player paused.
    fn signal_paused(&mut self) {}

    /// Is the opponent still connected? Always true for AI.
    fn is_connected(&self) -> bool { true }

    /// Opponent's display name.
    fn opponent_name(&self) -> &str;

    /// Set local player's round-ready signal (round number they're ready for).
    fn set_local_round_ready(&mut self, _round: u32) {}

    /// Get opponent's round-ready signal (0 = not ready).
    fn opponent_round_ready(&self) -> u32 { 0 }
}
