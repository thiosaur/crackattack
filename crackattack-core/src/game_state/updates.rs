// Game state updates - input, swap, blocks, garbage, creep shift

use super::GameState;
use crate::constants::*;
use crate::block::{BlockId, BlockState};
use crate::garbage::{GarbageId, GarbageState};
use crate::grid::{CellRef, BlockPhase, GarbagePhase};
use crate::input::{InputSnapshot, INPUT_SWAP, INPUT_DIRECTIONS};
use crate::gameplay_event::GameplayEvent;

impl GameState {
    pub(crate) fn process_input(&mut self, input: &InputSnapshot) {
        // Use pressed_this_frame for movement when state is 0.
        // KeyDown+KeyUp can both arrive between ticks (input is event-driven);
        // if release happens before tick, state=0 but pressed_this_frame has the press.
        let mut move_cmd = input.move_cmd();
        let pressed_dirs = input.pressed_this_frame & INPUT_DIRECTIONS;
        if move_cmd == 0 && pressed_dirs != 0 {
            // Key was pressed and released between ticks - use the captured press
            // BUT only if button_down_move doesn't already match (prevents double-move in same frame)
            if (self.swapper.button_down_move as u8) & pressed_dirs == 0 {
                move_cmd = pressed_dirs as i32;
            }
        }

        self.swapper.process_move_cmd(move_cmd);

        // Use was_pressed for edge detection
        if input.was_pressed(INPUT_SWAP) {
            self.swapper.request_swap();
        }

        // Track advance button state for creep speed
        self.creep.set_advance(input.advance());
    }

    pub(crate) fn process_swap(&mut self) {
        if self.swapper.queued_swap {
            let x = self.swapper.x;
            let y = self.swapper.y;

            // Validate swap: static block always OK, static empty requires
            // canSwapAt (no falling below, no hanging above), at least one side must have a block.
            let left_ok = self.grid.is_static_block(x, y)
                || (self.grid.is_static_empty(x, y) && self.grid.can_swap_at(x, y));
            let right_ok = self.grid.is_static_block(x + 1, y)
                || (self.grid.is_static_empty(x + 1, y) && self.grid.can_swap_at(x + 1, y));
            let has_block = self.grid.is_static_block(x, y) || self.grid.is_static_block(x + 1, y);

            if left_ok && right_ok && has_block {
                self.gameplay_events.push(crate::gameplay_event::GameplayEvent::SwapPerformed);
                // Get cells and block IDs before starting swap
                let left_cell = self.grid.get(x, y);
                let right_cell = self.grid.get(x + 1, y);
                let left_block_id = left_cell.block_id();
                let right_block_id = right_cell.block_id();

                // Start swap with block tracking (for notifyLanding combo propagation)
                self.swapper.start_swap_with_blocks(left_block_id, right_block_id);

                if let Some(id) = left_block_id {
                    if let Some(block) = self.blocks.get_mut(id) {
                        block.physics_flags = BlockState::SWAPPING | BlockState::SWAP_DIRECTION;  // Moving RIGHT
                    }
                    self.grid.set(x, y, CellRef::block(id, BlockPhase::Swapping));
                } else {
                    self.grid.set_empty_immutable(x, y);
                }

                if let Some(id) = right_block_id {
                    if let Some(block) = self.blocks.get_mut(id) {
                        block.physics_flags = BlockState::SWAPPING;  // Moving LEFT (no direction flag)
                    }
                    self.grid.set(x + 1, y, CellRef::block(id, BlockPhase::Swapping));
                } else {
                    self.grid.set_empty_immutable(x + 1, y);
                }
            } else {
                self.swapper.queued_swap = false;
            }
        }

        if self.swapper.is_swap_complete() {
            // Use saved swap position, not current cursor position (saved at swap start)
            let x = self.swapper.swap_x;
            let y = self.swapper.swap_y;

            let left_cell = self.grid.get(x, y);
            let right_cell = self.grid.get(x + 1, y);

            // Swap positions and end any combo involvement from notifyLanding.
            // When swap finalizes, blocks get new combo from elimination check,
            // so any combo assigned via notifyLanding should be ended.
            if let Some(id) = left_cell.block_id() {
                self.end_combo_involvement(id);
                if let Some(block) = self.blocks.get_mut(id) {
                    block.grid_x = x + 1;
                    block.physics_flags = BlockState::STATIC;
                }
            }
            if let Some(id) = right_cell.block_id() {
                self.end_combo_involvement(id);
                if let Some(block) = self.blocks.get_mut(id) {
                    block.grid_x = x;
                    block.physics_flags = BlockState::STATIC;
                }
            }

            let new_left = if let Some(id) = right_cell.block_id() {
                CellRef::block(id, BlockPhase::Static)
            } else {
                CellRef::empty()
            };
            let new_right = if let Some(id) = left_cell.block_id() {
                CellRef::block(id, BlockPhase::Static)
            } else {
                CellRef::empty()
            };

            self.grid.set(x, y, new_left);
            self.grid.set(x + 1, y, new_right);

            // Create combo only if BOTH blocks are moving; both elimination checks share the same combo
            let has_left_block = left_cell.is_block();
            let has_right_block = right_cell.is_block();
            let shared_combo = if has_left_block && has_right_block {
                self.combos.allocate(self.time_step).map(|c| c.0)
            } else {
                None
            };

            // Request elimination check for both positions with shared combo
            if has_left_block {
                self.elimination_requests.push((x + 1, y, shared_combo));  // left block moved to x+1
            }
            if has_right_block {
                self.elimination_requests.push((x, y, shared_combo));  // right block moved to x
            }

            self.swapper.finish_swap();
        }
    }

    // Block and garbage updates are separate passes; they produce identical results because neither reads state modified by the other.
    pub(crate) fn update_blocks(&mut self) {
        self.work_block_ids.clear();
        self.work_block_ids.extend(self.blocks.iter_active().map(|b| b.id));
        // take() moves the vec out of self so we can iterate and mutate self
        // put it back at the end to preserve capacity
        let mut block_ids = std::mem::take(&mut self.work_block_ids);

        // Collect blocks to remove (include combo_id to pass to blocks above when they start falling)
        let mut to_remove: Vec<(BlockId, i32, i32, Option<u16>)> = Vec::new();
        let mut to_awaken: Vec<(BlockId, i32, i32)> = Vec::new();
        // Collect dying blocks that just started dying (need pop_countdown from combo)
        let mut need_pop_countdown: Vec<(BlockId, Option<u16>)> = Vec::new();

        let current_time = self.time_step;
        for &id in &block_ids {
            if let Some(block) = self.blocks.get_mut(id) {
                // Dying blocks: animation_countdown is a countdown (decremented each frame).
                // Awaking blocks: animation_countdown is an absolute timestamp (not decremented).
                if block.is_dying() && block.animation_countdown > 0 {
                    block.animation_countdown -= 1;
                }

                // When block just started dying, grab elimination magnitude from combo for spark count
                if block.is_dying() && block.animation_countdown == DYING_DELAY - 1 {
                    need_pop_countdown.push((id, block.combo_id));
                }

                // Dying blocks — countdown reached 0
                if block.is_dying() && block.animation_countdown <= 0 {
                    to_remove.push((id, block.grid_x, block.grid_y, block.combo_id));
                }

                // Awaking blocks: pop_countdown triggers visual effect, animation_countdown triggers transition to static
                if block.is_awaking() {
                    // Pop effect
                    if block.pop_countdown > 0 && current_time >= block.pop_countdown {
                        block.pop_countdown = 0;  // Consume the countdown
                        // Visual effect would be emitted here (sound, animation)
                    }
                    // Transition to static
                    if block.animation_countdown > 0 && current_time >= block.animation_countdown {
                        to_awaken.push((id, block.grid_x, block.grid_y));
                    }
                }
            }
        }

        // Set pop_countdown from combo's latest_magnitude (must be done after block iteration)
        for (id, combo_id) in need_pop_countdown {
            if let Some(cid) = combo_id {
                let latest_mag = self.combos.get_including_dead(crate::combo::ComboId(cid))
                    .map(|c| c.latest_magnitude)
                    .unwrap_or(0);
                if let Some(block) = self.blocks.get_mut(id) {
                    block.pop_countdown = latest_mag;
                }
            }
        }

        // Remove dying blocks and trigger gravity on blocks above
        for (id, x, y, combo_id) in to_remove {
            // Decrement combo involvement before removing
            if let Some(cid) = combo_id {
                let combo = self.combos.get_mut_including_dead(crate::combo::ComboId(cid));
                debug_assert!(combo.is_some(),
                    "Dying block has combo_id={} but combo not found at tick {}", cid, self.time_step);
                if let Some(combo) = combo {
                    combo.decrement_involvement();
                }
            }

            self.remove_block(id);

            // Propagate fall upward WITH the dying block's combo.
            // This is how chains propagate: blocks above inherit the combo.
            self.propagate_fall_up_with_combo(x, y + 1, false, combo_id);

            self.elimination_requests.push((x, y + 1, None));
        }

        // Awaken blocks - transition to Static or Falling
        for (id, x, y) in to_awaken {
            // Check if should start falling (empty space below)
            let will_fall = self.grid.is_static_empty(x, y - 1);

            if let Some(block) = self.blocks.get_mut(id) {
                let combo_id = block.combo_id;

                if will_fall {
                    // Start falling - keep combo_id for when it lands
                    block.physics_flags = BlockState::FALLING;
                    block.sub_grid_y = 0;
                    self.grid.set(x, y, CellRef::block(id, BlockPhase::Falling));
                } else {
                    // Land and become static - propagate combo_id for chain reactions
                    block.physics_flags = BlockState::STATIC;
                    // Awaking blocks from garbage shattering CAN have combo_id
                    // (passed via spawn_awaking_block). Clear it and decrement
                    // involvement so the combo can complete properly.
                    if let Some(cid) = block.combo_id {
                        if let Some(combo) = self.combos.get_mut_including_dead(crate::combo::ComboId(cid)) {
                            combo.decrement_involvement();
                        }
                    }
                    block.combo_id = None;
                    self.grid.set(x, y, CellRef::block(id, BlockPhase::Static));
                    // Request elimination check with combo_id
                    self.elimination_requests.push((x, y, combo_id));
                }
            }
        }
        block_ids.clear();
        self.work_block_ids = block_ids;
    }

    /// Update garbage - handle awaking and shattering
    pub(crate) fn update_garbage(&mut self) {
        self.work_garbage_ids.clear();
        self.work_garbage_ids.extend(self.garbage.iter_active().map(|g| g.id));
        let garbage_ids = std::mem::take(&mut self.work_garbage_ids);

        // Track garbage to remove (shattering finished)
        let mut to_remove: Vec<(GarbageId, i32, i32, i32, i32)> = Vec::new();
        // Track garbage that finished awaking
        let mut to_awaken: Vec<(GarbageId, i32, i32, i32, i32)> = Vec::new();

        let current_time = self.time_step;
        for &id in &garbage_ids {
            if let Some(garbage) = self.garbage.get_mut(id) {
                // animation_countdown and pop_countdown are absolute timestamps, checked against current_time

                // Shattering garbage - cleanup when animation_countdown expires
                if garbage.is_shattering() && garbage.animation_countdown > 0 && current_time >= garbage.animation_countdown {
                    to_remove.push((id, garbage.grid_x, garbage.grid_y, garbage.width, garbage.height));
                }

                // Awaking garbage — section pop progression
                if garbage.is_awaking() {
                    let total = garbage.width * garbage.height;
                    if garbage.sections_popped < total
                        && garbage.pop_countdown > 0
                        && current_time >= garbage.pop_countdown
                    {
                        garbage.sections_popped += 1;
                        if garbage.sections_popped < total {
                            garbage.pop_direction = garbage.pop_direction.next();
                            garbage.pop_countdown = current_time + crate::constants::INTERNAL_POP_DELAY;
                        }
                    }

                    // Transition to static/falling when animation_countdown expires
                    if garbage.animation_countdown > 0 && current_time >= garbage.animation_countdown {
                        to_awaken.push((id, garbage.grid_x, garbage.grid_y, garbage.width, garbage.height));
                    }
                }
            }
        }

        // Remove finished shattering garbage and trigger gravity above
        for (id, gx, gy, gw, gh) in to_remove {
            // Clear only grid cells still referencing this garbage
            // (cells with awaking blocks have already been overwritten)
            for h in 0..gh {
                for w in 0..gw {
                    let cell = self.grid.get(gx + w, gy + h);
                    // Only clear if cell still points to this garbage
                    if cell.garbage_id() == Some(id) {
                        self.grid.clear(gx + w, gy + h);
                    }
                }
            }

            // Deallocate garbage
            self.garbage.deallocate(id);

            // Trigger gravity for cells above the garbage
            for w in 0..gw {
                self.propagate_fall_up(gx + w, gy + gh, false);
            }
        }

        // Process awakened garbage - transition to static or falling
        for (id, gx, gy, gw, gh) in to_awaken {
            // Check if should fall (any cell below is empty)
            let will_fall = self.can_garbage_fall(gx, gy, gw);

            if let Some(garbage) = self.garbage.get_mut(id) {
                if will_fall {
                    // Start falling
                    garbage.physics_state = GarbageState::Falling;
                    garbage.sub_grid_y = 0;
                    garbage.fall_delay_countdown = 0;  // No hang delay for awakened garbage
                    for h in 0..gh {
                        for w in 0..gw {
                            self.grid.set_garbage_phase(gx + w, gy + h, GarbagePhase::Falling);
                        }
                    }
                } else {
                    // Become static
                    garbage.physics_state = GarbageState::Static;
                    for h in 0..gh {
                        for w in 0..gw {
                            self.grid.set_garbage_phase(gx + w, gy + h, GarbagePhase::Static);
                        }
                    }
                }
            }
        }
        // Restore work buffer
        let mut gids = garbage_ids;
        gids.clear();
        self.work_garbage_ids = gids;
    }

    /// Update animation counts
    pub(crate) fn update_animation_counts(&mut self) {
        self.awaking_count = 0;
        self.dying_count = 0;

        // Count awaking/dying blocks
        for block in self.blocks.iter_active() {
            if block.is_awaking() {
                self.awaking_count += 1;
            }
            if block.is_dying() {
                self.dying_count += 1;
            }
        }

        // Count awaking garbage (shattering garbage does NOT count)
        for garbage in self.garbage.iter_active() {
            if garbage.is_awaking() {
                self.awaking_count += 1;
            }
        }
    }

    /// Shift entire grid up by one row. Returns false if can't shift (top row occupied).
    pub(crate) fn shift_grid_up(&mut self) -> bool {
        // Guard: can't shift if top row is occupied
        if self.grid.top_occupied_row >= PLAY_HEIGHT as i32 - 1 {
            return false;
        }

        self.gameplay_events.push(GameplayEvent::CreepShift);

        // Shift ALL cells up (from top to bottom to avoid overwriting)
        for y in (0..PLAY_HEIGHT as i32 - 1).rev() {
            for x in (0..PLAY_WIDTH as i32).rev() {
                // Copy entire cell including phase (handles Immutable empty cells during swaps)
                let cell = self.grid.get(x, y);
                self.grid.set(x, y + 1, cell);
            }
        }

        // Reset bottom row to empty
        for x in (0..PLAY_WIDTH as i32).rev() {
            self.grid.clear(x, 0);
        }

        // Update row counters
        self.grid.top_occupied_row += 1;
        self.grid.top_effective_row += 1;

        // Shift block entity y positions
        for block in self.blocks.iter_active_mut() {
            block.grid_y += 1;
        }

        // Shift garbage entity y positions
        for garbage in self.garbage.iter_active_mut() {
            garbage.grid_y += 1;
        }

        // Shift swapper
        self.swapper.shift_up();

        // Shift already-queued elimination requests along with the blocks they
        // reference. Our queue stores coordinates, so we must translate them
        // manually or requests will scan stale cells.
        for req in self.elimination_requests.iter_mut() {
            req.1 += 1;
        }

        // Generate and spawn new nascent row at y=0
        self.spawn_incoming_row_inline();

        // Request elimination check for the row that just became visible (y=1)
        // All checks share a single combo so the entire row forms one chain.
        let shared_combo = self.combos.allocate(self.time_step).map(|c| c.0);
        if shared_combo.is_none() {
            // Degrade instead of panicking: the rising row is checked without a shared
            // combo (requests accept None). This is deterministic and logs a loud signal.
            let active: Vec<_> = self.combos.iter_active()
                .map(|c| format!("{:?}(inv={})", c.id, c.involvement_count))
                .collect();
            eprintln!("COMBO POOL EXHAUSTED (creep row) at tick {} — row check WITHOUT combo.\n  Active combos: {:?}",
                self.time_step, active);
        }
        for x in (0..PLAY_WIDTH as i32).rev() {
            self.elimination_requests.push((x, 1, shared_combo));
        }

        true
    }

    /// Create falling garbage from the garbage generator queue
    /// Called when queued garbage is ready to drop
    /// Returns true if garbage was created, false if no room (enables retry)
    pub(crate) fn create_falling_garbage(&mut self, height: i32, width: i32, flavor: i32) -> bool {
        // Limit height
        let height = height.min(MAX_GARBAGE_HEIGHT);

        // Determine drop row: if top_occupied_row >= SAFE_HEIGHT, drop just above it;
        // otherwise drop at SAFE_HEIGHT + 1 (minimum visible position).
        let drop_row = if self.grid.top_occupied_row >= SAFE_HEIGHT as i32 {
            self.grid.top_occupied_row + 1
        } else {
            SAFE_HEIGHT as i32 + 1
        };

        // Check if there's room
        if drop_row + height > PLAY_HEIGHT as i32 - 1 {
            return false;  // No room — will retry later
        }

        // Full-width garbage always at x=0, others get random position
        let x = if width == PLAY_WIDTH as i32 {
            0
        } else {
            self.rng.number((PLAY_WIDTH as i32 + 1) - width)
        };

        let spawn_y = drop_row;

        // Update top_occupied_row BEFORE creating garbage so subsequent drops find their position
        self.grid.top_occupied_row = drop_row + height - 1;

        // Allocate garbage
        if let Some(id) = self.garbage.allocate(x, spawn_y, width, height, flavor) {
            // Set as falling with initial fall flag
            if let Some(garbage) = self.garbage.get_mut(id) {
                garbage.physics_state = GarbageState::Falling;
                garbage.initial_fall = true;
                garbage.sub_grid_y = 0;
                garbage.animation_countdown = self.time_step + HANG_DELAY;
                garbage.fall_delay_countdown = self.time_step + HANG_DELAY;  // Wait before falling
            }
            self.gameplay_events.push(crate::gameplay_event::GameplayEvent::GarbageDropped { width, height });

            // Set grid cells (the hang is handled via garbage.animation_countdown, not via grid phase)
            for h in 0..height {
                for w in 0..width {
                    self.grid.set(x + w, spawn_y + h,
                        CellRef::garbage(id, GarbagePhase::Falling));
                }
            }

            // Update top_effective_row
            let new_top = spawn_y + height - 1;
            if new_top > self.grid.top_effective_row {
                self.grid.top_effective_row = new_top;
            }
            return true;
        }
        false
    }

}
