//! Visual-only state for garbage "flavor" image assignment.

pub const GARBAGE_FLAVOR_COUNT: usize = 7;
const MIN_FLAVOR_HEIGHT: i32 = 4;

pub struct GarbageFlavorImageState {
    pub active_garbage_id: Option<i32>,
    pub flavor_tex_idx: usize,
    pub flavor_x: i32,
    pub flavor_y: i32,
    /// Visual-only RNG state for flavor image selection.
    pub rng_state: u32,
}

impl Default for GarbageFlavorImageState {
    fn default() -> Self {
        Self {
            active_garbage_id: None,
            flavor_tex_idx: 0,
            flavor_x: 0,
            flavor_y: 0,
            // Fixed non-zero seed. The flavor choice is visual only, but it must be
            // reproducible (render dumps, render parity tests), so it never depends on
            // the wall clock.
            rng_state: 0x9E37_79B9,
        }
    }
}

impl GarbageFlavorImageState {
    pub fn next_random_index(&mut self, max: usize) -> usize {
        debug_assert!(max > 0);
        self.rng_state = self.rng_state.wrapping_mul(1103515245).wrapping_add(12345);
        ((self.rng_state & 0x7fff_ffff) as usize) % max
    }

    pub fn update(&mut self, garbages: &[crackattack_core::frame_data::GarbageSnapshot]) {
        // Drop the image when its garbage is gone.
        if let Some(id) = self.active_garbage_id {
            if !garbages.iter().any(|g| g.id == id) {
                self.active_garbage_id = None;
            }
        }

        if self.active_garbage_id.is_some() {
            return;
        }

        // Assign the image to the first eligible garbage.
        for g in garbages {
            if g.height < MIN_FLAVOR_HEIGHT || g.width < 3 {
                continue;
            }
            self.active_garbage_id = Some(g.id);
            self.flavor_tex_idx = self.next_random_index(GARBAGE_FLAVOR_COUNT);
            // Position derived from the garbage id (deterministic).
            let id = g.id as u32;
            self.flavor_x = 1 + (id as i32 % (g.width - 2).max(1));
            self.flavor_y = 1;
            break;
        }
    }
}
