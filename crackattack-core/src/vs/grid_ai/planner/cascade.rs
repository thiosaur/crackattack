//! The cascade simulator: gravity, runs of three, shattering, repeated until stable. A pure
//! function of the board; it approximates the engine (see the module doc of `planner`).

use crate::constants::PLAY_WIDTH;

use super::board::*;

/// The outcome of simulating one candidate swap's cascade.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Cascade {
    /// Match rounds triggered (about the combo multiplier; round 1 isn't a chain link).
    pub chain_depth: i32,
    /// Total blocks eliminated across all rounds (about the combo magnitude).
    pub total_cleared: i32,
    /// Largest single round's elimination (drives width garbage).
    pub max_round: i32,
    /// Garbage cells shattered.
    pub garbage_shattered: i32,
}

/// Settle every block straight down onto the floor / garbage / the block below.
pub(super) fn apply_gravity(b: &mut PlanBoard) {
    for x in 0..b.width {
        // `write` is the next free row from the bottom of the current segment.
        // Garbage and blocked cells are fixed (blocks rest on them), so they reset the segment.
        let mut write = 0;
        for y in 0..b.height {
            let v = b.get(x, y);
            if v == PLAN_GARBAGE || v == PLAN_BLOCKED {
                write = y + 1; // blocks above land on top of the garbage
            } else if v >= 0 {
                if write != y {
                    b.set(x, write, v);
                    b.set(x, y, PLAN_EMPTY);
                }
                write += 1;
            }
            // empty cells are skipped; their slot is reused by a falling block above
        }
    }
}

/// The board once everything in the air has landed (nothing matched yet).
pub(crate) fn settled(board: &PlanBoard) -> PlanBoard {
    let mut b = board.clone();
    apply_gravity(&mut b);
    b
}

/// Find every cell in a 3+ run (horizontal or vertical) of matching flavours.
/// Returns the marked cells (a mask indexed by `x*height+y`), their count, and the largest
/// single run.
pub(super) fn find_matches(b: &PlanBoard) -> (Vec<bool>, i32, i32) {
    let mut marked = vec![false; b.cell.len()];
    let mut count = 0;
    let mut largest = 0;
    let mut mark = |idx: usize, marked: &mut Vec<bool>| {
        if !marked[idx] {
            marked[idx] = true;
            count += 1;
        }
    };
    // Horizontal runs.
    for y in 0..b.height {
        let mut run = 1;
        for x in 1..=b.width {
            let same =
                x < b.width && b.is_block(x, y) && b.is_block(x - 1, y) && flavor_match(b.get(x, y), b.get(x - 1, y));
            if same {
                run += 1;
            } else {
                if run >= 3 {
                    largest = largest.max(run as i32);
                    for k in (x - run)..x {
                        mark(k * b.height + y, &mut marked);
                    }
                }
                run = 1;
            }
        }
    }
    // Vertical runs.
    for x in 0..b.width {
        let mut run = 1;
        for y in 1..=b.height {
            let same =
                y < b.height && b.is_block(x, y) && b.is_block(x, y - 1) && flavor_match(b.get(x, y), b.get(x, y - 1));
            if same {
                run += 1;
            } else {
                if run >= 3 {
                    largest = largest.max(run as i32);
                    for k in (y - run)..y {
                        mark(x * b.height + k, &mut marked);
                    }
                }
                run = 1;
            }
        }
    }
    (marked, count, largest)
}

/// Remove garbage cells 4-adjacent to any matched cell; return how many.
pub(super) fn shatter_adjacent(b: &mut PlanBoard, marked: &[bool]) -> i32 {
    let mut to_clear: Vec<usize> = Vec::new();
    let h = b.height;
    for x in 0..b.width {
        for y in 0..h {
            if b.get(x, y) != PLAN_GARBAGE {
                continue;
            }
            let touches = (x > 0 && marked[(x - 1) * h + y])
                || (x + 1 < b.width && marked[(x + 1) * h + y])
                || (y > 0 && marked[x * h + (y - 1)])
                || (y + 1 < h && marked[x * h + (y + 1)]);
            if touches {
                to_clear.push(x * h + y);
            }
        }
    }
    for &idx in &to_clear {
        b.cell[idx] = PLAN_EMPTY;
    }
    to_clear.len() as i32
}

/// Run the cascade on `b` (mutated): settle gravity, clear 3+ runs, repeat until stable.
/// A board that triggers nothing returns an all-zero cascade.
pub(super) fn evaluate_cascade(b: &mut PlanBoard) -> Cascade {
    let mut result = Cascade::default();
    loop {
        apply_gravity(b);
        let (marked, count, largest) = find_matches(b);
        if count == 0 {
            break;
        }
        result.chain_depth += 1;
        result.total_cleared += count;
        result.max_round = result.max_round.max(largest);
        result.garbage_shattered += shatter_adjacent(b, &marked);
        for (idx, &m) in marked.iter().enumerate() {
            if m {
                b.cell[idx] = PLAN_EMPTY;
            }
        }
    }
    result
}

/// Settle the board without a swap and report the cascade gravity alone creates.
pub(crate) fn evaluate_gravity(board: &PlanBoard) -> Cascade {
    evaluate_cascade(&mut board.clone())
}

/// Simulate the cascade after swapping `(x,y)`<->`(x+1,y)` on a copy of `board`.
/// Returns the chain depth, blocks cleared, largest round, and garbage shattered.
pub(crate) fn evaluate_swap(board: &PlanBoard, x: usize, y: usize) -> Cascade {
    let mut b = board.clone();
    let tmp = b.get(x, y);
    b.set(x, y, b.get(x + 1, y));
    b.set(x + 1, y, tmp);
    evaluate_cascade(&mut b)
}

/// Attack value of a cascade, in units of "garbage rows sent", per the generator: each
/// chain link past the first ships a full-width row (`multiplier - 1`), and a combo whose
/// total exceeds 3 ships width garbage (about `total - 3` cells, a fraction of a row).
/// Chains dominate, exactly as in real play.
pub(crate) fn attack_value(c: &Cascade) -> i32 {
    let chain_rows = (c.chain_depth - 1).max(0);
    let width_cells = (c.total_cleared - 3).max(0);
    chain_rows * PLAY_WIDTH as i32 + width_cells
}

/// Whether swapping `(x,y)`<->`(x+1,y)` immediately completes a 3+ run, checked statically
/// against post-swap flavours (base-flavour matching). On a settled board (which a
/// `readPlanBoard` snapshot always is, and which gravity-neutral setup swaps preserve) this
/// is **exact** for a cascade's first round when the swap doesn't drop a block, so it
/// serves as a cheap prefilter that lets the full [`evaluate_swap`] run only on genuine
/// trigger candidates.
pub(super) fn makes_run3(b: &PlanBoard, x: usize, y: usize) -> bool {
    let va = b.get(x, y);
    let vb = b.get(x + 1, y);
    // Post-swap value at a cell (the two swapped cells exchange).
    let val = |cx: usize, cy: usize| -> i16 {
        if cy == y && cx == x {
            vb
        } else if cy == y && cx == x + 1 {
            va
        } else {
            b.get(cx, cy)
        }
    };
    let matches = |v: i16, base: i16| v >= 0 && map_flavor_to_base_flavor(v) == base;

    for (ax, v) in [(x, vb), (x + 1, va)] {
        if v < 0 {
            continue; // the cell became empty: anchors nothing
        }
        let base = map_flavor_to_base_flavor(v);
        let mut run = 1;
        let mut cx = ax;
        while cx > 0 && matches(val(cx - 1, y), base) {
            run += 1;
            cx -= 1;
        }
        cx = ax + 1;
        while cx < b.width && matches(val(cx, y), base) {
            run += 1;
            cx += 1;
        }
        if run >= 3 {
            return true;
        }
        run = 1;
        let mut cy = y;
        while cy > 0 && matches(val(ax, cy - 1), base) {
            run += 1;
            cy -= 1;
        }
        cy = y + 1;
        while cy < b.height && matches(val(ax, cy), base) {
            run += 1;
            cy += 1;
        }
        if run >= 3 {
            return true;
        }
    }
    false
}
