// GarbageGenerator - Queues and releases garbage

use crate::constants::*;
use crate::random::Random;

/// Element in the garbage queue
#[derive(Debug, Clone, Default)]
pub(crate) struct GarbageQueueElement {
    pub active: bool,
    pub height: i32,
    pub width: i32,
    pub flavor: i32,
    pub alarm: i32,  // Tick when this garbage should drop
}

/// Garbage generator - queues garbage and releases it after a delay
#[derive(Debug, Clone)]
pub(crate) struct GarbageGenerator {
    queue: [GarbageQueueElement; GARBAGE_QUEUE_SIZE],
    waiting_count: i32,
}

impl GarbageGenerator {
    pub(crate) fn new() -> Self {
        GarbageGenerator {
            queue: Default::default(),
            waiting_count: 0,
        }
    }

    /// Add garbage to the queue
    /// In fingerprint mode (deterministic), drop_time = timestamp + AVERAGE_DELAY
    /// Otherwise, adds some randomness
    pub(crate) fn add_to_queue(&mut self, height: i32, width: i32, flavor: i32,
                        timestamp: i32, rng: Option<&mut Random>) {
        if self.waiting_count as usize >= GARBAGE_QUEUE_SIZE {
            #[cfg(debug_assertions)]
            eprintln!("WARNING: Garbage queue full, dropping {}x{} flavor={}", height, width, flavor);
            return;
        }

        // Find free slot
        let mut slot_idx = None;
        for (i, elem) in self.queue.iter().enumerate() {
            if !elem.active {
                slot_idx = Some(i);
                break;
            }
        }

        if let Some(i) = slot_idx {
            let drop_time = self.determine_drop_time(timestamp, rng);

            self.queue[i] = GarbageQueueElement {
                active: true,
                height,
                width,
                flavor,
                alarm: drop_time,
            };
            self.waiting_count += 1;
        }
    }

    /// Determine when garbage should drop
    fn determine_drop_time(&self, timestamp: i32, rng: Option<&mut Random>) -> i32 {
        match rng {
            // With RNG: add randomness (normal gameplay)
            Some(rng) => {
                timestamp
                    + (AVERAGE_GARBAGE_DROP_DELAY - SPREAD_GARBAGE_DROP_DELAY / 2)
                    + rng.number(SPREAD_GARBAGE_DROP_DELAY)
            }
            // Without RNG: deterministic (fingerprint mode)
            None => timestamp + AVERAGE_GARBAGE_DROP_DELAY
        }
    }

    /// Process queue and hand due garbage to the provided callback.
    /// The callback cannot fail. On a creation failure the CALLER re-queues the garbage
    /// through `add_to_queue`.
    pub(crate) fn time_step_with_callback<F>(&mut self, current_tick: i32, mut create: F)
    where
        F: FnMut(i32, i32, i32),
    {
        let mut remaining = self.waiting_count;

        for elem in &mut self.queue {
            if remaining <= 0 {
                break;
            }

            if elem.active {
                remaining -= 1;

                if elem.alarm < current_tick {
                    create(elem.height, elem.width, elem.flavor);
                    elem.active = false;
                    self.waiting_count -= 1;
                }
            }
        }
    }

    /// Process queue and return garbage ready to drop (no retry on failure).
    // Production code uses `time_step_with_callback` (game_state/mod.rs). This Vec variant
    // only serves the queue tests.
    #[cfg(test)]
    pub fn time_step(&mut self, current_tick: i32) -> Vec<(i32, i32, i32)> {
        let mut to_create = Vec::new();
        self.time_step_with_callback(current_tick, |h, w, f| {
            to_create.push((h, w, f));
        });
        to_create
    }

    /// Get number of waiting garbage
    #[cfg(test)]
    pub fn waiting_count(&self) -> i32 {
        self.waiting_count
    }
}

impl Default for GarbageGenerator {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_add_to_queue() {
        let mut gen = GarbageGenerator::new();

        gen.add_to_queue(2, 6, 0, 100, None);
        assert_eq!(gen.waiting_count(), 1);

        gen.add_to_queue(1, 3, 0, 150, None);
        assert_eq!(gen.waiting_count(), 2);
    }

    #[test]
    fn test_time_step_releases() {
        let mut gen = GarbageGenerator::new();

        // Add garbage at tick 100, should drop at 100 + 300 = 400
        gen.add_to_queue(2, 6, 0, 100, None);
        assert_eq!(gen.waiting_count(), 1);

        // At tick 300, not ready yet
        let dropped = gen.time_step(300);
        assert!(dropped.is_empty());
        assert_eq!(gen.waiting_count(), 1);

        // At tick 400 (the alarm itself), still not released: the test is strict (<)
        let dropped = gen.time_step(400);
        assert!(dropped.is_empty());

        // At tick 401, should drop
        let dropped = gen.time_step(401);
        assert_eq!(dropped.len(), 1);
        assert_eq!(dropped[0], (2, 6, 0));
        assert_eq!(gen.waiting_count(), 0);
    }

    #[test]
    fn test_queue_full() {
        let mut gen = GarbageGenerator::new();

        // Fill the queue
        for i in 0..GARBAGE_QUEUE_SIZE {
            gen.add_to_queue(1, 1, 0, i as i32 * 10, None);
        }
        assert_eq!(gen.waiting_count() as usize, GARBAGE_QUEUE_SIZE);

        // Try to add one more - should be ignored
        gen.add_to_queue(1, 1, 0, 1000, None);
        assert_eq!(gen.waiting_count() as usize, GARBAGE_QUEUE_SIZE);
    }
}
