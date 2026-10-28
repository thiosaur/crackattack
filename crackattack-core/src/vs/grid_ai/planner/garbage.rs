//! Searches against garbage: assemble a match that touches a slab (shatter set-up), or dig out
//! the support a slab rests on (undermine).

use super::board::*;
use super::cascade::*;

/// The next step of a multi-swap garbage-shattering setup.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct SetupPlan {
    /// The next lateral swap to make: `(x,y)`<->`(x+1,y)`, in plan coordinates.
    pub x: usize,
    pub y: usize,
    /// Lateral swaps the full plan still needs from here (>= 1).
    pub cost: i32,
}

/// A winning candidate: how to assemble a 3-run in a garbage-adjacent window.
#[derive(Clone, Copy)]
pub(super) enum SetupCandidate {
    Horizontal { y: usize, x0: usize, sources: [usize; 3] },
    Vertical { x: usize, y0: usize, positions: [usize; 3] },
}

/// Goal-directed defensive planning: when no *single* swap shatters garbage, find the
/// cheapest **sequence of lateral swaps** that assembles a 3-run of one colour in a window
/// 4-adjacent to a garbage slab (under it, on top of it, or beside it), the moves a human
/// makes when buried.
///
/// Two window orientations are searched:
///  - **horizontal**: 3 adjacent cells in one row, filled by shuttling three same-colour
///    blocks along that row's contiguous block segment;
///  - **vertical**: 3 stacked cells in one column, each row supplying one same-colour block
///    laterally from its own segment (the classic "stack a colour against the slab"
///    technique, much more often available since a row only needs *one* matching block).
///
/// Only block<->block swaps inside fully-occupied row segments are considered: those never
/// change column occupancy, so they are gravity-neutral. The plan cannot destabilise the
/// stack, and re-planning every action tick converges (each executed swap reduces the
/// remaining cost by exactly 1 while the board is otherwise still, so the best plan's cost
/// strictly decreases; a rising or settling board simply re-plans). The final
/// run-completing swap needs no special case: once a plan is one swap from done, the
/// strategic planner's fire branch sees a swap whose cascade shatters garbage and takes it.
///
/// Colours group by base flavour, matching the elimination rules. Horizontal windows take
/// the cheapest order-preserving assignment of three sources to the window slots (brute
/// force; segments are at most 6 wide, so at most 20 subsets); vertical windows take each
/// row's nearest source. The next move is chosen so it can never swap two matching blocks
/// (for horizontal, a matching neighbour would have been a strictly cheaper source of the
/// *minimal* subset; for vertical, the cell between the nearest source and the target would
/// have been nearer), so execution always makes progress. A cost-0 window is an
/// already-standing match the sim is about to clear, so it is skipped: the planner never
/// plans around transient states.
///
/// Deterministic: candidates scan bottom-up / left-to-right (horizontal rows first) and
/// ties keep the first find, so lower windows win. Returns the next swap plus remaining
/// cost, or `None` if no plan costs <= `max_cost`.
pub(crate) fn plan_shatter_setup(board: &PlanBoard, max_cost: i32) -> Option<SetupPlan> {
    let mut best_cost = i64::from(max_cost) + 1;
    let mut best: Option<SetupCandidate> = None;

    // --- Horizontal windows, rows bottom-up. ---
    for y in 0..board.height {
        let mut seg_start = 0;
        while seg_start < board.width {
            if !board.is_block(seg_start, y) {
                seg_start += 1;
                continue;
            }
            let mut seg_end = seg_start;
            while seg_end + 1 < board.width && board.is_block(seg_end + 1, y) {
                seg_end += 1;
            }

            if seg_end - seg_start >= 2 {
                // Source positions per base flavour within the segment (ascending x),
                // in first-insertion order like the JS Map.
                let mut groups: Vec<(i16, Vec<usize>)> = Vec::new();
                for x in seg_start..=seg_end {
                    let base = map_flavor_to_base_flavor(board.get(x, y));
                    match groups.iter_mut().find(|(k, _)| *k == base) {
                        Some((_, g)) => g.push(x),
                        None => groups.push((base, vec![x])),
                    }
                }
                for x0 in seg_start..=(seg_end - 2) {
                    if !cell_touches_garbage(board, x0, y)
                        && !cell_touches_garbage(board, x0 + 1, y)
                        && !cell_touches_garbage(board, x0 + 2, y)
                    {
                        continue;
                    }
                    for (_, sources) in &groups {
                        if sources.len() < 3 {
                            continue;
                        }
                        // Cheapest order-preserving assignment of any 3 sources to the
                        // window slots x0..x0+2. The *minimal* subset must be taken: the
                        // progress guarantee (no matching-blocks swap) relies on it.
                        let mut wf_cost = i64::MAX;
                        let mut wf_sources: Option<[usize; 3]> = None;
                        let n = sources.len();
                        for i in 0..(n - 2) {
                            for j in (i + 1)..(n - 1) {
                                for k in (j + 1)..n {
                                    let cost = sources[i].abs_diff(x0)
                                        + sources[j].abs_diff(x0 + 1)
                                        + sources[k].abs_diff(x0 + 2);
                                    let cost = cost as i64;
                                    if cost < wf_cost {
                                        wf_cost = cost;
                                        wf_sources = Some([sources[i], sources[j], sources[k]]);
                                    }
                                }
                            }
                        }
                        let Some(found) = wf_sources else {
                            continue;
                        };
                        if wf_cost == 0 || wf_cost >= best_cost {
                            continue;
                        }
                        best_cost = wf_cost;
                        best = Some(SetupCandidate::Horizontal { y, x0, sources: found });
                    }
                }
            }
            seg_start = seg_end + 1; // step past the segment
        }
    }

    // --- Vertical windows, columns left-to-right, bottom-up. ---
    for x in 0..board.width {
        let mut y0 = 0;
        while y0 + 2 < board.height {
            let cur = y0;
            y0 += 1;
            if !board.is_block(x, cur) || !board.is_block(x, cur + 1) || !board.is_block(x, cur + 2) {
                continue;
            }
            if !cell_touches_garbage(board, x, cur)
                && !cell_touches_garbage(board, x, cur + 1)
                && !cell_touches_garbage(board, x, cur + 2)
            {
                continue;
            }
            // Each row supplies its nearest block of the candidate colour laterally.
            let rows = [
                nearest_by_flavor(board, x, cur),
                nearest_by_flavor(board, x, cur + 1),
                nearest_by_flavor(board, x, cur + 2),
            ];
            for &(base, (d0, p0)) in &rows[0] {
                let r1 = rows[1].iter().find(|(k, _)| *k == base).map(|e| e.1);
                let r2 = rows[2].iter().find(|(k, _)| *k == base).map(|e| e.1);
                let (Some(r1), Some(r2)) = (r1, r2) else {
                    continue;
                };
                let cost = (d0 + r1.0 + r2.0) as i64;
                if cost == 0 || cost >= best_cost {
                    continue; // standing match / not better
                }
                best_cost = cost;
                best = Some(SetupCandidate::Vertical {
                    x,
                    y0: cur,
                    positions: [p0, r1.1, r2.1],
                });
            }
        }
    }

    let best = best?;
    let cost = best_cost as i32;
    match best {
        SetupCandidate::Horizontal { y, x0, sources } => {
            let slots = [x0, x0 + 1, x0 + 2];
            // Rightmost source that must move right, else leftmost that must move left.
            for i in (0..3).rev() {
                if sources[i] < slots[i] {
                    return Some(SetupPlan { x: sources[i], y, cost });
                }
            }
            for i in 0..3 {
                if sources[i] > slots[i] {
                    return Some(SetupPlan {
                        x: sources[i] - 1,
                        y,
                        cost,
                    });
                }
            }
            None // cost > 0 implies some source is out of place
        }
        SetupCandidate::Vertical { x, y0, positions } => {
            // The lowest row whose source is out of place steps toward x.
            for (i, &p) in positions.iter().enumerate() {
                if p < x {
                    return Some(SetupPlan { x: p, y: y0 + i, cost });
                }
                if p > x {
                    return Some(SetupPlan {
                        x: p - 1,
                        y: y0 + i,
                        cost,
                    });
                }
            }
            None // cost > 0 implies some source is out of place
        }
    }
}

/// The defensive fallback when no shatter setup exists at all: garbage is often perched on
/// a narrow tower of blocks (it lands on the tallest column), where no row has the width to
/// assemble a match. The human technique is to *dismantle the tower*: dig its load-bearing
/// blocks sideways into the neighbouring gap so they fall away and the slab descends, row
/// by row, onto the wider stack, where [`plan_shatter_setup`] takes over.
///
/// The direct candidate is a dig swap (a load-bearing block moved into an empty cell it can
/// fall through). If a sole slab support has an adjacent empty cell but the cell below that
/// pocket is occupied, the planner first slides that lower blocker aside; the next pass can
/// pull the support down one row. Direct digs win, then cursor distance; an optional seed
/// ranks exact ties. Returns a swap in plan coordinates, or `None`.
pub(crate) fn plan_undermine(
    board: &PlanBoard,
    cursor_x: i32,
    cursor_y: i32,
    tie_seed: Option<u32>,
) -> Option<(usize, usize)> {
    let mut best: Option<(usize, usize)> = None;
    let mut best_dist = i64::MAX;
    let mut best_rank = 0u32;
    let empty = |x: usize, y: usize| board.get(x, y) == PLAN_EMPTY;
    let supports_garbage = |x: usize, y: usize| -> bool {
        for yy in (y + 1)..board.height {
            let v = board.get(x, yy);
            if v == PLAN_GARBAGE {
                return true;
            }
            if v < 0 {
                return false; // a gap: nothing above rests on this block
            }
        }
        false
    };
    let distance =
        |x: usize, y: usize| -> i64 { (x as i64 - i64::from(cursor_x)).abs() + (y as i64 - i64::from(cursor_y)).abs() };
    for y in 1..board.height {
        for x in 0..board.width - 1 {
            // Exactly one side is a load-bearing block, the other an empty the block can
            // drop through (empty below the destination).
            let dig_right = board.is_block(x, y) && empty(x + 1, y) && empty(x + 1, y - 1) && supports_garbage(x, y);
            let dig_left = board.is_block(x + 1, y) && empty(x, y) && empty(x, y - 1) && supports_garbage(x + 1, y);
            if !dig_right && !dig_left {
                continue;
            }
            let dist = distance(x, y);
            let rank = seeded_rank(tie_seed, &[0x444947, x, y]);
            if dist < best_dist || (dist == best_dist && tie_seed.is_some() && rank > best_rank) {
                best_dist = dist;
                best_rank = rank;
                best = Some((x, y));
            }
        }
    }
    if best.is_some() {
        return best;
    }

    // A common perched-slab shape has an empty cell beside its sole support, but a block
    // immediately below that destination. A human first moves that lower blocker sideways,
    // creating a one-row-deeper pocket; the next planning pass can then move the support
    // into the pocket and let the slab descend.
    let h = board.height;
    let sole_support = |x: usize, y: usize| -> bool {
        if y + 1 >= h || board.get(x, y + 1) != PLAN_GARBAGE {
            return false;
        }
        let mut seen = vec![false; board.cell.len()];
        let mut pending = vec![x * h + y + 1];
        let mut supports: Vec<usize> = Vec::new();
        while let Some(idx) = pending.pop() {
            if seen[idx] {
                continue;
            }
            seen[idx] = true;
            let gx = idx / h;
            let gy = idx % h;
            if gy > 0 && board.is_block(gx, gy - 1) {
                let s = gx * h + gy - 1;
                if !supports.contains(&s) {
                    supports.push(s);
                }
            }
            let neighbours = [
                (gx as i64 - 1, gy as i64),
                (gx as i64 + 1, gy as i64),
                (gx as i64, gy as i64 - 1),
                (gx as i64, gy as i64 + 1),
            ];
            for (nx, ny) in neighbours {
                if nx >= 0
                    && (nx as usize) < board.width
                    && ny >= 0
                    && (ny as usize) < h
                    && board.get(nx as usize, ny as usize) == PLAN_GARBAGE
                {
                    pending.push(nx as usize * h + ny as usize);
                }
            }
        }
        supports.len() == 1 && supports[0] == x * h + y
    };

    best = None;
    best_dist = i64::MAX;
    best_rank = 0;
    for y in 1..board.height.saturating_sub(1) {
        for support_x in 0..board.width {
            if !board.is_block(support_x, y) || !sole_support(support_x, y) {
                continue;
            }
            for dir in [-1i64, 1] {
                let pocket = support_x as i64 + dir;
                if pocket < 0 || pocket >= board.width as i64 {
                    continue;
                }
                let pocket_x = pocket as usize;
                if !empty(pocket_x, y) || !board.is_block(pocket_x, y - 1) {
                    continue;
                }
                for clear_dir in [-1i64, 1] {
                    let clear = pocket_x as i64 + clear_dir;
                    if clear < 0 || clear >= board.width as i64 {
                        continue;
                    }
                    let clear_x = clear as usize;
                    if !empty(clear_x, y - 1) {
                        continue;
                    }
                    let swap_x = pocket_x.min(clear_x);
                    if makes_run3(board, swap_x, y - 1) {
                        continue;
                    }
                    let dist = distance(swap_x, y - 1);
                    let rank = seeded_rank(tie_seed, &[0x50554c4c, swap_x, y - 1]);
                    if dist < best_dist || (dist == best_dist && tie_seed.is_some() && rank > best_rank) {
                        best = Some((swap_x, y - 1));
                        best_dist = dist;
                        best_rank = rank;
                    }
                }
            }
        }
    }
    best
}
