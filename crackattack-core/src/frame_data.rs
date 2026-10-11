// Frame data - output for rendering

use crate::block::Block;
use crate::garbage::Garbage;
use crate::swapper::{Swapper, SwapperState, MoveDirection};



/// Structured snapshot of the session state.
/// Single source of truth for the runtime and renderer.
/// Not a flat enum — preserves the natural domain structure.
///
/// Invariants:
/// - `lifecycle == Ended` implies `result in { SessionWin, SessionLose }`
/// - `result in { SessionWin, SessionLose }` implies `lifecycle == Ended`
/// - `result in { RoundWin, RoundLose }` implies `lifecycle == ShowResult`
/// - `lifecycle in { Playing, Countdown }` implies `result == SessionResult::None`
/// - `interruption == Disconnected` is terminal — session is over
#[derive(Debug, Clone)]
pub struct SessionSnapshot {
    pub lifecycle: SessionLifecycle,
    pub interruption: SessionInterruption,
    pub result: SessionResult,
    pub vs_display: Option<VsDisplay>,
    pub countdown: Option<CountdownInfo>,
    /// A round is ENGAGED (countdown → outcome). Leaving now is destructive:
    /// rated loss + exit confirmation. False in the post-round phases and at
    /// the ready prompt — leaving there is a normal, direct exit.
    /// NOTE: derived from the session PHASE, not from `lifecycle` (which
    /// collapses the waiting phases into `Playing`).
    pub round_engaged: bool,
}

/// Phase of the round in the session lifecycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionLifecycle {
    /// Countdown before round start (3-2-1-GO).
    Countdown,
    /// Round in progress.
    Playing,
    /// Game zone fading out (RESOLVING_DURATION_TICKS = 150). Result may appear during this.
    Resolving,
    /// Show result phase after resolving (CELEBRATING_ROUND_TICKS = 50).
    ShowResult,
    /// Session ended: solo game over, or network forfeit (mid-round
    /// disconnect). A VS session never ends by itself — rounds chain
    /// as an open series until the player quits.
    Ended,
}

/// Active interruption that freezes the game.
/// None = no freeze, game runs normally.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionInterruption {
    /// No interruption.
    None,
    /// Scene preparing — round prepared, curtain opening, not interactive.
    ScenePreparing,
    /// Post-round consent (or the initial "Ready?") — player must consent
    /// before the round starts.
    ReadyPrompt {
        /// Round about to be played (1-based, unbounded — open series).
        round_number: i32,
        /// Opponent already consented (the "X wants a rematch" hint).
        remote_ready: bool,
        /// Opponent still connected. False = the offer contracted:
        /// only "Menu" remains, silently (no departure message).
        opponent_present: bool,
    },
    /// Waiting for remote opponent to confirm ready.
    WaitingRemoteReady { started_tick: i32 },
    /// Game is paused. Either player can mark themselves ready to resume.
    /// The game resumes when both are ready (consensual in network mode).
    Paused {
        local_ready: bool,
        remote_ready: bool,
        /// True only for VsNetwork — determines UI (Resume vs Ready to resume).
        is_network: bool,
    },
    /// Technical sync after pause resume (1-2s, silent).
    ResumeSync { started_tick: i32 },
    /// Network connection lost (lag).
    ConnectionLost { started_tick: i32 },
    /// Awaiting server verdict on round outcome (network only).
    AwaitingRoundVerdict,
    /// Opponent disconnected — terminal state, session is over.
    Disconnected,
}

/// Game result to display (texture message).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionResult {
    None,
    RoundWin,
    RoundLose,
    /// Terminal: solo game won / opponent forfeited (network disconnect).
    SessionWin,
    /// Terminal: solo game lost.
    SessionLose,
}


/// VS mode display info.
#[derive(Debug, Clone)]
pub struct VsDisplay {
    pub opponent_name: String,
    pub opponent_level: i32,
    /// Current round number (1-based, unbounded — open series).
    pub round_number: i32,
    /// Session tally: rounds won during this connection.
    pub session_wins: i32,
    /// Session tally: rounds lost during this connection.
    pub session_losses: i32,
    /// Opponent's post-game stats (received via network after round ends).
    pub opponent_stats: Option<crate::stats::GameStats>,
    /// Persistent identity of the opponent (received in the network handshake), used for the ladder.
    /// `None` = opponent without identity (local AI).
    pub opponent_uuid: Option<[u8; 16]>,
}

/// One-shot lifecycle event emitted by the session.
/// Consumed by the runtime for effects and audio.
/// Rule: no visual/audio side-effect by snapshot polling —
/// all transient effects must react to events.
#[derive(Debug, Clone)]
pub enum SessionEvent {
    /// Round grid created (fresh GameState). Curtain can open.
    RoundPrepared { round_number: i32 },
    /// A new round starts (reset level lights, clear round-scope particles).
    RoundStarted { round_number: i32 },
    /// Local player lost — visual fade starts (before verdict in network).
    LocalLoss,
    /// A round ends with a determined outcome (after verdict in network).
    /// There is no "match ended" event — the session is an open series;
    /// it ends by an explicit player action (Quitter), not a game event.
    RoundEnded { won: bool },
    /// Return to menu (full renderer cleanup).
    ReturnedToMenu,
    /// Opponent disconnected.
    OpponentDisconnected,
    /// Countdown digit changed (for sound effects).
    CountdownDigit { digit: CountdownDigit },
}


/// Countdown digit being displayed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CountdownDigit {
    Three,
    Two,
    One,
    Go,
}

/// Countdown view for the renderer.
#[derive(Debug, Clone)]
pub struct CountdownInfo {
    pub digit: CountdownDigit,
    pub ticks_into_digit: i32,
    pub ticks_per_digit: i32,
}

/// Game phase for UI rendering
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum GamePhase {
    #[default]
    Playing,    // Active gameplay
    Finished,   // Game over (win/lose)
    // Pause and Countdown are session concerns, not simulation phases.
}

/// UI state snapshot for rendering
#[derive(Debug, Clone, Default)]
pub struct UIState {
    /// Current game phase
    pub phase: GamePhase,
    /// Clock/timer value in game ticks
    pub clock_time: i32,
    /// Current score
    pub score: i32,
    /// Ticks since game ended (0 if still playing, for celebration animations)
    pub ticks_since_finished: i32,
    /// Current level (for level lights, 1-based)
    pub level: i32,
    /// Danger level (0.0 = safe, 1.0 = critical)
    pub danger_height: f32,
    /// Raw creep state for renderer-side danger bar
    pub creep_freeze: bool,
    /// Raw loss countdown for renderer-side danger bar
    pub loss_countdown: i32,
    /// Effective height of block stack (for renderer-side level lights)
    pub top_effective_row: i32,
    /// Rows left before the safe height, fraction included (`Creep::rows_to_loss`), and
    /// rise pace (`Creep::seconds_per_row`). Both drive the adaptive music.
    pub rows_to_loss: f32,
    pub seconds_per_row: f32,
}

#[derive(Debug, Clone)]
pub struct BlockSnapshot {
    pub id: i32,
    pub grid_x: i32,
    pub grid_y: i32,
    pub sub_grid_y: i32,
    pub flavor: i32,
    pub state: u8,
    pub animation_countdown: i32,
    pub pop_countdown: i32,
    pub axis_x: f32,
    pub axis_y: f32,
    pub pop_direction: u8,
    pub pop_color: i32,
    /// Combo chain level for this block (1 if no active chain).
    /// From the combo tracker, not the scoring system.
    pub chain_level: i32,
}

impl BlockSnapshot {
    pub(crate) fn from_block_with_chain_level(block: &Block, chain_level: i32) -> Self {
        BlockSnapshot {
            id: block.id.0 as i32,
            grid_x: block.grid_x,
            grid_y: block.grid_y,
            sub_grid_y: block.sub_grid_y,
            flavor: block.flavor,
            state: block.physics_flags.bits(),
            animation_countdown: block.animation_countdown,
            pop_countdown: block.pop_countdown,
            axis_x: block.axis_x,
            axis_y: block.axis_y,
            pop_direction: block.pop_direction as u8,
            pop_color: block.pop_color,
            chain_level,
        }
    }
}

#[derive(Debug, Clone)]
pub struct GarbageSnapshot {
    pub id: i32,
    pub grid_x: i32,
    pub grid_y: i32,
    pub sub_grid_y: i32,
    pub width: i32,
    pub height: i32,
    pub flavor: i32,
    pub state: u8,
    pub sections_popped: i32,
    pub pop_direction: u8,
    pub pop_color: i32,
    pub shatter_progress: f32,  // 0.0 to 1.0 for shattering garbage
    pub awaking_progress: f32,  // 0.0 to 1.0 for awaking garbage (reform animation)
    pub pop_progress: f32,      // 0.0 to 1.0 for current section pop animation
}

impl GarbageSnapshot {
    pub(crate) fn from_garbage(garbage: &Garbage, time_step: i32) -> Self {
        use crate::constants::{SHATTER_TIME, FINAL_POP_DELAY};
        use crate::garbage::GarbageState;

        // Compute shatter progress (0.0 to 1.0)
        let shatter_progress = if garbage.physics_state == GarbageState::Shattering && SHATTER_TIME > 0 {
            let time_since_start = SHATTER_TIME + time_step - garbage.animation_countdown;
            (time_since_start as f32 / SHATTER_TIME as f32).clamp(0.0, 1.0)
        } else {
            0.0
        };

        // Compute awaking progress (0.0 to 1.0) - reform animation
        let awaking_progress = compute_garbage_awaking_progress(
            garbage.physics_state,
            garbage.animation_countdown,
            time_step,
            FINAL_POP_DELAY,
        );

        // Compute per-section pop progress
        use crate::constants::INTERNAL_POP_DELAY;
        let total = garbage.width * garbage.height;
        let pop_progress = if garbage.physics_state == GarbageState::Awaking
            && garbage.sections_popped < total
            && garbage.pop_countdown > 0
        {
            let remaining = garbage.pop_countdown - time_step;
            if (0..INTERNAL_POP_DELAY).contains(&remaining) {
                1.0 - (remaining as f32 / INTERNAL_POP_DELAY as f32)
            } else {
                0.0
            }
        } else {
            0.0
        };

        GarbageSnapshot {
            id: garbage.id.0 as i32,
            grid_x: garbage.grid_x,
            grid_y: garbage.grid_y,
            sub_grid_y: garbage.sub_grid_y,
            width: garbage.width,
            height: garbage.height,
            flavor: garbage.flavor,
            state: garbage.physics_state as u8,
            sections_popped: garbage.sections_popped,
            pop_direction: garbage.pop_direction as u8,
            pop_color: garbage.pop_color,
            shatter_progress,
            awaking_progress,
            pop_progress,
        }
    }
}

fn compute_garbage_awaking_progress(
    state: crate::garbage::GarbageState,
    animation_countdown: i32,
    time_step: i32,
    unshatter_duration: i32,
) -> f32 {
    use crate::garbage::GarbageState;

    if state != GarbageState::Awaking || unshatter_duration <= 0 {
        return 0.0;
    }

    let remaining = animation_countdown - time_step;
    if remaining <= 0 {
        return 1.0;
    }

    let reform_phase = unshatter_duration / 4;
    if reform_phase <= 0 || remaining > reform_phase {
        return 0.0;
    }

    1.0 - (remaining as f32 / reform_phase as f32)
}

#[derive(Debug, Clone)]
pub struct SwapperSnapshot {
    pub grid_x: i32,
    pub grid_y: i32,
    pub state: u8,
    pub swap_factor: f32,
    /// MovePause countdown (6→0) for move animation
    pub move_lockout_countdown: i32,
    /// Last move direction: 0=None, 1=Left, 2=Right, 3=Up, 4=Down
    pub last_move_direction: u8,
}

impl SwapperSnapshot {
    pub(crate) fn from_swapper(swapper: &Swapper) -> Self {
        SwapperSnapshot {
            grid_x: swapper.x,
            grid_y: swapper.y,
            state: match swapper.state {
                SwapperState::Idle => 0,
                SwapperState::Swapping => 1,
            },
            swap_factor: swapper.swap_factor(),
            move_lockout_countdown: swapper.move_lockout_countdown,
            last_move_direction: match swapper.last_move_direction {
                MoveDirection::None => 0,
                MoveDirection::Left => 1,
                MoveDirection::Right => 2,
                MoveDirection::Up => 3,
                MoveDirection::Down => 4,
            },
        }
    }
}

use crate::constants::PLAY_WIDTH;

/// Debug info for a grid cell (for swap debugging)
#[derive(Debug, Clone, Copy, Default)]
// Extracted on every frame although only the terminal frontend reads it. This is
// deliberate: the cost is two cell reads per frame, and a cfg gate would cross the
// inter-crate ABI (the debug profile of the frontend and of core may differ).
pub struct CellDebugInfo {
    pub cell_type: u8,  // 0=Empty, 1=Block, 2=Garbage
    pub phase: u8,      // BlockPhase/GarbagePhase/EmptyPhase as u8
    pub block_state: u8, // Block::state bits (if block)
}

#[derive(Debug, Clone, Default)]
pub struct FrameData {
    pub time_step: i32,
    pub play_offset_y: f32,
    pub swapper: SwapperSnapshot,
    pub blocks: Vec<BlockSnapshot>,
    pub garbage: Vec<GarbageSnapshot>,
    pub next_row: [i32; PLAY_WIDTH],  // Flavors of upcoming row (y=0)
    pub score: i32,
    pub game_over: bool,
    /// Per-game statistics
    pub stats: crate::stats::GameStats,
    // UI state for rendering
    pub ui: UIState,
    // Debug: cells at swapper position
    pub debug_left_cell: CellDebugInfo,
    pub debug_right_cell: CellDebugInfo,
    pub gameplay_events: Vec<crate::gameplay_event::GameplayEvent>,
    // Gravity impacts this tick: (y, height) for renderer-side level light flashes
    pub gravity_impacts: Vec<(i32, i32)>,
}

impl Default for SwapperSnapshot {
    fn default() -> Self {
        SwapperSnapshot {
            grid_x: 0,
            grid_y: 0,
            state: 0,
            swap_factor: 0.0,
            move_lockout_countdown: 0,
            last_move_direction: 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::compute_garbage_awaking_progress;
    use crate::garbage::GarbageState;

    #[test]
    fn awaking_progress_stays_zero_until_last_quarter() {
        let alarm = 100;
        let progress = compute_garbage_awaking_progress(GarbageState::Awaking, alarm, 87, 50);
        assert_eq!(progress, 0.0);
    }

    #[test]
    fn awaking_progress_ramps_only_during_reform_phase() {
        let alarm = 100;
        let progress = compute_garbage_awaking_progress(GarbageState::Awaking, alarm, 94, 50);
        assert!((progress - 0.5).abs() < 0.001);
    }
}
