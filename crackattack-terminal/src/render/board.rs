// The board: its geometry, and blocks, garbage, cursor and dots painted on a fine grid
// (16 units per row, 8 per terminal line). Blocks are solid, and the whole stack rises
// above a fixed ground line. Everything moves by half rows (whole terminal lines), so the
// gaps between blocks always show; with `--smooth`, by sixteenths of a row.

use crackattack_core::constants::*;
use crackattack_core::frame_data::FrameData;
use crackattack_core::garbage::GARBAGE_FLAVOR_GRAY;
use crackattack_core::BlockState;

use super::palette::*;
use super::raster::{push_char, split_line, Canvas, Pen};

thread_local! {
    /// `--smooth`: rows move by sixteenths instead of halves. A terminal character holds
    /// two colors only, so the one-unit gap between two stacked blocks shows only when it
    /// falls on the edge of a character: always with half-row steps, in 2 positions out
    /// of 8 with sixteenths. The same goes for the cursor bracket ticks.
    pub(crate) static SMOOTH_RISE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Fine vertical units: per row, and per terminal line.
pub(super) const ROW_UNITS: i32 = 16;
pub(super) const LINE_UNITS: i32 = 8;
/// Terminal columns per cell: a gap column (cursor brackets, garbage seams) then the block.
pub(super) const CELL_COLUMNS: usize = 4;
const BLOCK_COLUMNS: usize = 3;
/// Width of the playfield in terminal columns (with the closing gap column on the right).
pub(super) const FIELD_COLUMNS: usize = PLAY_WIDTH * CELL_COLUMNS + 1;
/// Lines shown below the ground: the part of the rising row not pushed up yet.
pub(super) const LINES_BELOW_GROUND: i32 = 2;
/// Vertical span painted, in units: from the bottom of the rising row to the top of the
/// playfield.
pub(super) const BOTTOM_UNIT: i32 = -LINES_BELOW_GROUND * LINE_UNITS;
pub(super) const TOP_UNIT: i32 = SAFE_HEIGHT as i32 * ROW_UNITS;
pub(super) const SPAN_UNITS: usize = (TOP_UNIT - BOTTOM_UNIT) as usize;

/// Dying block: a small square, which shrinks during the second half of the delay.
const DYING_GLYPHS: [char; 2] = ['■', '▪'];
/// How long a garbage flashes once hit (ticks).
const SHATTER_FLASH_TICKS: i32 = 8;
/// Last ticks before a block of a shattered garbage pops: it flips like a card, from the
/// garbage color to its own (as the graphical version, `4 * INTERNAL_POP_DELAY / 5`).
pub(super) const POP_ROTATE_TIME: i32 = 4 * INTERNAL_POP_DELAY / 5;
/// Thinnest a flipping block gets, as a fraction of a block.
const FLIP_MIN_HEIGHT: f32 = 0.2;
/// How far two swapping blocks move apart vertically where they cross (units).
pub(super) const SWAP_LIFT_UNITS: i32 = 5;

/// Marks the playable area: a dot at the center of each empty cell. The dots rise with
/// the stack; the glyph and line are picked so the dot lands closest to the center.
pub(super) const DOTS: [char; 3] = ['.', '·', '˙'];
/// Height of each dot's middle within its character, in units from the bottom (8 = top),
/// measured with Menlo: a period sits well above the bottom (room for descenders), so at
/// rest, with the center on a line boundary, the dot above the line below (˙) is the
/// closest, about one unit low, where a period in the line above is three units high.
pub(super) const DOT_HEIGHTS: [f32; 3] = [2.5, 4.2, 6.5];
/// Cursor: two brackets around its two cells, rising with the blocks and framing their
/// row, one unit beyond it at the top and the bottom. Each is a bar in the gap column,
/// with a tick at the top and at the bottom running over the first column of the block
/// next to it. This is the thickness of the ticks (units).
pub(super) const CURSOR_TICK_UNITS: i32 = 2;

/// Bottom of a row in units above the ground, from its grid row, the stack rise and the
/// sub-row fall offset. Row 1 sits on the ground when the stack has not risen. Snapped
/// to whole terminal lines (half rows) unless `--smooth`.
pub(super) fn bottom_unit(grid_y: i32, sub_grid_y: i32, play_offset_y: f32) -> i32 {
    let rows = grid_y as f32 - 1.0 + play_offset_y + sub_grid_y as f32 / STEPS_PER_GRID as f32;
    let step = if SMOOTH_RISE.with(|smooth| smooth.get()) { 1 } else { LINE_UNITS };
    let units = rows * ROW_UNITS as f32 / step as f32;
    if step == 1 {
        return units.round() as i32;
    }
    // A row reaches a line only once it has fully risen into it (the margin absorbs
    // float error on exact halves).
    (units + 1e-3).floor() as i32 * step
}

/// Paints blocks and garbage on the fine grid.
pub(super) fn paint(frame: &FrameData) -> Canvas {
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
        let state = BlockState::from_bits_truncate(block.state);
        let is_dying = state.contains(BlockState::DYING);
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
        if state.contains(BlockState::AWAKING) {
            // From a shattered garbage.
            let remaining = block.pop_countdown - frame.time_step;
            let under_slab = canvas.get(left as usize + 1, bottom + ROW_UNITS / 2).is_some();
            if block.pop_countdown != 0 && remaining >= POP_ROTATE_TIME && under_slab {
                // Still inside its garbage, which is drawn (and flashes) as one slab.
                continue;
            }
            let (awaking_color, height) =
                awaking_look(remaining, block.pop_countdown, block.pop_color, color);
            color = awaking_color;
            let shrink = ((ROW_UNITS - 1) as f32 * (1.0 - height) / 2.0).round() as i32;
            low += shrink;
            high -= shrink;
            if low >= high {
                continue;
            }
        } else if state.contains(BlockState::SWAPPING) {
            // Slides toward the other cell, the block going right in
            // the upper part of the row and the one going left in the lower part, so both
            // stay visible where they cross.
            let rightward = state.contains(BlockState::SWAP_DIRECTION);
            let other = if rightward { x + 1 } else { x.wrapping_sub(1) };
            for cell in [x, other].into_iter().filter(|&c| c < PLAY_WIDTH) {
                // Every line the row touches: the dot sits in one of them.
                for line in bottom.div_euclid(LINE_UNITS)..=(bottom + ROW_UNITS - 1).div_euclid(LINE_UNITS) {
                    canvas.no_dots.push((cell * CELL_COLUMNS + CELL_COLUMNS / 2, line));
                }
            }
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
    let top = bottom + ROW_UNITS - 1;
    let (left, right) = (cursor_x * CELL_COLUMNS, (cursor_x + 2) * CELL_COLUMNS);
    let ticks = cursor_ticks(bottom);
    for (bar, tick) in [(left, left + 1), (right, right - 1)] {
        canvas.fill(bar..=bar, bottom - 1, top + 1, CURSOR_COLOR);
        for (low, high) in ticks {
            canvas.fill(tick..=tick, low, high, CURSOR_COLOR);
        }
    }

    canvas
}

/// Units `[low, high)` of the bottom and top bracket ticks of a cursor row starting at
/// `bottom`. The brackets frame the row: bars and ticks reach into the gaps above and
/// below it (`bottom - 1` and `top`), so the ticks end flush with the bars. A terminal
/// character shows two colors only: a tick sharing its character with the brick below
/// (or above) would spill over it, so it then moves into the next character inside its
/// own row (only with `--smooth`: half-row steps never need it).
pub(super) fn cursor_ticks(bottom: i32) -> [(i32, i32); 2] {
    let top = bottom + ROW_UNITS - 1;
    let line_start = |unit: i32| unit.div_euclid(LINE_UNITS) * LINE_UNITS;
    // The brick below ends at bottom - 2 (bottom - 1 is the gap).
    let low = if line_start(bottom) <= bottom - 2 {
        (line_start(bottom) + LINE_UNITS, line_start(bottom) + LINE_UNITS + CURSOR_TICK_UNITS)
    } else {
        (bottom - 1, bottom + CURSOR_TICK_UNITS)
    };
    // The brick above starts at top + 1 (top is the gap).
    let end = line_start(top - 1) + LINE_UNITS;
    let high = if end > top + 1 {
        (line_start(top - 1) - CURSOR_TICK_UNITS, line_start(top - 1))
    } else {
        (top - CURSOR_TICK_UNITS, top + 1)
    };
    [low, high]
}

pub(super) fn garbage_color(flavor: i32, flashing: bool) -> u8 {
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
pub(super) fn awaking_look(remaining: i32, pop_countdown: i32, pop_color: i32, color: u8) -> (u8, f32) {
    if pop_countdown == 0 || remaining <= 0 {
        return (color, 1.0);
    }
    if remaining >= POP_ROTATE_TIME {
        return (garbage_color(pop_color, false), 1.0);
    }
    let progress = 1.0 - remaining as f32 / POP_ROTATE_TIME as f32;
    // Never flat: an empty cell would show its dot for a moment.
    let height = (1.0 - 2.0 * progress).abs().max(FLIP_MIN_HEIGHT);
    let shown = if progress < 0.5 { garbage_color(pop_color, false) } else { color };
    (shown, height)
}

/// Dot glyph for the center of a cell of the playable area, if it falls on this line.
/// The playable area is the rows the cursor can reach (1 to `SAFE_HEIGHT - 1`); the row
/// above stays clear: the stack rises into it and garbage arrives through it.
/// A block fills units `[bottom, bottom + 15)`, so its center is at `bottom + 7.5`.
pub(super) fn dot_at(col: usize, line: i32, rise: f32) -> Option<char> {
    if col % CELL_COLUMNS != CELL_COLUMNS / 2 || col >= PLAY_WIDTH * CELL_COLUMNS {
        return None;
    }
    (1..SAFE_HEIGHT as i32).find_map(|row| {
        let center = bottom_unit(row, 0, rise) as f32 + (ROW_UNITS - 1) as f32 / 2.0;
        // The closest (line, glyph): in the line holding the center or the one below.
        let home = (center as i32).div_euclid(LINE_UNITS);
        let (best_line, glyph) = [home, home - 1]
            .into_iter()
            .flat_map(|l| (0..DOTS.len()).map(move |g| (l, g)))
            .min_by(|a, b| {
                let off = |(l, g): (i32, usize)| ((l * LINE_UNITS) as f32 + DOT_HEIGHTS[g] - center).abs();
                off(*a).total_cmp(&off(*b))
            })?;
        let in_view = (0..TOP_UNIT / LINE_UNITS).contains(&best_line);
        (in_view && best_line == line).then_some(DOTS[glyph])
    })
}

/// The board's characters for terminal line `line` (0 = just above the ground), from the
/// painted canvas: blocks, cursor, dying squares and the dots of empty cells. Colors are
/// left set at the end: the caller resets them.
pub(super) fn push_line(out: &mut String, canvas: &Canvas, line: i32, rise: f32) {
    let base = line * LINE_UNITS;
    let mut pen = Pen::default();
    for col in 0..FIELD_COLUMNS {
        if let Some(glyph) = canvas.mark(col, line) {
            pen.switch(out, Pen::ink(DYING_COLOR));
            out.push(glyph);
            continue;
        }
        let units: [Option<u8>; 8] = std::array::from_fn(|u| canvas.get(col, base + u as i32));
        if units.iter().all(Option::is_none) && !canvas.no_dots.contains(&(col, line)) {
            if let Some(dot) = dot_at(col, line, rise) {
                pen.switch(out, Pen::ink(DOT_COLOR));
                out.push(dot);
                continue;
            }
        }
        let (lower, upper, k) = split_line(&units);
        push_char(out, &mut pen, lower, upper, k);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::tests::lone_block_frame;
    use crackattack_core::{GameState, InputSnapshot};

    #[test]
    fn bracket_ticks_stay_within_the_row_and_its_gaps() {
        for bottom in 0..ROW_UNITS * 2 {
            let top = bottom + ROW_UNITS - 1;
            let [(low, low_end), (high_start, high)] = cursor_ticks(bottom);
            assert!(bottom - 1 <= low && low_end <= high_start && high <= top + 1, "bottom {}", bottom);
        }
    }

    #[test]
    fn dots_mark_each_cell_and_rise_with_the_stack() {
        SMOOTH_RISE.with(|smooth| smooth.set(true));
        let count = |rise: f32| (0..TOP_UNIT / LINE_UNITS)
            .flat_map(|line| (0..FIELD_COLUMNS).map(move |col| (col, line)))
            .filter(|&(col, line)| dot_at(col, line, rise).is_some())
            .count();
        assert_eq!(count(0.0), PLAY_WIDTH * (SAFE_HEIGHT - 1));
        // Row 1 (units 0 to 15, center 7.5): at rest, a high dot at the top of line 0;
        // a quarter row higher, a middle dot in line 1; half a row higher, top of line 1.
        let col = CELL_COLUMNS / 2;
        assert_eq!(dot_at(col, 0, 0.0), Some('˙'));
        assert_eq!(dot_at(col, 1, 0.0), None);
        assert_eq!(dot_at(col, 1, 0.25), Some('·'));
        assert_eq!(dot_at(col, 1, 0.5), Some('˙'));
        assert_eq!(dot_at(col + 1, 0, 0.0), None);
    }

    #[test]
    fn each_dot_lands_close_to_its_cell_center_whatever_the_rise() {
        for smooth in [false, true] {
            SMOOTH_RISE.with(|s| s.set(smooth));
            for step in 0..ROW_UNITS {
                let rise = step as f32 / ROW_UNITS as f32;
                let col = CELL_COLUMNS / 2;
                for row in 1..SAFE_HEIGHT as i32 {
                    let center = bottom_unit(row, 0, rise) as f32 + (ROW_UNITS - 1) as f32 / 2.0;
                    let found: Vec<f32> = (0..TOP_UNIT / LINE_UNITS)
                        .filter_map(|line| dot_at(col, line, rise).map(|g| {
                            let i = DOTS.iter().position(|d| *d == g).unwrap();
                            (line * LINE_UNITS) as f32 + DOT_HEIGHTS[i]
                        }))
                        .filter(|y| (y - center).abs() < ROW_UNITS as f32 / 2.0)
                        .collect();
                    if center + 1.5 >= TOP_UNIT as f32 {
                        continue;
                    }
                    assert_eq!(found.len(), 1, "smooth {} rise {}/16 row {}", smooth, step, row);
                    // Half-row steps: always 1 unit off; sixteenths: up to 2, in the hole
                    // between a high dot and the period of the line above.
                    let tolerance = if smooth { 2.0 } else { 1.0 };
                    assert!((found[0] - center).abs() <= tolerance, "smooth {} rise {}/16 row {}: dot at {}, center {}", smooth, step, row, found[0], center);
                }
            }
        }
    }

    #[test]
    fn a_dying_block_is_a_small_square_at_its_cell_center() {
        let mut game = GameState::new(7);
        let mut frame = game.step(&InputSnapshot::new());
        let block = frame.blocks.iter_mut().find(|b| b.grid_y == 3).unwrap();
        block.state |= BlockState::DYING.bits();
        block.animation_countdown = DYING_DELAY;
        let (x, y) = (block.grid_x as usize, block.grid_y);
        let canvas = paint(&frame);
        let center = bottom_unit(y, 0, frame.play_offset_y) + ROW_UNITS / 2;
        let col = x * CELL_COLUMNS + CELL_COLUMNS / 2;
        assert_eq!(canvas.mark(col, center.div_euclid(LINE_UNITS)), Some('■'));
        // The cell itself is left empty around the square.
        assert_eq!(canvas.get(col, center), None);
    }

    /// Units of column `col` painted in `color`.
    fn painted(canvas: &Canvas, col: usize, color: u8) -> Vec<i32> {
        (BOTTOM_UNIT..TOP_UNIT).filter(|&u| canvas.get(col, u) == Some(color)).collect()
    }

    #[test]
    fn swapping_blocks_slide_and_cross_one_above_the_other() {
        let mut frame = lone_block_frame((BlockState::SWAPPING | BlockState::SWAP_DIRECTION).bits()); // going right
        frame.swapper.swap_factor = 0.5;
        let color = COLORS[frame.blocks[0].flavor as usize];
        let bottom = bottom_unit(3, 0, frame.play_offset_y);
        let left = CELL_COLUMNS + 1 + CELL_COLUMNS / 2;
        let canvas = paint(&frame);
        assert!(painted(&canvas, CELL_COLUMNS + 1, color).is_empty());
        let units = painted(&canvas, left, color);
        assert_eq!(units, (bottom + SWAP_LIFT_UNITS..bottom + ROW_UNITS - 1).collect::<Vec<_>>());
        // The block going left keeps the lower part of the row.
        frame.blocks[0].state = BlockState::SWAPPING.bits();
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
        let mut frame = lone_block_frame(BlockState::AWAKING.bits());
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
    fn the_hit_flash_covers_the_whole_garbage_and_a_flip_never_empties_the_cell() {
        // Pre-pop blocks inside a flashing slab leave it whole.
        let mut frame = lone_block_frame(BlockState::AWAKING.bits());
        frame.blocks[0].pop_color = 0;
        frame.blocks[0].pop_countdown = frame.time_step + POP_ROTATE_TIME + 20;
        frame.garbage = vec![crackattack_core::frame_data::GarbageSnapshot {
            id: 1, grid_x: 0, grid_y: 3, sub_grid_y: 0, width: 6, height: 1, flavor: 0,
            state: 8, sections_popped: 0, pop_direction: 0, pop_color: 0,
            shatter_progress: 0.05, awaking_progress: 0.0, pop_progress: 0.0,
        }];
        let col = CELL_COLUMNS + 2;
        assert_eq!(painted(&paint(&frame), col, GARBAGE_COLOR).len(), 0);
        assert!(!painted(&paint(&frame), col, GARBAGE_FLASH_COLOR).is_empty());
        // Halfway through its flip, the block is thin but there.
        frame.garbage.clear();
        frame.blocks[0].pop_countdown = frame.time_step + POP_ROTATE_TIME / 2;
        let canvas = paint(&frame);
        let shown = (BOTTOM_UNIT..TOP_UNIT).filter(|&u| canvas.get(col, u).is_some()).count();
        assert!(shown >= 2, "flip shows {} units", shown);
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
    fn the_cursor_has_the_height_and_position_of_its_blocks() {
        let mut game = GameState::new(7);
        let frame = game.step(&InputSnapshot::new());
        let canvas = paint(&frame);
        let col = frame.swapper.grid_x as usize * CELL_COLUMNS;
        let bottom = bottom_unit(frame.swapper.grid_y, 0, frame.play_offset_y);
        let painted: Vec<i32> = (BOTTOM_UNIT..TOP_UNIT)
            .filter(|&u| canvas.get(col, u) == Some(CURSOR_COLOR))
            .collect();
        // The bars frame the row: they reach into the gaps above and below it.
        assert_eq!(painted, (bottom - 1..bottom + ROW_UNITS).collect::<Vec<_>>());
        // The ticks of the brackets run over the first column of each block, top and bottom.
        let ticks: Vec<i32> = cursor_ticks(bottom).iter().flat_map(|&(l, h)| l..h).collect();
        for tick in [col + 1, col + 2 * CELL_COLUMNS - 1] {
            for &u in &ticks {
                assert_eq!(canvas.get(tick, u), Some(CURSOR_COLOR));
            }
            assert_ne!(canvas.get(tick, bottom + ROW_UNITS / 2), Some(CURSOR_COLOR));
        }
    }

    #[test]
    fn by_default_the_stack_moves_by_half_rows() {
        assert_eq!(bottom_unit(1, 0, 0.0), 0);
        assert_eq!(bottom_unit(1, 0, 0.45), 0);
        assert_eq!(bottom_unit(1, 0, 0.5), LINE_UNITS);
        assert_eq!(bottom_unit(1, 0, 0.99), LINE_UNITS);
        assert_eq!(bottom_unit(0, 0, 0.3), -ROW_UNITS);
        assert_eq!(bottom_unit(2, STEPS_PER_GRID / 2, 0.0), ROW_UNITS * 3 / 2);
    }

    #[test]
    fn the_stack_rises_by_sixteenths_of_a_row_with_smooth() {
        SMOOTH_RISE.with(|smooth| smooth.set(true));
        assert_eq!(bottom_unit(1, 0, 0.0), 0);
        assert_eq!(bottom_unit(1, 0, 0.5), ROW_UNITS / 2);
        assert_eq!(bottom_unit(0, 0, 0.25), -ROW_UNITS * 3 / 4);
        // A block halfway through its fall to row 2.
        assert_eq!(bottom_unit(2, STEPS_PER_GRID / 2, 0.0), ROW_UNITS * 3 / 2);
    }
}
