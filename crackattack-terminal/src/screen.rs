// Text captures of the screen, for autoplay runs and checks.
//
// The renderer's output is decoded the way a terminal would (characters, 256-color SGR,
// reverse video, cursor home), then written as plain text: the characters shown, and for
// each one a letter for its ink (glyph color: the lower part of an eighth block) and its
// paper (background: the upper part). Captures can be compared and searched without
// images.

use crate::render::COLOR_LETTERS;

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
                        'H' => (self.row, self.col) = (0, 0),
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
        self.layer(|cell| cell.ch)
    }

    fn layer(&self, f: impl Fn(&Cell) -> char) -> Vec<String> {
        self.rows
            .iter()
            .map(|line| line.iter().map(&f).collect::<String>().trim_end().to_string())
            .collect()
    }

    /// One capture: the characters, then the ink and paper letters.
    pub(crate) fn capture(&self, title: &str) -> String {
        let ink = |cell: &Cell| match cell.ink {
            Some(color) => letter(color),
            // An eighth block with no ink: its lower part is the default background.
            None if cell.ch != ' ' && !('▁'..='█').contains(&cell.ch) => '-',
            None => ' ',
        };
        let paper = |cell: &Cell| cell.paper.map_or(' ', letter);
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

/// Letter of a color in captures; `?` for a color the game does not use.
pub(crate) fn letter(color: u8) -> char {
    COLOR_LETTERS.iter().find(|c| c.0 == color).map_or('?', |c| c.1)
}

/// Header of a capture file: what the letters mean.
pub(crate) fn legend() -> String {
    let mut out = String::from(
        "Crack Attack! terminal captures. Each capture shows the characters, then for each\n\
         one its ink (glyph color; for an eighth block such as ▃, the lower part) and its\n\
         paper (background; the upper part). '-' is text in the default color, a space the\n\
         default background.\n",
    );
    for (color, letter, name) in COLOR_LETTERS {
        out.push_str(&format!("  {}  {:<24} (256-color {})\n", letter, name, color));
    }
    out.push('\n');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

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
