// From the fine canvas to terminal characters: each character shows 8 vertical units
// with at most two colors, split at an eighth (▁▂▃▄▅▆▇█).

use std::fmt::Write;

use super::board::{BOTTOM_UNIT, FIELD_COLUMNS, SPAN_UNITS, TOP_UNIT};
use super::palette::CURSOR_COLOR;

/// Eighth blocks, from 1/8 to 7/8 of the height, drawn from the bottom.
pub(super) const LOWER_EIGHTHS: [char; 7] = ['▁', '▂', '▃', '▄', '▅', '▆', '▇'];
/// Weight of a cursor unit when a character picks its two colors (a whole line's worth).
const CURSOR_WEIGHT: usize = 8;

/// Fine paint buffer: a color (or nothing) per terminal column and vertical unit.
pub(super) struct Canvas {
    cells: Vec<Option<u8>>,
    /// Glyphs drawn over the board at (column, line): dying blocks.
    pub(super) marks: Vec<(usize, i32, char)>,
    /// Dot places (column, line) to leave blank: the cells of a swap, which a sliding
    /// block crosses.
    pub(super) no_dots: Vec<(usize, i32)>,
}

impl Canvas {
    pub(super) fn new() -> Self {
        Self { cells: vec![None; FIELD_COLUMNS * SPAN_UNITS], marks: Vec::new(), no_dots: Vec::new() }
    }

    pub(super) fn mark(&self, col: usize, line: i32) -> Option<char> {
        self.marks.iter().find(|m| m.0 == col && m.1 == line).map(|m| m.2)
    }

    pub(super) fn get(&self, col: usize, unit: i32) -> Option<u8> {
        let u = unit - BOTTOM_UNIT;
        if col >= FIELD_COLUMNS || u < 0 || u as usize >= SPAN_UNITS {
            return None;
        }
        self.cells[u as usize * FIELD_COLUMNS + col]
    }

    /// Paints columns `cols` over units `[bottom, top)`, clipped to the canvas.
    pub(super) fn fill(&mut self, cols: std::ops::RangeInclusive<usize>, bottom: i32, top: i32, color: u8) {
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

/// One terminal character for 8 stacked units (`units[0]` at the bottom): the split
/// (lower color, upper color, height of the lower part in eighths) that matches the most
/// units.
pub(super) fn split_line(units: &[Option<u8>; 8]) -> (Option<u8>, Option<u8>, usize) {
    // The cursor counts more than anything else: its thin bracket ticks must never be
    // dropped when a character spans three colors (block, tick, gap).
    let weight = |u: &Option<u8>| if *u == Some(CURSOR_COLOR) { CURSOR_WEIGHT } else { 1 };
    let most_common = |part: &[Option<u8>]| -> Option<u8> {
        let mut best = (0, None);
        for candidate in part {
            let n: usize = part.iter().filter(|u| *u == candidate).map(weight).sum();
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
        let errors: usize = units[..k].iter().filter(|u| **u != lower).map(weight).sum::<usize>()
            + units[k..].iter().filter(|u| **u != upper).map(weight).sum::<usize>();
        if errors < best.0 {
            best = (errors, lower, upper, k);
        }
    }
    (best.1, best.2, best.3)
}

/// Colors in effect while the board is written, so that a run of characters in the same
/// colors sets them once.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(super) struct Pen {
    fg: Option<u8>,
    bg: Option<u8>,
    reverse: bool,
}

impl Pen {
    pub(super) fn ink(fg: u8) -> Self {
        Self { fg: Some(fg), ..Self::default() }
    }

    /// Switches the terminal to `to`, in one sequence with only what differs.
    pub(super) fn switch(&mut self, out: &mut String, to: Pen) {
        if *self == to {
            return;
        }
        let mut params: Vec<String> = Vec::with_capacity(3);
        if self.fg != to.fg {
            params.push(to.fg.map_or("39".into(), |c| format!("38;5;{}", c)));
        }
        if self.bg != to.bg {
            params.push(to.bg.map_or("49".into(), |c| format!("48;5;{}", c)));
        }
        if self.reverse != to.reverse {
            params.push(if to.reverse { "7" } else { "27" }.into());
        }
        let _ = write!(out, "\x1b[{}m", params.join(";"));
        *self = to;
    }
}

/// Writes one character from its split: two colors at most, foreground for the lower
/// part. An empty upper part over a color takes reverse video, so the glyph (lower part)
/// is drawn in the terminal background.
pub(super) fn push_char(out: &mut String, pen: &mut Pen, lower: Option<u8>, upper: Option<u8>, k: usize) {
    let (lower, upper, k) = match (lower, upper) {
        // One color over the whole height.
        _ if k == 0 => (upper, upper, 8),
        _ if k == 8 || lower == upper => (lower, lower, 8),
        _ => (lower, upper, k),
    };
    let (glyph, colors) = match (lower, upper) {
        (None, None) => (' ', Pen::default()),
        (Some(c), Some(u)) if k == 8 || c == u => ('█', Pen::ink(c)),
        (Some(c), Some(u)) => (LOWER_EIGHTHS[k - 1], Pen { bg: Some(u), ..Pen::ink(c) }),
        (Some(c), None) => (LOWER_EIGHTHS[k - 1], Pen::ink(c)),
        (None, Some(u)) => (LOWER_EIGHTHS[k - 1], Pen { reverse: true, ..Pen::ink(u) }),
    };
    pen.switch(out, colors);
    out.push(glyph);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::palette::COLORS;

    #[test]
    fn a_run_of_one_color_sets_it_once() {
        let mut out = String::new();
        let mut pen = Pen::default();
        for _ in 0..3 {
            push_char(&mut out, &mut pen, Some(5), Some(5), 8);
        }
        push_char(&mut out, &mut pen, Some(5), Some(9), 3);
        push_char(&mut out, &mut pen, None, Some(9), 3);
        push_char(&mut out, &mut pen, None, None, 0);
        assert_eq!(out, "\x1b[38;5;5m███\x1b[48;5;9m▃\x1b[38;5;9;49;7m▃\x1b[39;27m ");
    }

    #[test]
    fn a_uniform_line_is_one_full_color() {
        assert_eq!(split_line(&[Some(3); 8]).2 % 8, 0);
        let (lower, upper, k) = split_line(&[None; 8]);
        assert_eq!((lower, upper), (None, None));
        assert!(k == 0 || k == 8);
    }

    #[test]
    fn a_bracket_tick_survives_between_a_block_and_a_gap() {
        // Block below, a 2-unit tick, then the 1-unit gap: three colors in one character.
        let (b, w) = (Some(COLORS[0]), Some(CURSOR_COLOR));
        for units in [[b, b, b, b, w, w, None, b], [b, b, b, b, b, w, w, None], [None, w, w, b, b, b, b, b]] {
            let (lower, upper, _) = split_line(&units);
            assert!(lower == w || upper == w, "tick lost in {:?}", units);
        }
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
}
