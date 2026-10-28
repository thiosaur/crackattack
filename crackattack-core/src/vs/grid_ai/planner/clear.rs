//! A plain clear a few swaps away: when no single swap clears anything, the shortest sequence of
//! swaps after which one does. Nothing is fired for its value; this is the "do something
//! useful" fallback for a board that is stuck (the stack is high and the board cannot be raised).

use super::board::*;
use super::cascade::*;
use super::garbage::SetupPlan;

/// The legal swaps of a board that change it: `(x, y)` in plan coordinates.
fn movable_swaps(board: &PlanBoard) -> Vec<(usize, usize)> {
    let mut swaps = Vec::new();
    for y in 0..board.height {
        for x in 0..board.width - 1 {
            if can_swap(board, x, y) && board.get(x, y) != board.get(x + 1, y) {
                swaps.push((x, y));
            }
        }
    }
    swaps
}

/// `board` after the swap, with everything in the air landed.
fn after_swap(board: &PlanBoard, x: usize, y: usize) -> PlanBoard {
    let mut b = board.clone();
    let tmp = b.get(x, y);
    b.set(x, y, b.get(x + 1, y));
    b.set(x + 1, y, tmp);
    settled(&b)
}

/// The most blocks a last swap clears from `board` (0 when none triggers anything).
fn best_clear(board: &PlanBoard) -> i32 {
    movable_swaps(board)
        .into_iter()
        .map(|(x, y)| evaluate_swap(board, x, y).total_cleared)
        .max()
        .unwrap_or(0)
}

/// The first swap of the shortest sequence (at most `max_swaps` swaps, the last one clearing
/// something) on a board where no single swap clears. Among sequences of the same length, the one
/// that clears most; equal ones are separated by the seeded rank. `cost` is the sequence length.
pub(crate) fn plan_clear_setup(board: &PlanBoard, max_swaps: i32, tie_seed: Option<u32>) -> Option<SetupPlan> {
    let first_swaps = movable_swaps(board);
    // Boards after one swap, kept for the deeper levels.
    let level1: Vec<((usize, usize), PlanBoard)> =
        first_swaps.iter().map(|&(x, y)| ((x, y), after_swap(board, x, y))).collect();
    let mut best: Option<(i32, u32, (usize, usize))> = None;
    let consider = |best: &mut Option<(i32, u32, (usize, usize))>, cleared: i32, first: (usize, usize)| {
        let rank = seeded_rank(tie_seed, &[first.0, first.1, 0x434c_5352]);
        if best.map_or(true, |(c, r, _)| cleared > c || (cleared == c && rank < r)) {
            *best = Some((cleared, rank, first));
        }
    };
    if max_swaps >= 2 {
        for (first, b1) in &level1 {
            let cleared = best_clear(b1);
            if cleared > 0 {
                consider(&mut best, cleared, *first);
            }
        }
        if let Some((_, _, (x, y))) = best {
            return Some(SetupPlan { x, y, cost: 2 });
        }
    }
    if max_swaps >= 3 {
        for (first, b1) in &level1 {
            for (x, y) in movable_swaps(b1) {
                let b2 = after_swap(b1, x, y);
                let cleared = best_clear(&b2);
                if cleared > 0 {
                    consider(&mut best, cleared, *first);
                }
            }
        }
        if let Some((_, _, (x, y))) = best {
            return Some(SetupPlan { x, y, cost: 3 });
        }
    }
    None
}
