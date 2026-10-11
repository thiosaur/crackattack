// Swapper (cursor)

use crate::constants::*;
use crate::block::BlockId;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum SwapperState {
    #[default]
    Idle,
    Swapping,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum MoveDirection {
    #[default]
    None,
    Left,
    Right,
    Up,
    Down,
}

#[derive(Debug, Clone)]
pub(crate) struct Swapper {
    pub x: i32,
    pub y: i32,
    pub state: SwapperState,
    pub(crate) swap_countdown: i32,
    pub move_lockout_countdown: i32,
    pub(crate) queued_swap: bool,
    /// Queued move direction (moves during move_pause are queued)
    pub(crate) queued_move: i32,
    /// Button down tracking for edge detection
    pub(crate) button_down_move: i32,
    // Track swapping blocks for landing combo propagation
    /// Left block ID during swap (originally at swap_x)
    pub(crate) swap_left_block: Option<BlockId>,
    /// Right block ID during swap (originally at swap_x + 1)
    pub(crate) swap_right_block: Option<BlockId>,
    /// X position at swap start
    pub(crate) swap_x: i32,
    /// Y position at swap start
    pub(crate) swap_y: i32,
    /// Last move direction for MovePause animation
    pub last_move_direction: MoveDirection,
}

impl Swapper {
    pub fn new() -> Self {
        Swapper {
            x: INITIAL_SWAPPER_X,
            y: INITIAL_SWAPPER_Y,
            state: SwapperState::Idle,
            swap_countdown: 0,
            move_lockout_countdown: 0,
            queued_swap: false,
            queued_move: 0,
            button_down_move: 0,
            swap_left_block: None,
            swap_right_block: None,
            swap_x: 0,
            swap_y: 0,
            last_move_direction: MoveDirection::None,
        }
    }

    /// Process move input.
    /// move_cmd is a bitmask: LEFT=1, RIGHT=2, UP=4, DOWN=8
    ///
    /// Move pause can queue a new move, but it does not block movement processing.
    /// Only an active swap returns early and suppresses further commands for the tick.
    ///
    /// Key invariant: move processing is NOT blocked by move lockout, only by an active swap.
    pub(crate) fn process_move_cmd(&mut self, move_cmd: i32) {
        

        // Reset button state when released
        if move_cmd == 0 {
            self.button_down_move = 0;
        }

        // Handle move pause state - queue new moves (does NOT block processMove)
        if self.move_lockout_countdown > 0 {
            // If move pressed (different from current button_down) and no swap queued
            if move_cmd != 0 && self.button_down_move != move_cmd && !self.queued_swap {
                self.queued_move = move_cmd;
                self.button_down_move = move_cmd;
            }
        }

        // Handle swapping state - queue moves and RETURN EARLY
        if self.state == SwapperState::Swapping {
            if self.button_down_move == 0 && move_cmd != 0 {
                self.queued_move = move_cmd;
                self.button_down_move = move_cmd;
            }
            return;
        }

        // Process movement (only Swapping blocks, handled above with early return)
        self.process_move_internal(move_cmd);
    }

    /// Internal move processing
    fn process_move_internal(&mut self, move_cmd: i32) {
        use crate::input::{INPUT_LEFT, INPUT_RIGHT, INPUT_UP, INPUT_DOWN};

        // Use queued move if available, otherwise current input
        let cmd = if self.queued_move != 0 { self.queued_move } else { move_cmd };

        // No command, or same button still held without queue
        if cmd == 0 || (self.button_down_move == cmd && self.queued_move == 0) {
            return;
        }

        // Calculate new position
        let mut new_x = self.x;
        let mut new_y = self.y;

        // Handle only one direction (prioritize horizontal)
        if cmd & (INPUT_LEFT as i32) != 0 {
            new_x = (self.x - 1).max(0);
        } else if cmd & (INPUT_RIGHT as i32) != 0 {
            new_x = (self.x + 1).min(PLAY_WIDTH as i32 - 2);
        } else if cmd & (INPUT_UP as i32) != 0 {
            // Upper limit is SAFE_HEIGHT - 1, not SAFE_HEIGHT
            new_y = (self.y + 1).min(SAFE_HEIGHT as i32 - 1);
        } else if cmd & (INPUT_DOWN as i32) != 0 {
            new_y = (self.y - 1).max(1);
        }

        // If position changed, update and set move pause
        if new_x != self.x || new_y != self.y {
            // Record move direction for MovePause animation
            if new_x < self.x {
                self.last_move_direction = MoveDirection::Left;
            } else if new_x > self.x {
                self.last_move_direction = MoveDirection::Right;
            } else if new_y > self.y {
                self.last_move_direction = MoveDirection::Up;
            } else {
                self.last_move_direction = MoveDirection::Down;
            }
            self.x = new_x;
            self.y = new_y;
            self.move_lockout_countdown = MOVE_DELAY;
            self.button_down_move = cmd;
        }

        // Clear queued move
        self.queued_move = 0;
    }

    /// Request a swap
    pub(crate) fn request_swap(&mut self) {
        if self.state == SwapperState::Idle {
            self.queued_swap = true;
        }
    }

    /// Start swap animation with block IDs for combo propagation
    pub(crate) fn start_swap_with_blocks(&mut self, left_block: Option<BlockId>, right_block: Option<BlockId>) {
        self.state = SwapperState::Swapping;
        self.swap_countdown = SWAP_DELAY;
        self.queued_swap = false;
        self.swap_left_block = left_block;
        self.swap_right_block = right_block;
        self.swap_x = self.x;
        self.swap_y = self.y;
    }

    /// Check if swap is complete
    pub(crate) fn is_swap_complete(&self) -> bool {
        self.state == SwapperState::Swapping && self.swap_countdown <= 0
    }

    /// Finish swap animation
    pub(crate) fn finish_swap(&mut self) {
        self.state = SwapperState::Idle;
        self.swap_countdown = 0;
        self.swap_left_block = None;
        self.swap_right_block = None;
    }

    /// Get swap progress (0.0 to 1.0)
    pub fn swap_factor(&self) -> f32 {
        if self.state == SwapperState::Swapping && SWAP_DELAY > 0 {
            1.0 - (self.swap_countdown as f32 / SWAP_DELAY as f32)
        } else {
            0.0
        }
    }

    /// Shift up when creep happens
    pub(crate) fn shift_up(&mut self) {
        self.y += 1;
        // Also shift swap position if a swap is in progress
        if self.state == SwapperState::Swapping {
            self.swap_y += 1;
        }
    }

    /// Time step
    pub fn time_step(&mut self) {
        if self.move_lockout_countdown > 0 {
            self.move_lockout_countdown -= 1;
        }
        if self.swap_countdown > 0 {
            self.swap_countdown -= 1;
        }
    }
}

impl Default for Swapper {
    fn default() -> Self {
        Self::new()
    }
}
