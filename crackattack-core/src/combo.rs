use crate::constants::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ComboId(pub u16);

impl ComboId {
    pub(crate) const INVALID: ComboId = ComboId(u16::MAX);

    pub(crate) fn is_valid(self) -> bool {
        self.0 != u16::MAX
    }
}

impl Default for ComboId {
    fn default() -> Self {
        ComboId::INVALID
    }
}

#[derive(Debug, Clone)]
pub(crate) struct Combo {
    pub id: ComboId,
    pub(crate) involvement_count: i32,  // Blocks involved
    pub multiplier: i32,         // Chain multiplier
    pub blocks_eliminated: i32,  // Total blocks eliminated
    pub(crate) creation_time: i32,      // Time step when created
    pub(crate) time_stamp: i32,         // Time step of most recent elimination
    pub magnitude: i32,          // Current step's normal elimination count
    pub(crate) special_magnitude: i32,  // Gray/colorless elimination count (tracked separately from normal magnitude)
    pub(crate) latest_magnitude: i32,   // Computed for spark count: magnitude + special_magnitude + multiplier - 1
    pub x: i32,                   // Grid X of last elimination kernel (for sign/mote placement)
    pub y: i32,                   // Grid Y of last elimination kernel
    /// Flavor of the block that kicked off the most recent
    /// elimination for this combo (overwritten on every step so
    /// the value reflects the *latest* chain step). `-1` if the
    /// combo has never eliminated anything yet. Used by the
    /// chain-ring renderer to tint rings with the combo colour.
    pub flavor: i32,
    /// Cells from the most recent elimination step (for ChainProgressed event).
    pub(crate) last_trigger_cells: Vec<(i32, i32)>,
}

impl Default for Combo {
    fn default() -> Self {
        Combo {
            id: ComboId::INVALID,
            involvement_count: 0,
            multiplier: 1,
            blocks_eliminated: 0,
            creation_time: 0,
            time_stamp: 0,
            magnitude: 0,
            special_magnitude: 0,
            latest_magnitude: 0,
            x: 0,
            y: 0,
            flavor: -1,
            last_trigger_cells: Vec::new(),
        }
    }
}

impl Combo {
    pub(crate) fn is_active(&self) -> bool {
        self.id.is_valid()
    }

    pub(crate) fn increment_involvement(&mut self) {
        self.involvement_count += 1;
    }

    pub(crate) fn decrement_involvement(&mut self) {
        assert!(self.involvement_count > 0,
            "COMBO INVOLVEMENT UNDERFLOW: combo {:?} has count {} before decrement \
             (creation_time={}, time_stamp={})",
            self.id, self.involvement_count, self.creation_time, self.time_stamp);
        self.involvement_count -= 1;
    }

    /// Record an elimination for this combo.
    ///
    /// Separate normal magnitude from special (gray) magnitude for accurate garbage calculation.
    pub(crate) fn record_elimination(&mut self, count: i32, is_special: bool, current_time: i32) {
        // Record when this elimination happened
        self.time_stamp = current_time;

        // Track total blocks eliminated
        self.blocks_eliminated += count;

        // Only increase multiplier if this is a chain (elimination on a later tick)
        // Multiplier signs start at chain 3. Chain 2 increases the multiplier but does not emit a visible multiplier sign.
        let is_chain = current_time != self.creation_time;
        if is_chain {
            self.multiplier += 1;
        }

        // Accumulate in the correct counter (normal vs special)
        if is_special {
            self.special_magnitude += count;
        } else {
            self.magnitude += count;
        }
    }

    /// Finalize this combo for the current time step.
    ///
    /// This computes latest_magnitude and resets magnitude/special_magnitude.
    /// Should be called once per tick AFTER all eliminations are processed.
    pub(crate) fn finalize_step(&mut self, current_time: i32) {
        // Only process if we had an elimination this tick
        if self.time_stamp == current_time {
            // Compute latest_magnitude including special_magnitude
            self.latest_magnitude = self.special_magnitude + self.magnitude + self.multiplier - 1;

            // Reset magnitudes for next tick
            self.magnitude = 0;
            self.special_magnitude = 0;
        }
    }
}

// Combo arena
use std::collections::BinaryHeap;
use std::cmp::Reverse;

pub(crate) struct ComboArena {
    combos: [Combo; COMBO_STORE_SIZE],
    // Use min-heap to always allocate lowest free ID
    free_heap: BinaryHeap<Reverse<u16>>,
}

impl ComboArena {
    pub(crate) fn new() -> Self {
        let combos = std::array::from_fn(|_| Combo::default());
        let free_heap: BinaryHeap<Reverse<u16>> = (0..COMBO_STORE_SIZE as u16).map(Reverse).collect();
        ComboArena { combos, free_heap }
    }

    pub(crate) fn allocate(&mut self, time_step: i32) -> Option<ComboId> {
        let Reverse(idx) = self.free_heap.pop()?;
        debug_assert!(!self.combos[idx as usize].is_active(),
            "COMBO DOUBLE ALLOC: slot {} is still active! (id={:?}, involvement={})",
            idx, self.combos[idx as usize].id, self.combos[idx as usize].involvement_count);
        let id = ComboId(idx);
        self.combos[idx as usize] = Combo {
            id,
            involvement_count: 0,
            multiplier: 1,
            blocks_eliminated: 0,
            creation_time: time_step,
            time_stamp: 0,
            magnitude: 0,
            special_magnitude: 0,
            latest_magnitude: 0,
            x: 0,
            y: 0,
            flavor: -1,
            last_trigger_cells: Vec::new(),
        };
        Some(id)
    }

    pub(crate) fn deallocate(&mut self, id: ComboId) {
        if id.is_valid() {
            let idx = id.0 as usize;
            // Deallocated combos retain data; blocks may carry stale combo references that trigger resurrection.
            // Just mark id as invalid so is_active() returns false
            self.combos[idx].id = ComboId::INVALID;
            self.free_heap.push(Reverse(id.0));
        }
    }

    /// Access a combo, including deallocated ones.
    /// The chain flow needs this on purpose: a block can carry a `combo_id` past
    /// deallocation (the data is retained and the combo can be resurrected, see
    /// `resurrect`). Elsewhere, prefer `get`.
    pub(crate) fn get_including_dead(&self, id: ComboId) -> Option<&Combo> {
        if id.is_valid() {
            Some(&self.combos[id.0 as usize])
        } else {
            None
        }
    }

    /// Variante mutable de [`Self::get_including_dead`].
    pub(crate) fn get_mut_including_dead(&mut self, id: ComboId) -> Option<&mut Combo> {
        if id.is_valid() {
            Some(&mut self.combos[id.0 as usize])
        } else {
            None
        }
    }

    /// Resurrect a deallocated combo (blocks may reference stale IDs).
    ///
    /// When a block with an old chain_combo_id lands after the combo was deallocated,
    /// the combo data is still in memory. This method restores the combo's id and
    /// removes it from the free heap so it won't be double-allocated.
    ///
    /// Returns true if resurrection occurred, false if combo was already active.
    pub(crate) fn resurrect(&mut self, id: ComboId) -> bool {
        if !id.is_valid() {
            return false;
        }
        let combo = &mut self.combos[id.0 as usize];
        if combo.is_active() {
            return false; // Already active, nothing to do
        }
        // GUARD: resurrection means a block carried a combo_id past deallocation.
        // Stale reference path — the combo is resurrected with its old data.
        #[cfg(debug_assertions)]
        eprintln!("COMBO RESURRECT: slot {} resurrected (creation_time={}, involvement={})",
            id.0, combo.creation_time, combo.involvement_count);
        // Resurrect: restore id
        combo.id = id;
        // Remove from free_heap to prevent double-allocation
        // Rebuild heap without this id (O(n) but resurrection is rare)
        let id_val = id.0;
        self.free_heap = self.free_heap.drain().filter(|r| r.0 != id_val).collect();
        true
    }

    /// Iterate over all active combos
    pub(crate) fn iter_active(&self) -> impl Iterator<Item = &Combo> {
        self.combos.iter().filter(|c| c.is_active())
    }

    /// Iterate over all active combos (mutably)
    pub(crate) fn iter_active_mut(&mut self) -> impl Iterator<Item = &mut Combo> {
        self.combos.iter_mut().filter(|c| c.is_active())
    }
}

impl Default for ComboArena {
    fn default() -> Self {
        Self::new()
    }
}
