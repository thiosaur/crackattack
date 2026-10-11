// BSD/macOS LCG random number generator for deterministic simulation.

/// Simple LCG random number generator matching BSD/macOS rand() behavior
pub(crate) struct Random {
    state: u32,
}

impl Random {
    /// Create a new RNG.
    pub(crate) fn new(seed: u32) -> Self {
        Random { state: seed }
    }


    /// Get current RNG state (for fingerprinting)
    pub(crate) fn state(&self) -> u32 {
        self.state
    }

    /// Generate next random number (like rand).
    /// Uses BSD/macOS algorithm: state = state * 1103515245 + 12345, return 31 bits.
    #[allow(clippy::should_implement_trait)] // Named 'next' to match external RNG interface
    pub(crate) fn next(&mut self) -> i32 {
        self.state = self.state.wrapping_mul(1103515245).wrapping_add(12345);
        // Return 31 bits (same as BSD/macOS rand())
        (self.state & 0x7fffffff) as i32
    }

    /// Random number in range [0, max)
    pub(crate) fn number(&mut self, max: i32) -> i32 {
        debug_assert!(max > 0, "Random::number called with max={}", max);
        if max <= 0 {
            return 0;
        }
        self.next() % max
    }

    /// Random number when max is a power of 2
    pub(crate) fn number2(&mut self, max: i32) -> i32 {
        self.next() & (max - 1)
    }

    /// 1 in `chance` probability of returning true
    pub(crate) fn chance_in(&mut self, chance: i32) -> bool {
        self.number(chance) == 0
    }

}

impl Default for Random {
    fn default() -> Self {
        Random::new(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_determinism() {
        let mut rng1 = Random::new(12345);
        let mut rng2 = Random::new(12345);

        for _ in 0..100 {
            assert_eq!(rng1.next(), rng2.next());
        }
    }

    /// Reference vector, computed independently of this code (Python, same formula
    /// `state * 1103515245 + 12345 mod 2^32`, 31 bits returned). The determinism test above
    /// is true for any generator; this one fails if a constant changes, which would silently
    /// break the C++/Rust parity and every recorded game.
    #[test]
    fn test_reference_vector() {
        let mut rng = Random::new(12345);
        let got: Vec<i32> = (0..5).map(|_| rng.next()).collect();
        assert_eq!(got, [1406932606, 654583775, 1449466924, 229283573, 1109335178]);
        assert_eq!(rng.state(), 3256818826);
        // Seed 0 starts with the increment itself.
        assert_eq!(Random::new(0).next(), 12345);
    }

    #[test]
    fn test_number_range() {
        let mut rng = Random::new(42);
        for _ in 0..1000 {
            let n = rng.number(10);
            assert!(n >= 0 && n < 10);
        }
    }
}
