//! Per-game statistics tracking.
//!
//! `GameStats` accumulates counters during gameplay by observing
//! `GameplayEvent`s emitted each tick. The renderer reads the stats
//! at game over for the end-of-game screen.

use crate::gameplay_event::GameplayEvent;

/// Per-game statistics, accumulated during play.
#[derive(Debug, Clone, Default)]
pub struct GameStats {
    /// Total blocks eliminated
    pub blocks_eliminated: u32,
    /// Number of chains (×2+)
    pub chains_count: u32,
    /// Highest chain level reached
    pub max_chain_level: u32,
    /// Total garbage lines sent to opponent
    pub garbage_sent: u32,
    /// Total garbage lines received from opponent
    pub garbage_received: u32,
    /// Number of garbage pieces shattered
    pub garbage_shattered: u32,
    /// Number of swaps performed
    pub swaps_performed: u32,
    /// Game duration in ticks (÷50 = seconds)
    pub duration_ticks: u32,
}

impl GameStats {
    pub fn new() -> Self {
        Self::default()
    }

    /// Process one tick's worth of gameplay events.
    pub fn process_events(&mut self, events: &[GameplayEvent]) {
        self.duration_ticks += 1;

        for event in events {
            match event {
                GameplayEvent::EliminationStarted(e) => {
                    self.blocks_eliminated += e.magnitude as u32;
                }
                GameplayEvent::ChainProgressed(e) => {
                    self.chains_count += 1;
                    if e.chain_level as u32 > self.max_chain_level {
                        self.max_chain_level = e.chain_level as u32;
                    }
                }
                GameplayEvent::GarbageShattering(_) => {
                    self.garbage_shattered += 1;
                }
                GameplayEvent::SwapPerformed => {
                    self.swaps_performed += 1;
                }
                _ => {}
            }
        }
    }

    /// Record garbage sent (called from game logic, not from events)
    pub(crate) fn record_garbage_sent(&mut self, lines: u32) {
        self.garbage_sent += lines;
    }

    /// Record garbage received
    pub(crate) fn record_garbage_received(&mut self, lines: u32) {
        self.garbage_received += lines;
    }

    /// Duration in seconds
    pub fn duration_seconds(&self) -> f32 {
        self.duration_ticks as f32 / 50.0
    }

    /// Duration formatted as MM:SS
    pub fn duration_formatted(&self) -> String {
        let secs = self.duration_seconds() as u32;
        format!("{}:{:02}", secs / 60, secs % 60)
    }

    /// Blocks eliminated per minute
    pub fn blocks_per_minute(&self) -> f32 {
        if self.duration_ticks == 0 { return 0.0; }
        self.blocks_eliminated as f32 / (self.duration_seconds() / 60.0)
    }
}
