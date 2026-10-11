//! Gameplay events emitted by the core per tick.
//!
//! The renderer consumes these to trigger visual effects and audio.
//! The core says WHAT happened; the renderer decides HOW it looks and sounds.
//! Events are transient — emitted once at the tick they occur, cleared next tick.
//! Single event channel for both visual and audio effects.

/// Block just finished falling and landed on a surface.
#[derive(Debug, Clone)]
pub struct BlockLandedEvent {
    pub block_id: i32,
    pub grid_x: i32,
    pub grid_y: i32,
    pub flavor: i32,
    /// Some(combo_id) if this block fell as a result of an elimination cascade.
    pub combo_id: Option<i32>,
}

/// A group of blocks was eliminated (combo detected, blocks entering DYING state).
#[derive(Debug, Clone)]
pub struct EliminationStartedEvent {
    pub combo_id: i32,
    pub cells: Vec<(i32, i32)>,
    pub flavor: i32,
    pub magnitude: i32,
}

/// A chain progressed to a new level (×2, ×3, ×4...).
/// Emitted when a combo enriches an existing chain, at the moment
/// the trigger cells are still known.
#[derive(Debug, Clone)]
pub struct ChainProgressedEvent {
    pub combo_id: i32,
    pub chain_level: i32,
    pub trigger_cells: Vec<(i32, i32)>,
    pub flavor: i32,
}

/// Garbage piece landed after falling.
#[derive(Debug, Clone)]
pub struct GarbageLandedEvent {
    pub grid_x: i32,
    pub grid_y: i32,
    pub width: i32,
    pub height: i32,
}

/// Garbage started shattering (breaking into blocks).
#[derive(Debug, Clone)]
pub struct GarbageShatteringEvent {
    pub grid_x: i32,
    pub grid_y: i32,
    pub width: i32,
    pub height: i32,
}

/// Block awoke from shattered garbage.
#[derive(Debug, Clone)]
pub struct BlockAwakingEvent {
    pub block_id: i32,
    pub grid_x: i32,
    pub grid_y: i32,
    pub flavor: i32,
}

#[derive(Debug, Clone)]
pub enum GameplayEvent {
    BlockLanded(BlockLandedEvent),
    EliminationStarted(EliminationStartedEvent),
    ChainProgressed(ChainProgressedEvent),
    GarbageLanded(GarbageLandedEvent),
    GarbageShattering(GarbageShatteringEvent),
    BlockAwaking(BlockAwakingEvent),
    /// Player initiated a swap.
    SwapPerformed,
    /// A garbage piece was dropped from the top of the screen.
    GarbageDropped { width: i32, height: i32 },
    CreepShift,
    GameOver { won: bool },
    CountdownTick(i32),
    CountdownGo,
}
