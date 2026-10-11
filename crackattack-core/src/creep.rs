// Creep (rising floor)

use crate::constants::*;
// LOSS_DELAY / LOSS_DELAY_ELIMINATION are defined in constants.rs (exported for the renderer).

#[derive(Debug, Clone)]
pub(crate) struct Creep {
    pub offset: i32,          // Current sub-grid offset (0 to STEPS_PER_GRID)
    pub(crate) timer: i32,           // Timer accumulator
    pub(crate) timer_step: i32,      // Current timer step size (increases over time)
    pub(crate) advance_timer: i32,   // Countdown to timer acceleration
    pub(crate) advance_held: bool,   // Player holding advance button (current frame)
    pub(crate) advance_latch: bool,  // Latch — stays true until grid shift + button release
    pub creep_freeze: bool,   // Frozen due to safe height violation
    pub loss_countdown: i32,      // Countdown to game over (when frozen)
}

impl Creep {
    pub(crate) fn new() -> Self {
        Creep {
            offset: 0,
            timer: 0,
            timer_step: CREEP_INITIAL_TIMER_STEP,
            advance_timer: CREEP_INCREMENT_DELAY,
            advance_held: false,
            advance_latch: false,
            creep_freeze: false,
            loss_countdown: 0,
        }
    }

    /// Tick only the acceleration timer (during countdown, when creep itself is paused).
    /// The acceleration timer must advance during countdown to stay in sync
    /// with the absolute game clock.
    pub(crate) fn tick_acceleration_only(&mut self) {
        if self.advance_timer > 0 {
            self.advance_timer -= 1;
            if self.advance_timer == 0 && self.timer_step < CREEP_MAX_TIMER_STEP {
                self.timer_step = (self.timer_step + CREEP_TIMER_STEP_INCREMENT)
                    .min(CREEP_MAX_TIMER_STEP);
                self.advance_timer = CREEP_INCREMENT_DELAY;
            }
        }
    }


    /// Set advance held state (player pressing advance button)
    pub(crate) fn set_advance(&mut self, held: bool) {
        self.advance_held = held;
    }

    /// Time step with animation state and safe height check
    /// Returns (should_shift, lost) - lost=true means game over
    pub(crate) fn time_step(&mut self, has_active_animations: bool, safe_height_violated: bool) -> (bool, bool) {
        // Timer acceleration always runs (same logic as during the countdown).
        self.tick_acceleration_only();

        // If animations are active, creep pauses
        if has_active_animations {
            // Give grace period if in danger during eliminations
            if self.creep_freeze && self.loss_countdown < LOSS_DELAY_ELIMINATION {
                self.loss_countdown = LOSS_DELAY_ELIMINATION;
            }
            // Check if safe height violation ended
            if self.creep_freeze && !safe_height_violated {
                self.creep_freeze = false;
                self.loss_countdown = 0;
            }
            return (false, false);
        }

        // If we're in a creep freeze (safe height violation)
        if self.creep_freeze {
            // Check if violation ended
            if !safe_height_violated {
                self.creep_freeze = false;
                self.loss_countdown = 0;
            } else {
                // Count down to game over
                self.loss_countdown -= 1;
                if self.loss_countdown <= 0 {
                    return (false, true);  // GAME OVER
                }
                return (false, false);  // No creep, but not lost yet
            }
        }

        // Check for new safe height violation
        if safe_height_violated {
            self.creep_freeze = true;
            self.loss_countdown = LOSS_DELAY;
            return (false, false);
        }

        // Advance is a latch: once set, it stays true until grid shift + button release
        let is_advancing = self.advance_latch || self.advance_held;

        // Normal creep accumulation
        let timer_increment = if is_advancing {
            // Fast advance when player holds button or latch is set
            const CREEP_ADVANCE_TIMER_STEP: i32 = 1200;
            if self.timer_step < CREEP_ADVANCE_TIMER_STEP {
                CREEP_ADVANCE_TIMER_STEP
            } else {
                self.timer_step
            }
        } else {
            self.timer_step
        };

        // Set latch if advancing
        if is_advancing {
            self.advance_latch = true;
        }

        self.timer += timer_increment;

        // Process creep accumulation
        let mut shifted = false;
        while self.timer >= CREEP_DELAY {
            self.timer -= CREEP_DELAY;

            if self.advance_latch {
                self.offset += CREEP_ADVANCE_VELOCITY;
                if self.offset > STEPS_PER_GRID {
                    self.offset = STEPS_PER_GRID;
                }
            } else {
                self.offset += 1;
            }

            // Check for full grid shift
            if self.offset >= STEPS_PER_GRID {
                self.offset = 0;
                shifted = true;

                // Clear latch only after grid shift AND when button is released
                if self.advance_latch && !self.advance_held {
                    self.advance_latch = false;
                }
            }
        }

        debug_assert!(self.offset >= 0 && self.offset <= STEPS_PER_GRID,
            "Creep offset out of range: {}", self.offset);

        (shifted, false)
    }

    /// Get visual offset for rendering (0.0 to 1.0)
    /// Interpolates between grid steps using the timer fraction
    /// for smooth visual progression instead of discrete jumps.
    pub(crate) fn visual_offset(&self) -> f32 {
        let base = self.offset as f32 / STEPS_PER_GRID as f32;
        let frac = self.timer as f32 / CREEP_DELAY as f32;
        let step = if self.advance_latch {
            CREEP_ADVANCE_VELOCITY as f32 / STEPS_PER_GRID as f32
        } else {
            1.0 / STEPS_PER_GRID as f32
        };
        base + frac * step
    }

    /// Seconds for the stack to rise one row at the current normal pace (no manual raise).
    /// About 72 s at the start of a game, 0.6 s at maximum speed.
    pub(crate) fn seconds_per_row(&self) -> f32 {
        (STEPS_PER_GRID * CREEP_DELAY) as f32 / (self.timer_step.max(1) * STEPS_PER_SECOND) as f32
    }

    /// Rows the stack can still rise before the safe height, minus the fraction of a
    /// row already risen. 0 when the stack is blocked at the top.
    pub(crate) fn rows_to_loss(&self, top_effective_row: i32) -> f32 {
        if self.creep_freeze {
            return 0.0;
        }
        let rows_left = (SAFE_HEIGHT as i32 - 1 - top_effective_row).max(0) as f32;
        (rows_left - self.visual_offset()).max(0.0)
    }

    /// Called when grid shift failed (top row occupied). Retry next tick.
    pub(crate) fn retry_shift(&mut self) {
        self.timer += CREEP_DELAY;
        self.offset = STEPS_PER_GRID - 1;
    }
}

impl Default for Creep {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn creep_rate_and_rows_to_loss() {
        let mut c = Creep::new();
        assert!((c.seconds_per_row() - 72.0).abs() < 1e-3); // start of game
        c.timer_step = CREEP_MAX_TIMER_STEP;
        assert!((c.seconds_per_row() - 0.6).abs() < 1e-3); // maximum speed
        // Stack at row 9: 3 rows before the safe height (12), minus the risen fraction.
        c.offset = STEPS_PER_GRID / 2;
        c.timer = 0;
        assert!((c.rows_to_loss(9) - 2.5).abs() < 1e-3);
        assert_eq!(c.rows_to_loss(14), 0.0);
        c.creep_freeze = true;
        assert_eq!(c.rows_to_loss(5), 0.0);
    }
}
