// Big block letters: words painted on the board's fine canvas, for the countdown and the
// messages that cover the board. A glyph is 7 pixels tall and 3 to 5 wide; a pixel is
// `columns` terminal columns by `units` vertical units (eighths of a line).

use super::board::{field_columns, top_unit, LINE_UNITS};
use super::raster::Canvas;

pub(super) const GLYPH_HEIGHT: usize = 7;

const GLYPHS: &[(char, &[&str; GLYPH_HEIGHT])] = &[
    ('1', &["  #  ", " ##  ", "  #  ", "  #  ", "  #  ", "  #  ", " ### "]),
    ('2', &[" ### ", "#   #", "    #", "   # ", "  #  ", " #   ", "#####"]),
    ('3', &[" ### ", "#   #", "    #", "  ## ", "    #", "#   #", " ### "]),
    ('4', &["   # ", "  ## ", " # # ", "#  # ", "#####", "   # ", "   # "]),
    ('5', &["#####", "#    ", "#### ", "    #", "    #", "#   #", " ### "]),
    ('6', &[" ### ", "#    ", "#    ", "#### ", "#   #", "#   #", " ### "]),
    ('7', &["#####", "    #", "   # ", "  #  ", "  #  ", "  #  ", "  #  "]),
    ('8', &[" ### ", "#   #", "#   #", " ### ", "#   #", "#   #", " ### "]),
    ('9', &[" ### ", "#   #", "#   #", " ####", "    #", "    #", " ### "]),
    ('0', &[" ### ", "#   #", "#  ##", "# # #", "##  #", "#   #", " ### "]),
    ('G', &[" ###", "#   ", "#   ", "# ##", "#  #", "#  #", " ###"]),
    ('O', &[" ## ", "#  #", "#  #", "#  #", "#  #", "#  #", " ## "]),
    ('U', &["#  #", "#  #", "#  #", "#  #", "#  #", "#  #", " ## "]),
    ('Y', &["#   #", "#   #", " # # ", "  #  ", "  #  ", "  #  ", "  #  "]),
    ('W', &["#   #", "#   #", "#   #", "# # #", "# # #", "## ##", "#   #"]),
    ('I', &["###", " # ", " # ", " # ", " # ", " # ", "###"]),
    ('N', &["#   #", "##  #", "# # #", "#  ##", "#   #", "#   #", "#   #"]),
    ('L', &["#   ", "#   ", "#   ", "#   ", "#   ", "#   ", "####"]),
    ('S', &[" ###", "#   ", "#   ", " ## ", "   #", "   #", "### "]),
    ('E', &["####", "#   ", "#   ", "### ", "#   ", "#   ", "####"]),
];

fn glyph(c: char) -> Option<&'static [&'static str; GLYPH_HEIGHT]> {
    GLYPHS.iter().find(|g| g.0 == c).map(|g| g.1)
}

/// Width of `word` in pixels, one blank pixel between letters; letters without a glyph
/// count as nothing.
pub(super) fn word_width(word: &str) -> usize {
    let widths: Vec<usize> = word.chars().filter_map(glyph).map(|g| g[0].len()).collect();
    widths.iter().sum::<usize>() + widths.len().saturating_sub(1)
}

/// Paints `word` with its left edge at column `left` and its top at unit `top`, in `color`.
/// With `outline`, a dark border (one column, `outline_units` vertical units) is painted
/// first, so the word reads over anything.
pub(super) fn paint_word(
    canvas: &mut Canvas, word: &str, left: usize, top: i32, (columns, units): (usize, i32), color: u8,
    outline: Option<(u8, i32)>,
) {
    let mut pixels = Vec::new();
    let mut x = 0;
    for letter in word.chars().filter_map(glyph) {
        for (row, line) in letter.iter().enumerate() {
            for (dx, c) in line.chars().enumerate() {
                if c == '#' {
                    let high = top - row as i32 * units;
                    pixels.push((left + (x + dx) * columns, high - units, high));
                }
            }
        }
        x += letter[0].len() + 1;
    }
    if let Some((border, thickness)) = outline {
        for &(col, low, high) in &pixels {
            canvas.fill(col.saturating_sub(1)..=col + columns, low - thickness, high + thickness, border);
        }
    }
    for &(col, low, high) in &pixels {
        canvas.fill(col..=col + columns - 1, low, high, color);
    }
}

/// A message that covers the board: some text, big words, some text (the words may be
/// absent, the message is then text only).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Banner {
    /// Plain text lines above the big words (all the text when there are no words).
    pub above: Vec<String>,
    /// Words drawn in big blocks, one under the other.
    pub words: Vec<String>,
    pub color: u8,
    /// Plain text lines under the big words.
    pub below: Vec<String>,
}

const TEXT_COLOR: u8 = 255;

/// The pixel size for words of up to `widest` pixels in a field `columns` wide whose
/// `rows` pixel rows must fit in `units`: as wide as the field allows (4 columns at most),
/// as tall as 8 units per column (16 at most), so a word keeps its height when it gets
/// longer (the digits of round 10 are as tall as those of round 9, only narrower), then
/// squeezed in height if the words are too tall for the field.
pub(super) fn fit(widest: usize, rows: usize, columns: usize, units: i32) -> (usize, i32) {
    let across = (columns / widest.max(1)).clamp(1, 4);
    let wanted = (8 * across as i32).clamp(6, 16);
    (across, wanted.min(units / rows.max(1) as i32).max(3))
}

/// The canvas of a banner with big words: the board is not painted, the words are, and
/// the text lines are glyph marks around them, the whole centered over the playfield.
pub(super) fn banner_canvas(banner: &Banner) -> Canvas {
    let mut canvas = Canvas::new();
    let columns = field_columns();
    // A word wider than the field even at one column per pixel (round 1000 in the compact
    // field) is written as text instead of being cut off.
    let owned;
    let banner = if banner.words.iter().any(|w| word_width(w) > columns) {
        owned = Banner { above: [banner.above.clone(), banner.words.clone()].concat(), words: Vec::new(), ..banner.clone() };
        &owned
    } else {
        banner
    };
    let (above, below) = (banner.above.len() as i32, banner.below.len() as i32);
    let n = banner.words.len();
    let rows = if n == 0 { 0 } else { GLYPH_HEIGHT * n + n - 1 };
    let widest = banner.words.iter().map(|w| word_width(w)).max().unwrap_or(1);
    let playfield = top_unit();
    let spare = playfield - (above + below) * LINE_UNITS;
    let (across, height) = fit(widest, rows, columns, spare);
    let words_units = rows as i32 * height;
    let total = (above + below) * LINE_UNITS + words_units;
    // Whole lines for the text: the block starts on a line boundary.
    let base = ((playfield - total) / 2).div_euclid(LINE_UNITS) * LINE_UNITS;
    let words_bottom = base + below * LINE_UNITS;
    let words_top = words_bottom + words_units;
    for (i, word) in banner.words.iter().enumerate() {
        let left = columns.saturating_sub(word_width(word) * across) / 2;
        let top = words_top - (i * (GLYPH_HEIGHT + 1)) as i32 * height;
        paint_word(&mut canvas, word, left, top, (across, height), banner.color, None);
    }
    let text_at = |canvas: &mut Canvas, text: &str, line: i32| {
        let left = columns.saturating_sub(text.chars().count()) / 2;
        // Spaces too: they hide the dots of the grid between the words of a line.
        for (i, c) in text.chars().enumerate() {
            canvas.marks.push((left + i, line, c, TEXT_COLOR));
        }
    };
    for (k, text) in banner.below.iter().enumerate() {
        text_at(&mut canvas, text, base / LINE_UNITS + below - 1 - k as i32);
    }
    let above_base = (words_top + LINE_UNITS - 1).div_euclid(LINE_UNITS) * LINE_UNITS;
    for (k, text) in banner.above.iter().enumerate() {
        text_at(&mut canvas, text, above_base / LINE_UNITS + above - 1 - k as i32);
    }
    canvas
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_glyph_is_a_rectangle_of_seven_rows_and_the_words_we_use_exist() {
        for (c, g) in GLYPHS {
            let width = g[0].len();
            assert!(g.iter().all(|row| row.len() == width), "glyph {} is ragged", c);
            assert!(g.iter().any(|row| row.contains('#')));
        }
        for word in ["YOU", "WIN", "LOSE", "GO", "0123456789"] {
            assert!(word.chars().all(|c| glyph(c).is_some()), "{}", word);
        }
        assert_eq!(word_width("YOU"), 5 + 4 + 4 + 2);
        assert_eq!(word_width("LOSE"), 4 * 4 + 3);
    }

    #[test]
    fn the_words_of_the_messages_fit_the_narrowest_field() {
        // Compact: 19 columns, a pixel one column wide.
        for word in ["YOU", "WIN", "LOSE", "GO"] {
            assert!(word_width(word) <= 19, "{}", word);
        }
        assert_eq!(fit(word_width("LOSE"), 15, 19, 13 * LINE_UNITS).0, 1);
    }
}
