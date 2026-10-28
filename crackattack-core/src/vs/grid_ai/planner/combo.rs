//! The crossed-column x8/x10 combo construction.

use super::board::*;
use super::cascade::*;

/// The next step toward a crossed-column combo worth 2*height blocks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct BigComboSetupPlan {
    pub x: usize,
    pub y: usize,
    pub cost: i32,
    /// Blocks the finished trigger clears (8 for four rows, 10 for five).
    pub size: i32,
}

/// Minimum adjacent swaps needed to put `left_flavor,right_flavor` at `(x,x+1)` in one
/// fully occupied row segment. The rest of the row may end in any order. `None` when the
/// pair cannot be made (the JS `Infinity`).
pub(super) fn row_pair_cost(board: &PlanBoard, x: usize, y: usize, left_flavor: i16, right_flavor: i16) -> Option<i32> {
    let (s, e) = segment_around(board, x, y)?;
    if x + 1 > e {
        return None;
    }
    let mut best: Option<i32> = None;
    for left in s..=e {
        if map_flavor_to_base_flavor(board.get(left, y)) != left_flavor {
            continue;
        }
        for right in s..=e {
            if map_flavor_to_base_flavor(board.get(right, y)) != right_flavor {
                continue;
            }
            // If the sources are reversed, the one adjacent swap where they cross moves
            // both into place, so the two individual distances double-count it.
            let cost = left.abs_diff(x) as i32 + right.abs_diff(x + 1) as i32 - i32::from(left > right);
            if best.is_none_or(|b| cost < b) {
                best = Some(cost);
            }
        }
    }
    best
}

/// A crossed-column target (without its cost).
#[derive(Clone, Copy)]
pub(super) struct BigComboTarget {
    x: usize,
    y0: usize,
    height: usize,
    left_flavor: i16,
    right_flavor: i16,
}

#[derive(Clone, Copy)]
pub(super) struct BigComboCandidate {
    target: BigComboTarget,
    cost: i32,
}

/// Remaining lateral-swap cost for one crossed-column target (`None` = unreachable).
pub(super) fn big_combo_cost(board: &PlanBoard, t: &BigComboTarget) -> Option<i32> {
    let trigger_row = t.height / 2;
    let mut cost = 0;
    for row in 0..t.height {
        let crossed = row == trigger_row;
        let row_cost = row_pair_cost(
            board,
            t.x,
            t.y0 + row,
            if crossed { t.right_flavor } else { t.left_flavor },
            if crossed { t.left_flavor } else { t.right_flavor },
        )?;
        cost += row_cost;
    }
    Some(cost)
}

/// Find a non-clearing adjacent swap that reduces a crossed-column plan by one step.
pub(super) fn next_big_combo_swap(
    board: &PlanBoard,
    candidate: &BigComboCandidate,
    tie_seed: Option<u32>,
) -> Option<(usize, usize)> {
    let t = &candidate.target;
    let mut best: Option<(usize, usize)> = None;
    let mut best_rank = 0u32;
    let mut scratch = board.clone();
    for y in t.y0..t.y0 + t.height {
        for x in 0..board.width - 1 {
            let a = board.get(x, y);
            let c = board.get(x + 1, y);
            if a < 0 || c < 0 || map_flavor_to_base_flavor(a) == map_flavor_to_base_flavor(c) {
                continue;
            }
            // Setup moves must not accidentally fire a smaller clear and destroy the
            // construction. The final crossed-row trigger belongs to the fire branch.
            if makes_run3(board, x, y) {
                continue;
            }
            scratch.cell.copy_from_slice(&board.cell);
            scratch.set(x, y, c);
            scratch.set(x + 1, y, a);
            if big_combo_cost(&scratch, t) != Some(candidate.cost - 1) {
                continue;
            }
            let rank = seeded_rank(tie_seed, &[0x424947, x, y]);
            if best.is_none() || (tie_seed.is_some() && rank > best_rank) {
                best = Some((x, y));
                best_rank = rank;
            }
        }
    }
    best
}

/// Build the human crossed-column combo: two adjacent columns of different colours, four or
/// five blocks tall, with their centre pair crossed. Swapping that pair completes both
/// columns at once for an x8 or x10 clear.
///
/// Setup moves are lateral block<->block swaps inside occupied rows, so gravity cannot
/// invalidate the target. Every returned move reduces the exact adjacent-swap cost by one
/// and completes no match itself; once the construction is ready, the normal fire search
/// sees and executes the centre trigger. Five-row x10 targets outrank four-row x8 targets,
/// then lower cost wins. A seeded rank breaks otherwise equal targets without changing
/// reproducibility.
pub(crate) fn plan_big_combo_setup(
    board: &PlanBoard,
    max_cost: i32,
    tie_seed: Option<u32>,
) -> Option<BigComboSetupPlan> {
    let mut flavors: Vec<i16> = Vec::new();
    for &v in &board.cell {
        if v >= 0 {
            let base = map_flavor_to_base_flavor(v);
            if !flavors.contains(&base) {
                flavors.push(base);
            }
        }
    }
    flavors.sort_unstable();

    struct Best {
        candidate: BigComboCandidate,
        next: (usize, usize),
        rank: u32,
    }
    let mut best: Option<Best> = None;
    let max_height = 5.min(board.height);
    let mut height = max_height;
    while height >= 4 {
        let mut y0 = 0;
        while y0 + height <= board.height {
            for x in 0..board.width - 1 {
                for &left_flavor in &flavors {
                    for &right_flavor in &flavors {
                        if left_flavor == right_flavor {
                            continue;
                        }
                        let target = BigComboTarget {
                            x,
                            y0,
                            height,
                            left_flavor,
                            right_flavor,
                        };
                        let Some(cost) = big_combo_cost(board, &target) else {
                            continue;
                        };
                        if cost <= 0 || cost > max_cost {
                            continue;
                        }
                        let candidate = BigComboCandidate { target, cost };
                        let Some(next) = next_big_combo_swap(board, &candidate, tie_seed) else {
                            continue;
                        };
                        let rank = seeded_rank(
                            tie_seed,
                            &[0x583130, x, y0, left_flavor as usize, right_flavor as usize],
                        );
                        let better = match &best {
                            None => true,
                            Some(b) => {
                                height > b.candidate.target.height
                                    || (height == b.candidate.target.height && cost < b.candidate.cost)
                                    || (height == b.candidate.target.height
                                        && cost == b.candidate.cost
                                        && tie_seed.is_some()
                                        && rank > b.rank)
                            }
                        };
                        if better {
                            best = Some(Best { candidate, next, rank });
                        }
                    }
                }
            }
            y0 += 1;
        }
        height -= 1;
    }
    best.map(|b| BigComboSetupPlan {
        x: b.next.0,
        y: b.next.1,
        cost: b.candidate.cost,
        size: b.candidate.target.height as i32 * 2,
    })
}
