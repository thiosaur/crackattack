//! The planning board: a small grid of flavours, empties and garbage, with the queries the
//! planners share (legal swaps, segments, nearest colours, hashing, seeded tie ranks).

use crate::constants::BF_GRAY;

/// Logical cell codes (non-negative values are block flavours).
pub(crate) const PLAN_EMPTY: i16 = -1;

pub(crate) const PLAN_GARBAGE: i16 = -2;
/// A cell the planners must treat as fixed and inert: occupied, not a block (it matches nothing
/// and cannot be swapped), not garbage (it does not shatter). Awaking blocks are such cells.
pub(crate) const PLAN_BLOCKED: i16 = -3;

/// A lightweight, mutable copy of the board for cascade simulation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PlanBoard {
    /// Column-major cells: `cell[x * height + y]`, y = 0 at the bottom playable row.
    pub cell: Vec<i16>,
    pub width: usize,
    pub height: usize,
}

impl PlanBoard {
    /// An all-empty board.
    pub fn new(width: usize, height: usize) -> Self {
        PlanBoard {
            cell: vec![PLAN_EMPTY; width * height],
            width,
            height,
        }
    }

    /// The cell at `(x, y)`.
    pub fn get(&self, x: usize, y: usize) -> i16 {
        self.cell[x * self.height + y]
    }

    /// Overwrite the cell at `(x, y)`.
    pub fn set(&mut self, x: usize, y: usize, v: i16) {
        self.cell[x * self.height + y] = v;
    }

    /// Whether `(x,y)` holds a static block (a non-negative flavour).
    pub(super) fn is_block(&self, x: usize, y: usize) -> bool {
        self.get(x, y) >= 0
    }
}

/// Base (matchable) flavour of a block flavour. The Rust sim has no special flavours, so
/// this is the identity (the reference version collapses special colours onto normal ones).
pub(super) fn map_flavor_to_base_flavor(flavor: i16) -> i16 {
    flavor
}

/// Whether two block flavours match for elimination (non-X path): same base flavour, and
/// gray never matches (in the reference sim gray is not on the planning board).
pub(super) fn flavor_match(a: i16, b: i16) -> bool {
    let (a, b) = (map_flavor_to_base_flavor(a), map_flavor_to_base_flavor(b));
    a == b && i32::from(a) != BF_GRAY
}

/// Whether the two horizontally-adjacent cells `(x,y)`/`(x+1,y)` can be swapped: neither is
/// garbage, and at least one is a block (a block<->empty move counts; empty<->empty and
/// same-flavour swaps are no-ops the caller filters).
pub(crate) fn can_swap(b: &PlanBoard, x: usize, y: usize) -> bool {
    let a = b.get(x, y);
    let c = b.get(x + 1, y);
    let fixed = |v: i16| v == PLAN_GARBAGE || v == PLAN_BLOCKED;
    if fixed(a) || fixed(c) {
        return false;
    }
    a >= 0 || c >= 0
}

/// A deterministic pseudo-random rank used only to break equal planner choices.
/// Reproduces the reference 32-bit operations exactly.
pub(super) fn seeded_rank(seed: Option<u32>, words: &[usize]) -> u32 {
    let Some(seed) = seed else {
        return 0;
    };
    let mut h: u32 = seed;
    for &word in words {
        // (word + 1) | 0, then Math.imul
        let w = (word as i64 + 1) as i32 as u32;
        h ^= w.wrapping_mul(0x9e37_79b1);
        h ^= h >> 16;
        h = h.wrapping_mul(0x85eb_ca6b);
        h ^= h >> 13;
    }
    h
}

/// Whether `(x,y)` is 4-adjacent to a garbage cell.
pub(super) fn cell_touches_garbage(b: &PlanBoard, x: usize, y: usize) -> bool {
    (y + 1 < b.height && b.get(x, y + 1) == PLAN_GARBAGE)
        || (y > 0 && b.get(x, y - 1) == PLAN_GARBAGE)
        || (x + 1 < b.width && b.get(x + 1, y) == PLAN_GARBAGE)
        || (x > 0 && b.get(x - 1, y) == PLAN_GARBAGE)
}

/// The maximal contiguous run of blocks in row `y` containing `x`, or `None`.
pub(super) fn segment_around(b: &PlanBoard, x: usize, y: usize) -> Option<(usize, usize)> {
    if !b.is_block(x, y) {
        return None;
    }
    let mut s = x;
    while s > 0 && b.is_block(s - 1, y) {
        s -= 1;
    }
    let mut e = x;
    while e + 1 < b.width && b.is_block(e + 1, y) {
        e += 1;
    }
    Some((s, e))
}

/// Per base flavour, the nearest source to `x` within row `y`'s segment around `x`:
/// `(base flavour, (distance, position))`, in first-insertion order (like a JS `Map`).
/// The nearest position wins; the leftmost on ties, from the ascending scan.
pub(super) fn nearest_by_flavor(b: &PlanBoard, x: usize, y: usize) -> Vec<(i16, (usize, usize))> {
    let mut out: Vec<(i16, (usize, usize))> = Vec::new();
    let Some((s, e)) = segment_around(b, x, y) else {
        return out;
    };
    for p in s..=e {
        let base = map_flavor_to_base_flavor(b.get(p, y));
        let d = p.abs_diff(x);
        match out.iter_mut().find(|(k, _)| *k == base) {
            Some(entry) => {
                if d < entry.1 .0 {
                    entry.1 = (d, p);
                }
            }
            None => out.push((base, (d, p))),
        }
    }
    out
}

/// FNV-1a hash of a plan board's contents. Two boards with equal hash are (for all
/// practical purposes) identical, so a controller can cache board-pure plan results across
/// ticks while the cursor walks, recomputing only when the board actually changes. Any
/// collision would be deterministic too (same boards, same hash, on every client), so
/// lockstep safety is unaffected.
pub(crate) fn hash_plan_board(b: &PlanBoard) -> u32 {
    let mut h: u32 = 0x811c_9dc5;
    h = (h ^ b.width as u32).wrapping_mul(0x0100_0193);
    h = (h ^ b.height as u32).wrapping_mul(0x0100_0193);
    for &v in &b.cell {
        h = (h ^ u32::from(v as u16)).wrapping_mul(0x0100_0193);
    }
    h
}
