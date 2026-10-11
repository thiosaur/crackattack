
use crate::constants::*;
use crate::block::PopDirection;

// Garbage flavors
pub const GARBAGE_FLAVOR_NORMAL: i32 = 0;
pub const GARBAGE_FLAVOR_GRAY: i32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GarbageId(pub u16);

impl GarbageId {
    pub const INVALID: GarbageId = GarbageId(u16::MAX);

    pub fn is_valid(self) -> bool {
        self.0 != u16::MAX
    }
}

impl Default for GarbageId {
    fn default() -> Self {
        GarbageId::INVALID
    }
}

/// Garbage state
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[repr(u8)]
pub enum GarbageState {
    #[default]
    Static = 1,      // 1 << 0 = 1
    Falling = 2,     // 1 << 1 = 2
    Awaking = 4,     // 1 << 2 = 4
    Shattering = 8,  // 1 << 3 = 8
}

#[derive(Debug, Clone)]
pub struct Garbage {
    pub id: GarbageId,
    pub flavor: i32,
    pub grid_x: i32,
    pub grid_y: i32,
    pub width: i32,
    pub height: i32,
    pub sub_grid_y: i32,  // Fine Y position (sub-grid)
    pub(crate) physics_state: GarbageState,
    pub animation_countdown: i32,
    pub fall_delay_countdown: i32,  // Countdown until garbage begins falling
    pub pop_countdown: i32,
    pub pop_direction: PopDirection,
    pub pop_color: i32,
    pub sections_popped: i32,
    pub(crate) initial_fall: bool,
    pub combo_id: Option<u16>,
}

impl Default for Garbage {
    fn default() -> Self {
        Garbage {
            id: GarbageId::INVALID,
            flavor: GARBAGE_FLAVOR_NORMAL,
            grid_x: 0,
            grid_y: 0,
            width: 1,
            height: 1,
            sub_grid_y: 0,
            physics_state: GarbageState::Static,
            animation_countdown: 0,
            fall_delay_countdown: 0,
            pop_countdown: 0,
            pop_direction: PopDirection::None,
            pop_color: 0,
            sections_popped: 0,
            initial_fall: false,
            combo_id: None,
        }
    }
}

impl Garbage {
    pub(crate) fn new_static(id: GarbageId, x: i32, y: i32, width: i32, height: i32, flavor: i32) -> Self {
        Garbage {
            id,
            grid_x: x,
            grid_y: y,
            width,
            height,
            flavor,
            physics_state: GarbageState::Static,
            ..Default::default()
        }
    }

    pub(crate) fn is_active(&self) -> bool {
        self.id.is_valid()
    }

    pub fn is_static(&self) -> bool {
        self.physics_state == GarbageState::Static
    }

    pub(crate) fn is_falling(&self) -> bool {
        self.physics_state == GarbageState::Falling
    }

    pub fn is_shattering(&self) -> bool {
        self.physics_state == GarbageState::Shattering
    }

    pub fn is_awaking(&self) -> bool {
        self.physics_state == GarbageState::Awaking
    }

    /// Determine if this garbage should shatter when touched by another garbage.
    /// due_to = None means direct contact with elimination (always shatters).
    pub(crate) fn consider_shattering(&self, due_to: Option<&Garbage>) -> bool {
        let Some(neighbor) = due_to else {
            return true;  // Direct contact always causes shatter
        };

        // Propagation rule: gray garbage only shatters through gray neighbors,
        // non-gray garbage only shatters through non-gray neighbors.
        match self.flavor {
            GARBAGE_FLAVOR_GRAY => neighbor.flavor == GARBAGE_FLAVOR_GRAY,
            _ => neighbor.flavor != GARBAGE_FLAVOR_GRAY,
        }
    }
}

// Garbage arena (pool)
// Bitmap allocation: always picks the lowest free ID.
pub struct GarbageArena {
    garbage: [Garbage; GARBAGE_STORE_SIZE],
    /// Bitmap: true = allocated, false = free
    store_map: [bool; GARBAGE_STORE_SIZE],
    garbage_count: usize,
}

impl GarbageArena {
    pub fn new() -> Self {
        let garbage = std::array::from_fn(|_| Garbage::default());
        let store_map = [false; GARBAGE_STORE_SIZE];
        GarbageArena { garbage, store_map, garbage_count: 0 }
    }

    /// Find lowest free ID
    fn find_free_id(&self) -> Option<u16> {
        for (n, &allocated) in self.store_map.iter().enumerate() {
            if !allocated {
                return Some(n as u16);
            }
        }
        None  // No free slots
    }

    pub(crate) fn allocate(&mut self, x: i32, y: i32, width: i32, height: i32, flavor: i32) -> Option<GarbageId> {
        let idx = self.find_free_id();
        debug_assert!(idx.is_some(),
            "GARBAGE POOL EXHAUSTED: {} active garbage", self.garbage_count);
        let idx = idx?;
        self.store_map[idx as usize] = true;
        self.garbage_count += 1;
        let id = GarbageId(idx);
        self.garbage[idx as usize] = Garbage::new_static(id, x, y, width, height, flavor);
        Some(id)
    }

    pub(crate) fn deallocate(&mut self, id: GarbageId) {
        if id.is_valid() {
            let idx = id.0 as usize;
            if self.store_map[idx] {
                // We just mark the id as invalid so is_active() returns false
                self.garbage[idx].id = GarbageId::INVALID;
                self.store_map[idx] = false;
                self.garbage_count -= 1;
            }
        }
    }

    pub fn get(&self, id: GarbageId) -> Option<&Garbage> {
        if id.is_valid() && (id.0 as usize) < GARBAGE_STORE_SIZE {
            debug_assert!(self.store_map[id.0 as usize],
                "STALE GARBAGE ID: slot {} not allocated", id.0);
            if self.store_map[id.0 as usize] { Some(&self.garbage[id.0 as usize]) } else { None }
        } else {
            None
        }
    }

    pub fn get_mut(&mut self, id: GarbageId) -> Option<&mut Garbage> {
        if id.is_valid() && (id.0 as usize) < GARBAGE_STORE_SIZE {
            debug_assert!(self.store_map[id.0 as usize],
                "STALE GARBAGE ID: slot {} not allocated", id.0);
            if self.store_map[id.0 as usize] { Some(&mut self.garbage[id.0 as usize]) } else { None }
        } else {
            None
        }
    }

    pub fn iter_active(&self) -> impl Iterator<Item = &Garbage> {
        self.garbage.iter().filter(|g| g.is_active())
    }

    pub(crate) fn iter_active_mut(&mut self) -> impl Iterator<Item = &mut Garbage> {
        self.garbage.iter_mut().filter(|g| g.is_active())
    }

    pub fn count_active(&self) -> usize {
        self.garbage_count
    }
}

impl Default for GarbageArena {
    fn default() -> Self {
        Self::new()
    }
}
