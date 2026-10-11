//! Frame pacing and render state: fixed-tick clocks, wall-clock accumulators,
//! smoothed panic intensity, transition grace period.

use std::time::{Duration, Instant};

/// Timing and frame-rate scheduling fields.
pub(crate) struct SchedulingState {
    /// Fixed-step QA clock (`session:qa.fixed_step`): exactly one simulation tick per frame.
    /// Set from `BootConfig` at bootstrap.
    pub(crate) fixed_step: bool,
    pub(crate) last_tick: Instant,
    pub(crate) tick_accumulator: Duration,
    /// Wall-clock instant of the last `render()` call.
    pub(crate) last_render_instant: Instant,
    /// Wall-clock accumulator in seconds, ticked by `render_dt`
    /// each frame.
    pub(crate) wall_clock_seconds: f32,
    /// Last render_dt computed by the event loop (for render to use).
    pub(crate) last_render_dt: f32,
    /// Low-pass-filtered `panic_intensity`.
    pub(crate) panic_smoothed: f32,
    /// Flash frequency of the panic beacon, in Hz.
    pub(crate) panic_freq: f32,
    pub(crate) frame_counter: u32,
    /// Grace period (in ticks) that keeps the full 50 Hz rate after a curtain
    /// transition ends. It avoids an abrupt 50 to 15 Hz drop right at the end of
    /// the animation. Reloaded while a curtain is active, decremented afterwards.
    pub(crate) transition_grace: u32,
}

impl SchedulingState {
    pub(crate) fn new() -> Self {
        Self {
            fixed_step: false,
            last_tick: Instant::now(),
            tick_accumulator: Duration::ZERO,
            last_render_instant: Instant::now(),
            wall_clock_seconds: 0.0,
            last_render_dt: 0.0,
            panic_smoothed: 0.0,
            panic_freq: 0.8,
            frame_counter: 0,
            transition_grace: 0,
        }
    }
}
