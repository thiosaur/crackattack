//! The reactive play of the easy tier: the nearest swap that makes a match.

use crate::game_state::GameState;

use super::controller::AiController;
use super::select::Best;
use super::view::*;

impl AiController {
    /// The reactive tier's target: the nearest swap that makes a match, else (when allowed) a dig
    /// into a gap to open up new matches; never an exact undo of the last swap.
    pub(super) fn plan_reactive(
        &self,
        game: &GameState,
        cursor_x: i32,
        cursor_y: i32,
        avoid: Avoid,
    ) -> Option<SwapPlan> {
        let target = self.find_swap(game, cursor_x, cursor_y).or_else(|| {
            if self.tuning.flatten {
                self.find_flatten(game, cursor_x, cursor_y)
            } else {
                None
            }
        });
        target.filter(|t| !avoid.contains(*t))
    }

    /// A horizontal swap `(x, y)<->(x+1, y)` of two resting blocks whose exchange completes a
    /// 3+ run. Prefers a swap that also *shatters garbage* (its match lands next to a garbage
    /// slab): the single most important thing a player does under garbage pressure, since a
    /// shatter turns a whole slab back into matchable blocks and relieves the stack. A seeded
    /// rank chooses among equally useful swaps; an unseeded controller keeps nearest-cursor,
    /// bottom-up/left-to-right behaviour.
    pub(super) fn find_swap(&self, game: &GameState, cursor_x: i32, cursor_y: i32) -> Option<SwapPlan> {
        let max_row = (game.grid.top_effective_row + 1).min(HEIGHT - 1);
        let mut best = Best::new();
        let mut shatter = Best::new();
        for y in 1..=max_row {
            for x in 0..WIDTH - 1 {
                if !swappable_block(game, x, y) || !swappable_block(game, x + 1, y) {
                    continue;
                }
                if flavor_at(game, x, y) == flavor_at(game, x + 1, y) {
                    continue; // no-op swap
                }
                if !self.swap_makes_match(game, x, y) {
                    continue;
                }
                let plan = SwapPlan { x, y };
                let dist = (x - cursor_x).abs() + (y - cursor_y).abs();
                let rank = self.tie_rank(0x5357_4150, x, y);
                // A match on a cell touching garbage shatters that slab (the eliminated block
                // is adjacent to it). Either swapped cell anchors the match.
                if garbage_neighbor(game, x, y) || garbage_neighbor(game, x + 1, y) {
                    shatter.offer(0, rank, dist, plan);
                }
                best.offer(0, rank, dist, plan);
            }
        }
        shatter.item().or(best.item())
    }

    /// Whether swapping `(x, y)<->(x+1, y)` completes a 3+ run in the row or in either touched
    /// column. Evaluated against the *post-swap* flavours (the two cells exchange; every other
    /// cell is read from the grid).
    pub(super) fn swap_makes_match(&self, game: &GameState, x: i32, y: i32) -> bool {
        let fa = flavor_at(game, x, y);
        let fb = flavor_at(game, x + 1, y);

        // Post-swap flavour at a cell, or None if it cannot be part of a match.
        let at = |cx: i32, cy: i32| -> Option<i32> {
            if cy == y && cx == x {
                return Some(fb);
            }
            if cy == y && cx == x + 1 {
                return Some(fa);
            }
            if !game.grid.is_static_block(cx, cy) {
                return None;
            }
            Some(flavor_at(game, cx, cy))
        };

        // Each swapped cell anchors a horizontal run (its row) and a vertical run (its
        // column). Any 3+ run means the swap eliminates.
        for (ax, f) in [(x, fb), (x + 1, fa)] {
            let mut run = 1;
            let mut cx = ax - 1;
            while cx >= 0 {
                match at(cx, y) {
                    Some(g) if flavor_match(g, f) => run += 1,
                    _ => break,
                }
                cx -= 1;
            }
            let mut cx = ax + 1;
            while cx < WIDTH {
                match at(cx, y) {
                    Some(g) if flavor_match(g, f) => run += 1,
                    _ => break,
                }
                cx += 1;
            }
            if run >= 3 {
                return true;
            }

            run = 1;
            let mut cy = y - 1;
            while cy >= 1 {
                match at(ax, cy) {
                    Some(g) if flavor_match(g, f) => run += 1,
                    _ => break,
                }
                cy -= 1;
            }
            let mut cy = y + 1;
            while cy < HEIGHT {
                match at(ax, cy) {
                    Some(g) if flavor_match(g, f) => run += 1,
                    _ => break,
                }
                cy += 1;
            }
            if run >= 3 {
                return true;
            }
        }
        false
    }
}
