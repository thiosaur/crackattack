//! The hands: turns a chosen swap into key presses (cursor walk, press/release pulsing, pacing
//! cooldown, when they are busy) and refuses to undo its own recent swaps.

use crate::game_state::GameState;
use crate::swapper::SwapperState;
use crate::input::{INPUT_DOWN, INPUT_LEFT, INPUT_RIGHT, INPUT_SWAP, INPUT_UP};

use super::controller::{AiController, RecentSwap};
use super::planner::{hash_plan_board, PlanBoard};
use super::view::*;

impl AiController {
    /// Keys the hands must not touch this tick: a press needs a neutral tick after it so the
    /// swapper's debounce re-arms; a swap in progress is waited out. A raise that is still under
    /// way (the creep latches a manual advance until the row lands) only blocks a new raise, not
    /// the swaps: with the creep frozen at the top the latch never clears, and holding every key
    /// for it would lock the bot out for good.
    pub(super) fn hold_keys(&mut self, game: &GameState) -> Option<u8> {
        if self.release_next {
            self.release_next = false;
            return Some(0);
        }
        if game.swapper.state == SwapperState::Swapping {
            return Some(0);
        }
        None
    }

    /// Pacing after a swap: true (and one tick spent) while the cooldown runs.
    pub(super) fn resting(&mut self) -> bool {
        if self.cooldown > 0 {
            self.cooldown -= 1;
            true
        } else if self.move_rest > 0 {
            self.move_rest -= 1;
            true
        } else {
            false
        }
    }

    /// One input toward `target`: walk the cursor there one axis at a time (pulsing each
    /// press), then swap and start the pacing cooldown. `board` is what the plan was made on,
    /// remembered to refuse an exact undo of this swap.
    pub(super) fn act_on(&mut self, game: &GameState, target: SwapPlan, board: &PlanBoard) -> u8 {
        let (cursor_x, cursor_y) = (game.swapper.x, game.swapper.y);
        let dir = if cursor_x < target.x {
            INPUT_RIGHT
        } else if cursor_x > target.x {
            INPUT_LEFT
        } else if cursor_y < target.y {
            INPUT_UP
        } else if cursor_y > target.y {
            INPUT_DOWN
        } else {
            0
        };
        if dir != 0 {
            self.release_next = true;
            self.move_rest = self.step_rest(game);
            return dir;
        }

        // Aligned: swap, then pace by difficulty.
        self.last_swap = Some((target, hash_plan_board(board)));
        self.remember_swap(game, target, board);
        self.cooldown = self.tuning.cooldown;
        self.release_next = true;
        INPUT_SWAP
    }

    /// Rest after a cursor step: the tuning's gap plus a seeded draw up to its jitter.
    fn step_rest(&self, game: &GameState) -> i32 {
        let (gap, jitter) = (self.tuning.move_gap, self.tuning.move_jitter);
        if jitter <= 0 || self.decision_seed.is_none() {
            return gap;
        }
        let draw = self.tie_rank(0x4d4f_5645, game.swapper.x, game.time_step) % (jitter as u32 + 1);
        gap + draw as i32
    }

    /// Keep the row each of the last few swaps produced.
    fn remember_swap(&mut self, game: &GameState, target: SwapPlan, board: &PlanBoard) {
        let y = target.y - 1;
        if y < 0 || y >= board.height as i32 {
            return;
        }
        let mut after: Vec<i16> = (0..board.width).map(|x| board.get(x, y as usize)).collect();
        after.swap(target.x as usize, target.x as usize + 1);
        if self.recent_swaps.len() >= RECENT_SWAPS {
            self.recent_swaps.remove(0);
        }
        self.recent_swaps.push(RecentSwap { x: target.x, after, time_step: game.time_step });
    }

    /// The swaps a planner must not return because they would take the bot back to where it was.
    /// Two kinds, both about not undoing its own work:
    /// - the last swap, when performing it again would recreate the exact board from before it
    ///   (constructive planners may change targets after every move, and two attractive targets
    ///   would select inverse first steps forever);
    /// - any swap that would turn a row that a recent swap just produced back into what it was
    ///   (the same swap, on the same row contents), if that swap was recent. The whole board changes between two decisions
    ///   (the stack rises, blocks fall), so the exact-board test misses these; the row does not.
    pub(super) fn immediate_undo(&self, game: &GameState, board: &mut PlanBoard) -> Avoid {
        let mut avoid = Avoid::default();
        if let Some((plan, before)) = self.last_swap {
            let y = plan.y - 1;
            if y >= 0 && y < board.height as i32 && plan.x >= 0 && plan.x + 1 < board.width as i32 {
                let (x, y) = (plan.x as usize, y as usize);
                let (left, right) = (board.get(x, y), board.get(x + 1, y));
                board.set(x, y, right);
                board.set(x + 1, y, left);
                let undo_hash = hash_plan_board(board);
                board.set(x, y, left);
                board.set(x + 1, y, right);
                if undo_hash == before {
                    avoid.push(plan);
                }
            }
        }
        if !self.tuning.row_undo_guard {
            return avoid;
        }
        for recent in self.recent_swaps.iter().rev() {
            if game.time_step - recent.time_step > RECENT_WINDOW {
                continue;
            }
            for y in 0..board.height {
                let same_row = recent.after.iter().enumerate().all(|(x, &v)| board.get(x, y) == v);
                let plan = SwapPlan::from_plan(recent.x as usize, y);
                if same_row && !avoid.contains(plan) && avoid.len() < 2 {
                    avoid.push(plan);
                }
            }
        }
        avoid
    }
}

/// How many swaps are remembered, and for how long (ticks) one can still be undone by mistake.
const RECENT_SWAPS: usize = 4;
const RECENT_WINDOW: i32 = 400;
