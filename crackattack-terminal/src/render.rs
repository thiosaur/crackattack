// Terminal renderer - ANSI 256 colors + UTF-8 eighth blocks
//
// The playfield is painted on a fine vertical grid (16 units per row, 8 per terminal line),
// then each terminal character shows at most two colors split at an eighth (▁▂▃▄▅▆▇█).
// Blocks are solid, the whole stack rises smoothly above a fixed ground line, and falling
// blocks and garbage move by sixteenths of a row.

use crackattack_core::frame_data::{FrameData, CellDebugInfo};
use crackattack_core::constants::*;
use crackattack_core::garbage::GARBAGE_FLAVOR_GRAY;
use std::fmt::Write;

use crate::danger::DangerBars;
use std::sync::atomic::{AtomicBool, Ordering};

/// `--debug`: shows the state of the two cells under the cursor below the board.
pub(crate) static SHOW_DEBUG: AtomicBool = AtomicBool::new(false);

// ANSI escape codes
pub(crate) const RESET: &str = "\x1b[0m";
pub(crate) const CLEAR: &str = "\x1b[2J";
pub(crate) const HOME: &str = "\x1b[H";
pub(crate) const HIDE_CURSOR: &str = "\x1b[?25l";
pub(crate) const SHOW_CURSOR: &str = "\x1b[?25h";
pub(crate) const ALT_SCREEN_ON: &str = "\x1b[?1049h";
pub(crate) const ALT_SCREEN_OFF: &str = "\x1b[?1049l";
/// Synchronized update: the terminal shows the frame only once it is complete (no tearing).
const SYNC_BEGIN: &str = "\x1b[?2026h";
const SYNC_END: &str = "\x1b[?2026l";

/// Block colors (256-color palette), indexed by flavor: purple, blue, green, yellow,
/// orange, gray. Close to the colors of the graphical version.
const COLORS: [u8; 6] = [127, 62, 34, 184, 208, 244];
/// Rising row (row 0), still being pushed up: dark versions of the same colors.
const DIM_COLORS: [u8; 6] = [53, 17, 22, 58, 94, 238];
/// Dying block (about to vanish): a small white square at the center of its cell, which
/// shrinks during the second half of the delay.
const DYING_COLOR: u8 = 255;
const DYING_GLYPHS: [char; 2] = ['■', '▪'];
/// Garbage: normal (red) and gray.
const GARBAGE_COLOR: u8 = 124;
const GARBAGE_GRAY_COLOR: u8 = 242;
/// Brighter garbage, the moment it is hit (start of its shatter).
const GARBAGE_FLASH_COLOR: u8 = 167;
const GARBAGE_GRAY_FLASH_COLOR: u8 = 250;
const SHATTER_FLASH_TICKS: i32 = 8;
/// Last ticks before a block of a shattered garbage pops: it flips like a card, from the
/// garbage color to its own (as the graphical version, `4 * INTERNAL_POP_DELAY / 5`).
const POP_ROTATE_TIME: i32 = 4 * INTERNAL_POP_DELAY / 5;
/// How far two swapping blocks move apart vertically where they cross (units).
const SWAP_LIFT_UNITS: i32 = 5;

/// Ground line under the playfield: the line just above it is underlined, so the ground
/// takes no line of its own. The underline is dark gray where the terminal supports
/// underline colors (otherwise it takes the text color).
const GROUND: &str = "\x1b[4m\x1b[58;5;240m";

/// Fine vertical units: per row, and per terminal line.
const ROW_UNITS: i32 = 16;
const LINE_UNITS: i32 = 8;
/// Terminal columns per cell: a gap column (cursor brackets, garbage seams) then the block.
const CELL_COLUMNS: usize = 4;
const BLOCK_COLUMNS: usize = 3;
/// Width of the playfield in terminal columns (with the closing gap column on the right).
const FIELD_COLUMNS: usize = PLAY_WIDTH * CELL_COLUMNS + 1;
/// Lines shown below the ground: the part of the rising row not pushed up yet.
const LINES_BELOW_GROUND: i32 = 2;
/// Vertical span painted, in units: from the bottom of the rising row to the top of the
/// playfield.
const BOTTOM_UNIT: i32 = -LINES_BELOW_GROUND * LINE_UNITS;
const TOP_UNIT: i32 = SAFE_HEIGHT as i32 * ROW_UNITS;
const SPAN_UNITS: usize = (TOP_UNIT - BOTTOM_UNIT) as usize;

/// Marks the playable area: a dot at the center of each empty cell. The dots rise with
/// the stack; within a line the dot glyph sits low, middle or high to follow the center.
const DOT_COLOR: u8 = 240;
const DOTS: [char; 3] = ['.', '·', '˙'];
/// Cursor: two bars in the gap columns on each side of its two cells, painted like the
/// blocks so they have the same height and rise with them.
const CURSOR_COLOR: u8 = 231;

/// Danger bars on each side of the board (final countdown): purple, then red for the last
/// second; the dark shades while fading out.
const DANGER_PURPLE: [u8; 2] = [129, 54];
const DANGER_RED: [u8; 2] = [196, 88];
/// Space between each danger bar and the board.
const DANGER_GAP: &str = "   ";

/// Eighth blocks, from 1/8 to 7/8 of the height, drawn from the bottom.
const LOWER_EIGHTHS: [char; 7] = ['▁', '▂', '▃', '▄', '▅', '▆', '▇'];

pub(crate) fn initialize() {
    print!("{}{}{}", ALT_SCREEN_ON, HIDE_CURSOR, CLEAR);
}

pub(crate) fn cleanup() {
    print!("{}{}", SHOW_CURSOR, ALT_SCREEN_OFF);
}

/// Fine paint buffer: a color (or nothing) per terminal column and vertical unit.
struct Canvas {
    cells: Vec<Option<u8>>,
    /// Glyphs drawn over the board at (column, line): dying blocks.
    marks: Vec<(usize, i32, char)>,
}

impl Canvas {
    fn new() -> Self {
        Self { cells: vec![None; FIELD_COLUMNS * SPAN_UNITS], marks: Vec::new() }
    }

    fn mark(&self, col: usize, line: i32) -> Option<char> {
        self.marks.iter().find(|m| m.0 == col && m.1 == line).map(|m| m.2)
    }

    fn get(&self, col: usize, unit: i32) -> Option<u8> {
        let u = unit - BOTTOM_UNIT;
        if col >= FIELD_COLUMNS || u < 0 || u as usize >= SPAN_UNITS {
            return None;
        }
        self.cells[u as usize * FIELD_COLUMNS + col]
    }

    /// Paints columns `cols` over units `[bottom, top)`, clipped to the canvas.
    fn fill(&mut self, cols: std::ops::RangeInclusive<usize>, bottom: i32, top: i32, color: u8) {
        for unit in bottom.max(BOTTOM_UNIT)..top.min(TOP_UNIT) {
            let u = (unit - BOTTOM_UNIT) as usize;
            for col in cols.clone() {
                if col < FIELD_COLUMNS {
                    self.cells[u * FIELD_COLUMNS + col] = Some(color);
                }
            }
        }
    }
}

/// Bottom of a row in units above the ground, from its grid row, the stack rise and the
/// sub-row fall offset. Row 1 sits on the ground when the stack has not risen.
fn bottom_unit(grid_y: i32, sub_grid_y: i32, play_offset_y: f32) -> i32 {
    let rows = grid_y as f32 - 1.0 + play_offset_y + sub_grid_y as f32 / STEPS_PER_GRID as f32;
    (rows * ROW_UNITS as f32).round() as i32
}

/// Paints blocks and garbage on the fine grid.
fn paint(frame: &FrameData) -> Canvas {
    let mut canvas = Canvas::new();
    let rise = frame.play_offset_y;

    // Garbage first: the blocks a shattered garbage turns into are painted over it.
    for garbage in &frame.garbage {
        let flashing = garbage.shatter_progress > 0.0
            && garbage.shatter_progress * (SHATTER_TIME as f32) < SHATTER_FLASH_TICKS as f32;
        let color = garbage_color(garbage.flavor, flashing);
        let bottom = bottom_unit(garbage.grid_y, garbage.sub_grid_y, rise);
        let left = garbage.grid_x.max(0) as usize * CELL_COLUMNS + 1;
        let right = (garbage.grid_x + garbage.width).max(1) as usize * CELL_COLUMNS - 1;
        // One slab: the seams between its cells are filled.
        canvas.fill(left..=right, bottom, bottom + garbage.height * ROW_UNITS - 1, color);
    }

    let swap_factor = frame.swapper.swap_factor;
    for block in &frame.blocks {
        let x = block.grid_x as usize;
        if x >= PLAY_WIDTH || block.grid_y < 0 {
            continue;
        }
        let flavor = (block.flavor.max(0) as usize).min(COLORS.len() - 1);
        let is_dying = (block.state & 0x08) != 0; // BlockState::DYING
        let bottom = bottom_unit(block.grid_y, block.sub_grid_y, rise);
        if is_dying {
            // A small square on the line of the cell center, in the middle column.
            let center = bottom + ROW_UNITS / 2;
            let second_half = block.animation_countdown * 2 < DYING_DELAY;
            canvas.marks.push((
                x * CELL_COLUMNS + CELL_COLUMNS / 2,
                center.div_euclid(LINE_UNITS),
                DYING_GLYPHS[second_half as usize],
            ));
            continue;
        }
        let mut color = if block.grid_y == 0 {
            DIM_COLORS[flavor]
        } else {
            COLORS[flavor]
        };
        // One unit of gap on top keeps stacked blocks apart.
        let (mut low, mut high) = (bottom, bottom + ROW_UNITS - 1);
        let mut left = (x * CELL_COLUMNS + 1) as i32;
        if (block.state & 0x10) != 0 {
            // BlockState::AWAKING: from a shattered garbage.
            let (awaking_color, height) =
                awaking_look(block.pop_countdown - frame.time_step, block.pop_countdown, block.pop_color, color);
            color = awaking_color;
            let shrink = ((ROW_UNITS - 1) as f32 * (1.0 - height) / 2.0).round() as i32;
            low += shrink;
            high -= shrink;
            if low >= high {
                continue;
            }
        } else if (block.state & 0x02) != 0 {
            // BlockState::SWAPPING: slides toward the other cell, the block going right in
            // the upper part of the row and the one going left in the lower part, so both
            // stay visible where they cross.
            let rightward = (block.state & 0x20) != 0; // BlockState::SWAP_DIRECTION
            let shift = (swap_factor * CELL_COLUMNS as f32).round() as i32;
            let lift = ((swap_factor * std::f32::consts::PI).sin() * SWAP_LIFT_UNITS as f32).round() as i32;
            if rightward {
                left += shift;
                low += lift;
            } else {
                left -= shift;
                high -= lift;
            }
        }
        let left = left.max(0) as usize;
        canvas.fill(left..=left + BLOCK_COLUMNS - 1, low, high, color);
    }

    // Cursor last, over garbage seams: same height and position as the blocks of its row.
    let cursor_x = frame.swapper.grid_x.max(0) as usize;
    let bottom = bottom_unit(frame.swapper.grid_y, 0, rise);
    for col in [cursor_x * CELL_COLUMNS, (cursor_x + 2) * CELL_COLUMNS] {
        canvas.fill(col..=col, bottom, bottom + ROW_UNITS - 1, CURSOR_COLOR);
    }

    canvas
}

fn garbage_color(flavor: i32, flashing: bool) -> u8 {
    match (flavor == GARBAGE_FLAVOR_GRAY, flashing) {
        (true, false) => GARBAGE_GRAY_COLOR,
        (true, true) => GARBAGE_GRAY_FLASH_COLOR,
        (false, false) => GARBAGE_COLOR,
        (false, true) => GARBAGE_FLASH_COLOR,
    }
}

/// Look of a block born from a shattered garbage: (color, height as a fraction of a
/// block). It wears the garbage color until its pop, then flips: it flattens, turns to
/// its own color halfway, and grows back. `remaining` = ticks until the pop;
/// `pop_countdown` 0 means the pop is over.
fn awaking_look(remaining: i32, pop_countdown: i32, pop_color: i32, color: u8) -> (u8, f32) {
    if pop_countdown == 0 || remaining <= 0 {
        return (color, 1.0);
    }
    if remaining >= POP_ROTATE_TIME {
        return (garbage_color(pop_color, false), 1.0);
    }
    let progress = 1.0 - remaining as f32 / POP_ROTATE_TIME as f32;
    let height = (1.0 - 2.0 * progress).abs();
    let shown = if progress < 0.5 { garbage_color(pop_color, false) } else { color };
    (shown, height)
}

/// One terminal character for 8 stacked units (`units[0]` at the bottom): the split
/// (lower color, upper color, height of the lower part in eighths) that matches the most
/// units.
fn split_line(units: &[Option<u8>; 8]) -> (Option<u8>, Option<u8>, usize) {
    let most_common = |part: &[Option<u8>]| -> Option<u8> {
        let mut best = (0, None);
        for candidate in part {
            let n = part.iter().filter(|u| *u == candidate).count();
            if n > best.0 {
                best = (n, *candidate);
            }
        }
        best.1
    };
    let mut best = (usize::MAX, None, None, 0);
    for k in 0..=8 {
        let lower = most_common(&units[..k]);
        let upper = most_common(&units[k..]);
        let errors = units[..k].iter().filter(|u| **u != lower).count()
            + units[k..].iter().filter(|u| **u != upper).count();
        if errors < best.0 {
            best = (errors, lower, upper, k);
        }
    }
    (best.1, best.2, best.3)
}

/// Writes one character from its split: two colors at most, foreground for the lower
/// part. An empty upper part over a color takes reverse video, so the glyph (lower part)
/// is drawn in the terminal background.
fn push_char(out: &mut String, lower: Option<u8>, upper: Option<u8>, k: usize) {
    let (lower, upper, k) = match (lower, upper) {
        // One color over the whole height.
        _ if k == 0 => (upper, upper, 8),
        _ if k == 8 || lower == upper => (lower, lower, 8),
        _ => (lower, upper, k),
    };
    match (lower, upper) {
        (None, None) => out.push(' '),
        (Some(c), Some(u)) if k == 8 || c == u => {
            let _ = write!(out, "\x1b[38;5;{}m█\x1b[39m", c);
        }
        (Some(c), Some(u)) => {
            let _ = write!(out, "\x1b[38;5;{};48;5;{}m{}\x1b[39;49m", c, u, LOWER_EIGHTHS[k - 1]);
        }
        (Some(c), None) => {
            let _ = write!(out, "\x1b[38;5;{}m{}\x1b[39m", c, LOWER_EIGHTHS[k - 1]);
        }
        (None, Some(u)) => {
            let _ = write!(out, "\x1b[38;5;{};7m{}\x1b[27;39m", u, LOWER_EIGHTHS[k - 1]);
        }
    }
}

/// Dot glyph for the center of a cell of the playable area, if it falls on this line.
/// The playable area is the rows the cursor can reach (1 to `SAFE_HEIGHT - 1`); the row
/// above stays clear: the stack rises into it and garbage arrives through it.
/// A row's center is half a row above its bottom.
fn dot_at(col: usize, line: i32, rise: f32) -> Option<char> {
    if col % CELL_COLUMNS != CELL_COLUMNS / 2 || col >= PLAY_WIDTH * CELL_COLUMNS {
        return None;
    }
    (1..SAFE_HEIGHT as i32).find_map(|row| {
        let center = bottom_unit(row, 0, rise) + ROW_UNITS / 2;
        if center < 0 || center >= TOP_UNIT || center.div_euclid(LINE_UNITS) != line {
            return None;
        }
        let within = center.rem_euclid(LINE_UNITS) as usize * DOTS.len() / LINE_UNITS as usize;
        Some(DOTS[within])
    })
}

/// What is drawn around the board besides the game itself.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Hud {
    pub danger: DangerBars,
    /// Chain just reached (×2, ×3…), shown for a moment.
    pub chain: Option<i32>,
}

/// Key reminder at the bottom of the side panel.
const KEY_HINTS: [(&str, &str); 5] = [
    ("Arrows", "move"),
    ("Space", "swap"),
    ("Enter", "raise"),
    ("P", "pause"),
    ("Q", "quit"),
];

/// Side panel text for line `i` (0 = top) of `lines`: game info at the top, the chain
/// just reached below it, the keys at the bottom.
fn panel_line(i: usize, lines: usize, frame: &FrameData, hud: &Hud) -> Option<String> {
    let seconds = frame.ui.clock_time / crate::TICKS_PER_SECOND;
    match i {
        0 => Some(format!("\x1b[1mScore  {}\x1b[22m", frame.score)),
        2 => Some(format!("Time   {}:{:02}", seconds / 60, seconds % 60)),
        3 => Some(match frame.stats.max_chain_level {
            n if n >= 2 => format!("Best   x{}", n),
            _ => "Best   -".to_string(),
        }),
        6 => hud.chain.map(|n| format!("\x1b[1;38;5;220mChain x{}!\x1b[22;39m", n)),
        _ => {
            let first_hint = lines.checked_sub(KEY_HINTS.len())?;
            let (key, action) = KEY_HINTS.get(i.checked_sub(first_hint)?)?;
            Some(format!("\x1b[38;5;244m{:<7}{}\x1b[39m", key, action))
        }
    }
}

/// One danger bar character for this line: the bar rises from the ground by eighths.
fn push_danger(out: &mut String, line: i32, bars: DangerBars) {
    let filled = (bars.height * TOP_UNIT as f32).round() as i32 - line * LINE_UNITS;
    if line < 0 || bars.alpha <= 0.0 || filled <= 0 {
        out.push(' ');
        return;
    }
    let shades = if bars.red { DANGER_RED } else { DANGER_PURPLE };
    let color = if bars.alpha > 0.5 { shades[0] } else { shades[1] };
    let glyph = if filled >= LINE_UNITS { '█' } else { LOWER_EIGHTHS[filled as usize - 1] };
    let _ = write!(out, "\x1b[38;5;{}m{}\x1b[39m", color, glyph);
}

/// The whole frame as one string (written at once, inside a synchronized update).
///
/// When `paused`, the board is hidden (no planning during the pause) and a message
/// replaces it.
fn compose(frame: &FrameData, paused: bool, hud: &Hud) -> String {
    let danger = if paused { DangerBars::default() } else { hud.danger };
    let canvas = if paused { Canvas::new() } else { paint(frame) };
    let top_line = TOP_UNIT / LINE_UNITS;
    let pause_line = top_line / 2;
    let mut out = String::with_capacity(8 * 1024);
    out.push_str(SYNC_BEGIN);
    out.push_str(HOME);

    let lines = (top_line + LINES_BELOW_GROUND) as usize;
    for (i, line) in (-LINES_BELOW_GROUND..top_line).rev().enumerate() {
        let base = line * LINE_UNITS;
        // The line just above the ground carries the ground line.
        let ground = if line == 0 { GROUND } else { "" };
        // Left danger bar, a gap, then the board.
        push_danger(&mut out, line, danger);
        out.push_str(DANGER_GAP);
        out.push_str(ground);
        let pause_text = match line - pause_line {
            0 => Some("PAUSE"),
            -1 => Some("P: resume"),
            -2 => Some("Q: quit"),
            _ => None,
        };
        if paused {
            let _ = write!(out, "\x1b[1m{:^width$}\x1b[22m", pause_text.unwrap_or(""), width = FIELD_COLUMNS);
        }
        for col in 0..FIELD_COLUMNS {
            if paused {
                continue;
            }
            if let Some(glyph) = canvas.mark(col, line) {
                let _ = write!(out, "\x1b[38;5;{}m{}\x1b[39m", DYING_COLOR, glyph);
                continue;
            }
            let units: [Option<u8>; 8] = std::array::from_fn(|u| canvas.get(col, base + u as i32));
            if units.iter().all(Option::is_none) {
                if let Some(dot) = dot_at(col, line, frame.play_offset_y) {
                    let _ = write!(out, "\x1b[38;5;{}m{}\x1b[39m", DOT_COLOR, dot);
                    continue;
                }
            }
            let (lower, upper, k) = split_line(&units);
            push_char(&mut out, lower, upper, k);
        }
        out.push_str(RESET);
        out.push_str(DANGER_GAP);
        push_danger(&mut out, line, danger);

        // Side panel: game info and keys.
        if let Some(text) = panel_line(i, lines, frame, hud) {
            let _ = write!(out, "   {}", text);
        }
        out.push_str("\x1b[K\r\n");
    }

    // Debug (`--debug`): swapper cell info
    if SHOW_DEBUG.load(Ordering::Relaxed) {
        out.push_str("\x1b[K\r\n");
        out.push_str("Swap cells: L=");
        push_cell_debug(&mut out, &frame.debug_left_cell);
        out.push_str(" R=");
        push_cell_debug(&mut out, &frame.debug_right_cell);
        out.push_str("\x1b[K\r\n");
    }

    // Game over message
    if frame.game_over {
        out.push_str("\r\n   GAME OVER!\x1b[K\r\n");
    }
    out.push_str(SYNC_END);
    out
}

pub(crate) fn render(frame: &FrameData, hud: &Hud) {
    print!("{}", compose(frame, false, hud));
}

/// The pause screen: board hidden, score kept.
pub(crate) fn render_paused(frame: &FrameData) {
    print!("{}", compose(frame, true, &Hud::default()));
}

fn push_cell_debug(out: &mut String, cell: &CellDebugInfo) {
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
    let _ = write!(out, "{}:p{}({}) s{:02b}", type_char, cell.phase, phase_name, cell.block_state);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crackattack_core::GameState;
    use crackattack_core::InputSnapshot;

    #[test]
    fn a_uniform_line_is_one_full_color() {
        assert_eq!(split_line(&[Some(3); 8]).2 % 8, 0);
        let (lower, upper, k) = split_line(&[None; 8]);
        assert_eq!((lower, upper), (None, None));
        assert!(k == 0 || k == 8);
    }

    #[test]
    fn a_boundary_splits_at_the_right_eighth() {
        let mut units = [None; 8];
        for u in units.iter_mut().take(3) {
            *u = Some(5);
        }
        assert_eq!(split_line(&units), (Some(5), None, 3));
        let mut two = [Some(1); 8];
        for u in two.iter_mut().skip(6) {
            *u = Some(2);
        }
        assert_eq!(split_line(&two), (Some(1), Some(2), 6));
    }

    #[test]
    fn dots_mark_each_cell_and_rise_with_the_stack() {
        let count = |rise: f32| (0..TOP_UNIT / LINE_UNITS)
            .flat_map(|line| (0..FIELD_COLUMNS).map(move |col| (col, line)))
            .filter(|&(col, line)| dot_at(col, line, rise).is_some())
            .count();
        assert_eq!(count(0.0), PLAY_WIDTH * (SAFE_HEIGHT - 1));
        // Row 1 center: middle of line 1 at rest, a line higher after half a row.
        let col = CELL_COLUMNS / 2;
        assert_eq!(dot_at(col, 1, 0.0), Some('.'));
        assert_eq!(dot_at(col, 1, 0.25), Some('·'));
        assert_eq!(dot_at(col, 2, 0.5), Some('.'));
        assert_eq!(dot_at(col + 1, 1, 0.0), None);
    }

    #[test]
    fn danger_bars_frame_the_board_on_both_sides() {
        let mut game = GameState::new(7);
        let frame = game.step(&InputSnapshot::new());
        let half = DangerBars { height: 0.5, red: false, alpha: 1.0 };
        let text = compose(&frame, false, &Hud { danger: half, chain: None });
        let purple = format!("\x1b[38;5;{}m█", DANGER_PURPLE[0]);
        // Half the board lines on each side.
        assert_eq!(text.matches(&purple).count(), 2 * (TOP_UNIT / LINE_UNITS / 2) as usize);
        let red = DangerBars { red: true, ..half };
        assert!(compose(&frame, false, &Hud { danger: red, chain: None }).contains(&format!("\x1b[38;5;{}m█", DANGER_RED[0])));
        assert!(!compose(&frame, true, &Hud { danger: half, chain: None }).contains(&purple));
    }

    #[test]
    fn a_dying_block_is_a_small_square_at_its_cell_center() {
        let mut game = GameState::new(7);
        let mut frame = game.step(&InputSnapshot::new());
        let block = frame.blocks.iter_mut().find(|b| b.grid_y == 3).unwrap();
        block.state |= 0x08;
        block.animation_countdown = DYING_DELAY;
        let (x, y) = (block.grid_x as usize, block.grid_y);
        let canvas = paint(&frame);
        let center = bottom_unit(y, 0, frame.play_offset_y) + ROW_UNITS / 2;
        let col = x * CELL_COLUMNS + CELL_COLUMNS / 2;
        assert_eq!(canvas.mark(col, center.div_euclid(LINE_UNITS)), Some('■'));
        // The cell itself is left empty around the square.
        assert_eq!(canvas.get(col, center), None);
    }

    /// A frame with a single block, on row 3 of column 1, and no garbage.
    fn lone_block_frame(state: u8) -> FrameData {
        let mut game = GameState::new(7);
        let mut frame = game.step(&InputSnapshot::new());
        let mut block = frame.blocks.iter().find(|b| b.grid_y == 3).unwrap().clone();
        block.grid_x = 1;
        block.state = state;
        frame.blocks = vec![block];
        frame.garbage.clear();
        frame.swapper.grid_y = 10;
        frame
    }

    /// Units of column `col` painted in `color`.
    fn painted(canvas: &Canvas, col: usize, color: u8) -> Vec<i32> {
        (BOTTOM_UNIT..TOP_UNIT).filter(|&u| canvas.get(col, u) == Some(color)).collect()
    }

    #[test]
    fn swapping_blocks_slide_and_cross_one_above_the_other() {
        let mut frame = lone_block_frame(0x02 | 0x20); // SWAPPING, going right
        frame.swapper.swap_factor = 0.5;
        let color = COLORS[frame.blocks[0].flavor as usize];
        let bottom = bottom_unit(3, 0, frame.play_offset_y);
        let left = CELL_COLUMNS + 1 + CELL_COLUMNS / 2;
        let canvas = paint(&frame);
        assert!(painted(&canvas, CELL_COLUMNS + 1, color).is_empty());
        let units = painted(&canvas, left, color);
        assert_eq!(units, (bottom + SWAP_LIFT_UNITS..bottom + ROW_UNITS - 1).collect::<Vec<_>>());
        // The block going left keeps the lower part of the row.
        frame.blocks[0].state = 0x02;
        frame.blocks[0].grid_x = 2;
        let canvas = paint(&frame);
        let units = painted(&canvas, left, color);
        assert_eq!(units, (bottom..bottom + ROW_UNITS - 1 - SWAP_LIFT_UNITS).collect::<Vec<_>>());
        // Swap over: back in place, whole.
        frame.swapper.swap_factor = 0.0;
        let canvas = paint(&frame);
        assert_eq!(painted(&canvas, 2 * CELL_COLUMNS + 1, color).len(), (ROW_UNITS - 1) as usize);
    }

    #[test]
    fn a_shattered_garbage_block_flips_from_garbage_to_its_color() {
        let mut frame = lone_block_frame(0x10); // AWAKING
        let color = COLORS[frame.blocks[0].flavor as usize];
        frame.blocks[0].pop_color = 0;
        let col = CELL_COLUMNS + 2;
        let mut look = |remaining: i32| {
            frame.blocks[0].pop_countdown = frame.time_step + remaining;
            let canvas = paint(&frame);
            (painted(&canvas, col, GARBAGE_COLOR).len(), painted(&canvas, col, color).len())
        };
        let full = (ROW_UNITS - 1) as usize;
        assert_eq!(look(POP_ROTATE_TIME + 5), (full, 0));
        let (garbage, own) = look(POP_ROTATE_TIME * 3 / 4);
        assert!(garbage > 0 && garbage < full && own == 0);
        let (garbage, own) = look(POP_ROTATE_TIME / 4);
        assert!(garbage == 0 && own > 0 && own < full);
        frame.blocks[0].pop_countdown = 0;
        assert_eq!(painted(&paint(&frame), col, color).len(), full);
    }

    #[test]
    fn garbage_flashes_when_hit_and_holds_until_its_blocks_appear() {
        assert_eq!(garbage_color(0, true), GARBAGE_FLASH_COLOR);
        assert_eq!(garbage_color(GARBAGE_FLAVOR_GRAY, false), GARBAGE_GRAY_COLOR);
        // Late in the shatter, the slab is still drawn.
        let mut game = GameState::new(7);
        let mut frame = game.step(&InputSnapshot::new());
        let garbage = crackattack_core::frame_data::GarbageSnapshot {
            id: 1, grid_x: 0, grid_y: 11, sub_grid_y: 0, width: 6, height: 1, flavor: 0,
            state: 8, sections_popped: 0, pop_direction: 0, pop_color: 0,
            shatter_progress: 0.9, awaking_progress: 0.0, pop_progress: 0.0,
        };
        frame.garbage = vec![garbage];
        let canvas = paint(&frame);
        let bottom = bottom_unit(11, 0, frame.play_offset_y);
        assert_eq!(canvas.get(CELL_COLUMNS * 2, bottom + 2), Some(GARBAGE_COLOR));
    }

    #[test]
    fn the_panel_shows_game_info_the_chain_and_the_keys() {
        let mut frame = FrameData::default();
        frame.score = 420;
        frame.ui.clock_time = 65 * crate::TICKS_PER_SECOND;
        let hud = Hud { chain: Some(3), ..Hud::default() };
        let text = compose(&frame, false, &hud);
        assert!(text.contains("Score  420"));
        assert!(text.contains("Time   1:05"));
        assert!(text.contains("Best   -"));
        assert!(text.contains("Chain x3!"));
        for (key, action) in KEY_HINTS {
            assert!(text.contains(&format!("{:<7}{}", key, action)));
        }
        assert!(!compose(&frame, false, &Hud::default()).contains("Chain"));
    }

    #[test]
    fn the_pause_hides_the_board() {
        let mut game = GameState::new(7);
        let frame = game.step(&InputSnapshot::new());
        let text = compose(&frame, true, &Hud::default());
        assert!(text.contains("PAUSE"));
        assert!(text.contains("Q: quit"));
        assert!(!text.contains('█'));
        assert_eq!(text.matches("\r\n").count(), compose(&frame, false, &Hud::default()).matches("\r\n").count());
    }

    #[test]
    fn the_cursor_has_the_height_and_position_of_its_blocks() {
        let mut game = GameState::new(7);
        let frame = game.step(&InputSnapshot::new());
        let canvas = paint(&frame);
        let col = frame.swapper.grid_x as usize * CELL_COLUMNS;
        let bottom = bottom_unit(frame.swapper.grid_y, 0, frame.play_offset_y);
        let painted: Vec<i32> = (BOTTOM_UNIT..TOP_UNIT)
            .filter(|&u| canvas.get(col, u) == Some(CURSOR_COLOR))
            .collect();
        assert_eq!(painted, (bottom..bottom + ROW_UNITS - 1).collect::<Vec<_>>());
    }

    #[test]
    fn the_stack_rises_by_sixteenths_of_a_row() {
        assert_eq!(bottom_unit(1, 0, 0.0), 0);
        assert_eq!(bottom_unit(1, 0, 0.5), ROW_UNITS / 2);
        assert_eq!(bottom_unit(0, 0, 0.25), -ROW_UNITS * 3 / 4);
        // A block halfway through its fall to row 2.
        assert_eq!(bottom_unit(2, STEPS_PER_GRID / 2, 0.0), ROW_UNITS * 3 / 2);
    }

    #[test]
    fn a_frame_has_one_line_per_two_row_halves_and_a_ground() {
        let mut game = GameState::new(7);
        let frame = game.step(&InputSnapshot::new());
        let text = compose(&frame, false, &Hud::default());
        let lines = text.matches("\r\n").count();
        // Playfield lines only (no debug line by default).
        assert_eq!(lines, (TOP_UNIT / LINE_UNITS + LINES_BELOW_GROUND) as usize);
        assert!(!text.contains("Swap cells"));
        assert_eq!(text.matches(GROUND).count(), 1);
    }
}
