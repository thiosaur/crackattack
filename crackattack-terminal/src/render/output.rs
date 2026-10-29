// What goes to the terminal: only the screen lines that changed since the last frame.
// Over SSH, a full redraw at 50 frames per second is about 430 KB/s; most frames change
// a line or two, and nearly half change nothing.

use std::fmt::Write;

use super::CLEAR;

/// Synchronized update: the terminal shows the frame only once it is complete (no tearing).
const SYNC_BEGIN: &str = "\x1b[?2026h";
const SYNC_END: &str = "\x1b[?2026l";

/// The screen lines the terminal shows, to write only the differences.
pub(super) struct Output {
    /// Lines as last written; empty until the first frame or after `invalidate`.
    shown: Vec<String>,
    /// Clear the screen before the next frame (first frame, or the terminal changed).
    clear: bool,
    /// Terminal width (columns), if known: longer lines are cut, so the side panel on
    /// the right gives way first.
    width: Option<usize>,
}

impl Output {
    pub(super) fn new() -> Self {
        Self { shown: Vec::new(), clear: true, width: None }
    }

    /// Forgets what the terminal shows (resized, or written to by something else): the
    /// next frame is drawn whole, on a cleared screen.
    pub(super) fn invalidate(&mut self) {
        self.shown.clear();
        self.clear = true;
    }

    /// A new terminal width: the next frame is drawn whole.
    pub(super) fn set_width(&mut self, width: Option<usize>) {
        if self.width != width {
            self.width = width;
            self.invalidate();
        }
    }

    /// The text that turns the screen into `lines`: each changed line is written at its
    /// place and erased to its end, lines no longer there are erased. Empty when nothing
    /// changed. Every line must end with the default colors.
    pub(super) fn update(&mut self, lines: Vec<String>) -> String {
        let lines: Vec<String> = match self.width {
            Some(width) => lines.into_iter().map(|line| clip(line, width)).collect(),
            None => lines,
        };
        let mut out = String::new();
        if self.clear {
            out.push_str(CLEAR);
            self.clear = false;
        }
        for (i, line) in lines.iter().enumerate() {
            if self.shown.get(i) != Some(line) {
                let _ = write!(out, "\x1b[{};1H{}\x1b[K", i + 1, line);
            }
        }
        for i in lines.len()..self.shown.len() {
            let _ = write!(out, "\x1b[{};1H\x1b[K", i + 1);
        }
        self.shown = lines;
        if out.is_empty() {
            return out;
        }
        format!("{}{}{}", SYNC_BEGIN, out, SYNC_END)
    }
}

/// `line` cut to fit `width` columns. The last column stays free: a character written
/// there would be erased with the end of the line. Escape sequences are all kept, so the
/// colors still end where they should.
fn clip(line: String, width: usize) -> String {
    let room = width.saturating_sub(1);
    let mut out = String::with_capacity(line.len());
    let mut shown = 0;
    let mut chars = line.chars();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            out.push(c);
            // "ESC [", parameters, then a final byte from @ to ~.
            out.extend(chars.next());
            for p in chars.by_ref() {
                out.push(p);
                if ('@'..='~').contains(&p) {
                    break;
                }
            }
        } else if shown < room {
            out.push(c);
            shown += 1;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(text: &[&str]) -> Vec<String> {
        text.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn lines_are_cut_to_the_terminal_width_keeping_their_colors() {
        assert_eq!(clip("abc\x1b[38;5;1mdef\x1b[39mgh".to_string(), 5), "abc\x1b[38;5;1md\x1b[39m");
        assert_eq!(clip("ab".to_string(), 80), "ab");
        let mut output = Output::new();
        output.set_width(Some(4));
        assert!(output.update(lines(&["abcdef"])).contains("\x1b[1;1Habc\x1b[K"));
    }

    #[test]
    fn only_the_changed_lines_are_written() {
        let mut output = Output::new();
        let first = output.update(lines(&["a", "b", "c"]));
        assert!(first.contains(CLEAR) && first.contains("\x1b[1;1Ha") && first.contains("\x1b[3;1Hc"));
        assert_eq!(output.update(lines(&["a", "b", "c"])), "");
        let next = output.update(lines(&["a", "B", "c"]));
        assert_eq!(next, format!("{}\x1b[2;1HB\x1b[K{}", SYNC_BEGIN, SYNC_END));
    }

    #[test]
    fn lines_gone_are_erased_and_invalidate_redraws_everything() {
        let mut output = Output::new();
        output.update(lines(&["a", "b", "debug"]));
        assert_eq!(output.update(lines(&["a", "b"])), format!("{}\x1b[3;1H\x1b[K{}", SYNC_BEGIN, SYNC_END));
        output.invalidate();
        let all = output.update(lines(&["a", "b"]));
        assert!(all.contains(CLEAR) && all.contains("\x1b[1;1Ha") && all.contains("\x1b[2;1Hb"));
    }
}
