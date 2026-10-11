
// Block flavors (same indices as the original C++; the palette lives in the renderers)
/// Gray: the only colorless flavor (it never matches).
pub const BF_GRAY: i32 = 5;

// Timing
pub const STEPS_PER_SECOND: i32 = 50;
pub const STEPS_PER_GRID: i32 = 60;

// Delays
pub const DYING_DELAY: i32 = 90;
pub const INTERNAL_POP_DELAY: i32 = 15;
pub(crate) const INITIAL_POP_DELAY: i32 = 50 + INTERNAL_POP_DELAY;
pub(crate) const FINAL_POP_DELAY: i32 = 50;
pub const SHATTER_TIME: i32 = INITIAL_POP_DELAY;  // ~1.3s animation shatter
pub(crate) const HANG_DELAY: i32 = 3;
pub const MOVE_DELAY: i32 = 6;

// Velocities
pub(crate) const FALL_VELOCITY: i32 = 20;
pub(crate) const SWAP_VELOCITY: i32 = 10;
pub(crate) const SWAP_DELAY: i32 = STEPS_PER_GRID / SWAP_VELOCITY;

// Creep (rising floor)
pub(crate) const CREEP_DELAY: i32 = 1200;
pub(crate) const CREEP_ADVANCE_VELOCITY: i32 = 3;
pub(crate) const CREEP_TIMER_STEP_INCREMENT: i32 = 20;
pub(crate) const CREEP_INITIAL_TIMER_STEP: i32 = CREEP_TIMER_STEP_INCREMENT;
pub(crate) const CREEP_MAX_TIMER_STEP: i32 = 2400;
pub(crate) const CREEP_INCREMENT_DELAY: i32 = 10 * STEPS_PER_SECOND;
/// Seconds for the stack to rise one row at the start of a game (72 s).
pub const CREEP_START_SECONDS_PER_ROW: f32 =
    (STEPS_PER_GRID * CREEP_DELAY) as f32 / (CREEP_INITIAL_TIMER_STEP * STEPS_PER_SECOND) as f32;

// Loss delay (defeat countdown, used by creep.rs). Exported here so the renderer
// interprets `loss_countdown` with the engine's value instead of a local copy.
pub const LOSS_DELAY: i32 = 7 * STEPS_PER_SECOND;         // 350 time steps (7 seconds)
pub const LOSS_DELAY_ELIMINATION: i32 = STEPS_PER_SECOND; // 50 time steps (1 second)

// Grid dimensions
pub const PLAY_WIDTH: usize = 6;
pub(crate) const PLAY_HEIGHT: usize = 45;
pub const SAFE_HEIGHT: usize = 13;
pub const VISIBLE_HEIGHT: usize = SAFE_HEIGHT + 1;

// Storage sizes
pub(crate) const BLOCK_STORE_SIZE: usize = PLAY_WIDTH * PLAY_HEIGHT;
pub(crate) const GARBAGE_STORE_SIZE: usize = 2 * PLAY_HEIGHT;
pub(crate) const COMBO_STORE_SIZE: usize = 8;

// Compile-time consistency checks
const _: () = assert!(SAFE_HEIGHT < PLAY_HEIGHT);
const _: () = assert!(VISIBLE_HEIGHT <= PLAY_HEIGHT);
const _: () = assert!(COMBO_STORE_SIZE >= 4);

// Gameplay rules
pub(crate) const MIN_PATTERN_LENGTH: i32 = 3;
pub const MAX_GARBAGE_HEIGHT: i32 = 11;
pub(crate) const NUM_NORMAL_FLAVORS: i32 = 5;
pub(crate) const GARBAGE_TO_GARBAGE_SHATTER: i32 = 2;  // 1 in 2 chance of creating garbage instead of blocks

// Score constants
pub(crate) const MIN_PATTERN_SCORE: i32 = 2;  // Score for exactly 3 blocks eliminated
pub(crate) const GRAY_SCORE: i32 = 3;         // GC_GRAY_SCORE - multiplier for gray block eliminations

// Initial swapper position
pub(crate) const INITIAL_SWAPPER_X: i32 = (PLAY_WIDTH / 2 - 1) as i32;
pub(crate) const INITIAL_SWAPPER_Y: i32 = 4;

// Garbage generation
pub(crate) const GARBAGE_QUEUE_SIZE: usize = 8;
pub(crate) const AVERAGE_GARBAGE_DROP_DELAY: i32 = 300;  // ~6 seconds
pub(crate) const SPREAD_GARBAGE_DROP_DELAY: i32 = 40;    // ±0.4 seconds randomness

// Countdown
