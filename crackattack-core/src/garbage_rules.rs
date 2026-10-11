// Garbage calculation rules

use crate::constants::*;
use crate::garbage::GARBAGE_FLAVOR_GRAY;

// Flavors are defined once, in garbage.rs.
pub(crate) use crate::garbage::GARBAGE_FLAVOR_NORMAL;

/// Garbage to send (height, width, flavor)
#[derive(Debug, Clone, Copy)]
pub(crate) struct GarbageSend {
    pub height: i32,
    pub width: i32,
    pub flavor: i32,
}

/// Calculate garbage from an elimination event
/// Returns list of garbage to send
///
/// - Gray: if specMag >= 3, sends (specMag - 2) gray rows of full width
/// - Normal: if mag > 3, sends garbage of width (mag-1), splits if needed
///
/// Same rules as `GarbageGenerator::comboElimination` of the original game (PLAY_WIDTH = 6,
/// MIN_PATTERN_LENGTH = 3): up to 6 blocks one piece, 7 to 10 two pieces, 11 and more pieces
/// of width 5 plus a remainder when it is at least 3.
pub(crate) fn calculate_elimination_garbage(
    magnitude: i32,
    special_magnitude: i32,
) -> Vec<GarbageSend> {
    let mut result = Vec::new();

    // Gray garbage from special (gray block) eliminations
    if special_magnitude >= MIN_PATTERN_LENGTH {
        let mut spec_mag = special_magnitude - MIN_PATTERN_LENGTH + 2;
        while spec_mag > 1 {
            spec_mag -= 1;
            // Special garbage: gray row of full width
            result.push(GarbageSend {
                height: 1,
                width: PLAY_WIDTH as i32,
                flavor: GARBAGE_FLAVOR_GRAY,
            });
        }
    }

    // Normal garbage from regular eliminations
    if magnitude > MIN_PATTERN_LENGTH {
        let mut mag = magnitude;

        if mag <= PLAY_WIDTH as i32 {
            // Single piece of garbage
            result.push(GarbageSend {
                height: 1,
                width: mag - 1,
                flavor: GARBAGE_FLAVOR_NORMAL,
            });
        } else if mag < 2 * PLAY_WIDTH as i32 - 1 {
            // Split into two pieces
            result.push(GarbageSend {
                height: 1,
                width: mag - (mag >> 1),
                flavor: GARBAGE_FLAVOR_NORMAL,
            });
            result.push(GarbageSend {
                height: 1,
                width: mag >> 1,
                flavor: GARBAGE_FLAVOR_NORMAL,
            });
        } else {
            // Very large: split into multiple full-width pieces
            mag += MIN_PATTERN_LENGTH;
            while mag > PLAY_WIDTH as i32 - 1 {
                result.push(GarbageSend {
                    height: 1,
                    width: PLAY_WIDTH as i32 - 1,
                    flavor: GARBAGE_FLAVOR_NORMAL,
                });
                mag -= PLAY_WIDTH as i32 - 1;
            }

            if mag >= MIN_PATTERN_LENGTH {
                result.push(GarbageSend {
                    height: 1,
                    width: mag,
                    flavor: GARBAGE_FLAVOR_NORMAL,
                });
            }
        }
    }

    result
}

/// Calculate garbage from combo completion
/// Returns garbage to send (if any)
///
/// - If multiplier > 1: sends garbage of height (multiplier - 1) and full width
pub(crate) fn calculate_completion_garbage(multiplier: i32) -> Option<GarbageSend> {
    if multiplier > 1 {
        Some(GarbageSend {
            height: multiplier - 1,
            width: PLAY_WIDTH as i32,
            flavor: GARBAGE_FLAVOR_NORMAL,
        })
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_no_garbage_for_3_match() {
        // 3-match doesn't generate garbage
        let result = calculate_elimination_garbage(3, 0);
        assert!(result.is_empty());
    }

    #[test]
    fn test_4_match_generates_width_3() {
        // 4-match generates width 3 (4-1)
        let result = calculate_elimination_garbage(4, 0);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].width, 3);
        assert_eq!(result[0].height, 1);
    }

    #[test]
    fn test_5_match_generates_width_4() {
        // 5-match generates width 4 (5-1)
        let result = calculate_elimination_garbage(5, 0);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].width, 4);
    }

    #[test]
    fn test_gray_elimination() {
        // 4 gray blocks: specMag=4, generates (4-3+2-1) = 2 gray rows? No, let me recalculate
        // spec_mag = 4 - 3 + 2 = 3, then while (3 > 1) -> send, 3-1=2 -> while (2>1) -> send, 2-1=1 -> stop
        // So 2 gray rows
        let result = calculate_elimination_garbage(0, 4);
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].flavor, GARBAGE_FLAVOR_GRAY);
        assert_eq!(result[0].width, PLAY_WIDTH as i32);
    }

    /// Widths of the pieces sent for a normal elimination of `mag` blocks.
    fn widths(mag: i32) -> Vec<i32> {
        calculate_elimination_garbage(mag, 0).iter().map(|g| g.width).collect()
    }

    #[test]
    fn test_6_match_is_a_single_piece_of_width_5() {
        assert_eq!(widths(6), [5]);
    }

    #[test]
    fn test_7_to_10_match_is_split_in_two_pieces() {
        // Hand-computed from the original rule: first = m - (m >> 1), second = m >> 1.
        assert_eq!(widths(7), [4, 3]);
        assert_eq!(widths(8), [4, 4]);
        assert_eq!(widths(9), [5, 4]);
        assert_eq!(widths(10), [5, 5]);
        for m in 7..=10 {
            let r = calculate_elimination_garbage(m, 0);
            assert!(r.iter().all(|g| g.height == 1 && g.flavor == GARBAGE_FLAVOR_NORMAL), "m={m}");
        }
    }

    #[test]
    fn test_11_match_and_more_use_pieces_of_width_5() {
        // mag += 3, then pieces of 5 while mag > 5, then the remainder if it is >= 3.
        assert_eq!(widths(11), [5, 5, 4]); // 14 -> 9 -> 4
        assert_eq!(widths(12), [5, 5, 5]); // 15 -> 10 -> 5
        assert_eq!(widths(13), [5, 5, 5]); // 16 -> 11 -> 6 -> 1: remainder dropped
        assert_eq!(widths(14), [5, 5, 5]); // 17 -> 12 -> 7 -> 2: remainder dropped
        assert_eq!(widths(15), [5, 5, 5, 3]); // 18 -> 13 -> 8 -> 3
        assert_eq!(widths(16), [5, 5, 5, 4]); // 19 -> 14 -> 9 -> 4
    }

    #[test]
    fn test_gray_rows_are_spec_magnitude_minus_2() {
        let n = |spec| calculate_elimination_garbage(0, spec).len();
        assert_eq!(n(2), 0, "below the minimum pattern length");
        assert_eq!(n(3), 1);
        assert_eq!(n(4), 2);
        assert_eq!(n(5), 3);
    }

    #[test]
    fn test_gray_rows_come_before_normal_garbage() {
        let r = calculate_elimination_garbage(5, 3);
        assert_eq!(r.len(), 2);
        assert_eq!((r[0].flavor, r[0].width, r[0].height), (GARBAGE_FLAVOR_GRAY, PLAY_WIDTH as i32, 1));
        assert_eq!((r[1].flavor, r[1].width, r[1].height), (GARBAGE_FLAVOR_NORMAL, 4, 1));
    }

    #[test]
    fn test_multiplier_garbage() {
        // x1 = no garbage
        assert!(calculate_completion_garbage(1).is_none());

        // x2 = height 1
        let g = calculate_completion_garbage(2).unwrap();
        assert_eq!(g.height, 1);
        assert_eq!(g.width, PLAY_WIDTH as i32);

        // x3 = height 2
        let g = calculate_completion_garbage(3).unwrap();
        assert_eq!(g.height, 2);

        // x5 = height 4
        let g = calculate_completion_garbage(5).unwrap();
        assert_eq!(g.height, 4);
    }
}
