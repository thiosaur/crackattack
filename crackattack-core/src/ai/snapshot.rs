use crate::constants::*;
use crate::game_state::GameState;
use crate::grid::{BlockPhase, GarbagePhase, EmptyPhase};

// Grid dimensions for AI

pub(crate) const GRID_WIDTH: usize = PLAY_WIDTH;       // 6
pub(crate) const GRID_HEIGHT: usize = SAFE_HEIGHT;     // 13

pub(crate) const SWAP_WIDTH: usize = PLAY_WIDTH - 1;   // 5 (cursor can be at x=0..4)
pub(crate) const SWAP_HEIGHT: usize = SAFE_HEIGHT - 1; // 12 (y=0 is below visible in SwapGrid)

pub(crate) const CF_EMPTY: u8      = 0;
pub(crate) const CF_BLOCK: u8      = 1 << 0;
pub(crate) const CF_GARBAGE: u8    = 1 << 1;
pub(crate) const CF_FALLING: u8    = 1 << 2;
pub(crate) const CF_HANGING: u8    = 1 << 3;
pub(crate) const CF_SHATTERING: u8 = 1 << 4;
pub(crate) const CF_IMMUTABLE: u8  = 1 << 5;
pub(crate) const CF_AWAKING: u8   = 1 << 6;

pub(crate) const CF_UNSWAPPABLE: u8 = CF_GARBAGE | CF_FALLING | CF_HANGING | CF_SHATTERING | CF_IMMUTABLE | CF_AWAKING;

pub(crate) mod swap_priority {
    pub(crate) const SETUP_LOW: i8  = 2;
    pub(crate) const SETUP: i8      = 3;
    pub(crate) const SETUP_HIGH: i8 = 5;
    pub(crate) const MATCH: i8      = 10;
    pub(crate) const MATCH_4: i8    = 12;
    pub(crate) const CHAIN: i8      = 15;
    pub(crate) const COMBO: i8      = 18;
    pub(crate) const CRITICAL: i8   = 25;
}

pub(crate) mod swap_base {
    pub(crate) const IMPOSSIBLE: i8 = 0;
    pub(crate) const POSSIBLE: i8   = 1;
}

#[derive(Clone, Copy, Default)]
pub(crate) struct Cell {
    pub(crate) flags: u8,
    pub(crate) flavor: i8, // -1 = no block, 0..NUM_NORMAL_FLAVORS-1
}

// GridSnapshot

#[derive(Clone)]
pub(crate) struct GridSnapshot {
    pub(crate) cells: [[Cell; GRID_HEIGHT]; GRID_WIDTH],
    pub(crate) top_line: i32,
    pub(crate) peak_height: i32,
    pub(crate) in_danger: bool,
}

impl Default for GridSnapshot {
    fn default() -> Self {
        GridSnapshot {
            cells: [[Cell::default(); GRID_HEIGHT]; GRID_WIDTH],
            top_line: 0,
            peak_height: 0,
            in_danger: false,
        }
    }
}

impl GridSnapshot {
    pub(crate) fn cell_at(&self, x: i32, y: i32) -> &Cell {
        &self.cells[x as usize][y as usize]
    }

    pub(crate) fn has_block(&self, x: i32, y: i32) -> bool {
        if x < 0 || x >= GRID_WIDTH as i32 || y < 0 || y >= GRID_HEIGHT as i32 {
            return false;
        }
        let f = self.cells[x as usize][y as usize].flags;
        (f & CF_BLOCK) != 0 && (f & CF_UNSWAPPABLE) == 0
    }

    pub(crate) fn get_flavor_at(&self, x: i32, y: i32) -> i32 {
        if x < 0 || x >= GRID_WIDTH as i32 || y < 0 || y >= GRID_HEIGHT as i32 {
            return -1;
        }
        self.cells[x as usize][y as usize].flavor as i32
    }
}



pub(crate) fn build_grid_snapshot(game: &GameState) -> GridSnapshot {
    let mut snap = GridSnapshot::default();
    let mut max_height: i32 = 0;

    for x in 0..GRID_WIDTH as i32 {
        for y in 0..GRID_HEIGHT as i32 {
            let cell_ref = game.grid.get(x, y);
            let c = &mut snap.cells[x as usize][y as usize];

            c.flags = CF_EMPTY;
            c.flavor = -1;

            if cell_ref.is_block() {
                c.flags |= CF_BLOCK;
                // Get flavor from block arena
                if let Some(block_id) = cell_ref.block_id() {
                    if let Some(block) = game.blocks.get(block_id) {
                        c.flavor = block.flavor as i8;
                    }
                }
            }
            if cell_ref.is_garbage() {
                c.flags |= CF_GARBAGE;
            }

            // Phase flags
            let phase = cell_ref.phase;
            if cell_ref.is_block() {
                if phase == BlockPhase::Falling as u8 {
                    c.flags |= CF_FALLING;
                }
                if phase == BlockPhase::HangingThenFall as u8 {
                    c.flags |= CF_HANGING;
                }
                if phase == BlockPhase::Dying as u8 {
                    c.flags |= CF_SHATTERING;
                }
                if phase == BlockPhase::Swapping as u8 {
                    c.flags |= CF_IMMUTABLE;
                }
                if phase == BlockPhase::Awaking as u8 {
                    c.flags |= CF_AWAKING;
                }
            } else if cell_ref.is_garbage() {
                if phase == GarbagePhase::Falling as u8 {
                    c.flags |= CF_FALLING;
                }
                if phase == GarbagePhase::HangingThenFall as u8 {
                    c.flags |= CF_HANGING;
                }
                if phase == GarbagePhase::Shattering as u8 {
                    c.flags |= CF_SHATTERING;
                }
            } else if cell_ref.is_empty()
                && phase == EmptyPhase::Immutable as u8 {
                    c.flags |= CF_IMMUTABLE;
                }

            if c.flags != CF_EMPTY && y + 1 > max_height {
                max_height = y + 1;
            }
        }
    }

    snap.top_line = game.grid.top_effective_row;
    snap.peak_height = max_height;
    snap.in_danger = game.grid.check_safe_height_violation()
        || (max_height >= GRID_HEIGHT as i32 - 2);

    snap
}

#[derive(Clone)]
pub(crate) struct SwapGrid {
    pub(crate) cells: [[i8; SWAP_HEIGHT]; SWAP_WIDTH],
}

impl Default for SwapGrid {
    fn default() -> Self {
        SwapGrid {
            cells: [[0i8; SWAP_HEIGHT]; SWAP_WIDTH],
        }
    }
}

impl SwapGrid {
    pub(crate) fn cell_at(&self, x: i32, y: i32) -> i8 {
        if x < 0 || x >= SWAP_WIDTH as i32 || y < 0 || y >= SWAP_HEIGHT as i32 {
            return 0;
        }
        self.cells[x as usize][y as usize]
    }

    pub(crate) fn set(&mut self, x: i32, y: i32, value: i8) {
        if x >= 0 && x < SWAP_WIDTH as i32 && y >= 0 && y < SWAP_HEIGHT as i32 {
            self.cells[x as usize][y as usize] = value;
        }
    }

    pub(crate) fn merge(&mut self, other: &SwapGrid) {
        for x in 0..SWAP_WIDTH {
            for y in 0..SWAP_HEIGHT {
                if other.cells[x][y] > self.cells[x][y] {
                    self.cells[x][y] = other.cells[x][y];
                }
            }
        }
    }

    pub(crate) fn apply_mask(&mut self, base: &SwapGrid) {
        for x in 0..SWAP_WIDTH {
            for y in 0..SWAP_HEIGHT {
                if base.cells[x][y] == swap_base::IMPOSSIBLE {
                    self.cells[x][y] = 0;
                }
            }
        }
    }

    /// Find the highest-scored swap position. On equal scores, prefer the
    /// nearest candidate (Manhattan distance) for deterministic tie-breaking.
    pub(crate) fn find_best(&self, cursor_x: i32, cursor_y: i32, out_x: &mut i32, out_y: &mut i32) -> bool {
        let mut best_x: i32 = -1;
        let mut best_y: i32 = -1;
        let mut best_score: i32 = 0;
        let mut best_dist: i32 = 999;

        for x in 0..SWAP_WIDTH as i32 {
            for y in 0..SWAP_HEIGHT as i32 {
                let score = self.cell_at(x, y) as i32;
                if score <= 0 {
                    continue;
                }
                let dist = (x - cursor_x).abs() + (y - cursor_y).abs();
                if score > best_score || (score == best_score && dist < best_dist) {
                    best_score = score;
                    best_dist = dist;
                    best_x = x;
                    best_y = y;
                }
            }
        }

        if best_x >= 0 {
            *out_x = best_x;
            *out_y = best_y;
            true
        } else {
            false
        }
    }
}

// A swap is only worth considering for block+empty or two different blocks.
// Empty+empty and identical block pairs are no-op positions.
pub(crate) fn can_swap_at(grid: &GridSnapshot, sx: i32, sy: i32) -> bool {
    let grid_y = sy + 1;
    if sx < 0 || sx >= SWAP_WIDTH as i32 || grid_y < 1 || grid_y >= GRID_HEIGHT as i32 {
        return false;
    }

    let left = grid.cell_at(sx, grid_y);
    let right = grid.cell_at(sx + 1, grid_y);

    if (left.flags & CF_UNSWAPPABLE) != 0 || (right.flags & CF_UNSWAPPABLE) != 0 {
        return false;
    }

    let left_is_block = (left.flags & CF_BLOCK) != 0;
    let right_is_block = (right.flags & CF_BLOCK) != 0;
    let left_empty = left.flags == CF_EMPTY;
    let right_empty = right.flags == CF_EMPTY;

    if left_empty && right_empty {
        return false;
    }

    if left_is_block && right_is_block
        && left.flavor == right.flavor {
            return false;
        }

    true
}

pub(crate) fn build_swap_grid(grid: &GridSnapshot) -> SwapGrid {
    let mut result = SwapGrid::default();

    for x in 0..SWAP_WIDTH as i32 {
        for y in 0..SWAP_HEIGHT as i32 {
            if can_swap_at(grid, x, y) {
                result.set(x, y, swap_base::POSSIBLE);
            }
        }
    }

    result
}
