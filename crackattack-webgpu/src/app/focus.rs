//! Pure helpers for linear focus movement.
//!
//! Game convention: **vertical** navigation wraps (`ring_move`) and **horizontal**
//! navigation clamps (`clamp_move`). Exceptions: the on-screen keyboard wraps horizontally,
//! and scrollable vertical lists clamp.
//!
//! These functions are pure index arithmetic. They know nothing about screens, disabled
//! items or cooldowns; callers own those policies.

/// Moves **cyclically** in a list of `count` items (vertical navigation wraps).
///
/// - `count == 0` returns `current` unchanged (empty list, nothing to move).
/// - Any `|delta| >= 1` is accepted; the step crosses the edges as many times as needed.
///
/// The caller must guarantee `current < count`. Otherwise the result is in range but
/// meaningless, so clamp `current` BEFORE the call if the list may have shrunk.
pub(crate) fn ring_move(current: usize, count: usize, delta: i32) -> usize {
    if count == 0 {
        return current;
    }
    (current as i64 + delta as i64).rem_euclid(count as i64) as usize
}

/// Moves in a list of `count` items and **stops at the ends** (horizontal navigation clamps).
///
/// - `count == 0` returns `current` unchanged.
/// - Any `|delta| >= 1` is accepted; the result is clamped to the ends, never out of range.
pub(crate) fn clamp_move(current: usize, count: usize, delta: i32) -> usize {
    if count == 0 {
        return current;
    }
    (current as i64 + delta as i64).clamp(0, count as i64 - 1) as usize
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ring_wraps_forward_and_backward() {
        // Forward: last + 1 gives first.
        assert_eq!(ring_move(3, 4, 1), 0);
        // Backward: first - 1 gives last.
        assert_eq!(ring_move(0, 4, -1), 3);
        // Inner move, no wrap.
        assert_eq!(ring_move(1, 4, 1), 2);
        assert_eq!(ring_move(2, 4, -1), 1);
    }

    #[test]
    fn ring_handles_large_deltas() {
        // |delta| > 1 crosses the edges as many times as needed.
        assert_eq!(ring_move(2, 4, 5), 3); // 2 + 5 = 7 = 3 (mod 4)
        assert_eq!(ring_move(1, 4, -6), 3); // 1 - 6 = -5 = 3 (mod 4)
        assert_eq!(ring_move(0, 3, 3), 0); // full turn
    }

    #[test]
    fn ring_degenerate_counts() {
        // Empty list: `current` is unchanged (no panic, no division by zero).
        assert_eq!(ring_move(0, 0, 1), 0);
        assert_eq!(ring_move(7, 0, -1), 7);
        // Single item: any direction stays in place.
        assert_eq!(ring_move(0, 1, 1), 0);
        assert_eq!(ring_move(0, 1, -1), 0);
    }

    #[test]
    fn clamp_stops_at_edges() {
        // Lower and upper stops.
        assert_eq!(clamp_move(0, 4, -1), 0);
        assert_eq!(clamp_move(3, 4, 1), 3);
        // Inner move.
        assert_eq!(clamp_move(1, 4, 1), 2);
        assert_eq!(clamp_move(2, 4, -1), 1);
        // |delta| > 1 is clamped to the ends.
        assert_eq!(clamp_move(1, 4, 10), 3);
        assert_eq!(clamp_move(2, 4, -10), 0);
    }

    #[test]
    fn clamp_degenerate_counts() {
        assert_eq!(clamp_move(0, 0, 1), 0);
        assert_eq!(clamp_move(5, 0, -1), 5);
        assert_eq!(clamp_move(0, 1, 1), 0);
        assert_eq!(clamp_move(0, 1, -1), 0);
        // `current` out of range is brought back into the list by the clamp.
        assert_eq!(clamp_move(9, 4, 0), 3);
    }
}
