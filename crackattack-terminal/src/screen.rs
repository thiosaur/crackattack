// Text captures of the screen, for autoplay runs and checks.
//
// The renderer's output is decoded the way a terminal would (characters, 256-color SGR,
// reverse video, cursor home), then written as plain text: the characters shown, and for
// each one a letter for its ink (glyph color: the lower part of an eighth block) and its
// paper (background: the upper part). Captures can be compared and searched without
// images.

use crate::render::palette::*;
use crate::render::PANEL_COLUMN;

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Cell {
    pub ch: char,
    /// Glyph color (`None`: the terminal's default).
    pub ink: Option<u8>,
    /// Background color (`None`: the terminal's default).
    pub paper: Option<u8>,
}

const BLANK: Cell = Cell { ch: ' ', ink: None, paper: None };

/// A virtual terminal screen, fed with what would be printed.
pub(crate) struct Screen {
    rows: Vec<Vec<Cell>>,
    row: usize,
    col: usize,
    fg: Option<u8>,
    bg: Option<u8>,
    reverse: bool,
}

impl Screen {
    pub(crate) fn new() -> Self {
        Self { rows: Vec::new(), row: 0, col: 0, fg: None, bg: None, reverse: false }
    }

    /// Decodes `text` onto the screen. Only what the renderer emits is understood; other
    /// escape sequences are skipped.
    pub(crate) fn feed(&mut self, text: &str) {
        let mut chars = text.chars().peekable();
        while let Some(c) = chars.next() {
            match c {
                '\x1b' => {
                    if chars.peek() != Some(&'[') {
                        continue;
                    }
                    chars.next();
                    let mut params = String::new();
                    let mut last = ' ';
                    for p in chars.by_ref() {
                        if ('@'..='~').contains(&p) {
                            last = p;
                            break;
                        }
                        params.push(p);
                    }
                    if params.starts_with('?') {
                        continue; // private modes: alternate screen, sync, cursor
                    }
                    match last {
                        // Cursor position "row;col", from 1; home without parameters.
                        'H' => {
                            let mut at = params.split(';').map(|n| n.parse::<usize>().unwrap_or(1).max(1) - 1);
                            (self.row, self.col) = (at.next().unwrap_or(0), at.next().unwrap_or(0));
                        }
                        'J' if params == "2" => self.rows.clear(),
                        // Erase to the end of the line.
                        'K' if params.is_empty() || params == "0" => {
                            if let Some(line) = self.rows.get_mut(self.row) {
                                line.truncate(self.col);
                            }
                        }
                        'm' => self.sgr(&params),
                        _ => {}
                    }
                }
                '\r' => self.col = 0,
                '\n' => self.row += 1,
                _ => {
                    let (ink, paper) = if self.reverse { (self.bg, self.fg) } else { (self.fg, self.bg) };
                    if self.rows.len() <= self.row {
                        self.rows.resize(self.row + 1, Vec::new());
                    }
                    let line = &mut self.rows[self.row];
                    if line.len() <= self.col {
                        line.resize(self.col + 1, BLANK);
                    }
                    line[self.col] = Cell { ch: c, ink, paper };
                    self.col += 1;
                }
            }
        }
    }

    fn sgr(&mut self, params: &str) {
        let codes: Vec<u16> = params.split(';').map(|p| p.parse().unwrap_or(0)).collect();
        let mut i = 0;
        while i < codes.len() {
            match codes[i] {
                0 => (self.fg, self.bg, self.reverse) = (None, None, false),
                7 => self.reverse = true,
                27 => self.reverse = false,
                39 => self.fg = None,
                49 => self.bg = None,
                38 | 48 | 58 if codes.get(i + 1) == Some(&5) => {
                    let color = codes.get(i + 2).map(|&c| c as u8);
                    match codes[i] {
                        38 => self.fg = color,
                        48 => self.bg = color,
                        _ => {} // underline color
                    }
                    i += 2;
                }
                _ => {} // bold, underline: not captured
            }
            i += 1;
        }
    }

    #[cfg(test)]
    pub(crate) fn cell(&self, row: usize, col: usize) -> Cell {
        self.rows.get(row).and_then(|line| line.get(col)).copied().unwrap_or(BLANK)
    }

    /// The characters shown, one string per line, trailing spaces removed.
    pub(crate) fn text(&self) -> Vec<String> {
        self.layer(|_, cell| cell.ch)
    }

    fn layer(&self, f: impl Fn(usize, &Cell) -> char) -> Vec<String> {
        self.rows
            .iter()
            .map(|line| line.iter().enumerate().map(|(col, cell)| f(col, cell)).collect::<String>().trim_end().to_string())
            .collect()
    }

    /// One capture: the characters, then the ink and paper letters.
    pub(crate) fn capture(&self, title: &str) -> String {
        let ink = |col: usize, cell: &Cell| match cell.ink {
            Some(color) => letter(col, color),
            // An eighth block with no ink: its lower part is the default background.
            None if cell.ch != ' ' && !('▁'..='█').contains(&cell.ch) => '-',
            None => ' ',
        };
        let paper = |col: usize, cell: &Cell| cell.paper.map_or(' ', |color| letter(col, color));
        let mut out = format!("=== {} ===\n", title);
        for (name, lines) in [("text", self.text()), ("ink", self.layer(ink)), ("paper", self.layer(paper))] {
            out.push_str(&format!("--- {} ---\n", name));
            for line in lines {
                out.push_str(&line);
                out.push('\n');
            }
        }
        out
    }
}

/// Letter for each color of the board in text captures (`--capture`): capital letters for
/// blocks, lowercase for their dark rising-row versions.
const BOARD_LETTERS: &[(u8, char, &str)] = &[
    (COLORS[0], 'P', "purple block"), (COLORS[1], 'B', "blue block"),
    (COLORS[2], 'G', "green block"), (COLORS[3], 'Y', "yellow block"),
    (COLORS[4], 'O', "orange block"), (COLORS[5], 'A', "gray block"),
    (DIM_COLORS[0], 'p', "purple, rising row"), (DIM_COLORS[1], 'b', "blue, rising row"),
    (DIM_COLORS[2], 'g', "green, rising row"), (DIM_COLORS[3], 'y', "yellow, rising row"),
    (DIM_COLORS[4], 'o', "orange, rising row"), (DIM_COLORS[5], 'a', "gray, rising row"),
    (GARBAGE_COLOR, 'R', "garbage"), (GARBAGE_FLASH_COLOR, 'r', "garbage, hit"),
    (GARBAGE_GRAY_COLOR, 'K', "gray garbage"), (GARBAGE_GRAY_FLASH_COLOR, 'k', "gray garbage, hit"),
    (CURSOR_COLOR, 'W', "cursor"), (DYING_COLOR, 'w', "dying block"),
    (DOT_COLOR, 'd', "dots"),
    (DANGER_PURPLE[0], 'V', "danger, purple"), (DANGER_PURPLE[1], 'v', "danger, purple fading"),
    (DANGER_RED[0], 'X', "danger, red"), (DANGER_RED[1], 'x', "danger, red fading"),
];
/// Same for the side panel, which reuses some board colors (the logo is red and purple
/// like garbage and purple blocks, the key hints gray like gray blocks): its own letters,
/// so that a letter always means one thing.
const PANEL_LETTERS: &[(u8, char, &str)] = &[
    (CHAIN_COLOR, 'C', "chain popup"),
    (LOGO_COLORS[0], 'L', "logo, CRACK"), (LOGO_COLORS[1], 'M', "logo, ATTACK!"),
    (HINT_COLOR, 'H', "key hints"),
];

/// Letter of a color at column `col` in captures (the board and the side panel have their
/// own letters); `?` for a color not used there.
pub(crate) fn letter(col: usize, color: u8) -> char {
    let letters = if col < PANEL_COLUMN { BOARD_LETTERS } else { PANEL_LETTERS };
    letters.iter().find(|c| c.0 == color).map_or('?', |c| c.1)
}

/// Header of a capture file: what the letters mean.
pub(crate) fn legend() -> String {
    let mut out = String::from(
        "Crack Attack! terminal captures. Each capture shows the characters, then for each\n\
         one its ink (glyph color; for an eighth block such as ▃, the lower part) and its\n\
         paper (background; the upper part). '-' is text in the default color, a space the\n\
         default background.\n",
    );
    for (title, letters) in [("Board:", BOARD_LETTERS), ("Side panel:", PANEL_LETTERS)] {
        out.push_str(title);
        out.push('\n');
        for (color, letter, name) in letters {
            out.push_str(&format!("  {}  {:<24} (256-color {})\n", letter, name, color));
        }
    }
    out.push('\n');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_cursor_moves_to_a_position() {
        let mut screen = Screen::new();
        screen.feed("\x1b[Habc\r\ndef\x1b[2;2HX\x1b[1;3H\x1b[K");
        assert_eq!(screen.text(), vec!["ab", "dXf"]);
    }

    #[test]
    fn colors_and_reverse_video_are_decoded() {
        let mut screen = Screen::new();
        screen.feed("\x1b[?2026h\x1b[Hab\x1b[38;5;127m█\x1b[39m\r\n\x1b[38;5;62;48;5;124m▃\x1b[39;49m\x1b[38;5;231;7m▂\x1b[27;39m");
        assert_eq!(screen.text(), vec!["ab█", "▃▂"]);
        assert_eq!(screen.cell(0, 2).ink, Some(127));
        assert_eq!(screen.cell(1, 0), Cell { ch: '▃', ink: Some(62), paper: Some(124) });
        // Reverse: the glyph takes the background, the upper part the color.
        assert_eq!(screen.cell(1, 1), Cell { ch: '▂', ink: None, paper: Some(231) });
        let capture = screen.capture("t");
        assert!(capture.contains("--- ink ---\n--P\nB\n"));
        assert!(capture.contains("--- paper ---\n\nRW\n"));
    }

    #[test]
    fn erase_to_end_of_line_removes_older_text() {
        let mut screen = Screen::new();
        screen.feed("\x1b[Habc Chain x2!\r\nkeep");
        screen.feed("\x1b[Habc\x1b[K\r\n");
        assert_eq!(screen.text(), vec!["abc", "keep"]);
    }

    #[test]
    fn a_new_frame_overwrites_the_previous_one() {
        let mut screen = Screen::new();
        screen.feed("\x1b[Hxyz\r\nuvw");
        screen.feed("\x1b[Hab");
        assert_eq!(screen.text(), vec!["abz", "uvw"]);
    }
}
