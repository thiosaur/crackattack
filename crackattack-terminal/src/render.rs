// Terminal renderer - ANSI colors + UTF-8 symbols

use crackattack_core::frame_data::{FrameData, CellDebugInfo};
use crackattack_core::constants::*;

// ANSI escape codes
pub(crate) const RESET: &str = "\x1b[0m";
pub(crate) const CLEAR: &str = "\x1b[2J";
pub(crate) const HOME: &str = "\x1b[H";
pub(crate) const HIDE_CURSOR: &str = "\x1b[?25l";
pub(crate) const SHOW_CURSOR: &str = "\x1b[?25h";
pub(crate) const ALT_SCREEN_ON: &str = "\x1b[?1049h";
pub(crate) const ALT_SCREEN_OFF: &str = "\x1b[?1049l";

// Colors (indexed by flavor 0-5)
const COLORS: [&str; 6] = [
    "\x1b[35m",  // Purple (magenta)
    "\x1b[34m",  // Blue
    "\x1b[32m",  // Green
    "\x1b[33m",  // Yellow
    "\x1b[91m",  // Orange (light red)
    "\x1b[90m",  // Gray
];

// Dim colors for nascent row
const DIM_COLORS: [&str; 6] = [
    "\x1b[2;35m",  // Dim Purple
    "\x1b[2;34m",  // Dim Blue
    "\x1b[2;32m",  // Dim Green
    "\x1b[2;33m",  // Dim Yellow
    "\x1b[2;91m",  // Dim Orange
    "\x1b[2;90m",  // Dim Gray
];

// UTF-8 symbols
const BLOCK: &str = "■";
const EMPTY: &str = "·";
const GARBAGE: &str = "█";
const DYING: &str = "×";

pub(crate) fn initialize() {
    print!("{}{}{}", ALT_SCREEN_ON, HIDE_CURSOR, CLEAR);
}

pub(crate) fn cleanup() {
    print!("{}{}", SHOW_CURSOR, ALT_SCREEN_OFF);
}

pub(crate) fn render(frame: &FrameData) {
    // Build grid
    let mut grid: [[Option<CellDisplay>; PLAY_WIDTH]; VISIBLE_HEIGHT] =
        [[None; PLAY_WIDTH]; VISIBLE_HEIGHT];

    // Fill blocks
    for block in &frame.blocks {
        let x = block.grid_x as usize;
        let y = block.grid_y as usize;
        if x < PLAY_WIDTH && y < VISIBLE_HEIGHT {
            let is_dying = (block.state & 0x08) != 0;  // BlockState::DYING
            grid[y][x] = Some(CellDisplay::Block {
                flavor: block.flavor as usize,
                dying: is_dying,
            });
        }
    }

    // Fill garbage
    for garbage in &frame.garbage {
        // Skip garbage that is mostly shattered
        if garbage.shatter_progress > 0.3 {
            continue;
        }

        let gx = garbage.grid_x as usize;
        // Compute effective Y position accounting for sub-grid offset (falling animation)
        // sub_grid_y ranges from 0 to STEPS_PER_GRID (60), add half for rounding
        let gy = (garbage.grid_y + (garbage.sub_grid_y + STEPS_PER_GRID as i32 / 2) / STEPS_PER_GRID as i32) as usize;
        for dy in 0..garbage.height as usize {
            for dx in 0..garbage.width as usize {
                let x = gx + dx;
                let y = gy + dy;
                if x < PLAY_WIDTH && y < VISIBLE_HEIGHT {
                    grid[y][x] = Some(CellDisplay::Garbage);
                }
            }
        }
    }

    // Cursor position
    let cursor_x = frame.swapper.grid_x as usize;
    let cursor_y = frame.swapper.grid_y as usize;

    // Render
    print!("{}", HOME);

    // Render from top (SAFE_HEIGHT) down to row 1 (not row 0)
    for y in (1..=SAFE_HEIGHT).rev() {
        for x in 0..PLAY_WIDTH {
            // Left cursor bracket
            if x == cursor_x && y == cursor_y {
                print!("[");
            } else {
                print!(" ");
            }

            // Cell content
            match grid[y][x] {
                Some(CellDisplay::Block { flavor, dying }) => {
                    let color = COLORS.get(flavor).unwrap_or(&COLORS[0]);
                    let symbol = if dying { DYING } else { BLOCK };
                    print!("{}{}{}", color, symbol, RESET);
                }
                Some(CellDisplay::Garbage) => {
                    print!("{}{}{}", COLORS[5], GARBAGE, RESET);
                }
                None => {
                    print!("{}", EMPTY);
                }
            }

            // Right cursor bracket
            if x == cursor_x + 1 && y == cursor_y {
                print!("]");
            } else {
                print!(" ");
            }
        }

        // Score on right side
        if y == SAFE_HEIGHT {
            print!("   Score: {}", frame.score);
        }

        print!("\r\n");
    }

    // Render nascent row (next row to appear) - dimmed, same spacing as normal rows
    for x in 0..PLAY_WIDTH {
        let flavor = frame.next_row[x] as usize;
        let color = DIM_COLORS.get(flavor).unwrap_or(&DIM_COLORS[0]);
        print!(" {}{}{} ", color, BLOCK, RESET);
    }
    print!("  [{:.0}%]", frame.play_offset_y * 100.0);
    print!("\r\n");

    // Debug: swapper cell info
    print!("\r\n");
    print!("Swap cells: L=");
    print_cell_debug(&frame.debug_left_cell);
    print!(" R=");
    print_cell_debug(&frame.debug_right_cell);
    print!("\r\n");

    // Game over message
    if frame.game_over {
        print!("\r\n   GAME OVER!\r\n");
    }
}

fn print_cell_debug(cell: &CellDebugInfo) {
    let type_char = match cell.cell_type {
        0 => 'E',  // Empty
        1 => 'B',  // Block
        2 => 'G',  // Garbage
        _ => '?',
    };
    let phase_name = match (cell.cell_type, cell.phase) {
        (0, 0) => "Static",
        (0, 30) => "Immutable",
        (1, 0) => "Static",
        (1, 1) => "Falling",
        (1, 2) => "Hanging",
        (1, 3) => "Awaking",
        (1, 10) => "Swapping",
        (1, 11) => "Dying",
        (2, 0) => "Static",
        (2, 1) => "Falling",
        (2, 2) => "Hanging",
        (2, 3) => "Awaking",
        (2, 20) => "Shatter",
        _ => "???",
    };
    print!("{}:p{}({}) s{:02b}", type_char, cell.phase, phase_name, cell.block_state);
}

#[derive(Clone, Copy)]
enum CellDisplay {
    Block { flavor: usize, dying: bool },
    Garbage,
}
