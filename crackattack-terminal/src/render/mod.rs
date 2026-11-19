// Terminal renderer - ANSI 256 colors + UTF-8 eighth blocks
//
// FrameData → `board` paints the game on a fine canvas → `raster` turns each 8 units of
// it into one character → `screen_lines` (here) lays the screen out: danger bars, board,
// side panel (`panel`) and the `--debug` line (`debug`) → `output` writes only the lines
// that changed. Colors are in `palette`.

mod board;
mod debug;
mod output;
pub(crate) mod palette;
mod panel;
mod raster;
#[cfg(test)]
mod tests;

use crackattack_core::frame_data::FrameData;
use std::fmt::Write;
use std::sync::atomic::Ordering;

use crate::danger::DangerBars;
use board::{field_columns, lines_below_ground, top_unit, LINE_UNITS};
use palette::{DANGER_PURPLE, DANGER_RED};
use raster::LOWER_EIGHTHS;

pub(crate) use board::{COMPACT, SMOOTH_RISE};
pub(crate) use debug::SHOW_DEBUG;
pub(crate) use panel::{game_over_ticks, KEYS};

// ANSI escape codes
const RESET: &str = "\x1b[0m";
const CLEAR: &str = "\x1b[2J";
const HIDE_CURSOR: &str = "\x1b[?25l";
const SHOW_CURSOR: &str = "\x1b[?25h";
const ALT_SCREEN_ON: &str = "\x1b[?1049h";
const ALT_SCREEN_OFF: &str = "\x1b[?1049l";
/// No line wrap at the right edge: a line too long for the window is cut, never spilled
/// onto the next one (lines are also cut to the known width, see `Renderer::set_width`).
const WRAP_OFF: &str = "\x1b[?7l";
const WRAP_ON: &str = "\x1b[?7h";

/// Ground line under the playfield: the line just above it is underlined, so the ground
/// takes no line of its own. The underline is dark gray where the terminal supports
/// underline colors (otherwise it takes the text color).
const GROUND: &str = "\x1b[4m\x1b[58;5;240m";

/// Space between each danger bar and the board.
const DANGER_GAP: &str = "   ";
/// Space between the right danger bar and the side panel.
const PANEL_GAP: &str = "   ";
/// `--compact`: the screen is the shellcade arcade canvas, 80×24.
const COMPACT_LINES: usize = 24;

/// Terminal lines the normal screen needs (two per row), the `--debug` lines included:
/// a shorter terminal gets the compact screen.
pub(crate) fn tall_lines() -> usize {
    let debug = if SHOW_DEBUG.load(Ordering::Relaxed) { 2 } else { 0 };
    // The playfield, then the rising row below the ground.
    2 * crackattack_core::constants::SAFE_HEIGHT + 2 + debug
}

/// Characters `text` shows, escape sequences left out.
fn visible_width(text: &str) -> usize {
    let mut width = 0;
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            chars.next();
            chars.by_ref().find(|c| ('@'..='~').contains(c));
        } else {
            width += 1;
        }
    }
    width
}

/// First terminal column of the side panel: everything left of it is the board and its
/// danger bars.
pub(crate) fn panel_column() -> usize {
    2 * (1 + DANGER_GAP.len()) + field_columns() + PANEL_GAP.len()
}

pub(crate) fn initialize() {
    print!("{}{}{}{}", ALT_SCREEN_ON, WRAP_OFF, HIDE_CURSOR, CLEAR);
}

pub(crate) fn cleanup() {
    print!("{}{}{}", SHOW_CURSOR, WRAP_ON, ALT_SCREEN_OFF);
}

/// What is drawn around the board besides the game itself.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Hud {
    pub danger: DangerBars,
    /// Chain just reached (×2, ×3…), shown for a moment.
    pub chain: Option<i32>,
    /// Ticks since the game was lost: the logo turns into GAME OVER.
    pub game_over: Option<i32>,
}

/// One danger bar character for this line: the bar rises from the ground by eighths.
fn push_danger(out: &mut String, line: i32, bars: DangerBars) {
    let filled = (bars.height * top_unit() as f32).round() as i32 - line * LINE_UNITS;
    if line < 0 || bars.alpha <= 0.0 || filled <= 0 {
        out.push(' ');
        return;
    }
    let shades = if bars.red { DANGER_RED } else { DANGER_PURPLE };
    let color = if bars.alpha > 0.5 { shades[0] } else { shades[1] };
    let glyph = if filled >= LINE_UNITS { '█' } else { LOWER_EIGHTHS[filled as usize - 1] };
    let _ = write!(out, "\x1b[38;5;{}m{}\x1b[39m", color, glyph);
}

/// The screen, one string per terminal line, each ending with the default colors.
///
/// When `paused`, the board is hidden (no planning during the pause) and a message
/// replaces it.
fn screen_lines(frame: &FrameData, paused: bool, hud: &Hud) -> Vec<String> {
    let danger = if paused { DangerBars::default() } else { hud.danger };
    let canvas = (!paused).then(|| board::paint(frame));
    let top_line = top_unit() / LINE_UNITS;
    let pause_line = top_line / 2;
    let board_lines = (top_line + lines_below_ground()) as usize;
    // Compact: the board, centered, on the shellcade canvas.
    let count = if board::compact() { COMPACT_LINES.max(board_lines) } else { board_lines };
    let first_board_line = (count - board_lines) / 2;
    let mut lines = Vec::with_capacity(count + 2);

    for i in 0..count {
        let mut out = String::with_capacity(512);
        let line = top_line - 1 - (i as i32 - first_board_line as i32);
        if (-lines_below_ground()..top_line).contains(&line) {
            // The line just above the ground carries the ground line.
            let ground = if line == 0 { GROUND } else { "" };
            // Left danger bar, a gap, then the board.
            push_danger(&mut out, line, danger);
            out.push_str(DANGER_GAP);
            out.push_str(ground);
            match &canvas {
                Some(canvas) => board::push_line(&mut out, canvas, line, frame.play_offset_y),
                None => {
                    let pause_text = match line - pause_line {
                        0 => "PAUSE",
                        -1 => "P: resume",
                        -2 => "Q: quit",
                        _ => "",
                    };
                    let _ = write!(out, "\x1b[1m{:^width$}\x1b[22m", pause_text, width = field_columns());
                }
            }
            out.push_str(RESET);
            out.push_str(DANGER_GAP);
            push_danger(&mut out, line, danger);
        }

        // Side panel: game info and keys.
        if let Some(text) = panel::panel_line(i, count, frame, hud) {
            let board = panel_column() - PANEL_GAP.len();
            let _ = write!(out, "{:width$}{}{}", "", PANEL_GAP, text, width = board.saturating_sub(visible_width(&out)));
        }
        lines.push(out);
    }

    // Debug (`--debug`): swapper cell info, after a blank line.
    if SHOW_DEBUG.load(Ordering::Relaxed) {
        let mut out = String::new();
        debug::push_swap_cells(&mut out, frame);
        lines.extend([String::new(), out]);
    }
    lines
}

/// Draws the frames, writing only what changed since the last one.
pub(crate) struct Renderer {
    output: output::Output,
}

impl Renderer {
    pub(crate) fn new() -> Self {
        Self { output: output::Output::new() }
    }

    /// The text that brings the terminal to this frame; empty when nothing changed.
    pub(crate) fn frame(&mut self, frame: &FrameData, hud: &Hud) -> String {
        self.output.update(screen_lines(frame, false, hud))
    }

    /// Same for the pause screen: board hidden, score kept.
    pub(crate) fn paused(&mut self, frame: &FrameData) -> String {
        self.output.update(screen_lines(frame, true, &Hud::default()))
    }

    /// The terminal width (columns), if known: lines are cut to it, the side panel on the
    /// right (its logo first) giving way. Also redraws everything, as after a resize.
    pub(crate) fn set_width(&mut self, width: Option<usize>) {
        self.output.set_width(width);
        self.output.invalidate();
    }
}

/// A whole frame drawn on a blank terminal, as one string: for tests.
#[cfg(test)]
fn compose(frame: &FrameData, paused: bool, hud: &Hud) -> String {
    output::Output::new().update(screen_lines(frame, paused, hud))
}
