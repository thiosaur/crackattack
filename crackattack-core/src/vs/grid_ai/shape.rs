//! Shaping the stack when nothing can be cleared: dig a peak into a gap, or cluster colours.

use crate::game_state::GameState;

use super::controller::AiController;
use super::select::Best;
use super::view::*;

/// Cluster-score weight of a same-flavour neighbour above or below.
const CLUSTER_VERTICAL: i32 = 2;
/// Cluster-score weight of a same-flavour neighbour left or right.
const CLUSTER_HORIZONTAL: i32 = 1;

impl AiController {
    /// The nearest "dig" swap: a static block beside an empty cell it would *fall* into (the
    /// cell below the empty is also empty), which lowers a peak and shuffles colours around to
    /// create new matches. Only used as a fallback when no clearing swap exists.
    pub(super) fn find_flatten(&self, game: &GameState, cursor_x: i32, cursor_y: i32) -> Option<SwapPlan> {
        let max_row = (game.grid.top_effective_row + 1).min(HEIGHT - 1);
        let mut best = Best::new();
        for y in 2..=max_row {
            for x in 0..WIDTH - 1 {
                let left_block = swappable_block(game, x, y);
                let right_block = swappable_block(game, x + 1, y);
                let left_empty = game.grid.get(x, y).is_empty();
                let right_empty = game.grid.get(x + 1, y).is_empty();
                // Exactly one side is a movable block, the other an empty the block can drop
                // through (empty below the destination).
                let can_dig = (left_block && right_empty && game.grid.get(x + 1, y - 1).is_empty())
                    || (right_block && left_empty && game.grid.get(x, y - 1).is_empty());
                if !can_dig {
                    continue;
                }
                let dist = (x - cursor_x).abs() + (y - cursor_y).abs();
                best.offer(-dist, self.tie_rank(0x464c_4154, x, y), dist, SwapPlan { x, y });
            }
        }
        best.item()
    }

    /// Constructive fallback (top tiers): when no clear exists, make the swap that best
    /// *clusters* same-flavour blocks, increasing same-flavour adjacencies and weighting
    /// vertical pairs higher (they build columns that survive the stack settling). Only
    /// positive-gain swaps are considered, so it never oscillates; nearest-cursor breaks ties.
    pub(super) fn find_build(&self, game: &GameState, cursor_x: i32, cursor_y: i32) -> Option<SwapPlan> {
        let max_row = (game.grid.top_effective_row + 1).min(HEIGHT - 1);
        let mut best = Best::new();
        for y in 1..=max_row {
            for x in 0..WIDTH - 1 {
                if !swappable_block(game, x, y) || !swappable_block(game, x + 1, y) {
                    continue;
                }
                let fa = flavor_at(game, x, y);
                let fb = flavor_at(game, x + 1, y);
                if fa == fb {
                    continue; // no-op swap
                }
                // Gain = clustering after the swap minus before it.
                let before = self.cluster_score(game, x, y, fa, None) + self.cluster_score(game, x + 1, y, fb, None);
                let after = self.cluster_score(game, x, y, fb, Some((x + 1, fa)))
                    + self.cluster_score(game, x + 1, y, fa, Some((x, fb)));
                let gain = after - before;
                if gain <= 0 {
                    continue;
                }
                let dist = (x - cursor_x).abs() + (y - cursor_y).abs();
                best.offer(gain, self.tie_rank(0x4255_494c, x, y), dist, SwapPlan { x, y });
            }
        }
        best.item()
    }

    /// Weighted count of same-flavour static-block neighbours of `(x, y)` treated as holding
    /// `flavor`. Vertical neighbours score `CLUSTER_VERTICAL` (column matches survive settling),
    /// horizontal ones `CLUSTER_HORIZONTAL`. `swapped` optionally overrides one neighbour cell's
    /// flavour on row `y` to reflect a pending swap: `(column, flavor)`.
    pub(super) fn cluster_score(
        &self,
        game: &GameState,
        x: i32,
        y: i32,
        flavor: i32,
        swapped: Option<(i32, i32)>,
    ) -> i32 {
        let flavor_of = |cx: i32, cy: i32| -> Option<i32> {
            if let Some((sx, sf)) = swapped {
                if cx == sx && cy == y {
                    return Some(sf);
                }
            }
            if !game.grid.is_static_block(cx, cy) {
                return None;
            }
            Some(flavor_at(game, cx, cy))
        };
        let mut score = 0;
        let mut add = |cx: i32, cy: i32, weight: i32| {
            if let Some(g) = flavor_of(cx, cy) {
                if flavor_match(g, flavor) {
                    score += weight;
                }
            }
        };
        if y + 1 < HEIGHT {
            add(x, y + 1, CLUSTER_VERTICAL);
        }
        if y > 1 {
            add(x, y - 1, CLUSTER_VERTICAL);
        }
        if x + 1 < WIDTH {
            add(x + 1, y, CLUSTER_HORIZONTAL);
        }
        if x >= 1 {
            add(x - 1, y, CLUSTER_HORIZONTAL);
        }
        score
    }
}
