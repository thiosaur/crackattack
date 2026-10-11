// State hashing for determinism verification. Each sub-hash isolates a system for debugging.

use crate::game_state::GameState;
use crate::constants::{PLAY_WIDTH, PLAY_HEIGHT};

const FNV_OFFSET_BASIS: u64 = 14695981039346656037; // 0xcbf29ce484222325
const FNV_PRIME: u64 = 1099511628211;              // 0x100000001b3

struct FnvHasher {
    state: u64,
}

impl FnvHasher {
    fn new() -> Self {
        FnvHasher { state: FNV_OFFSET_BASIS }
    }

    fn write(&mut self, b: u8) {
        self.state ^= b as u64;
        self.state = self.state.wrapping_mul(FNV_PRIME);
    }

    fn write16(&mut self, v: u16) {
        self.write((v & 0xFF) as u8);
        self.write(((v >> 8) & 0xFF) as u8);
    }

    fn write32(&mut self, v: u32) {
        self.write((v & 0xFF) as u8);
        self.write(((v >> 8) & 0xFF) as u8);
        self.write(((v >> 16) & 0xFF) as u8);
        self.write(((v >> 24) & 0xFF) as u8);
    }

    fn write64(&mut self, v: u64) {
        self.write32((v & 0xFFFFFFFF) as u32);
        self.write32(((v >> 32) & 0xFFFFFFFF) as u32);
    }

    fn write_bool(&mut self, b: bool) {
        self.write(if b { 1 } else { 0 });
    }

    fn finish(&self) -> u64 {
        self.state
    }
}

/// Sub-fingerprints for each game system
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fingerprint {
    pub tick: i32,
    pub rng_state: u32,
    pub hash_grid: u64,
    pub hash_blocks: u64,
    pub hash_garbage: u64,
    pub hash_swapper: u64,
    pub hash_score: u64,
    pub block_count: usize,
    pub garbage_count: usize,
    /// Debug: top_effective_row for loss detection analysis
    pub top_effective_row: i32,
    /// Combined hash of all systems
    pub combined: u64,
}

impl Fingerprint {
    /// Compute fingerprint from game state
    pub fn from_state(state: &GameState) -> Self {
        let tick = state.time_step;
        let rng_state = state.rng.state();

        let hash_grid = hash_grid(&state.grid);
        let hash_blocks = hash_blocks(&state.blocks);
        let hash_garbage = hash_garbage(&state.garbage);
        let hash_swapper = hash_swapper(&state.swapper);
        let hash_score = hash_score(&state.score, tick);

        let block_count = state.blocks.count_active();
        let garbage_count = state.garbage.count_active();
        let top_effective_row = state.grid.top_effective_row;

        // Combined hash
        let mut hasher = FnvHasher::new();
        hasher.write32(tick as u32);
        hasher.write32(rng_state);
        hasher.write64(hash_grid);
        hasher.write64(hash_blocks);
        hasher.write64(hash_garbage);
        hasher.write64(hash_swapper);
        hasher.write64(hash_score);
        let combined = hasher.finish();

        Fingerprint {
            tick,
            rng_state,
            hash_grid,
            hash_blocks,
            hash_garbage,
            hash_swapper,
            hash_score,
            block_count,
            garbage_count,
            top_effective_row,
            combined,
        }
    }


}

// The per-system sub-hashes must stay: `combined` is computed from them, which keeps
// the parity snapshots stable. The parity tools have their own implementation of the format.

use crate::grid::Grid;

fn hash_grid(grid: &Grid) -> u64 {
    let mut hasher = FnvHasher::new();

    // Hash each cell in order (x then y)
    for x in 0..PLAY_WIDTH {
        for y in 0..PLAY_HEIGHT {
            let cell = grid.get(x as i32, y as i32);
            hasher.write16(cell.id as u16);
            hasher.write(cell.cell_type as u8);
            // Phase is stored as u8
            hasher.write(cell.phase);
        }
    }

    hasher.write32(grid.top_occupied_row as u32);
    hasher.write32(grid.top_effective_row as u32);

    hasher.finish()
}

use crate::block::BlockArena;

fn hash_blocks(blocks: &BlockArena) -> u64 {
    let mut hasher = FnvHasher::new();

    // Collect active blocks and sort by ID for stable order
    let mut active: Vec<_> = blocks.iter_active().collect();
    active.sort_by_key(|b| b.id.0);

    for block in active {
        hasher.write16(block.id.0);
        hasher.write32(block.flavor as u32);
        hasher.write32(block.grid_x as u32);
        hasher.write32(block.grid_y as u32);
        hasher.write32(block.sub_grid_y as u32);
        hasher.write(block.physics_flags.bits());
        hasher.write32(block.animation_countdown as u32);
        // fall_delay_countdown excluded from hash — ephemeral state
        hasher.write32(block.pop_countdown as u32);
        hasher.write(block.pop_direction as u8);
        hasher.write32(block.pop_color as u32);
        // combo_id (use 0xFFFF if none, 0x0001 if present)
        let combo_id: u16 = if block.combo_id.is_some() { 0x0001 } else { 0xFFFF };
        hasher.write16(combo_id);
        // Skip axis_x/axis_y (floats, derived from RNG)
    }

    hasher.finish()
}

use crate::garbage::GarbageArena;

fn hash_garbage(garbage: &GarbageArena) -> u64 {
    let mut hasher = FnvHasher::new();

    // Collect active garbage and sort by ID for stable order
    let mut active: Vec<_> = garbage.iter_active().collect();
    active.sort_by_key(|g| g.id.0);

    for g in active {
        hasher.write16(g.id.0);
        hasher.write32(g.flavor as u32);
        hasher.write32(g.grid_x as u32);
        hasher.write32(g.grid_y as u32);
        hasher.write32(g.width as u32);
        hasher.write32(g.height as u32);
        hasher.write32(g.sub_grid_y as u32);
        hasher.write(g.physics_state as u8);
        hasher.write32(g.animation_countdown as u32);
        // fall_delay_countdown excluded from hash — ephemeral state
        hasher.write32(g.pop_countdown as u32);
        hasher.write(g.pop_direction as u8);
        hasher.write32(g.pop_color as u32);
        hasher.write32(g.sections_popped as u32);
        hasher.write_bool(g.initial_fall);
        // combo_id — excluded from hash, use 0
        hasher.write16(0);
    }

    hasher.finish()
}

use crate::swapper::Swapper;

fn hash_swapper(swapper: &Swapper) -> u64 {
    let mut hasher = FnvHasher::new();

    hasher.write32(swapper.x as u32);
    hasher.write32(swapper.y as u32);
    hasher.write(swapper.state as u8);
    hasher.write32(swapper.swap_countdown as u32);
    hasher.write32(swapper.move_lockout_countdown as u32);
    // queued_swap excluded: intra-tick transient (set by input, consumed next tick).
    // If the rest of the state matches, queued_swap is necessarily identical.

    hasher.finish()
}

use crate::score::ScoringSystem;

fn hash_score(score: &ScoringSystem, _time_step: i32) -> u64 {
    let mut hasher = FnvHasher::new();

    // Hash (score + backlog) as total earned
    let total_earned = score.value() + score.backlog();
    hasher.write32(total_earned as u32);

    // Combo multiplier: always 1 in Classic scoring
    hasher.write32(1u32);

    hasher.finish()
}

