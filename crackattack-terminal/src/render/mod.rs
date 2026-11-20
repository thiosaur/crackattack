// Terminal renderer - ANSI 256 colors + UTF-8 eighth blocks
//
// FrameData → `board` paints the game on a fine canvas → `raster` turns each 8 units of
// it into one character → `screen_lines` (here) lays the screen out: danger bars, board,
// side panel (`panel`) and the `--debug` line (`debug`) → `output` writes only the lines
// that changed. Colors are in `palette`.

mod board;
mod countdown;
mod debug;
mod font;
mod menu;
mod output;
pub(crate) mod palette;
mod panel;
mod raster;
mod lobby;
pub(crate) mod versus;
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
pub(crate) use menu::{Page as MenuPage, COMPUTER as MENU_COMPUTER, TOP as MENU_TOP, TOP_ONLINE as MENU_TOP_ONLINE};
pub(crate) use countdown::Countdown;
pub(crate) use font::Banner;
pub(crate) use lobby::LobbyView;
pub(crate) use versus::{banner, Versus};

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

/// Every screen (menu, solo, duel) is drawn in a zone of this width, the same in both sizes:
/// the terminal must be at least this wide. The last column stays free (see `output::clip`).
pub(crate) const ZONE_COLUMNS: usize = 80;
/// The compact zone's height: the terminal must be at least this tall.
pub(crate) const MIN_ROWS: usize = COMPACT_LINES;

/// Where the screen goes on the terminal.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Placement {
    /// The terminal is smaller than 80x24: nothing can be drawn but a message.
    TooSmall { columns: usize, rows: usize },
    /// A zone of `ZONE_COLUMNS` columns, 24 lines (compact) or `tall_lines()` (normal),
    /// centered; `origin` is its top left corner, as (column, row) from 0.
    Zone { origin: (usize, usize), tall: bool },
}

/// Where to draw on a terminal of `size` (columns, rows): the normal zone when the terminal
/// is tall enough (28 lines, 30 with `--debug`), the compact one otherwise or when
/// `compact` is forced, a message under 80x24. An unknown size is treated as 80x24 with no
/// centering, as is a display-less run.
pub(crate) fn place(size: Option<(usize, usize)>, compact: bool) -> Placement {
    let Some((columns, rows)) = size else {
        return Placement::Zone { origin: (0, 0), tall: false };
    };
    if columns < ZONE_COLUMNS || rows < MIN_ROWS {
        return Placement::TooSmall { columns, rows };
    }
    let tall = !compact && rows >= tall_lines();
    let height = if tall { tall_lines() } else { COMPACT_LINES };
    Placement::Zone { origin: ((columns - ZONE_COLUMNS) / 2, (rows - height) / 2), tall }
}

/// What a terminal smaller than 80x24 shows, centered in the terminal itself.
fn too_small_lines(columns: usize, rows: usize) -> Vec<String> {
    let text = ["Terminal too small", "80x24 needed", &format!("now {}x{}", columns, rows)];
    let mut lines = vec![String::new(); rows.saturating_sub(text.len()) / 2];
    lines.extend(text.iter().map(|line| format!("{}{}", " ".repeat(columns.saturating_sub(line.len()) / 2), line)));
    lines
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

// Written with `write!`, not `print!`: a terminal that is gone (dropped ssh connection)
// must not make the game panic on its way out.
pub(crate) fn initialize() {
    use std::io::Write;
    let _ = write!(std::io::stdout(), "{}{}{}{}", ALT_SCREEN_ON, WRAP_OFF, HIDE_CURSOR, CLEAR);
}

pub(crate) fn cleanup() {
    use std::io::Write;
    let _ = write!(std::io::stdout(), "{}{}{}", SHOW_CURSOR, WRAP_ON, ALT_SCREEN_OFF);
}

/// What is drawn around the board besides the game itself.
#[derive(Clone, Debug, Default)]
pub(crate) struct Hud {
    pub danger: DangerBars,
    /// Chain just reached (×2, ×3…), shown for a moment.
    pub chain: Option<i32>,
    /// Ticks since the game was lost: the logo turns into GAME OVER.
    pub game_over: Option<i32>,
    /// Duel against the computer: the opponent's preview and the tally.
    pub versus: Option<Versus>,
    /// Text that replaces the board (countdown, ready prompt, result), one entry per line.
    pub banner: Option<Banner>,
    /// The 3-2-1-GO digit drawn over the board, which stays visible around it.
    pub countdown: Option<Countdown>,
    /// `--lobby`: who is connected (in the side panel, where the logo is).
    pub lobby: Option<LobbyView>,
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
    // While paused nothing of the game shows (no planning): only the pause message.
    let paused_hud;
    let hud = if paused {
        // The panel keeps its shape (in a duel: the opponent's name and the tally, with an empty
        // preview), so that the pause does not look like a solo game.
        paused_hud = Hud {
            banner: hud.banner.clone(),
            versus: hud.versus.clone().map(|v| Versus { preview: None, ..v }),
            lobby: hud.lobby.clone(),
            ..Hud::default()
        };
        &paused_hud
    } else {
        hud
    };
    let danger = hud.danger;
    let banner = hud.banner.as_ref();
    // A banner with big words is a canvas of its own; one of text only replaces the board
    // line by line (below); with none, the board.
    let mut canvas = match banner {
        _ if paused => None,
        Some(banner) if !banner.words.is_empty() => Some(font::banner_canvas(banner)),
        Some(_) => None,
        None => Some(board::paint(frame)),
    };
    if let (Some(canvas), Some(countdown), None) = (&mut canvas, &hud.countdown, banner) {
        countdown.paint(canvas);
    }
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
                None => match banner {
                    // A message of the session, one entry per line, starting just above the middle.
                    Some(banner) => {
                        let k = usize::try_from(pause_line + 1 - line).ok();
                        let shown = k.and_then(|k| banner.above.get(k)).map_or("", String::as_str);
                        let pad = field_columns().saturating_sub(shown.chars().count());
                        // The first line is the title.
                        let title = if k == Some(0) { ("\x1b[1m", "\x1b[22m") } else { ("", "") };
                        let _ = write!(out, "{}{}{}{}{}", " ".repeat(pad / 2), title.0, shown, title.1, " ".repeat(pad - pad / 2));
                    }
                    None => {
                        let pause_text = match line - pause_line {
                            0 => "PAUSE",
                            -1 => "P: resume",
                            -2 => "Q: quit",
                            _ => "",
                        };
                        let _ = write!(out, "\x1b[1m{:^width$}\x1b[22m", pause_text, width = field_columns());
                    }
                },
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
    placement: Option<Placement>,
    /// Lines of the zone, for the menu (the game's own screen has its own count).
    zone_rows: usize,
}

impl Renderer {
    pub(crate) fn new() -> Self {
        Self { output: output::Output::new(), placement: None, zone_rows: COMPACT_LINES }
    }

    /// The message that replaces the screen under 80x24, if that is where we are.
    pub(crate) fn too_small(&self) -> bool {
        matches!(self.placement, Some(Placement::TooSmall { .. }))
    }

    fn too_small_screen(&mut self) -> Option<String> {
        match self.placement {
            Some(Placement::TooSmall { columns, rows }) => Some(self.output.update(too_small_lines(columns, rows))),
            _ => None,
        }
    }

    /// The text that brings the terminal to this frame; empty when nothing changed.
    pub(crate) fn frame(&mut self, frame: &FrameData, hud: &Hud) -> String {
        self.too_small_screen().unwrap_or_else(|| self.output.update(screen_lines(frame, false, hud)))
    }

    /// Same for the pause screen: board hidden, score kept.
    pub(crate) fn paused(&mut self, frame: &FrameData, hud: &Hud) -> String {
        self.too_small_screen().unwrap_or_else(|| self.output.update(screen_lines(frame, true, hud)))
    }

    /// The start menu with `selected` chosen, centered in the zone like the games.
    /// A list that may be longer than the screen shows (see `menu::list_lines`).
    pub(crate) fn menu_list(&mut self, heading: &str, items: &[String], selected: usize, hint: &str, lobby: Option<&LobbyView>) -> String {
        self.too_small_screen().unwrap_or_else(|| self.output.update(menu::list_lines(heading, items, selected, hint, ZONE_COLUMNS, self.zone_rows, lobby)))
    }

    pub(crate) fn menu(&mut self, page: &MenuPage, selected: usize, lobby: Option<&LobbyView>) -> String {
        self.too_small_screen().unwrap_or_else(|| self.output.update(menu::lines(page, selected, ZONE_COLUMNS, self.zone_rows, lobby)))
    }

    /// Where to draw, after a launch or a resize (width or height): everything is redrawn
    /// whole when it changes. Lines are cut to the zone, the side panel (its logo first)
    /// giving way.
    pub(crate) fn set_placement(&mut self, placement: Placement) {
        if self.placement == Some(placement) {
            return;
        }
        self.placement = Some(placement);
        match placement {
            Placement::Zone { origin, tall } => {
                self.zone_rows = if tall { tall_lines() } else { COMPACT_LINES };
                self.output.set_area(origin, Some(ZONE_COLUMNS));
            }
            Placement::TooSmall { columns, .. } => self.output.set_area((0, 0), Some(columns)),
        }
        self.output.invalidate();
    }
}

/// A whole frame drawn on a blank terminal, as one string: for tests.
#[cfg(test)]
fn compose(frame: &FrameData, paused: bool, hud: &Hud) -> String {
    output::Output::new().update(screen_lines(frame, paused, hud))
}
