// Grid initialization and block generation

use super::GameState;
use crate::constants::*;
use crate::block::BlockId;
use crate::grid::{CellRef, BlockPhase};

impl GameState {
    /// Spawn the incoming row (y=0) with new flavors and update next_row for display.
    pub(super) fn spawn_incoming_row(&mut self) {
        self.spawn_incoming_row_inline();
    }

    /// Internal: generate and spawn incoming row (used by spawn_incoming_row and shift_grid_up)
    pub(super) fn spawn_incoming_row_inline(&mut self) {
        // Determine special block location (gray block)
        // 1 in 3 chance of NO special block
        const NO_SPECIAL_BLOCK_CHANCE_IN: i32 = 3;
        let special_block_location = if self.rng.chance_in(NO_SPECIAL_BLOCK_CHANCE_IN) {
            -1  // No special block
        } else {
            self.rng.number(PLAY_WIDTH as i32)
        };

        // Generate each column from right to left
        for x in (0..PLAY_WIDTH as i32).rev() {
            let flavor = if x == special_block_location {
                // Try to spawn gray block (flavor 5 = BF_GRAY)
                use crate::constants::BF_GRAY;

                // Check if gray would create 3-in-a-row
                let gray_would_triple_column =
                    BF_GRAY == self.last_row_c[x as usize] &&
                    self.last_row_c[x as usize] == self.second_to_last_row_c[x as usize];
                let gray_would_triple_global =
                    BF_GRAY == self.last_flavor_c && self.last_flavor_c == self.second_to_last_flavor_c;

                if gray_would_triple_column || gray_would_triple_global {
                    // Fallback: spawn normal block with anti-match
                    self.generate_creep_flavor_normal(x as usize)
                } else {
                    BF_GRAY
                }
            } else {
                // Normal block with anti-match
                self.generate_creep_flavor_normal(x as usize)
            };

            self.second_to_last_row_c[x as usize] = self.last_row_c[x as usize];
            self.last_row_c[x as usize] = flavor;
            self.second_to_last_flavor_c = self.last_flavor_c;
            self.last_flavor_c = flavor;

            self.spawn_block(x, 0, flavor);

            // Store for display (next_row is used for rendering the incoming row)
            self.next_row[x as usize] = flavor;
        }
    }

    pub(super) fn initialize_grid(&mut self) {
        // Initial grid layout:
        // - One random "short column" has height 2-3
        // - Other columns have height 7-8
        // - Anti-match constraints prevent immediate 3-in-a-row

        let short_column = self.rng.number(PLAY_WIDTH as i32);
        let mut top_row = 0;

        // Process columns from right to left
        for x in (0..PLAY_WIDTH as i32).rev() {
            // Generate column height
            let height = if x == short_column {
                2 + self.rng.number2(2)  // 2 or 3
            } else {
                7 + self.rng.number2(2)  // 7 or 8
            };

            if height - 1 > top_row {
                top_row = height - 1;
            }

            // Place blocks from top to bottom (y = height-1 down to 1)
            for y in (1..height).rev() {
                // Determine flavor with anti-match constraints
                let flavor = loop {
                    let f = self.rng.number(NUM_NORMAL_FLAVORS);

                    // Check 1: must differ from the block above (y+1)
                    if let Some(above_id) = self.grid.block_id_at(x, y + 1) {
                        let above_block = self.blocks.get(above_id)
                            .expect("Grid references non-existent block at init");
                        if above_block.flavor == f {
                            continue;  // Retry
                        }
                    }

                    // CHECK 2: Don't match block to the right (x+1)
                    // Skip check if at right edge
                    if x < PLAY_WIDTH as i32 - 1 {
                        if let Some(right_id) = self.grid.block_id_at(x + 1, y) {
                            let right_block = self.blocks.get(right_id)
                                .expect("Grid references non-existent block at init");
                            if right_block.flavor == f {
                                continue;  // Retry
                            }
                        }
                    }

                    break f;  // Flavor passes all checks
                };

                // Track last two rows for creep generation
                if y == 2 {
                    self.second_to_last_row_c[x as usize] = flavor;
                } else if y == 1 {
                    self.last_row_c[x as usize] = flavor;
                }

                self.spawn_block(x, y, flavor);
            }
        }

        // Update top rows
        self.grid.top_occupied_row = top_row;
        self.grid.top_effective_row = top_row;

        // Queue elimination checks for all initial blocks
        // This ensures any initial combos are resolved
        for y in 1..=top_row {
            for x in 0..PLAY_WIDTH as i32 {
                if self.grid.is_block(x, y) {
                    self.elimination_requests.push((x, y, None));
                }
            }
        }
    }

    pub(super) fn spawn_block(&mut self, x: i32, y: i32, flavor: i32) -> Option<BlockId> {
        let id = self.blocks.allocate(x, y, flavor)?;
        self.grid.set(x, y, CellRef::block(id, BlockPhase::Static));
        Some(id)
    }

    pub(super) fn remove_block(&mut self, id: BlockId) {
        if let Some(block) = self.blocks.get(id) {
            let x = block.grid_x;
            let y = block.grid_y;
            self.grid.clear(x, y);
        }
        self.blocks.deallocate(id);
    }

    /// Generate a normal flavor for creep row with anti-match constraints
    fn generate_creep_flavor_normal(&mut self, x: usize) -> i32 {
        loop {
            let flavor = self.rng.number(NUM_NORMAL_FLAVORS);
            if !Self::would_create_triple(
                flavor, x,
                &self.last_row_c, &self.second_to_last_row_c,
                self.last_flavor_c, self.second_to_last_flavor_c,
            ) {
                return flavor;
            }
        }
    }
}
