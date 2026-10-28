// Gravity system - falling blocks, garbage, landing detection

use super::GameState;
use crate::constants::*;
use crate::block::{BlockId, BlockState};
use crate::garbage::{GarbageId, GarbageState};
use crate::grid::{CellRef, BlockPhase, GarbagePhase};
use crate::gameplay_event::{GameplayEvent, GarbageLandedEvent};

impl GameState {
    /// Start a block falling.
    /// no_hang: if true, skip hang delay (block above already-falling)
    pub(super) fn start_block_falling(&mut self, id: BlockId, no_hang: bool) {
        self.start_block_falling_with_combo(id, no_hang, None);
    }

    /// Start a block falling with combo association
    /// combo_id: if Some, assigns the combo to this block with involvement tracking
    ///           if None, block starts falling WITHOUT combo involvement changes
    fn start_block_falling_with_combo(&mut self, id: BlockId, no_hang: bool, combo_id: Option<u16>) {
        // Check static before borrowing self for combo involvement
        let is_static = self.blocks.get(id).map(|b| b.is_static()).unwrap_or(false);
        if !is_static {
            return;
        }

        // Transfer combo involvement (borrows self mutably, then releases)
        if let Some(new_cid) = combo_id {
            self.begin_combo_involvement(id, new_cid);
        }

        // Now set falling state
        let block = self.blocks.get_mut(id).unwrap();
        block.physics_flags = BlockState::FALLING;
        let x = block.grid_x;
        let y = block.grid_y;

        if no_hang {
            block.fall_delay_countdown = 0;
            self.grid.set(x, y, CellRef::block(id, BlockPhase::Falling));
        } else {
            block.fall_delay_countdown = self.time_step + HANG_DELAY;
            self.grid.set(x, y, CellRef::block(id, BlockPhase::HangingThenFall));
        }

        // Propagate upward - pass the same combo_id we received (not block's combo_id)
        // This is critical: when called with None, propagate None
        self.propagate_fall_up_with_combo(x, y + 1, no_hang, combo_id);
    }

    /// When a block lands with a combo, propagate the combo to swapping blocks
    /// if they have matching flavor. This enables chain tracking during swaps.
    fn notify_landing(&mut self, x: i32, y: i32, flavor: i32, combo_id: u16) {
        use crate::swapper::SwapperState;

        // Only check if swapping
        if self.swapper.state != SwapperState::Swapping {
            return;
        }

        // Check if landing is one row above the swap position
        if y - 1 != self.swapper.y {
            return;
        }

        let swap_x = self.swapper.swap_x;

        // If landing at swap_x and rightBlock is moving and leftBlock has matching flavor
        // → assign combo to leftBlock
        if x == swap_x {
            if let Some(_right_id) = self.swapper.swap_right_block {
                // rightMoving_ check: right block exists
                if let Some(left_id) = self.swapper.swap_left_block {
                    if let Some(left_block) = self.blocks.get(left_id) {
                        if left_block.flavor == flavor {
                            // Assign combo to left block (beginComboInvolvement)
                            self.begin_combo_involvement(left_id, combo_id);
                        }
                    }
                }
            }
        }
        // If landing at swap_x + 1 and leftBlock is moving and rightBlock has matching flavor
        // → assign combo to rightBlock
        else if x == swap_x + 1 {
            if let Some(_left_id) = self.swapper.swap_left_block {
                // leftMoving_ check: left block exists
                if let Some(right_id) = self.swapper.swap_right_block {
                    if let Some(right_block) = self.blocks.get(right_id) {
                        if right_block.flavor == flavor {
                            // Assign combo to right block (beginComboInvolvement)
                            self.begin_combo_involvement(right_id, combo_id);
                        }
                    }
                }
            }
        }
    }

    /// Propagate fall upward
    pub(super) fn propagate_fall_up(&mut self, x: i32, y: i32, no_hang: bool) {
        self.propagate_fall_up_with_combo(x, y, no_hang, None);
    }

    /// Propagate fall upward with combo association.
    /// When a dying block triggers blocks above to fall, they all
    /// inherit the combo association (for chain tracking).
    pub(super) fn propagate_fall_up_with_combo(&mut self, x: i32, y: i32, no_hang: bool, combo_id: Option<u16>) {
        if y >= PLAY_HEIGHT as i32 {
            return;
        }

        let cell = self.grid.get(x, y);

        if cell.is_block() && cell.is_static() {
            if let Some(id) = cell.block_id() {
                // Start falling with combo_id - this will propagate the combo to blocks above
                self.start_block_falling_with_combo(id, no_hang, combo_id);
            }
        } else if cell.is_garbage() && cell.is_static() {
            if let Some(id) = cell.garbage_id() {
                self.start_garbage_falling(id, no_hang);
            }
        }
    }

    /// Start garbage falling with full check (cascade/propagation: allows falling cells below)
    fn start_garbage_falling(&mut self, id: GarbageId, no_hang: bool) {
        self.start_garbage_falling_impl(id, no_hang, true);
    }

    /// Start garbage falling with strict check (passive gravity: only empty cells below)
    fn start_garbage_falling_passive(&mut self, id: GarbageId) {
        self.start_garbage_falling_impl(id, false, false);
    }

    /// Implementation: allow_falling_below controls whether falling cells count as "empty"
    /// - Passive gravity: allow_falling_below = false
    /// - Cascade propagation: allow_falling_below = true
    fn start_garbage_falling_impl(&mut self, id: GarbageId, no_hang: bool, allow_falling_below: bool) {
        if let Some(garbage) = self.garbage.get_mut(id) {
            if garbage.physics_state != GarbageState::Static {
                return;  // Only start falling if static
            }

            let x = garbage.grid_x;
            let y = garbage.grid_y;
            let width = garbage.width;
            let height = garbage.height;

            // Check all bottom cells
            // - Passive gravity: all must be static-empty
            // - Cascade: static-empty OR actively-falling
            for w in 0..width {
                let below_empty = self.grid.is_static_empty(x + w, y - 1);
                if !below_empty {
                    if allow_falling_below {
                        let below_falling = self.grid.is_falling_at(x + w, y - 1);
                        if !below_falling {
                            return; // Cell below is not empty and not actively falling
                        }
                    } else {
                        return; // Passive gravity: cell below is not empty
                    }
                }
            }

            garbage.physics_state = GarbageState::Falling;

            if no_hang {
                garbage.fall_delay_countdown = 0;
                // Set all cells to Falling
                for h in 0..height {
                    for w in 0..width {
                        self.grid.set_garbage_phase(x + w, y + h, GarbagePhase::Falling);
                    }
                }
            } else {
                garbage.fall_delay_countdown = self.time_step + HANG_DELAY;
                // Set all cells to HangingThenFall
                for h in 0..height {
                    for w in 0..width {
                        self.grid.set_garbage_phase(x + w, y + h, GarbagePhase::HangingThenFall);
                    }
                }
            }

            // Propagate upward for all columns covered by garbage
            for w in 0..width {
                self.propagate_fall_up(x + w, y + height, no_hang);
            }
        }
    }

    /// Check if garbage can fall (all bottom cells must be static empty)
    pub(super) fn can_garbage_fall(&self, x: i32, y: i32, width: i32) -> bool {
        for w in 0..width {
            if !self.grid.is_static_empty(x + w, y - 1) {
                return false;
            }
        }
        true
    }

    pub(super) fn update_gravity(&mut self) {
        let current_time = self.time_step;

        // 1. Process hang delay expiration for blocks
        let block_ids: Vec<BlockId> = self.blocks.iter_active()
            .filter(|b| b.is_falling())
            .map(|b| b.id)
            .collect();

        for id in block_ids {
            if let Some(block) = self.blocks.get_mut(id) {
                // Check hang alarm expiration
                if block.fall_delay_countdown != 0 && current_time >= block.fall_delay_countdown {
                    block.fall_delay_countdown = 0;
                    // Transition HangingThenFall → Falling
                    self.grid.set(block.grid_x, block.grid_y, CellRef::block(id, BlockPhase::Falling));
                }
            }
        }

        // 2. Process hang delay expiration for garbage
        let garbage_ids: Vec<GarbageId> = self.garbage.iter_active()
            .filter(|g| g.is_falling())
            .map(|g| g.id)
            .collect();

        for id in garbage_ids {
            if let Some(garbage) = self.garbage.get_mut(id) {
                if garbage.fall_delay_countdown != 0 && current_time >= garbage.fall_delay_countdown {
                    garbage.fall_delay_countdown = 0;
                    // Transition HangingThenFall → Falling for all cells
                    for h in 0..garbage.height {
                        for w in 0..garbage.width {
                            self.grid.set_garbage_phase(garbage.grid_x + w, garbage.grid_y + h, GarbagePhase::Falling);
                        }
                    }
                }
            }
        }

        // 3. Detect new static blocks that should start falling (passive gravity)
        for y in 1..PLAY_HEIGHT as i32 {
            for x in 0..PLAY_WIDTH as i32 {
                let cell = self.grid.get(x, y);
                if cell.is_block() && cell.is_static()
                    && self.grid.is_static_empty(x, y - 1) {
                        if let Some(id) = cell.block_id() {
                            self.start_block_falling(id, false);
                        }
                    }
            }
        }

        // 3b. Detect static garbage that should start falling (passive gravity)
        // Only triggers when all cells below are truly empty
        // (not when cells below are falling — that is handled by the cascade path)
        let garbage_ids: Vec<GarbageId> = self.garbage.iter_active()
            .filter(|g| g.physics_state == GarbageState::Static)
            .map(|g| g.id)
            .collect();

        for id in garbage_ids {
            // Use passive version: only triggers when cells below are static-empty
            self.start_garbage_falling_passive(id);
        }

        // 4. Process falling blocks movement
        // Sort by y ascending (bottom-first) so lower blocks move before upper blocks
        let mut block_ids: Vec<(BlockId, i32)> = self.blocks.iter_active()
            .filter(|b| b.is_falling() && b.fall_delay_countdown == 0)
            .map(|b| (b.id, b.grid_y))
            .collect();
        block_ids.sort_by_key(|(_, y)| *y);
        let block_ids: Vec<BlockId> = block_ids.into_iter().map(|(id, _)| id).collect();

        // Collect landing notifications to process after block borrows are released
        let mut landing_notifications: Vec<(i32, i32, i32, u16)> = Vec::new();

        for id in block_ids {
            // Check if at grid element boundary before attempting move
            let (x, y, at_bottom, _old_f_y) = {
                let block = self.blocks.get(id).unwrap();
                (block.grid_x, block.grid_y, block.sub_grid_y == 0, block.sub_grid_y)
            };

            let mut landed = false;

            if let Some(block) = self.blocks.get_mut(id) {
                // 1. If at bottom of grid element, try to move down or land
                if at_bottom {
                    if self.grid.is_static_empty(x, y - 1) {
                        // Continue falling - move down and reset f_y
                        block.grid_y -= 1;
                        block.sub_grid_y = STEPS_PER_GRID;  // Reset to 60
                        self.grid.clear(x, y);
                        self.grid.set(block.grid_x, block.grid_y, CellRef::block(id, BlockPhase::Falling));
                    } else {
                        // Land
                        self.gameplay_events.push(crate::gameplay_event::GameplayEvent::BlockLanded(
                            crate::gameplay_event::BlockLandedEvent {
                                block_id: id.0 as i32,
                                grid_x: block.grid_x,
                                grid_y: block.grid_y,
                                flavor: block.flavor,
                                combo_id: block.combo_id.map(|c| c as i32),
                            }
                        ));
                        let combo_id = block.combo_id;
                        let flavor = block.flavor;  // Capture for notifyLanding
                        block.sub_grid_y = 0;
                        block.physics_flags = BlockState::STATIC;
                        // Do NOT clear combo_id here — it stays with the block until
                        // elimination check so involvement tracking for chains is preserved.
                        self.grid.set(x, y, CellRef::block(id, BlockPhase::Static));
                        self.elimination_requests.push((x, y, combo_id));
                        landed = true;

                        // Queue landing notification for later
                        if let Some(cid) = combo_id {
                            landing_notifications.push((x, y, flavor, cid));
                        }
                    }
                }

                // 2. Continue falling - decrement (only if still falling)
                if !landed {
                    block.sub_grid_y -= FALL_VELOCITY;
                }
            }
        }

        // Process landing notifications after block borrows are released
        for (x, y, flavor, combo_id) in landing_notifications {
            self.notify_landing(x, y, flavor, combo_id);
        }

        // 5. Process falling garbage movement
        // Sort by y ascending (bottom-first) so lower garbage moves before upper entities
        let mut garbage_ids: Vec<(GarbageId, i32)> = self.garbage.iter_active()
            .filter(|g| g.is_falling() && g.fall_delay_countdown == 0)
            .map(|g| (g.id, g.grid_y))
            .collect();
        garbage_ids.sort_by_key(|(_, y)| *y);
        let garbage_ids: Vec<GarbageId> = garbage_ids.into_iter().map(|(id, _)| id).collect();

        for id in garbage_ids {
            // Check if at grid element boundary before attempting move.
            // Check can_fall BEFORE taking mutable borrow
            let (x, y, width, height, at_bottom, was_initial, can_fall) = {
                let garbage = self.garbage.get(id).unwrap();
                let at_bottom = garbage.sub_grid_y == 0;
                let can_fall = if at_bottom {
                    self.can_garbage_fall(garbage.grid_x, garbage.grid_y, garbage.width)
                } else {
                    false
                };
                (garbage.grid_x, garbage.grid_y, garbage.width, garbage.height,
                 at_bottom, garbage.initial_fall, can_fall)
            };

            let mut landed = false;

            if let Some(garbage) = self.garbage.get_mut(id) {
                // 1. If at bottom of grid element, try to move down or land
                if at_bottom {
                    if can_fall {
                        // Continue falling - move down and reset f_y
                        garbage.grid_y -= 1;
                        garbage.sub_grid_y = STEPS_PER_GRID;  // Reset to 60

                        // Remove from old positions (top row only)
                        for w in 0..width {
                            self.grid.clear(x + w, y + height - 1);
                        }

                        // Add to new bottom row
                        for w in 0..width {
                            self.grid.set(x + w, garbage.grid_y,
                                CellRef::garbage(id, GarbagePhase::Falling));
                        }
                    } else {
                        // Land
                        self.gameplay_events.push(GameplayEvent::GarbageLanded(GarbageLandedEvent {
                            grid_x: x, grid_y: y, width, height, initial: was_initial,
                        }));
                        garbage.sub_grid_y = 0;
                        garbage.physics_state = GarbageState::Static;

                        // Impact notification (only for initial fall)
                        if was_initial {
                            garbage.initial_fall = false;
                            self.grid.notify_impact(y, height);
                            self.gravity_impacts.push((y, height));
                        }

                        // Update grid phase
                        for h in 0..height {
                            for w in 0..width {
                                self.grid.set_garbage_phase(x + w, y + h, GarbagePhase::Static);
                            }
                        }
                        landed = true;
                    }
                }

                // 2. Continue falling - decrement (only if still falling)
                if !landed {
                    garbage.sub_grid_y -= FALL_VELOCITY;
                }
            }
        }
    }
}
