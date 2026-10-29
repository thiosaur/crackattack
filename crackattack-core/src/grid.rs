use crate::constants::*;
use crate::block::BlockId;
use crate::garbage::{GarbageId, GarbageArena};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CellType {
    #[default]
    Empty,
    Block,
    Garbage,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BlockPhase {
    #[default]
    Static = 0,
    Falling = 1,
    HangingThenFall = 2,
    Awaking = 3,
    Swapping = 10,
    Dying = 11,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum GarbagePhase {
    #[default]
    Static = 0,
    Falling = 1,
    HangingThenFall = 2,
    Awaking = 3,
    Shattering = 20,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum EmptyPhase {
    #[default]
    Static = 0,
    Immutable = 30,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct CellRef {
    pub id: i16,  // -1 = empty, otherwise index in pool
    pub cell_type: CellType,
    pub phase: u8,
}

impl CellRef {
    pub fn empty() -> Self {
        CellRef {
            id: -1,
            cell_type: CellType::Empty,
            phase: EmptyPhase::Static as u8,
        }
    }

    pub(crate) fn empty_immutable() -> Self {
        CellRef {
            id: -1,
            cell_type: CellType::Empty,
            phase: EmptyPhase::Immutable as u8,
        }
    }

    pub fn block(id: BlockId, phase: BlockPhase) -> Self {
        CellRef {
            id: id.0 as i16,
            cell_type: CellType::Block,
            phase: phase as u8,
        }
    }

    pub fn garbage(id: GarbageId, phase: GarbagePhase) -> Self {
        CellRef {
            id: id.0 as i16,
            cell_type: CellType::Garbage,
            phase: phase as u8,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.cell_type == CellType::Empty
    }

    pub(crate) fn is_block(&self) -> bool {
        self.cell_type == CellType::Block
    }

    pub(crate) fn is_garbage(&self) -> bool {
        self.cell_type == CellType::Garbage
    }

    pub fn is_static(&self) -> bool {
        self.phase == 0  // Static = 0 for all types
    }

    pub fn block_id(&self) -> Option<BlockId> {
        if self.is_block() && self.id >= 0 {
            Some(BlockId(self.id as u16))
        } else {
            None
        }
    }

    pub(crate) fn garbage_id(&self) -> Option<GarbageId> {
        if self.is_garbage() && self.id >= 0 {
            Some(GarbageId(self.id as u16))
        } else {
            None
        }
    }

}

pub struct Grid {
    cells: [[CellRef; PLAY_HEIGHT]; PLAY_WIDTH],
    /// Topmost row with any content (includes falling garbage)
    pub(crate) top_occupied_row: i32,
    /// Topmost row with settled content (excludes initial drop)
    pub top_effective_row: i32,
}

impl Grid {
    pub fn new() -> Self {
        Grid {
            cells: [[CellRef::empty(); PLAY_HEIGHT]; PLAY_WIDTH],
            top_occupied_row: 0,
            top_effective_row: 0,
        }
    }

    pub fn get(&self, x: i32, y: i32) -> CellRef {
        if self.in_bounds(x, y) {
            self.cells[x as usize][y as usize]
        } else {
            CellRef::empty()
        }
    }

    pub fn set(&mut self, x: i32, y: i32, cell: CellRef) {
        debug_assert!(self.in_bounds(x, y),
            "Grid::set out of bounds: ({}, {})", x, y);
        if self.in_bounds(x, y) {
            self.cells[x as usize][y as usize] = cell;
        }
    }

    pub fn clear(&mut self, x: i32, y: i32) {
        self.set(x, y, CellRef::empty());
    }

    pub fn in_bounds(&self, x: i32, y: i32) -> bool {
        x >= 0 && x < PLAY_WIDTH as i32 && y >= 0 && y < PLAY_HEIGHT as i32
    }

    pub fn is_empty(&self, x: i32, y: i32) -> bool {
        self.get(x, y).is_empty()
    }

    pub(crate) fn is_block(&self, x: i32, y: i32) -> bool {
        self.get(x, y).is_block()
    }

    pub fn is_static(&self, x: i32, y: i32) -> bool {
        self.get(x, y).is_static()
    }

    pub(crate) fn is_static_block(&self, x: i32, y: i32) -> bool {
        let cell = self.get(x, y);
        cell.is_block() && cell.is_static()
    }

    pub(crate) fn is_static_empty(&self, x: i32, y: i32) -> bool {
        let cell = self.get(x, y);
        cell.is_empty() && cell.phase == EmptyPhase::Static as u8
    }

    pub fn block_id_at(&self, x: i32, y: i32) -> Option<BlockId> {
        self.get(x, y).block_id()
    }

    pub(crate) fn set_empty_immutable(&mut self, x: i32, y: i32) {
        self.set(x, y, CellRef::empty_immutable());
    }

    pub(crate) fn is_static_garbage(&self, x: i32, y: i32) -> bool {
        let cell = self.get(x, y);
        cell.is_garbage() && cell.is_static()
    }

    /// Check if cell contains a falling entity (block or garbage)
    /// Used for swap validation.
    pub(crate) fn is_falling_at(&self, x: i32, y: i32) -> bool {
        let cell = self.get(x, y);
        cell.phase == BlockPhase::Falling as u8  // Same value (1) for Block and Garbage
    }

    /// Check if cell contains a hanging entity (will fall)
    /// Used for swap validation.
    pub(crate) fn is_hanging_at(&self, x: i32, y: i32) -> bool {
        let cell = self.get(x, y);
        cell.phase == BlockPhase::HangingThenFall as u8  // Same value (2) for Block and Garbage
    }

    /// Check if an empty cell can be swapped into.
    /// Returns false if there's instability (falling below or hanging above)
    pub(crate) fn can_swap_at(&self, x: i32, y: i32) -> bool {
        !self.is_falling_at(x, y - 1) && !self.is_hanging_at(x, y + 1)
    }

    /// Update the phase of a garbage cell.
    pub(crate) fn set_garbage_phase(&mut self, x: i32, y: i32, phase: GarbagePhase) {
        debug_assert!(self.in_bounds(x, y),
            "Grid::set_garbage_phase out of bounds: ({}, {})", x, y);
        if self.in_bounds(x, y) {
            let cell = &mut self.cells[x as usize][y as usize];
            if cell.is_garbage() {
                cell.phase = phase as u8;
            }
        }
    }

    /// Update top_occupied_row and top_effective_row.
    /// - top_occupied_row: full scan for highest non-empty row.
    /// - top_effective_row: only scans down from current value (lowering).
    ///   Raising happens via notify_impact() and shift_grid_up().
    pub(crate) fn update_top_rows(&mut self, garbage_arena: &GarbageArena) {
        // Full scan for highest non-empty row
        self.top_occupied_row = 0;
        'outer_occupied: for y in (0..PLAY_HEIGHT as i32).rev() {
            for x in 0..PLAY_WIDTH as i32 {
                if !self.is_empty(x, y) {
                    self.top_occupied_row = y;
                    break 'outer_occupied;
                }
            }
        }

        // Scan down from current top: only lowers the value.
        // Raising happens via notify_impact and shift_grid_up.
        let old_top = self.top_effective_row;
        self.top_effective_row += 1;

        loop {
            self.top_effective_row -= 1;
            if self.top_effective_row < 0 {
                self.top_effective_row = 0;
                break;
            }

            let mut found = false;
            for x in (0..PLAY_WIDTH as i32).rev() {
                let cell = self.get(x, self.top_effective_row);

                if !cell.is_empty() {
                    // Check if it's garbage with initial_fall
                    if let Some(id) = cell.garbage_id() {
                        let garbage = garbage_arena.get(id);
                        debug_assert!(garbage.is_some(),
                            "Grid cell ({},{}) references non-existent garbage {:?}",
                            x, self.top_effective_row, id);
                        if let Some(garbage) = garbage {
                            if garbage.initial_fall {
                                continue; // Skip initial falling garbage
                            }
                        }
                    }
                    // Found stable content
                    found = true;
                    break;
                }
            }

            if found {
                break;
            }
        }

        let _ = old_top; // keeps the binding used
    }

    /// Called when a block or garbage lands.
    /// Raises top_effective_row if the impact is higher.
    pub(crate) fn notify_impact(&mut self, y: i32, height: i32) {
        let impact_top = y + height - 1;
        if impact_top > self.top_effective_row {
            self.top_effective_row = impact_top;
        }
    }

    /// Check if the safe height is violated (game over condition).
    pub(crate) fn check_safe_height_violation(&self) -> bool {
        self.top_effective_row >= SAFE_HEIGHT as i32 - 1
    }
}

impl Default for Grid {
    fn default() -> Self {
        Self::new()
    }
}
