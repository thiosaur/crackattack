//! What the bot perceives: the board as the planners see it, the cursor's reach, and the
//! questions the strategies ask about it (is anything moving, how much garbage, how tall).

use crate::block::BlockId;
use crate::constants::{BF_GRAY, PLAY_HEIGHT, PLAY_WIDTH, SAFE_HEIGHT};
use crate::game_state::GameState;
use crate::grid::{BlockPhase, CellRef, CellType};

use super::controller::AiController;
use super::planner::{PlanBoard, PLAN_BLOCKED, PLAN_GARBAGE};

pub(super) const WIDTH: i32 = PLAY_WIDTH as i32;
pub(super) const HEIGHT: i32 = PLAY_HEIGHT as i32;
pub(super) const SAFE: i32 = SAFE_HEIGHT as i32;

/// A swap target: the cursor cell `(x, y)` (grid row, 1-based) whose block swaps rightward.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct SwapPlan {
    pub(super) x: i32,
    pub(super) y: i32,
}

impl SwapPlan {
    /// The swap at planner coordinates: planner rows are grid rows - 1.
    pub(super) fn from_plan(x: usize, plan_row: usize) -> Self {
        Self { x: x as i32, y: plan_row as i32 + 1 }
    }
}

/// Swaps a planner must not return: the exact undo of the last swap, and the ones a caller found
/// unplayable and asked to plan again without. A handful at most, so it is `Copy`.
#[derive(Debug, Clone, Copy, Default)]
pub(super) struct Avoid([Option<SwapPlan>; 4]);

impl Avoid {
    pub(super) fn contains(&self, plan: SwapPlan) -> bool {
        self.0.contains(&Some(plan))
    }

    /// How many swaps are listed.
    pub(super) fn len(&self) -> usize {
        self.0.iter().flatten().count()
    }

    /// Add a swap; false when the list is full.
    pub(super) fn push(&mut self, plan: SwapPlan) -> bool {
        match self.0.iter_mut().find(|slot| slot.is_none()) {
            Some(slot) => {
                *slot = Some(plan);
                true
            }
            None => false,
        }
    }
}

/// What a planner asks the hands to do this decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Intent {
    /// Walk to this swap and play it.
    Swap(SwapPlan),
    /// Nothing worth playing: pull in the next row, if the board may be raised.
    Raise,
    /// Nothing to do now (the board is falling and supplies the next position by itself).
    Wait,
}

/// What the board is doing right now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum BoardMotion {
    /// Nothing moves.
    Settled,
    /// Blocks fall or are about to pop: the future can be projected from a grid copy.
    Gravity,
    /// Awaking blocks or moving garbage: wait.
    Other,
}

/// Block phases that gravity-projection understands.
pub(super) fn phase_is_falling(phase: u8) -> bool {
    phase == BlockPhase::Falling as u8 || phase == BlockPhase::HangingThenFall as u8
}

pub(super) fn is_awaking(phase: u8) -> bool {
    phase == BlockPhase::Awaking as u8
}

/// Whether blocks are waking up, and whether any of them belongs to a combo (when it wakes, its
/// match continues that combo: a chain).
pub(super) fn awaking_blocks(game: &GameState) -> (bool, bool) {
    let (mut any, mut linked) = (false, false);
    for y in 1..HEIGHT {
        for x in 0..WIDTH {
            let cell = game.grid.get(x, y);
            if cell.cell_type == CellType::Block && is_awaking(cell.phase) {
                any = true;
                linked |= block_of(game, cell).is_some_and(|b| b.combo_id.is_some());
            }
        }
    }
    (any, linked)
}

pub(super) fn phase_is_dying(phase: u8) -> bool {
    phase == BlockPhase::Dying as u8
}

impl AiController {
    /// The plan board of the live game: resting blocks and garbage as they are, every block in
    /// motion as a fixed obstacle (so the bot can plan whatever the board is doing).
    pub(super) fn board(&self, game: &GameState) -> PlanBoard {
        read_plan_board(game)
    }
}

/// The live chain's blocks (falling or popping, with a combo): `(column, row)`.
pub(super) fn chain_blocks(game: &GameState) -> Vec<(i32, i32)> {
    let mut blocks = Vec::new();
    for y in 1..HEIGHT {
        for x in 0..WIDTH {
            let cell = game.grid.get(x, y);
            // Awaking blocks keep their combo but land nowhere: they are no part of a live chain.
            if cell.cell_type != CellType::Block || cell.is_static() || cell.phase == BlockPhase::Awaking as u8 {
                continue;
            }
            if !block_of(game, cell).is_some_and(|b| b.combo_id.is_some()) {
                continue;
            }
            blocks.push((x, y));
        }
    }
    blocks
}

/// Classify a settled board, predictable block gravity, or another transient.
pub(super) fn board_motion(game: &GameState) -> BoardMotion {
    let mut gravity = false;
    for y in 1..HEIGHT {
        for x in 0..WIDTH {
            let cell = game.grid.get(x, y);
            match cell.cell_type {
                CellType::Block if !cell.is_static() => {
                    if !phase_is_falling(cell.phase) && !phase_is_dying(cell.phase) {
                        return BoardMotion::Other;
                    }
                    gravity = true;
                }
                CellType::Garbage if !cell.is_static() => return BoardMotion::Other,
                _ => {}
            }
        }
    }
    if gravity {
        BoardMotion::Gravity
    } else {
        BoardMotion::Settled
    }
}

/// Whether a falling/dying block belongs to the combo whose next landing can extend it.
pub(super) fn has_linked_gravity(game: &GameState) -> bool {
    for y in 1..HEIGHT {
        for x in 0..WIDTH {
            let cell = game.grid.get(x, y);
            if cell.cell_type != CellType::Block {
                continue;
            }
            if !phase_is_falling(cell.phase) && !phase_is_dying(cell.phase) {
                continue;
            }
            if block_of(game, cell).is_some_and(|b| b.combo_id.is_some()) {
                return true;
            }
        }
    }
    false
}

/// Exactly the legality check of `GameState::process_swap` for a swap rightward at `(x, y)`:
/// a resting block on each side, or a resting empty cell that nothing falls into or hangs
/// over, with at least one block.
pub(super) fn can_swap_now(game: &GameState, x: i32, y: i32) -> bool {
    if !(0..WIDTH - 1).contains(&x) || !(1..SAFE).contains(&y) {
        return false;
    }
    let side_ok = |cx: i32| {
        game.grid.is_static_block(cx, y) || (game.grid.is_static_empty(cx, y) && game.grid.can_swap_at(cx, y))
    };
    side_ok(x) && side_ok(x + 1) && (game.grid.is_static_block(x, y) || game.grid.is_static_block(x + 1, y))
}

/// Number of garbage cells on the board.
pub(super) fn garbage_cells(game: &GameState) -> i32 {
    let mut n = 0;
    for y in 1..HEIGHT {
        for x in 0..WIDTH {
            if game.grid.get(x, y).is_garbage() {
                n += 1;
            }
        }
    }
    n
}

/// Number of block cells on the board.
pub(super) fn block_count(game: &GameState) -> i32 {
    let mut n = 0;
    for y in 1..HEIGHT {
        for x in 0..WIDTH {
            if game.grid.get(x, y).is_block() {
                n += 1;
            }
        }
    }
    n
}

/// A cell that can take part in a swap: a resting (static) block.
pub(super) fn swappable_block(game: &GameState, x: i32, y: i32) -> bool {
    game.grid.is_static_block(x, y)
}

/// Whether any 4-neighbour of `(x, y)` is a garbage cell (so a match here shatters it).
pub(super) fn garbage_neighbor(game: &GameState, x: i32, y: i32) -> bool {
    let is_garbage = |cx: i32, cy: i32| game.grid.get(cx, cy).is_garbage();
    (y + 1 < HEIGHT && is_garbage(x, y + 1))
        || (y >= 1 && is_garbage(x, y - 1))
        || (x + 1 < WIDTH && is_garbage(x + 1, y))
        || (x >= 1 && is_garbage(x - 1, y))
}

/// Two block flavours match when they are equal and not the colourless gray.
pub(super) fn flavor_match(a: i32, b: i32) -> bool {
    a == b && a != BF_GRAY
}

/// The block record behind a block cell.
pub(super) fn block_of(game: &GameState, cell: CellRef) -> Option<&crate::block::Block> {
    cell.block_id().and_then(|id: BlockId| game.blocks.get(id))
}

/// Flavour of the block at `(x, y)`; `-1` when the cell holds no block.
pub(super) fn flavor_at(game: &GameState, x: i32, y: i32) -> i32 {
    block_of(game, game.grid.get(x, y)).map_or(-1, |b| b.flavor)
}

/// Snapshot of the grid as a [`PlanBoard`]. Blocks in motion (falling, dying, awaking) are fixed
/// obstacles, [`PLAN_BLOCKED`]: they cannot be swapped and no planner counts on where they end up.
/// Rows are read from grid row 1 (bottom playable) up to the top of the stack (+1 headroom).
pub(super) fn read_plan_board(game: &GameState) -> PlanBoard {
    let width = PLAY_WIDTH;
    let height = (game.grid.top_effective_row + 1).clamp(0, HEIGHT - 1) as usize;
    let mut board = PlanBoard::new(width, height);
    for y in 0..height {
        let gy = y as i32 + 1; // grid rows are 1-based (row 0 is the incoming creep)
        for x in 0..width {
            let cell = game.grid.get(x as i32, gy);
            if cell.is_garbage() {
                board.set(x, y, PLAN_GARBAGE);
            } else if cell.is_block() && cell.is_static() {
                board.set(x, y, flavor_at(game, x as i32, gy) as i16);
            } else if cell.is_block() {
                board.set(x, y, PLAN_BLOCKED);
            }
        }
    }
    board
}

/// Snapshot used while blocks are in their hang/fall window. Unlike [`read_plan_board`], this
/// includes falling blocks at their current grid cells; the cascade evaluator then settles them
/// after any candidate swap. Dying blocks are omitted because they will pop before gravity
/// settles; awaking/swapping blocks stay absent because their eventual contents cannot be
/// inferred safely from the grid alone.
///
/// The extra height follows `top_occupied_row`, not `top_effective_row`, because a falling
/// resident may currently sit above the resting stack.
pub(crate) fn read_gravity_plan_board(game: &GameState) -> PlanBoard {
    let width = PLAY_WIDTH;
    let top = game.grid.top_occupied_row.max(game.grid.top_effective_row);
    let height = (top + 1).clamp(0, HEIGHT - 1) as usize;
    let mut board = PlanBoard::new(width, height);
    for y in 0..height {
        let gy = y as i32 + 1;
        for x in 0..width {
            let cell = game.grid.get(x as i32, gy);
            if cell.is_garbage() {
                board.set(x, y, PLAN_GARBAGE);
            } else if cell.is_block() && (cell.is_static() || phase_is_falling(cell.phase) || is_awaking(cell.phase)) {
                board.set(x, y, flavor_at(game, x as i32, gy) as i16);
            }
        }
    }
    board
}
