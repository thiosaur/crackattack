use crate::constants::*;
use crate::block::{BlockArena, PopDirection};
use crate::garbage::{GarbageArena, GarbageId};
use crate::grid::Grid;
use crate::combo::ComboArena;
use crate::swapper::Swapper;
use crate::creep::Creep;
use crate::score::{ScoringMode, ScoringSystem};
use crate::random::Random;
use crate::input::InputSnapshot;
use crate::frame_data::FrameData;
use crate::gameplay_event::GameplayEvent;
use crate::garbage_generator::GarbageGenerator;

mod elimination;
mod extraction;
mod gravity;
mod updates;
mod grid_init;

#[cfg(test)]
mod tests;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GameStatus {
    Playing,
    Won,
    Lost,
}

/// Context for tracking garbage shattering cascade
#[derive(Debug, Default)]
pub struct ShatterContext {
    pub count: i32,              // Total cells being shattered
    pub top: i32,                // Highest row affected
    pub bottom: i32,             // Lowest row affected
    pub gray_shatter: bool,      // True if triggered by gray block elimination
    pub shattered_ids: Vec<GarbageId>,  // IDs already processed (avoid double-processing)
}

impl ShatterContext {
    pub fn new(gray_shatter: bool) -> Self {
        ShatterContext {
            count: 0,
            top: 0,
            bottom: PLAY_HEIGHT as i32,
            gray_shatter,
            shattered_ids: Vec::new(),
        }
    }
}

pub struct GameState {
    pub time_step: i32,
    /// Anchor of the DISPLAYED clock, set at the GO tick (end of the countdown).
    /// Matches the original 1.1.14: `time_step` advances DURING the countdown
    /// (creep scheduling, timestamps), but the original `Clock` is a separate
    /// counter that starts at GO. Here `clock_time = time_step - clock_base`.
    /// 0 means no countdown (immediate start).
    pub(crate) clock_base: i32,
    pub status: GameStatus,
    pub grid: Grid,
    pub blocks: BlockArena,
    pub garbage: GarbageArena,
    pub(crate) combos: ComboArena,
    pub(crate) swapper: Swapper,
    pub(crate) creep: Creep,
    pub score: ScoringSystem,  // read by tools/auto_qa (score regression invariant)
    pub(crate) rng: Random,
    // Garbage queue for outgoing garbage (sent to opponent)
    pub(crate) garbage_generator: GarbageGenerator,
    // Pending eliminations: (x, y, combo_id)
    pub(crate) elimination_requests: Vec<(i32, i32, Option<u16>)>,
    // Next row to appear (pre-generated for display)
    pub(crate) next_row: [i32; PLAY_WIDTH],
    // Pop direction cycling for awaking blocks
    pub(crate) last_pop_direction: PopDirection,
    // Anti-match tracking for creep row generation
    pub(crate) last_row_c: [i32; PLAY_WIDTH],
    pub(crate) second_to_last_row_c: [i32; PLAY_WIDTH],
    pub(crate) last_flavor_c: i32,
    pub(crate) second_to_last_flavor_c: i32,
    // Anti-match tracking for awaking blocks (separate tracking from creep rows)
    pub(crate) last_row_a: [i32; PLAY_WIDTH],
    pub(crate) second_to_last_row_a: [i32; PLAY_WIDTH],
    pub(crate) last_flavor_a: i32,
    pub(crate) second_to_last_flavor_a: i32,
    // Animation tracking
    pub(crate) awaking_count: i32,
    pub(crate) dying_count: i32,
    // Gravity impacts this tick: (y, height) for renderer-side level light flashes
    pub(crate) gravity_impacts: Vec<(i32, i32)>,
    // Countdown is managed by the session layer.
    // Sound events emitted during this tick
    pub(crate) gameplay_events: Vec<crate::gameplay_event::GameplayEvent>,
    /// Per-game statistics
    pub stats: crate::stats::GameStats,
    // Tick at which game ended (for celebration fade)
    pub(crate) finished_tick: Option<i32>,
    // Minimum score required to enter the high-score table in solo mode.
    // None = no win condition configured.
    pub ranking_entry_score: Option<i32>,
    // VS mode: garbage produced by combos goes to outgoing_garbage instead of self.
    // When false (solo), garbage is added to self.garbage_generator as before.
    pub vs_mode: bool,
    // Garbage destined for the opponent (drained by the VS session each tick).
    // Each element: (height, width, flavor).
    pub(crate) outgoing_garbage: Vec<(i32, i32, i32)>,
    // Reusable work buffers (avoid per-tick allocation)
    pub(crate) work_block_ids: Vec<crate::block::BlockId>,
    pub(crate) work_garbage_ids: Vec<crate::garbage::GarbageId>,
    pub(crate) work_elimination_requests: Vec<(i32, i32, Option<u16>)>,
    pub(crate) work_completed_combos: Vec<(crate::combo::ComboId, i32, i32, i32, i32, i32)>,
}

impl GameState {
    /// Create new game state with default Classic scoring.
    pub fn new(seed: u32) -> Self {
        Self::with_scoring(seed, ScoringMode::Classic)
    }

    /// Create new game state with specified scoring mode
    pub fn with_scoring(seed: u32, scoring_mode: ScoringMode) -> Self {
        let mut state = GameState {
            time_step: 1,
            status: GameStatus::Playing,
            grid: Grid::new(),
            blocks: BlockArena::new(),
            garbage: GarbageArena::new(),
            combos: ComboArena::new(),
            swapper: Swapper::new(),
            creep: Creep::new(),
            score: ScoringSystem::new(scoring_mode),
            rng: Random::new(seed),
            garbage_generator: GarbageGenerator::new(),
            elimination_requests: Vec::new(),
            next_row: [0; PLAY_WIDTH],
            last_pop_direction: PopDirection::Dir1,
            last_row_c: [0; PLAY_WIDTH],
            second_to_last_row_c: [0; PLAY_WIDTH],
            last_flavor_c: 0,
            second_to_last_flavor_c: 0,
            last_row_a: [0; PLAY_WIDTH],
            second_to_last_row_a: [0; PLAY_WIDTH],
            last_flavor_a: 0,
            second_to_last_flavor_a: 0,
            awaking_count: 0,
            dying_count: 0,
            gravity_impacts: Vec::new(),
            // Countdown is managed by the session layer.
            gameplay_events: Vec::new(),
            stats: crate::stats::GameStats::new(),
            finished_tick: None,
            ranking_entry_score: None,
            vs_mode: false,
            clock_base: 0,
            outgoing_garbage: Vec::new(),
            work_block_ids: Vec::with_capacity(64),
            work_garbage_ids: Vec::with_capacity(16),
            work_elimination_requests: Vec::with_capacity(32),
            work_completed_combos: Vec::with_capacity(8),
        };
        state.initialize_grid();
        state.spawn_incoming_row();


        state
    }

    /// End a block's combo involvement: decrement the old combo and clear combo_id.
    pub(crate) fn end_combo_involvement(&mut self, block_id: crate::block::BlockId) {
        if let Some(block) = self.blocks.get_mut(block_id) {
            if let Some(old_cid) = block.combo_id {
                let combo = self.combos.get_mut_including_dead(crate::combo::ComboId(old_cid));
                debug_assert!(combo.is_some(),
                    "end_combo_involvement: block {} has combo_id={} but combo not found",
                    block_id.0, old_cid);
                if let Some(combo) = combo {
                    combo.decrement_involvement();
                }
            }
            block.combo_id = None;
        }
    }

    /// Transfer a block's combo involvement: decrement old, set new, increment new.
    pub(crate) fn begin_combo_involvement(&mut self, block_id: crate::block::BlockId, combo_id: u16) {
        if let Some(block) = self.blocks.get_mut(block_id) {
            if let Some(old_cid) = block.combo_id {
                let old_combo = self.combos.get_mut_including_dead(crate::combo::ComboId(old_cid));
                debug_assert!(old_combo.is_some(),
                    "begin_combo_involvement: block {} has old combo_id={} but combo not found",
                    block_id.0, old_cid);
                if let Some(combo) = old_combo {
                    combo.decrement_involvement();
                }
            }
            block.combo_id = Some(combo_id);
            let new_combo = self.combos.get_mut_including_dead(crate::combo::ComboId(combo_id));
            debug_assert!(new_combo.is_some(),
                "begin_combo_involvement: target combo_id={} not found", combo_id);
            if let Some(combo) = new_combo {
                combo.increment_involvement();
            }
        }
    }

    /// Check if a flavor would create a 3-in-a-row (column or global sequence).
    pub(crate) fn would_create_triple(
        flavor: i32,
        x: usize,
        last_row: &[i32; PLAY_WIDTH],
        second_to_last_row: &[i32; PLAY_WIDTH],
        last_flavor: i32,
        second_to_last_flavor: i32,
    ) -> bool {
        let column = flavor == last_row[x] && last_row[x] == second_to_last_row[x];
        let global = flavor == last_flavor && last_flavor == second_to_last_flavor;
        column || global
    }

    /// Route garbage to opponent (VS mode) or to self (solo mode).
    pub fn send_garbage(&mut self, height: i32, width: i32, flavor: i32) {
        self.stats.record_garbage_sent(height as u32);
        if self.vs_mode {
            self.outgoing_garbage.push((height, width, flavor));
        } else {
            // Initial drop: average delay plus a random spread (deterministic, drawn from
            // the simulation RNG). Matches 1.1.14 (`GarbageGenerator::determineDropTime`).
            // A retry after a failed creation has no spread, like the original.
            self.garbage_generator.add_to_queue(
                height, width, flavor, self.time_step, Some(&mut self.rng),
            );
        }
    }

    /// Main game step
    pub fn step(&mut self, input: &InputSnapshot) -> FrameData {
        self.gameplay_events.clear();
        self.gravity_impacts.clear();
        self.outgoing_garbage.clear();

        if self.status != GameStatus::Playing {
            // Keep a local post-game clock running so the renderer can drive
            // win/lose celebration timing after gameplay has frozen.
            self.time_step += 1;
            return self.extract_frame_with_events();
        }

        self.time_step += 1;

        // Process input
        self.process_input(input);

        // Update swapper timers AFTER input
        self.swapper.time_step();

        // Process swap
        self.process_swap();

        // Update blocks
        self.update_blocks();

        // Update gravity
        self.update_gravity();

        // Update animation counts (needed for creep to check if animations are active)
        self.update_animation_counts();

        // Update creep (must run before elimination check)
        let has_active_animations = self.awaking_count > 0 || self.dying_count > 0;
        let safe_height_violated = self.grid.check_safe_height_violation();


        let (should_shift, lost) = self.creep.time_step(has_active_animations, safe_height_violated);

        if lost {
            // Solo win condition: enter the high-score table.
            // Win if the final score beats the current lowest ranked entry.
            // Strictly greater (matches 1.1.14, Score.cxx: `score > record[n].score`).
            // Winning means entering the table, so this must use the same comparison as
            // submit_score. Otherwise a tie with the last place would win without entering.
            self.status = if self.ranking_entry_score.is_some_and(|t| self.score.value() > t) {
                GameStatus::Won
            } else {
                GameStatus::Lost
            };
            self.finished_tick = Some(self.time_step);
            let won = self.status == GameStatus::Won;
            self.gameplay_events.push(GameplayEvent::GameOver { won });
        } else if should_shift
            && !self.shift_grid_up() {
                // Couldn't shift (top row occupied), retry next tick
                self.creep.retry_shift();
            }

        // Check eliminations
        self.check_eliminations();

        // Finalize combos
        // Computes latest_magnitude and resets magnitude for combos with eliminations this tick
        // Must happen after all eliminations for this tick are processed
        self.finalize_combos();

        // Process garbage queue
        // Use callback so garbage stays in queue if creation fails
        // We need to collect first because create_falling_garbage borrows self mutably
        let current_tick = self.time_step;
        let mut to_create = Vec::new();
        self.garbage_generator.time_step_with_callback(current_tick, |h, w, f| {
            to_create.push((h, w, f));
        });
        // Re-queue any that failed to create
        for (height, width, flavor) in to_create {
            if !self.create_falling_garbage(height, width, flavor) {
                // Creation failed — re-add to queue with retry delay
                self.garbage_generator.add_to_queue(
                    height, width, flavor,
                    self.time_step, None,
                );
            }
        }

        // Update garbage (shattering cleanup)
        // Shattering garbage runs AFTER garbage queue processing
        self.update_garbage();

        // Update top row tracking
        self.grid.update_top_rows(&self.garbage);

        // Update score - flush backlog to value
        self.score.flush_backlog();

        // Update game stats from this tick's events
        self.stats.process_events(&self.gameplay_events);

        // Validate invariants (debug builds only)
        #[cfg(debug_assertions)]
        self.validate_invariants();


        self.extract_frame_with_events()
    }

    /// Verify internal consistency at end of tick. Panics on corruption.
    #[cfg(debug_assertions)]
    fn validate_invariants(&self) {
        use crate::constants::*;

        // Grid ↔ Block arena: every block CellRef must point to an allocated block
        for x in 0..PLAY_WIDTH as i32 {
            for y in 0..PLAY_HEIGHT as i32 {
                let cell = self.grid.get(x, y);
                if cell.is_block() {
                    if let Some(id) = cell.block_id() {
                        assert!(self.blocks.get(id).is_some(),
                            "INVARIANT: Grid ({},{}) references non-existent block {:?}", x, y, id);
                    }
                }
                if cell.is_garbage() {
                    if let Some(id) = cell.garbage_id() {
                        assert!(self.garbage.get(id).is_some(),
                            "INVARIANT: Grid ({},{}) references non-existent garbage {:?}", x, y, id);
                    }
                }
            }
        }

        // No block with combo_id pointing to inactive combo
        for block in self.blocks.iter_active() {
            if let Some(cid) = block.combo_id {
                let combo = self.combos.get_including_dead(crate::combo::ComboId(cid));
                assert!(combo.is_some(),
                    "INVARIANT: Block {} at ({},{}) has stale combo_id={}",
                    block.id.0, block.grid_x, block.grid_y, cid);
            }
        }

        // No negative counters
        for combo in self.combos.iter_active() {
            assert!(combo.involvement_count >= 0,
                "INVARIANT: Combo {:?} has negative involvement_count={}",
                combo.id, combo.involvement_count);
        }
    }
}
