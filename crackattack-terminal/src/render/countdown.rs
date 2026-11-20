// The 3-2-1-GO countdown: big block digits drawn on the board's fine canvas, over the
// grid (which stays visible around them). Each digit pops in (it stretches to its full
// height in a few ticks, with a small overshoot) and has its own color.

use crackattack_core::frame_data::{CountdownDigit, SessionLifecycle, SessionSnapshot};

use super::board::{compact, field_columns, top_unit};
use super::font::{fit, paint_word, word_width, GLYPH_HEIGHT};
use super::palette::{COLORS as BLOCK_COLORS, COUNTDOWN_GO_COLOR, COUNTDOWN_ONE_COLOR, COUNTDOWN_OUTLINE_COLOR, DANGER_RED};
use super::raster::Canvas;

/// One color per digit: red, orange, yellow, then green for GO.
const COLORS: [u8; 4] = [DANGER_RED[0], BLOCK_COLORS[4], COUNTDOWN_ONE_COLOR, COUNTDOWN_GO_COLOR];
/// Dark outline under the digits, so they read over any block.
const OUTLINE_COLOR: u8 = COUNTDOWN_OUTLINE_COLOR;
/// Height of the digit over its first ticks, as a fraction of its full height.
const POP: [f32; 6] = [0.3, 0.6, 0.9, 1.15, 1.05, 1.0];
/// Outline thickness: one column, and this many vertical units.
const OUTLINE_UNITS: i32 = 4;

/// The digit on screen and how long it has been there.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Countdown {
    pub digit: CountdownDigit,
    pub ticks: i32,
}

impl Countdown {
    /// The countdown to draw for this snapshot, if the session is counting down.
    pub(crate) fn from_snapshot(snapshot: &SessionSnapshot) -> Option<Self> {
        if snapshot.lifecycle != SessionLifecycle::Countdown {
            return None;
        }
        let info = snapshot.countdown.as_ref()?;
        Some(Self { digit: info.digit, ticks: info.ticks_into_digit })
    }

    fn word_and_color(&self) -> (&'static str, u8) {
        match self.digit {
            CountdownDigit::Three => ("3", COLORS[0]),
            CountdownDigit::Two => ("2", COLORS[1]),
            CountdownDigit::One => ("1", COLORS[2]),
            CountdownDigit::Go => ("GO", COLORS[3]),
        }
    }

    /// Paints the digit over the middle of the board. A pixel is as wide as the field
    /// allows (digits bigger than GO, which has two letters), about 4 units per column.
    pub(super) fn paint(&self, canvas: &mut Canvas) {
        let (word, color) = self.word_and_color();
        let columns_max = if compact() { 3 } else { 4 };
        let (columns, full) = {
            let (across, _) = fit(word_width(word), GLYPH_HEIGHT, field_columns(), i32::MAX / 16);
            let across = across.min(columns_max);
            (across, if compact() { 8 } else { 16 })
        };
        let pop = POP.get(self.ticks.max(0) as usize).copied().unwrap_or(1.0);
        let units = ((full as f32) * pop).round() as i32;
        let width = word_width(word) * columns;
        let left = field_columns().saturating_sub(width) / 2;
        let top = top_unit() / 2 + (GLYPH_HEIGHT as i32 * units) / 2;
        paint_word(canvas, word, left, top, (columns, units), color, Some((OUTLINE_COLOR, OUTLINE_UNITS)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn painted(digit: CountdownDigit, ticks: i32) -> Canvas {
        let mut canvas = Canvas::new();
        Countdown { digit, ticks }.paint(&mut canvas);
        canvas
    }

    fn lit(canvas: &Canvas, color: u8) -> usize {
        let mut n = 0;
        for col in 0..field_columns() {
            for unit in -64..super::top_unit() {
                n += (canvas.get(col, unit) == Some(color)) as usize;
            }
        }
        n
    }

    #[test]
    fn every_digit_has_its_color_and_stays_inside_the_field() {
        for (digit, color) in [
            (CountdownDigit::Three, 196), (CountdownDigit::Two, 208),
            (CountdownDigit::One, 220), (CountdownDigit::Go, 46),
        ] {
            for compact in [false, true] {
                super::super::COMPACT.with(|c| c.set(compact));
                let canvas = painted(digit, 20);
                assert!(lit(&canvas, color) > 0, "{:?} compact={} not drawn", digit, compact);
                // Something dark around it, and nothing painted past the field's edge.
                assert!(lit(&canvas, OUTLINE_COLOR) > 0);
                assert_eq!(canvas.get(field_columns(), 0), None);
            }
        }
        super::super::COMPACT.with(|c| c.set(false));
    }

    #[test]
    fn the_digit_pops_in_then_settles_at_its_full_size() {
        let size = |ticks| lit(&painted(CountdownDigit::Three, ticks), 196);
        assert!(size(0) < size(2) && size(2) < size(3), "does not grow");
        // Overshoot, then back to the full size, which holds until the next digit.
        assert!(size(3) > size(5));
        assert_eq!(size(5), size(40));
    }

    #[test]
    fn only_a_countdown_session_draws_it() {
        use crackattack_core::vs::{GameSession, SoloSession};
        use crackattack_core::GameState;
        let mut session = SoloSession::new(GameState::new(3));
        let snapshot = session.snapshot();
        assert!(Countdown::from_snapshot(&snapshot).is_some());
        let none = crackattack_core::InputSnapshot::new();
        for _ in 0..200 {
            session.tick(&none);
        }
        assert!(Countdown::from_snapshot(&session.snapshot()).is_none());
    }
}
