mod snapshot;
mod danger;
mod heuristics;
mod planner;

use crate::constants::*;
use crate::game_state::GameState;
use crate::input::{INPUT_LEFT, INPUT_RIGHT, INPUT_UP, INPUT_DOWN, INPUT_SWAP};
use crate::swapper::SwapperState;

use snapshot::*;
use planner::{Planner, HISTORY_SIZE};

/// Main AI controller - single entry point for the autoplay system.
///
/// Usage:
/// ```ignore
/// let mut ai = AIController::new();
/// // Each tick:
/// let input_flags = ai.update(&game_state);
/// ```
pub struct AIController {
    planner: Planner,
    swap_cooldown: i32,
    move_cooldown: i32,
    swap_timer: i32,
    move_timer: i32,
    swap_target_x: i32,
    swap_target_y: i32,
    need_swap: bool,
    recent_swap_x: [i32; HISTORY_SIZE],
    recent_swap_y: [i32; HISTORY_SIZE],
}

impl AIController {
    /// Create a new AI controller with default cooldowns.
    pub fn new() -> Self {
        AIController {
            planner: Planner::new(),
            swap_cooldown: 8,
            move_cooldown: 3,
            swap_timer: 0,
            move_timer: 0,
            swap_target_x: -1,
            swap_target_y: -1,
            need_swap: false,

            recent_swap_x: [-1; HISTORY_SIZE],
            recent_swap_y: [-1; HISTORY_SIZE],
        }
    }

    /// Main update function: reads the game state and returns input flags.
    ///
    /// Returns a bitmask of `INPUT_LEFT`, `INPUT_RIGHT`, `INPUT_UP`, `INPUT_DOWN`, `INPUT_SWAP`.
    pub fn update(&mut self, game: &GameState) -> u8 {


        // Decrement timers
        if self.swap_timer > 0 { self.swap_timer -= 1; }
        if self.move_timer > 0 { self.move_timer -= 1; }

        // Read game state
        let cursor_x = game.swapper.x;
        let cursor_y = game.swapper.y;
        let can_swap = game.swapper.state != SwapperState::Swapping;
        let can_move = game.swapper.state != SwapperState::Swapping
            && game.swapper.move_lockout_countdown == 0;

        // Build grid snapshot
        let grid = build_grid_snapshot(game);

        // Convert cursorY to SwapGrid coordinates (0-indexed)
        let grid_cursor_y = cursor_y - 1;

        // If we're at target and can swap, do it
        if self.need_swap && cursor_x == self.swap_target_x && grid_cursor_y == self.swap_target_y
            && can_swap && self.swap_timer == 0 {
                self.swap_timer = self.swap_cooldown;
                self.need_swap = false;

                // Update history
                for i in (1..HISTORY_SIZE).rev() {
                    self.recent_swap_x[i] = self.recent_swap_x[i - 1];
                    self.recent_swap_y[i] = self.recent_swap_y[i - 1];
                }
                self.recent_swap_x[0] = self.swap_target_x;
                self.recent_swap_y[0] = self.swap_target_y;

                self.swap_target_x = -1;
                self.swap_target_y = -1;
                return INPUT_SWAP;
            }

        // Plan next move if needed
        if self.swap_target_x < 0 || !self.need_swap {
            let mut tx = 0i32;
            let mut ty = 0i32;
            let found = self.planner.plan_move(
                &grid, cursor_x, cursor_y,
                &mut tx, &mut ty,
                &self.recent_swap_x, &self.recent_swap_y,
            );
            if found {
                self.swap_target_x = tx;
                self.swap_target_y = ty;
                self.need_swap = true;
            } else {
                // No move found - nudge_cursor
                if can_move && self.move_timer == 0 {
                    let w = self.nudge_cursor(cursor_x, cursor_y);
                    if w != 0 {
                        self.move_timer = self.move_cooldown;
                        return w;
                    }
                }
                return 0;
            }
        }

        // Navigate to target
        if can_move && self.move_timer == 0 {
            let nav = self.navigate_to(cursor_x, cursor_y, self.swap_target_x, self.swap_target_y);
            if nav != 0 {
                self.move_timer = self.move_cooldown;
                return nav;
            }
        }

        0
    }

    fn max_height_any(game: &GameState) -> i32 {
        let mut max_h = 0i32;
        for x in 0..PLAY_WIDTH as i32 {
            for y in (1..GRID_HEIGHT as i32).rev() {
                let cell = game.grid.get(x, y);
                if !cell.is_empty() {
                    if y > max_h { max_h = y; }
                    break;
                }
            }
        }
        max_h
    }

    /// Fast-forward only for low-pressure states where the stack height is below 5.
    pub fn should_fast_forward(game: &GameState) -> bool {
        Self::max_height_any(game) < 5
    }

    fn navigate_to(&self, cursor_x: i32, cursor_y: i32, swap_target_x: i32, swap_target_y: i32) -> u8 {
        // Convert targetY from SwapGrid (0-indexed) to game (1-indexed)
        let game_swap_target_y = swap_target_y + 1;

        let dx = swap_target_x - cursor_x;
        let dy = game_swap_target_y - cursor_y;

        // Prioritize horizontal movement
        if dx > 0 { return INPUT_RIGHT; }
        if dx < 0 { return INPUT_LEFT; }
        if dy > 0 { return INPUT_UP; }
        if dy < 0 { return INPUT_DOWN; }

        0
    }

    fn nudge_cursor(&self, cursor_x: i32, cursor_y: i32) -> u8 {
        let _grid_y = cursor_y - 1;

        if cursor_x + 1 < SWAP_WIDTH as i32 { return INPUT_RIGHT; }
        if cursor_x > 0 { return INPUT_LEFT; }
        if _grid_y + 1 < SWAP_HEIGHT as i32 { return INPUT_UP; }
        if _grid_y > 0 { return INPUT_DOWN; }

        0
    }
}

impl Default for AIController {
    fn default() -> Self {
        Self::new()
    }
}
