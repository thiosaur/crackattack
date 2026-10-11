//! VirtualPlayer — Lightweight AI opponent for VS Computer mode.
//!
//! The VirtualPlayer doesn't play "for real" — it maintains a garbage queue,
//! periodically produces garbage to send to the player, and loses when its
//! queue height exceeds a difficulty-dependent threshold.

use crate::constants::*;

/// AI difficulty level with associated parameters.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AIDifficulty {
    Easy,
    Medium,
    Hard,
}

impl AIDifficulty {
    /// Display name for UI ("Logic", "Sigma", "Cortex")
    pub fn name(self) -> &'static str {
        match self {
            Self::Easy => "Logic",
            Self::Medium => "Sigma",
            Self::Hard => "Cortex",
        }
    }

    /// Base timer delay in game ticks before AI drops garbage.
    fn base_steps(self) -> i32 {
        match self {
            Self::Easy => STEPS_PER_SECOND * 15,   // 750 ticks = 15s
            Self::Medium => STEPS_PER_SECOND * 10,  // 500 ticks = 10s
            Self::Hard => STEPS_PER_SECOND * 5,     // 250 ticks = 5s
        }
    }

    /// Queue height at which the AI loses.
    fn loss_height(self) -> i32 {
        match self {
            Self::Easy => 4,
            Self::Medium => 10,
            Self::Hard => 20,
        }
    }
}

/// Garbage flavor constants
pub use crate::garbage::{GARBAGE_FLAVOR_NORMAL, GARBAGE_FLAVOR_GRAY};

/// An element in the garbage queue.
#[derive(Debug, Clone)]
pub(crate) struct GarbageQueueElement {
    pub height: i32,
    pub flavor: i32,
}

/// Queue of pending garbage blocks (vector-based with cached height).
#[derive(Debug, Clone)]
pub(crate) struct GarbageQueue {
    elements: Vec<GarbageQueueElement>,
    cached_height: i32, // -1 = invalid
}

impl Default for GarbageQueue {
    fn default() -> Self {
        Self::new()
    }
}

impl GarbageQueue {
    pub fn new() -> Self {
        Self {
            elements: Vec::new(),
            cached_height: -1,
        }
    }

    /// Total height of all garbage in queue.
    pub fn height(&mut self) -> i32 {
        if self.cached_height >= 0 {
            return self.cached_height;
        }
        let h: i32 = self.elements.iter().map(|e| e.height).sum();
        self.cached_height = h;
        h
    }

    /// Height of special (gray) garbage only.
    /// Do not reuse cached_height here; it stores total queue height, not the gray-only subtotal.
    pub(crate) fn special_height(&self) -> i32 {
        self.elements.iter()
            .filter(|e| e.flavor != GARBAGE_FLAVOR_NORMAL)
            .map(|e| e.height)
            .sum()
    }

    /// Add garbage to the queue.
    pub fn add(&mut self, height: i32, flavor: i32) {
        self.elements.push(GarbageQueueElement { height, flavor });
        self.cached_height = -1;
    }

    /// Remove the contiguous front run with the same specialness as the first element.
    /// Gray stops at the first normal, normal stops at the first gray.
    /// Returns the count of elements removed.
    pub(crate) fn remove_with_specials(&mut self) -> i32 {
        if self.elements.is_empty() {
            return 0;
        }
        let front_flavor = self.elements[0].flavor;
        let stop_at = if front_flavor == GARBAGE_FLAVOR_GRAY { GARBAGE_FLAVOR_NORMAL } else { GARBAGE_FLAVOR_GRAY };
        self.remove_to_first(stop_at)
    }

    /// Remove elements from front until hitting one with the given flavor.
    /// Returns count of elements removed.
    fn remove_to_first(&mut self, stop_flavor: i32) -> i32 {
        let mut count = 0;
        while !self.elements.is_empty() && self.elements[0].flavor != stop_flavor {
            self.elements.remove(0);
            count += 1;
        }
        if count > 0 {
            self.cached_height = -1;
        }
        count
    }

    /// Clear all elements.
    pub fn reset(&mut self) {
        self.elements.clear();
        self.cached_height = -1;
    }
}

/// AI state machine
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AIState {
    Waiting,
    Shattering,
}

/// Lightweight AI opponent that manages a garbage queue.
///
/// The VirtualPlayer doesn't play a real game — it:
/// 1. Receives garbage from the player (added to its queue)
/// 2. Periodically "shatters" its queue (removes chunks, 50% chance to add back)
/// 3. Periodically produces garbage to send to the player
/// 4. Loses when queue height exceeds loss_height
#[derive(Debug, Clone)]
pub(crate) struct VirtualPlayer {
    difficulty: AIDifficulty,
    queue: GarbageQueue,
    state: AIState,
    last_time: i32,
    last_shatter_height: i32,
    rng_state: u32, // Simple RNG for garbage-to-garbage shatter
}

impl VirtualPlayer {
    pub fn new(difficulty: AIDifficulty) -> Self {
        Self {
            difficulty,
            queue: GarbageQueue::new(),
            state: AIState::Waiting,
            last_time: 0,
            last_shatter_height: 0,
            rng_state: 42,
        }
    }

    /// Reset alarm to current time.
    pub(crate) fn reset_alarm(&mut self, current_time: i32) {
        self.last_time = current_time;
    }

    /// Absolute time when the next garbage drop happens.
    pub fn alarm(&self) -> i32 {
        self.last_time + self.difficulty.base_steps() + self.state_steps()
    }

    /// Additional delay based on current state.
    fn state_steps(&self) -> i32 {
        match self.state {
            AIState::Waiting => {
                CREEP_ADVANCE_VELOCITY * 5 + DYING_DELAY * 5 // 15 + 450 = 465
            }
            AIState::Shattering => {
                self.garbage_shatter_delay()
            }
        }
    }

    /// Time for shatter animation.
    fn garbage_shatter_delay(&self) -> i32 {
        INITIAL_POP_DELAY
            + (self.last_shatter_height * PLAY_WIDTH as i32 * INTERNAL_POP_DELAY)
            + FINAL_POP_DELAY
    }

    /// Calculate garbage to send to the player when alarm fires.
    /// Also triggers shatter() on internal queue.
    pub(crate) fn garbage_amount(&mut self) -> Vec<(i32, i32, i32)> {
        let mut output = Vec::new();
        let current_height = self.queue.height();
        let working_height = (SAFE_HEIGHT as i32 - 1) - current_height;

        // Gray garbage
        let num_grays = if working_height > 0 { working_height % 3 } else { 0 };
        // Normal garbage
        let num_normals = current_height + working_height;

        // Add gray garbage (1×6 each)
        for _ in 0..num_grays {
            output.push((1, PLAY_WIDTH as i32, GARBAGE_FLAVOR_GRAY));
        }

        // Add normal garbage in chunks
        let norm_div = num_normals / 3;
        let norm_mod = num_normals % 3;
        if norm_div > 0 {
            output.push((norm_div, PLAY_WIDTH as i32, GARBAGE_FLAVOR_NORMAL));
        }
        for _ in 0..norm_mod {
            output.push((1, PLAY_WIDTH as i32, GARBAGE_FLAVOR_NORMAL));
        }

        // Shatter own queue after producing output
        self.shatter();

        output
    }

    /// Shatter the internal garbage queue.
    /// Remove a chunk; 50% chance to add back per removed element.
    fn shatter(&mut self) {
        if self.queue.height() > 0 {
            self.state = AIState::Shattering;
            let _gray_height = self.queue.special_height();
            self.last_shatter_height = self.queue.remove_with_specials();

            // 50% chance to add garbage back for each removed piece
            for _ in 0..self.last_shatter_height {
                if self.chance_in(2) {
                    self.queue.add(1, GARBAGE_FLAVOR_NORMAL);
                }
            }
        } else {
            self.state = AIState::Waiting;
            self.last_shatter_height = 0;
        }
    }

    /// Check if the AI has lost (queue height exceeds threshold).
    pub(crate) fn determine_loss(&mut self) -> bool {
        self.queue.height() > self.difficulty.loss_height()
    }

    /// Map queue height to 0..=12 level-light range. Proportional display value, not a literal gameplay level.
    pub(crate) fn get_grid_level(&mut self) -> i32 {
        let height = self.queue.height();
        let loss_h = self.difficulty.loss_height();
        if loss_h == 0 { return 0; }
        let level = (height * 12) / loss_h;
        level.min(12)
    }

    /// Access the garbage queue (for adding incoming garbage from player).
    pub(crate) fn garbage_queue_mut(&mut self) -> &mut GarbageQueue {
        &mut self.queue
    }

    pub fn difficulty(&self) -> AIDifficulty {
        self.difficulty
    }

    /// Reset for a new round.
    pub fn reset(&mut self) {
        self.queue.reset();
        self.state = AIState::Waiting;
        self.last_time = 0;
        self.last_shatter_height = 0;
    }

    /// Simple RNG: 1 in `chance` probability of returning true.
    fn chance_in(&mut self, chance: i32) -> bool {
        self.rng_state = self.rng_state.wrapping_mul(1103515245).wrapping_add(12345);
        (self.rng_state >> 16) as i32 % chance == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_garbage_queue_height() {
        let mut q = GarbageQueue::new();
        assert_eq!(q.height(), 0);
        q.add(3, GARBAGE_FLAVOR_NORMAL);
        assert_eq!(q.height(), 3);
        q.add(2, GARBAGE_FLAVOR_GRAY);
        assert_eq!(q.height(), 5);
        assert_eq!(q.special_height(), 2);
    }

    #[test]
    fn test_garbage_queue_remove_with_specials() {
        let mut q = GarbageQueue::new();
        q.add(1, GARBAGE_FLAVOR_NORMAL);
        q.add(2, GARBAGE_FLAVOR_NORMAL);
        q.add(1, GARBAGE_FLAVOR_GRAY);
        // Front is normal → remove until gray
        let removed = q.remove_with_specials();
        assert_eq!(removed, 2);
        assert_eq!(q.height(), 1); // Only gray remains
    }

    #[test]
    fn test_virtual_player_alarm() {
        let vp = VirtualPlayer::new(AIDifficulty::Medium);
        // alarm = 0 + 500 + 465 = 965
        assert_eq!(vp.alarm(), 965);
    }

    #[test]
    fn test_virtual_player_loss_detection() {
        let mut vp = VirtualPlayer::new(AIDifficulty::Easy);
        assert!(!vp.determine_loss()); // height=0, loss_height=4
        for _ in 0..4 {
            vp.garbage_queue_mut().add(1, GARBAGE_FLAVOR_NORMAL);
        }
        assert!(!vp.determine_loss()); // height=4: exactly at the limit, not a loss (strict >)
        vp.garbage_queue_mut().add(1, GARBAGE_FLAVOR_NORMAL);
        assert!(vp.determine_loss()); // height=5 > 4
    }

    #[test]
    fn test_virtual_player_grid_level() {
        let mut vp = VirtualPlayer::new(AIDifficulty::Medium);
        assert_eq!(vp.get_grid_level(), 0);
        for _ in 0..5 {
            vp.garbage_queue_mut().add(1, GARBAGE_FLAVOR_NORMAL);
        }
        // 5 * 12 / 10 = 6
        assert_eq!(vp.get_grid_level(), 6);
    }

    #[test]
    fn test_garbage_amount_produces_output() {
        let mut vp = VirtualPlayer::new(AIDifficulty::Medium);
        let output = vp.garbage_amount();
        // With empty queue: working_height=12, grays=0, normals=12
        // → a single 4×6 NORMAL piece (12 / 3 = 4, remainder 0).
        assert_eq!(output, vec![(4, PLAY_WIDTH as i32, GARBAGE_FLAVOR_NORMAL)]);
    }
}
