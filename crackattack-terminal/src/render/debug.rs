// `--debug`: the state of the two cells under the cursor, below the board.

use crackattack_core::frame_data::{CellDebugInfo, FrameData};
use crackattack_core::grid::{BlockPhase, CellType, EmptyPhase, GarbagePhase};
use std::fmt::Write;
use std::sync::atomic::AtomicBool;

/// `--debug`: shows the state of the two cells under the cursor below the board.
pub(crate) static SHOW_DEBUG: AtomicBool = AtomicBool::new(false);

/// Phase names by cell type, keyed by the core enums' values.
const EMPTY_PHASES: &[(u8, &str)] =
    &[(EmptyPhase::Static as u8, "Static"), (EmptyPhase::Immutable as u8, "Immutable")];
const BLOCK_PHASES: &[(u8, &str)] = &[
    (BlockPhase::Static as u8, "Static"),
    (BlockPhase::Falling as u8, "Falling"),
    (BlockPhase::HangingThenFall as u8, "Hanging"),
    (BlockPhase::Awaking as u8, "Awaking"),
    (BlockPhase::Swapping as u8, "Swapping"),
    (BlockPhase::Dying as u8, "Dying"),
];
const GARBAGE_PHASES: &[(u8, &str)] = &[
    (GarbagePhase::Static as u8, "Static"),
    (GarbagePhase::Falling as u8, "Falling"),
    (GarbagePhase::HangingThenFall as u8, "Hanging"),
    (GarbagePhase::Awaking as u8, "Awaking"),
    (GarbagePhase::Shattering as u8, "Shatter"),
];

fn push_cell_debug(out: &mut String, cell: &CellDebugInfo) {
    let (type_char, phases) = match cell.cell_type {
        t if t == CellType::Empty as u8 => ('E', EMPTY_PHASES),
        t if t == CellType::Block as u8 => ('B', BLOCK_PHASES),
        t if t == CellType::Garbage as u8 => ('G', GARBAGE_PHASES),
        _ => ('?', &[][..]),
    };
    let phase_name = phases
        .iter()
        .find(|&&(phase, _)| phase == cell.phase)
        .map_or("???", |&(_, name)| name);
    let _ = write!(out, "{}:p{}({}) s{:02b}", type_char, cell.phase, phase_name, cell.block_state);
}

/// The debug line: both cells under the cursor.
pub(super) fn push_swap_cells(out: &mut String, frame: &FrameData) {
    out.push_str("Swap cells: L=");
    push_cell_debug(out, &frame.debug_left_cell);
    out.push_str(" R=");
    push_cell_debug(out, &frame.debug_right_cell);
}
