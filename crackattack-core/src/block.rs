// Block state and structure

use bitflags::bitflags;
use crate::constants::*;

bitflags! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
    pub struct BlockState: u8 {
        const NONE          = 0;
        const STATIC        = 1 << 0;
        const SWAPPING      = 1 << 1;
        const FALLING       = 1 << 2;
        const DYING         = 1 << 3;
        const AWAKING       = 1 << 4;
        const SWAP_DIRECTION = 1 << 5;  // 0=left, 1=right
    }
}

/// Pop direction for block/garbage animations. Cycles through four values.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[repr(u8)]
pub enum PopDirection {
    #[default]
    None = 0,
    Dir1 = 1,       // 1 << 0
    Dir2 = 2,       // 1 << 1
    Dir3 = 4,       // 1 << 2
    Dir4 = 8,       // 1 << 3
}

impl PopDirection {
    /// Shift left, wrapping Dir4 back to Dir1
    /// Sequence: Dir1 -> Dir2 -> Dir3 -> Dir4 -> Dir1 -> ...
    pub fn next(self) -> PopDirection {
        match self {
            PopDirection::None | PopDirection::Dir1 => PopDirection::Dir2,
            PopDirection::Dir2 => PopDirection::Dir3,
            PopDirection::Dir3 => PopDirection::Dir4,
            PopDirection::Dir4 => PopDirection::Dir1,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BlockId(pub u16);

impl BlockId {
    pub const INVALID: BlockId = BlockId(u16::MAX);

    pub fn is_valid(self) -> bool {
        self.0 != u16::MAX
    }
}

impl Default for BlockId {
    fn default() -> Self {
        BlockId::INVALID
    }
}

#[derive(Debug, Clone)]
pub struct Block {
    pub id: BlockId,
    pub flavor: i32,
    pub grid_x: i32,
    pub grid_y: i32,
    pub sub_grid_y: i32,  // Fine Y position (sub-grid)
    pub physics_flags: BlockState,
    pub animation_countdown: i32,
    pub fall_delay_countdown: i32,  // Countdown until block begins falling
    pub pop_countdown: i32,
    pub pop_direction: PopDirection,
    pub pop_color: i32,
    pub combo_id: Option<u16>,
    // Animation floats (derived from integers, deterministic)
    pub axis_x: f32,
    pub axis_y: f32,
}

impl Default for Block {
    fn default() -> Self {
        Block {
            id: BlockId::INVALID,
            flavor: 0,
            grid_x: 0,
            grid_y: 0,
            sub_grid_y: 0,
            physics_flags: BlockState::NONE,
            animation_countdown: 0,
            fall_delay_countdown: 0,
            pop_countdown: 0,
            pop_direction: PopDirection::None,
            pop_color: 0,
            combo_id: None,
            axis_x: 0.0,
            axis_y: 0.0,
        }
    }
}

impl Block {
    /// Initialize as static block.
    /// Note: pop_* fields are not reset (pool reuse semantics).
    pub(crate) fn initialize_static(&mut self, id: BlockId, x: i32, y: i32, flavor: i32) {
        self.id = id;
        self.grid_x = x;
        self.grid_y = y;
        self.flavor = flavor;
        self.sub_grid_y = 0;
        self.physics_flags = BlockState::STATIC;
        self.animation_countdown = 0;
        self.combo_id = None;
        // Preserves pop_* fields from pool reuse
    }

    pub(crate) fn is_active(&self) -> bool {
        self.id.is_valid()
    }

    pub fn is_static(&self) -> bool {
        self.physics_flags.contains(BlockState::STATIC)
    }

    pub(crate) fn is_falling(&self) -> bool {
        self.physics_flags.contains(BlockState::FALLING)
    }

    pub fn is_dying(&self) -> bool {
        self.physics_flags.contains(BlockState::DYING)
    }

    pub fn is_awaking(&self) -> bool {
        self.physics_flags.contains(BlockState::AWAKING)
    }

    /// Initialize block as awaking (from garbage shattering).
    /// `animation_countdown` and `pop_countdown` are absolute timestamps:
    /// `time_step + awake_delay` and `time_step + pop_delay`.
    pub(crate) fn initialize_awaking(
        &mut self,
        x: i32,
        y: i32,
        flavor: i32,
        pop_delay: i32,
        awake_delay: i32,
        pop_direction: PopDirection,
        pop_color: i32,
        combo_id: Option<u16>,
        time_step: i32,  // Current game time for absolute timestamp
    ) {
        self.grid_x = x;
        self.grid_y = y;
        self.flavor = flavor;
        self.sub_grid_y = 0;
        self.physics_flags = BlockState::AWAKING;
        self.animation_countdown = time_step + awake_delay;
        self.pop_countdown = time_step + pop_delay;
        self.pop_direction = pop_direction;
        self.pop_color = pop_color;
        self.combo_id = combo_id;
    }
}

// Block arena (pool)
// Bitmap allocation: always picks the lowest free ID.
pub struct BlockArena {
    blocks: [Block; BLOCK_STORE_SIZE],
    /// Bitmap: true = allocated, false = free
    store_map: [bool; BLOCK_STORE_SIZE],
    block_count: usize,
}

impl BlockArena {
    pub fn new() -> Self {
        let blocks = std::array::from_fn(|_| Block::default());
        let store_map = [false; BLOCK_STORE_SIZE];
        BlockArena { blocks, store_map, block_count: 0 }
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

    /// Allocate the lowest free ID to keep pool ordering deterministic.
    pub(crate) fn allocate(&mut self, x: i32, y: i32, flavor: i32) -> Option<BlockId> {
        let idx = self.find_free_id();
        debug_assert!(idx.is_some(),
            "BLOCK POOL EXHAUSTED: {} active blocks", self.block_count);
        let idx = idx?;
        self.store_map[idx as usize] = true;
        self.block_count += 1;
        let id = BlockId(idx);
        // initialize_static preserves pop_* fields from pool reuse
        self.blocks[idx as usize].initialize_static(id, x, y, flavor);
        Some(id)
    }

    /// Allocate and initialize an awaking block (from garbage shattering)
    /// time_step is required to compute absolute timestamps for animation_countdown/pop_countdown
    pub(crate) fn allocate_awaking(
        &mut self,
        x: i32,
        y: i32,
        flavor: i32,
        pop_delay: i32,
        awake_delay: i32,
        pop_direction: PopDirection,
        pop_color: i32,
        combo_id: Option<u16>,
        time_step: i32,
    ) -> Option<BlockId> {
        let idx = self.find_free_id()?;
        self.store_map[idx as usize] = true;
        self.block_count += 1;
        let id = BlockId(idx);
        let block = &mut self.blocks[idx as usize];
        block.id = id;
        block.initialize_awaking(
            x, y, flavor,
            pop_delay, awake_delay,
            pop_direction, pop_color, combo_id,
            time_step,
        );
        Some(id)
    }

    pub(crate) fn deallocate(&mut self, id: BlockId) {
        if id.is_valid() {
            let idx = id.0 as usize;
            if self.store_map[idx] {
                // We just mark the id as invalid so is_active() returns false
                self.blocks[idx].id = BlockId::INVALID;
                self.store_map[idx] = false;
                self.block_count -= 1;
            }
        }
    }

    pub fn get(&self, id: BlockId) -> Option<&Block> {
        if id.is_valid() && (id.0 as usize) < BLOCK_STORE_SIZE {
            debug_assert!(self.store_map[id.0 as usize],
                "STALE BLOCK ID: slot {} not allocated", id.0);
            if self.store_map[id.0 as usize] { Some(&self.blocks[id.0 as usize]) } else { None }
        } else {
            None
        }
    }

    pub fn get_mut(&mut self, id: BlockId) -> Option<&mut Block> {
        if id.is_valid() && (id.0 as usize) < BLOCK_STORE_SIZE {
            debug_assert!(self.store_map[id.0 as usize],
                "STALE BLOCK ID: slot {} not allocated", id.0);
            if self.store_map[id.0 as usize] { Some(&mut self.blocks[id.0 as usize]) } else { None }
        } else {
            None
        }
    }

    pub fn iter_active(&self) -> impl Iterator<Item = &Block> {
        self.blocks.iter().filter(|b| b.is_active())
    }

    pub(crate) fn iter_active_mut(&mut self) -> impl Iterator<Item = &mut Block> {
        self.blocks.iter_mut().filter(|b| b.is_active())
    }

    pub fn count_active(&self) -> usize {
        self.block_count
    }
}

impl Default for BlockArena {
    fn default() -> Self {
        Self::new()
    }
}
