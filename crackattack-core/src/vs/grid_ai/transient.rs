//! Play while the board is in motion: take clearing swaps away from a live chain
//! (opportunism), or time a swap during a fall (gravity).

use crate::game_state::GameState;

use super::controller::AiController;
use super::planner::{attack_value, evaluate_gravity, settled, PlanBoard};
use super::candidates::{trigger_swaps, Candidate};
use super::select::Best;
use super::view::*;

impl AiController {
    /// The target while blocks fall or pop. A fall linked to an active combo will score as a
    /// chain when it lands; if gravity alone already forms a match, status quo is best (touching
    /// its supports could turn a guaranteed chain into a lesser move), so play away from the
    /// chain's columns or not at all. Otherwise time a swap against the settled outcome.
    pub(super) fn plan_in_fall(
        &self,
        game: &GameState,
        board: &PlanBoard,
        avoid: Avoid,
    ) -> Option<SwapPlan> {
        if self.has_linked_gravity_chain(game, board) {
            if !self.tuning.opportunism_linked {
                return None;
            }
            return self.plan_opportunistic(game, &self.board(game), &chain_blocks(game), avoid);
        }
        self.plan_during_gravity(game, board, game.swapper.x, game.swapper.y, avoid, has_linked_gravity(game))
    }

    /// While blocks wake up: the swap that makes the most of what they will match with when they
    /// have woken. Planned on the board with the awaking blocks at their cells (with their
    /// flavours; gravity settles them, as it will when they wake) and legal on the live game now.
    pub(super) fn plan_wake_prep(&self, game: &GameState, avoid: Avoid) -> Option<SwapPlan> {
        let (any, linked) = awaking_blocks(game);
        if !any {
            return None;
        }
        let board = read_gravity_plan_board(game);
        self.plan_during_gravity(game, &board, game.swapper.x, game.swapper.y, avoid, linked)
    }

    /// A fall linked to a combo that gravity alone already completes into a chain.
    fn has_linked_gravity_chain(&self, game: &GameState, board: &PlanBoard) -> bool {
        has_linked_gravity(game) && evaluate_gravity(board).chain_depth > 0
    }

    /// Whether the live chain must be left alone this tick: the bot is told not to touch a chain
    /// that gravity will complete by itself (`opportunism_linked` off).
    pub(super) fn holds_chain(&self, game: &GameState, motion: BoardMotion) -> bool {
        motion == BoardMotion::Gravity
            && !self.tuning.opportunism_linked
            && self.has_linked_gravity_chain(game, &read_gravity_plan_board(game))
    }

    /// The board the meanwhile plays are planned on: in a fall, the one the fall will leave;
    /// otherwise the live one, moving blocks as obstacles.
    pub(super) fn meanwhile_board(&self, game: &GameState, motion: BoardMotion) -> PlanBoard {
        if motion == BoardMotion::Gravity {
            settled(&read_gravity_plan_board(game))
        } else {
            self.board(game)
        }
    }

    /// The play for the time the board is in motion and nothing can be cashed in: what the bot
    /// would do on a settled board, with the moving blocks as fixed obstacles. A play must be
    /// legal on the live game right now and stay out of the live chain's zone. The board is never
    /// raised meanwhile, so only a swap counts.
    ///
    /// `avoid` is what the decision already refuses (its own undos, found on the live board: the
    /// plan board here is the settled one, whose rows are not the ones that were swapped).
    pub(super) fn plan_meanwhile(&mut self, game: &GameState, motion: BoardMotion, mut avoid: Avoid) -> Intent {
        let board = self.meanwhile_board(game, motion);
        if motion == BoardMotion::Other && self.tuning.wake_prep {
            if let Some(plan) = self.plan_wake_prep(game, avoid) {
                return Intent::Swap(plan);
            }
        }
        let (cursor_x, cursor_y) = (game.swapper.x, game.swapper.y);
        let zone = chain_blocks(game);
        // The plan is made on where blocks will be, so its best swap may not be playable yet
        // (a cell still in the air, the live chain's columns): ask again without it.
        loop {
            let intent = if self.tuning.strategic {
                self.plan_strategic(game, board.clone(), cursor_x, cursor_y, avoid, true)
            } else {
                self.plan_reactive(game, cursor_x, cursor_y, avoid).map_or(Intent::Wait, Intent::Swap)
            };
            let Intent::Swap(plan) = intent else { return Intent::Raise };
            if can_swap_now(game, plan.x, plan.y) && !self.in_chain_zone(&zone, plan.x, plan.y) {
                return Intent::Swap(plan);
            }
            if !avoid.push(plan) {
                return Intent::Raise;
            }
        }
    }

    /// Whether a swap of `(x, gy)<->(x+1, gy)` would disturb the live chain: it touches the
    /// column of one of its blocks, at or below that block. That is where the blocks will land
    /// and what they will land on. Beside it or above it, a swap cannot change the landing; a
    /// wider zone (neighbouring columns, or rows above) measured worse (journal S2).
    pub(super) fn in_chain_zone(&self, chain: &[(i32, i32)], x: i32, gy: i32) -> bool {
        chain.iter().any(|&(cx, cy)| {
            // The swap covers columns x and x+1.
            x <= cx && x + 1 >= cx && gy <= cy
        })
    }

    /// While the board is in motion: the best swap among the resting blocks that triggers a
    /// cascade, outside the chain's zone. No set-ups, only what can be cashed in now; legality
    /// is the simulator's own (`can_swap_now`), since the plan board drops the moving cells.
    pub(super) fn plan_opportunistic(
        &self,
        game: &GameState,
        board: &PlanBoard,
        zone: &[(i32, i32)],
        avoid: Avoid,
    ) -> Option<SwapPlan> {
        let cursor = (game.swapper.x, game.swapper.y);
        let mut best = Best::new();
        let swaps = trigger_swaps(
            board,
            cursor,
            |plan| self.in_chain_zone(zone, plan.x, plan.y) || avoid.contains(plan),
            |plan| can_swap_now(game, plan.x, plan.y),
        );
        for Candidate { plan, cascade, dist } in swaps {
            let score = attack_value(&cascade) + cascade.garbage_shattered * self.tuning.shatter_weight;
            best.offer((self.biased(score, plan.y), cascade.total_cleared), 0, dist, plan);
        }
        best.item()
    }

    /// Pick a currently legal swap while blocks hang, but score it only after all live gravity
    /// has settled. This finds the human timing move: arrange resting blocks during the pause so
    /// a combo appears when the linked blocks land.
    pub(super) fn plan_during_gravity(
        &self,
        game: &GameState,
        board: &PlanBoard,
        cursor_x: i32,
        cursor_y: i32,
        avoid: Avoid,
        linked_gravity: bool,
    ) -> Option<SwapPlan> {
        let mut best = Best::new();
        let swaps = trigger_swaps(
            board,
            (cursor_x, cursor_y),
            |plan| avoid.contains(plan),
            |plan| can_swap_now(game, plan.x, plan.y),
        );
        for Candidate { plan, cascade, dist } in swaps {
            // A non-clearing setup made during a linked fall turns the first future match
            // into a real x2 chain. Account for that full-width garbage row, which the
            // board-only evaluator cannot infer from combo ownership.
            let immediate_clear = game.grid.get(plan.x, plan.y).is_block()
                && game.grid.get(plan.x + 1, plan.y).is_block()
                && self.swap_makes_match(game, plan.x, plan.y);
            let landing_chain_bonus = if linked_gravity && !immediate_clear { WIDTH } else { 0 };
            let score = attack_value(&cascade) + cascade.garbage_shattered * self.tuning.shatter_weight + landing_chain_bonus;
            let rank = self.tie_rank(0x4752_4156, plan.x, plan.y);
            best.offer((self.biased(score, plan.y), cascade.total_cleared), rank, dist, plan);
        }
        best.item()
    }
}
