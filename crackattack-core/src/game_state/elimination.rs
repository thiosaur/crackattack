// Elimination pipeline - pattern matching, combo tracking, garbage shattering

use super::{GameState, ShatterContext};
use crate::constants::*;
use crate::block::{BlockId, BlockState, PopDirection};
use crate::garbage::{GarbageId, GarbageState};
use crate::grid::{CellRef, BlockPhase, GarbagePhase};
use crate::garbage_rules::{calculate_elimination_garbage, calculate_completion_garbage};
use crate::gameplay_event::{GameplayEvent, GarbageShatteringEvent, BlockAwakingEvent};

impl GameState {
    pub(super) fn check_eliminations(&mut self) {
        self.work_elimination_requests.clear();
        self.work_elimination_requests.append(&mut self.elimination_requests);
        let requests = std::mem::take(&mut self.work_elimination_requests);


        for &(x, y, combo_id) in &requests {
            self.check_elimination_at(x, y, combo_id);
        }

        let mut buf = requests;
        buf.clear();
        self.work_elimination_requests = buf;
    }

    fn check_elimination_at(&mut self, start_x: i32, start_y: i32, chain_combo_id: Option<u16>) {
        let cell = self.grid.get(start_x, start_y);
        if !cell.is_block() || !cell.is_static() {
            return;
        }

        let flavor = if let Some(id) = cell.block_id() {
            self.blocks.get(id).map(|b| b.flavor)
        } else {
            return;
        };

        let Some(flavor) = flavor else { return };

        // Check horizontal
        let mut h_matches = vec![(start_x, start_y)];
        for dx in [-1i32, 1] {
            let mut nx = start_x + dx;
            while self.grid.in_bounds(nx, start_y) {
                let nc = self.grid.get(nx, start_y);
                if nc.is_block() && nc.is_static() {
                    if let Some(id) = nc.block_id() {
                        if let Some(b) = self.blocks.get(id) {
                            if b.flavor == flavor {
                                h_matches.push((nx, start_y));
                                nx += dx;
                                continue;
                            }
                        }
                    }
                }
                break;
            }
        }

        // Check vertical
        // Row 0 is the preparation/nascent row — never include it in eliminations
        let mut v_matches = vec![(start_x, start_y)];
        for dy in [-1i32, 1] {
            let mut ny = start_y + dy;
            while self.grid.in_bounds(start_x, ny) && ny >= 1 {
                let nc = self.grid.get(start_x, ny);
                if nc.is_block() && nc.is_static() {
                    if let Some(id) = nc.block_id() {
                        if let Some(b) = self.blocks.get(id) {
                            if b.flavor == flavor {
                                v_matches.push((start_x, ny));
                                ny += dy;
                                continue;
                            }
                        }
                    }
                }
                break;
            }
        }

        // Collect matches
        let mut to_eliminate = Vec::new();
        let h_valid = h_matches.len() >= MIN_PATTERN_LENGTH as usize;
        let v_valid = v_matches.len() >= MIN_PATTERN_LENGTH as usize;

        if h_valid {
            to_eliminate.extend(h_matches.iter().cloned());
        }
        if v_valid {
            for m in &v_matches {
                if !to_eliminate.contains(m) {
                    to_eliminate.push(*m);
                }
            }
        }

        if to_eliminate.is_empty() {
            // When landing with a combo but NO match found:
            // only decrement if the block's current combo matches the passed combo.
            if let Some(cid) = chain_combo_id {
                let block_has_this_combo = cell.block_id()
                    .and_then(|id| self.blocks.get(id))
                    .map(|b| b.combo_id == Some(cid))
                    .unwrap_or(false);
                if block_has_this_combo {
                    if let Some(combo) = self.combos.get_mut_including_dead(crate::combo::ComboId(cid)) {
                        combo.decrement_involvement();
                    }
                }
                // Clear the block's combo_id only if it matches
                if let Some(id) = cell.block_id() {
                    if let Some(block) = self.blocks.get_mut(id) {
                        if block.combo_id == Some(cid) {
                            block.combo_id = None;
                        }
                    }
                }
            }
            return;
        }

        // Determine gray_shatter (colorless flavor = gray block)
        let gray_shatter = Self::is_colorless_flavor(flavor);

        // Get or create combo for this elimination
        let combo_id = chain_combo_id.or_else(|| {
            // Create new combo if this is not a chain reaction
            let result = self.combos.allocate(self.time_step).map(|c| c.0);
            if result.is_none() {
                // Degrade instead of panicking: the elimination proceeds WITHOUT a combo
                // (no bonus or chain counted) rather than crashing mid-game.
                // Deterministic: both lockstep peers make the same decision.
                // The message below is kept as a loud signal: the pool of 8 is too small.
                let active: Vec<_> = self.combos.iter_active()
                    .map(|c| format!("{:?}(inv={},ct={},ts={})", c.id, c.involvement_count, c.creation_time, c.time_stamp))
                    .collect();
                eprintln!("COMBO POOL EXHAUSTED at tick {} — elimination proceeds WITHOUT combo.\n  Active combos: {:?}",
                    self.time_step, active);
            }
            result
        });

        // If using a chain combo, resurrect it if it was deallocated
        if chain_combo_id.is_some() {
            if let Some(cid) = combo_id {
                self.combos.resurrect(crate::combo::ComboId(cid));
            }
        }

        // Mark blocks as Dying and associate with combo.
        // beginComboInvolvement: decrement old combo (if any), set new, increment new.
        for &(x, y) in &to_eliminate {
            if let Some(id) = self.grid.get(x, y).block_id() {
                // Get old combo_id before setting new one
                let old_combo_id = self.blocks.get(id).and_then(|b| b.combo_id);

                if let Some(block) = self.blocks.get_mut(id) {
                    block.physics_flags = BlockState::DYING;
                    block.animation_countdown = DYING_DELAY;
                    block.combo_id = combo_id;
                    // Random::angle() for visual rotation axis is skipped
                    // (visual-only effect, excluded for determinism).
                }
                self.grid.set(x, y, CellRef::block(id, BlockPhase::Dying));

                // Decrement old combo (if any), then increment new combo
                if let Some(old_cid) = old_combo_id {
                    if let Some(old_combo) = self.combos.get_mut_including_dead(crate::combo::ComboId(old_cid)) {
                        old_combo.decrement_involvement();
                    }
                }

                // Increment new combo involvement
                if let Some(cid) = combo_id {
                    if let Some(combo) = self.combos.get_mut_including_dead(crate::combo::ComboId(cid)) {
                        combo.increment_involvement();
                    }
                }
            }
        }

        // Emit sound event for block dying (once per elimination group)

        // Emit gameplay event with localized data
        {
            self.gameplay_events.push(crate::gameplay_event::GameplayEvent::EliminationStarted(
                crate::gameplay_event::EliminationStartedEvent {
                    combo_id: combo_id.map(|c| c as i32).unwrap_or(-1),
                    cells: to_eliminate.clone(),
                    flavor,
                    magnitude: to_eliminate.len() as i32,
                }
            ));
        }

        // Record elimination in combo (separate normal vs special magnitude tracking)
        if let Some(cid) = combo_id {
            let current_time = self.time_step;
            if let Some(combo) = self.combos.get_mut_including_dead(crate::combo::ComboId(cid)) {
                combo.record_elimination(to_eliminate.len() as i32, gray_shatter, current_time);
                combo.x = start_x;
                combo.y = start_y;
                combo.flavor = flavor;
                combo.last_trigger_cells = to_eliminate.clone();
            }
        }

        // Initialize shatter context
        let mut ctx = ShatterContext::new(gray_shatter);

        // Check adjacent garbage for horizontal pattern
        if h_valid {
            let x_left = h_matches.iter().map(|(x, _)| *x).min().unwrap();
            let x_right = h_matches.iter().map(|(x, _)| *x).max().unwrap();
            let y = start_y;

            // Left and right edges
            self.shatter_garbage(x_left - 1, y, None, &mut ctx);
            self.shatter_garbage(x_right + 1, y, None, &mut ctx);

            // Above and below each block in pattern
            for x in x_left..=x_right {
                self.shatter_garbage(x, y - 1, None, &mut ctx);
                self.shatter_garbage(x, y + 1, None, &mut ctx);
            }
        }

        // Check adjacent garbage for vertical pattern
        if v_valid {
            let y_bottom = v_matches.iter().map(|(_, y)| *y).min().unwrap();
            let y_top = v_matches.iter().map(|(_, y)| *y).max().unwrap();
            let x = start_x;

            // Below and above edges
            self.shatter_garbage(x, y_bottom - 1, None, &mut ctx);
            self.shatter_garbage(x, y_top + 1, None, &mut ctx);

            // Left and right of each block in pattern
            for y in y_bottom..=y_top {
                self.shatter_garbage(x - 1, y, None, &mut ctx);
                self.shatter_garbage(x + 1, y, None, &mut ctx);
            }
        }

        // Traverse the shattered area row by row so each garbage row can schedule its own pop delay and awaking timing in deterministic grid order.
        let combo_for_shatter = combo_id;

        if ctx.count > 0 {
            // Calculate delays
            let awaken_delay = INITIAL_POP_DELAY + FINAL_POP_DELAY
                + INTERNAL_POP_DELAY * (ctx.count - 1);
            let mut pop_delay = INITIAL_POP_DELAY;

            // Build a map of (garbage_id, x, width) for each shattered garbage
            // so we can iterate in grid order
            let shattered_info: Vec<(GarbageId, i32, i32, i32, i32)> = ctx.shattered_ids.iter()
                .filter_map(|&id| {
                    self.garbage.get(id).map(|g| (id, g.grid_x, g.grid_y, g.width, g.height))
                })
                .collect();

            // Iterate row by row from bottom to top
            for s_y in ctx.bottom..ctx.top {
                // For each garbage that occupies this row, process it
                // (iterate x from left to right by checking garbage positions)
                let mut s_x = 0i32;
                while s_x < PLAY_WIDTH as i32 {
                    // Find garbage at this position
                    if let Some(&(garbage_id, gx, _gy, gw, _gh)) = shattered_info.iter()
                        .find(|&&(_, gx, gy, gw, gh)| {
                            s_x >= gx && s_x < gx + gw && s_y >= gy && s_y < gy + gh
                        })
                    {
                        // Process this row of the garbage
                        // Skip to after this garbage
                        self.process_shattering_garbage_row(
                            garbage_id,
                            s_y,
                            &mut pop_delay,
                            awaken_delay,
                            combo_for_shatter,
                        );
                        s_x = gx + gw;  // Skip to after this garbage
                    } else {
                        s_x += 1;
                    }
                }
            }
        }

        // Update score: gray and normal magnitudes are tracked separately
        let count = to_eliminate.len() as i32;
        let (magnitude, special_magnitude) = if gray_shatter {
            (0, count)
        } else {
            (count, 0)
        };
        let cid = combo_id.map(|id| id as i32).unwrap_or(-1);

        // Emit MULTIPLIER first, then ELIMINATION — the bonus formula reads baseScoreThisStep
        // which on_elimination updates; reversing the order would produce wrong scores.

        // If this was a chain, report the multiplier increase FIRST (before elimination)
        // (record_elimination already incremented combo.multiplier for chains)
        if chain_combo_id.is_some() {
            if let Some(combo) = combo_id.and_then(|id| self.combos.get_including_dead(crate::combo::ComboId(id))) {
                self.score.on_multiplier(cid, combo.multiplier);
            }
        }

        // Then call on_elimination
        self.score.on_elimination(cid, magnitude, special_magnitude, self.time_step);

        // Queue garbage based on elimination
        let garbage_to_send = calculate_elimination_garbage(magnitude, special_magnitude);
        for g in garbage_to_send {
            self.send_garbage(g.height, g.width, g.flavor);
        }
    }

    /// Check if block flavor is colorless (gray) — only BF_GRAY (5) is colorless
    fn is_colorless_flavor(flavor: i32) -> bool {
        flavor == crate::constants::BF_GRAY
    }

    /// Finalize combos at end of time step
    ///
    /// This computes latest_magnitude for combos that had eliminations this tick,
    /// then resets their magnitude for the next tick.
    /// Also handles combo completion: generates garbage and deallocates completed combos.
    pub(super) fn finalize_combos(&mut self) {
        let current_time = self.time_step;

        // First pass: finalize step and collect completed combos
        // Collect (id, multiplier, magnitude, x, y, flavor) for sign creation
        self.work_completed_combos.clear();
        let mut completed_combos = std::mem::take(&mut self.work_completed_combos);

        for combo in self.combos.iter_active_mut() {
            combo.finalize_step(current_time);

            if combo.involvement_count == 0 {
                completed_combos.push((
                    combo.id,
                    combo.multiplier,
                    combo.blocks_eliminated,
                    combo.x,
                    combo.y,
                    combo.flavor,
                ));
            }
        }

        // Process completed combos
        for &(combo_id, multiplier, _magnitude, _cx, _cy, flavor) in &completed_combos {
            // Generate completion garbage
            if let Some(g) = calculate_completion_garbage(multiplier) {
                self.send_garbage(g.height, g.width, g.flavor);
            }

            // Emit ChainProgressed event for chains ×2+
            if multiplier >= 2 {
                // Retrieve trigger cells from the combo (stored during record_elimination)
                let trigger_cells = self.combos.get_including_dead(combo_id)
                    .map(|c| c.last_trigger_cells.clone())
                    .unwrap_or_default();
                self.gameplay_events.push(crate::gameplay_event::GameplayEvent::ChainProgressed(
                    crate::gameplay_event::ChainProgressedEvent {
                        combo_id: combo_id.0 as i32,
                        chain_level: multiplier,
                        trigger_cells,
                        flavor,
                    }
                ));
            }
            // Report combo completion to score system
            self.score.on_combo_completed(combo_id.0 as i32);

            // GUARD: no block should still reference this combo at deallocation
            #[cfg(debug_assertions)]
            {
                let stale_blocks: Vec<_> = self.blocks.iter_active()
                    .filter(|b| b.combo_id == Some(combo_id.0))
                    .map(|b| format!("blk{}@({},{})", b.id.0, b.grid_x, b.grid_y))
                    .collect();
                assert!(stale_blocks.is_empty(),
                    "STALE REFS at dealloc: combo {:?} still referenced by blocks {:?} at tick {}",
                    combo_id, stale_blocks, current_time);
            }

            // Deallocate the combo
            self.combos.deallocate(combo_id);
        }
        completed_combos.clear();
        self.work_completed_combos = completed_combos;

        // INVARIANT: for every active combo, involvement_count must equal the
        // actual number of active blocks carrying that combo_id.
        #[cfg(debug_assertions)]
        for combo in self.combos.iter_active() {
            let real_count = self.blocks.iter_active()
                .filter(|b| b.combo_id == Some(combo.id.0))
                .count() as i32;
            assert_eq!(combo.involvement_count, real_count,
                "INVOLVEMENT MISMATCH at tick {}: combo {:?} has involvement={} but {} blocks \
                 reference it (creation_time={}, time_stamp={})",
                current_time, combo.id, combo.involvement_count, real_count,
                combo.creation_time, combo.time_stamp);
        }
    }

    /// Recursive garbage shattering - marks garbage and cascades to neighbors
    /// Returns true if this garbage was shattered
    fn shatter_garbage(&mut self, x: i32, y: i32, due_to: Option<GarbageId>, ctx: &mut ShatterContext) -> bool {
        // Get garbage at this position
        let cell = self.grid.get(x, y);
        let Some(garbage_id) = cell.garbage_id() else { return false };

        // Avoid double-processing
        if ctx.shattered_ids.contains(&garbage_id) {
            return false;
        }

        // Get garbage data
        let garbage = match self.garbage.get(garbage_id) {
            Some(g) => g,
            None => return false,
        };

        // Must be static to shatter
        if garbage.physics_state != GarbageState::Static {
            return false;
        }

        // Check consider_shattering logic
        let due_to_garbage = due_to.and_then(|id| self.garbage.get(id));
        if !garbage.consider_shattering(due_to_garbage) {
            return false;
        }

        // Save dimensions before modifying
        let gx = garbage.grid_x;
        let gy = garbage.grid_y;
        let gw = garbage.width;
        let gh = garbage.height;

        // Update shatter bounds
        let top = gy + gh;
        if top > ctx.top {
            ctx.top = top.min(PLAY_HEIGHT as i32);
        }
        if gy < ctx.bottom {
            ctx.bottom = gy.max(0);
        }
        ctx.count += gw * gh;
        ctx.shattered_ids.push(garbage_id);

        // Mark all cells of this garbage as Shattering
        for dy in 0..gh {
            for dx in 0..gw {
                self.grid.set_garbage_phase(gx + dx, gy + dy, GarbagePhase::Shattering);
            }
        }

        // Mark garbage state
        if let Some(g) = self.garbage.get_mut(garbage_id) {
            g.physics_state = GarbageState::Shattering;
        }

        self.gameplay_events.push(GameplayEvent::GarbageShattering(GarbageShatteringEvent {
            grid_x: gx, grid_y: gy, width: gw, height: gh,
        }));

        // Recursive cascade to neighbors
        // Left edge
        if gx > 0 {
            for h in 0..gh {
                if self.grid.is_static_garbage(gx - 1, gy + h) {
                    self.shatter_garbage(gx - 1, gy + h, Some(garbage_id), ctx);
                }
            }
        }

        // Right edge
        if gx + gw < PLAY_WIDTH as i32 {
            for h in 0..gh {
                if self.grid.is_static_garbage(gx + gw, gy + h) {
                    self.shatter_garbage(gx + gw, gy + h, Some(garbage_id), ctx);
                }
            }
        }

        // Bottom edge
        if gy > 1 {
            for w in 0..gw {
                if self.grid.is_static_garbage(gx + w, gy - 1) {
                    self.shatter_garbage(gx + w, gy - 1, Some(garbage_id), ctx);
                }
            }
        }

        // Top edge
        if gy + gh < PLAY_HEIGHT as i32 {
            for w in 0..gw {
                if self.grid.is_static_garbage(gx + w, gy + gh) {
                    self.shatter_garbage(gx + w, gy + gh, Some(garbage_id), ctx);
                }
            }
        }

        true
    }

    /// Generate next pop direction (cycles through Up, Right, Down, Left)
    fn next_pop_direction(&mut self) -> PopDirection {
        self.last_pop_direction = self.last_pop_direction.next();
        self.last_pop_direction
    }

    /// Spawn an awaking block (from garbage shattering) with anti-match logic
    fn spawn_awaking_block(
        &mut self,
        x: i32,
        y: i32,
        pop_delay: i32,
        awake_delay: i32,
        combo_id: Option<u16>,
        pop_color: i32,
    ) -> Option<BlockId> {
        // do-while loop with separate anti-match tracking
        let flavor = loop {
            let f = self.rng.number(NUM_NORMAL_FLAVORS);
            if !Self::would_create_triple(
                f, x as usize,
                &self.last_row_a, &self.second_to_last_row_a,
                self.last_flavor_a, self.second_to_last_flavor_a,
            ) {
                break f;
            }
        };

        // Update awaking block tracking
        self.second_to_last_row_a[x as usize] = self.last_row_a[x as usize];
        self.last_row_a[x as usize] = flavor;
        self.second_to_last_flavor_a = self.last_flavor_a;
        self.last_flavor_a = flavor;

        let pop_direction = self.next_pop_direction();

        let id = self.blocks.allocate_awaking(
            x, y, flavor,
            pop_delay, awake_delay,
            pop_direction, pop_color, combo_id,
            self.time_step,  // For absolute timestamps
        )?;

        // Awaking blocks are involved in the combo until they die or land
        if let Some(cid) = combo_id {
            let combo = self.combos.get_mut_including_dead(crate::combo::ComboId(cid));
            debug_assert!(combo.is_some(),
                "Awaking block has combo_id={} but combo not found", cid);
            if let Some(combo) = combo {
                combo.increment_involvement();
            }
        }

        self.grid.set(x, y, CellRef::block(id, BlockPhase::Awaking));

        self.gameplay_events.push(GameplayEvent::BlockAwaking(BlockAwakingEvent {
            block_id: id.0 as i32, grid_x: x, grid_y: y, flavor,
        }));

        Some(id)
    }

    /// Process ONE ROW of a shattering garbage at a specific y coordinate.
    /// Called once per row, not once per garbage.
    /// Returns true if this was the garbage's top row (and alarm was set)
    fn process_shattering_garbage_row(
        &mut self,
        garbage_id: GarbageId,
        s_y: i32,  // The Y coordinate of the row to process
        pop_delay: &mut i32,
        awake_delay: i32,
        combo_id: Option<u16>,
    ) -> bool {
        // Get garbage info
        let (gx, gy, gw, gh, gflavor) = {
            let garbage = match self.garbage.get(garbage_id) {
                Some(g) => g,
                None => return false,
            };
            (garbage.grid_x, garbage.grid_y, garbage.width, garbage.height, garbage.flavor)
        };

        // Calculate which row index within the garbage this is
        let row = s_y - gy;
        if row < 0 || row >= gh {
            return false;  // Not part of this garbage
        }

        // Check if should create garbage instead of blocks
        // Full-width garbage on ODD rows (row index 1, 3, 5...) has
        // 1/GARBAGE_TO_GARBAGE_SHATTER chance to become garbage instead of blocks
        let is_full_width = gw == PLAY_WIDTH as i32;
        let is_odd_row = row % 2 == 1;
        let make_garbage = if is_full_width && is_odd_row {
            self.rng.chance_in(GARBAGE_TO_GARBAGE_SHATTER)
        } else {
            false
        };

        if make_garbage {
            // Create awaking garbage (1 row, full width)
            // Awaking garbage is always full-width with normal flavor
            use crate::garbage::GARBAGE_FLAVOR_NORMAL;
            self.spawn_awaking_garbage(
                gx,
                s_y,
                PLAY_WIDTH as i32,  // Force full width
                1,  // height = 1
                GARBAGE_FLAVOR_NORMAL,          // Force normal flavor
                *pop_delay,
                awake_delay,
                combo_id,
            );
            *pop_delay += INTERNAL_POP_DELAY * gw;  // Advance delay for all cells
        } else {
            // Create awaking blocks for each cell in this row
            for col in 0..gw {
                self.spawn_awaking_block(
                    gx + col,
                    s_y,
                    *pop_delay,
                    awake_delay,
                    combo_id,
                    gflavor,
                );
                *pop_delay += INTERNAL_POP_DELAY;
            }
        }

        // Set alarm when processing the TOP row of the garbage
        let is_top_row = s_y + 1 == gy + gh;
        if is_top_row {
            if let Some(g) = self.garbage.get_mut(garbage_id) {
                g.animation_countdown = self.time_step + SHATTER_TIME;
            }
        }

        is_top_row
    }

    /// Spawn awaking garbage (from garbage shattering)
    fn spawn_awaking_garbage(
        &mut self,
        x: i32,
        y: i32,
        width: i32,
        height: i32,
        flavor: i32,
        pop_delay: i32,
        awake_delay: i32,
        combo_id: Option<u16>,
    ) -> Option<GarbageId> {
        let pop_dir = self.next_pop_direction();
        let id = self.garbage.allocate(x, y, width, height, flavor)?;

        // Set up as awaking
        if let Some(g) = self.garbage.get_mut(id) {
            g.physics_state = GarbageState::Awaking;
            g.animation_countdown = self.time_step + awake_delay;
            g.pop_countdown = self.time_step + pop_delay;
            g.sections_popped = 0;
            g.pop_direction = pop_dir;
            g.combo_id = combo_id;
        }

        // Set grid cells
        for h in 0..height {
            for w in 0..width {
                self.grid.set(x + w, y + h, CellRef::garbage(id, GarbagePhase::Awaking));
            }
        }

        Some(id)
    }
}
