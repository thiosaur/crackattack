// Tests across the render modules: painting then turning into characters, and the
// screen layout.

use super::board::*;
use super::palette::*;
use super::panel::*;
use super::raster::*;
use super::*;
use crackattack_core::{BlockState, GameState, InputSnapshot};

/// A frame with a single block, on row 3 of column 1, and no garbage.
pub(super) fn lone_block_frame(state: u8) -> FrameData {
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

/// Units of `col` shown in the cursor color once each character has picked its two
/// colors: what the terminal displays.
fn shown_white(canvas: &Canvas, col: usize) -> Vec<i32> {
    let mut units = Vec::new();
    for line in BOTTOM_UNIT.div_euclid(LINE_UNITS)..TOP_UNIT / LINE_UNITS {
        let cells: [Option<u8>; 8] = std::array::from_fn(|i| canvas.get(col, line * LINE_UNITS + i as i32));
        let (lower, upper, k) = split_line(&cells);
        for i in 0..8 {
            if (if i < k { lower } else { upper }) == Some(CURSOR_COLOR) {
                units.push(line * LINE_UNITS + i as i32);
            }
        }
    }
    units
}

#[test]
fn the_brackets_are_square_their_ticks_flush_with_the_bars() {
    let mut game = GameState::new(7);
    let mut frame = game.step(&InputSnapshot::new());
    frame.swapper.grid_x = 1;
    frame.swapper.grid_y = 3;
    for step in 0..ROW_UNITS {
        frame.play_offset_y = step as f32 / ROW_UNITS as f32;
        let canvas = paint(&frame);
        for (bar, tick) in [(CELL_COLUMNS, CELL_COLUMNS + 1), (3 * CELL_COLUMNS, 3 * CELL_COLUMNS - 1)] {
            let (bar, tick) = (shown_white(&canvas, bar), shown_white(&canvas, tick));
            assert!(!tick.is_empty(), "rise {}/16: no tick", step);
            assert_eq!(
                (tick.first(), tick.last()), (bar.first(), bar.last()),
                "rise {}/16: ticks {:?} not flush with the bar {:?}..{:?}", step, tick, bar.first(), bar.last()
            );
            // The bar is one piece.
            assert_eq!(bar.len() as i32, bar.last().unwrap() - bar.first().unwrap() + 1);
        }
    }
}

#[test]
fn the_cursor_ticks_show_and_never_spill_whatever_the_rise() {
    let mut game = GameState::new(7);
    let mut frame = game.step(&InputSnapshot::new());
    frame.swapper.grid_x = 1;
    frame.swapper.grid_y = 3;
    for step in 0..ROW_UNITS {
        frame.play_offset_y = step as f32 / ROW_UNITS as f32;
        let canvas = paint(&frame);
        let bottom = bottom_unit(3, 0, frame.play_offset_y);
        let top = bottom + ROW_UNITS - 1;
        for col in [CELL_COLUMNS + 1, 3 * CELL_COLUMNS - 1] {
            let mut white = 0;
            for line in BOTTOM_UNIT.div_euclid(LINE_UNITS)..TOP_UNIT / LINE_UNITS {
                let units: [Option<u8>; 8] =
                    std::array::from_fn(|i| canvas.get(col, line * LINE_UNITS + i as i32));
                let (lower, upper, k) = split_line(&units);
                for (i, original) in units.iter().enumerate() {
                    let unit = line * LINE_UNITS + i as i32;
                    let shown = if i < k { lower } else { upper };
                    if shown == Some(CURSOR_COLOR) {
                        white += 1;
                        assert!(
                            *original == shown || (bottom - 1..=top).contains(&unit),
                            "rise {}/16: white spills on unit {} ({:?})", step, unit, original
                        );
                    }
                }
            }
            assert!(white >= 2 * CURSOR_TICK_UNITS as usize, "rise {}/16: tick lost", step);
        }
    }
}

#[test]
fn by_default_the_gap_between_stacked_blocks_always_shows() {
    let mut game = GameState::new(7);
    let mut frame = game.step(&InputSnapshot::new());
    frame.swapper.grid_y = 12;
    for step in 0..ROW_UNITS {
        frame.play_offset_y = step as f32 / ROW_UNITS as f32;
        let canvas = paint(&frame);
        for block in frame.blocks.iter().filter(|b| b.grid_y >= 1) {
            let above = frame.blocks.iter().any(|b| b.grid_x == block.grid_x && b.grid_y == block.grid_y + 1);
            if !above {
                continue;
            }
            let col = block.grid_x as usize * CELL_COLUMNS + 2;
            let gap = bottom_unit(block.grid_y, 0, frame.play_offset_y) + ROW_UNITS - 1;
            let line = gap.div_euclid(LINE_UNITS);
            let units: [Option<u8>; 8] = std::array::from_fn(|i| canvas.get(col, line * LINE_UNITS + i as i32));
            let (lower, upper, k) = split_line(&units);
            let i = (gap - line * LINE_UNITS) as usize;
            let shown = if i < k { lower } else { upper };
            assert_eq!(shown, None, "rise {}/16: gap above ({}, {}) lost", step, block.grid_x, block.grid_y);
        }
    }
}

#[test]
fn danger_bars_frame_the_board_on_both_sides() {
    let mut game = GameState::new(7);
    let frame = game.step(&InputSnapshot::new());
    let half = DangerBars { height: 0.5, red: false, alpha: 1.0 };
    let text = compose(&frame, false, &Hud { danger: half, chain: None, ..Hud::default() });
    let purple = format!("\x1b[38;5;{}m█", DANGER_PURPLE[0]);
    // Half the board lines on each side.
    assert_eq!(text.matches(&purple).count(), 2 * (TOP_UNIT / LINE_UNITS / 2) as usize);
    let red = DangerBars { red: true, ..half };
    assert!(compose(&frame, false, &Hud { danger: red, chain: None, ..Hud::default() }).contains(&format!("\x1b[38;5;{}m█", DANGER_RED[0])));
    assert!(!compose(&frame, true, &Hud { danger: half, chain: None, ..Hud::default() }).contains(&purple));
}

#[test]
fn no_dots_in_the_cells_of_a_swap() {
    let mut frame = lone_block_frame((BlockState::SWAPPING | BlockState::SWAP_DIRECTION).bits()); // going right, into an empty cell
    frame.swapper.swap_factor = 0.5;
    let text = compose(&frame, false, &Hud::default());
    let mut screen = crate::screen::Screen::new();
    screen.feed(&text);
    let bottom = bottom_unit(3, 0, frame.play_offset_y);
    let lines = screen.text();
    // What the row shows at the center column of `cell`, over its two lines.
    let centers = |cell: usize| -> Vec<char> {
        (bottom.div_euclid(LINE_UNITS)..=(bottom + ROW_UNITS - 1).div_euclid(LINE_UNITS))
            .filter_map(|line| {
                let row = (TOP_UNIT / LINE_UNITS - 1 - line) as usize;
                lines[row].chars().nth(DANGER_GAP.len() + 1 + cell * CELL_COLUMNS + CELL_COLUMNS / 2)
            })
            .collect()
    };
    for cell in [1, 2] {
        assert!(!centers(cell).iter().any(|c| DOTS.contains(c)), "dot in cell {}", cell);
    }
    // Other cells of the row keep theirs.
    assert!(centers(4).iter().any(|c| DOTS.contains(c)));
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
    for key in &KEYS {
        assert!(text.contains(&format!("{:<7}{}", key.hint_key, key.hint_action)));
    }
    assert!(!compose(&frame, false, &Hud::default()).contains("Chain"));
}

#[test]
fn the_logo_shows_block_for_block_right_of_the_board() {
    let mut game = GameState::new(7);
    let frame = game.step(&InputSnapshot::new());
    let mut screen = crate::screen::Screen::new();
    screen.feed(&compose(&frame, false, &Hud::default()));
    // The panel starts after the board, the right danger bar and its gap.
    let panel = PANEL_COLUMN;
    let mut word = 0;
    for (y, row) in LOGO.lines().enumerate() {
        if row.trim().is_empty() {
            word += 1;
        }
        for (x, c) in row.chars().enumerate() {
            let cell = screen.cell(LOGO_FIRST_LINE + y, panel + x);
            if c == '#' {
                assert_eq!((cell.ch, cell.ink), ('█', Some(LOGO_COLORS[word])), "({}, {})", x, y);
            } else {
                assert_eq!(cell.ch, ' ', "({}, {})", x, y);
            }
        }
    }
    // Between the chain popup (line 6) and the key hints.
    let lines = (TOP_UNIT / LINE_UNITS + LINES_BELOW_GROUND) as usize;
    assert!(LOGO_FIRST_LINE > 6 && LOGO_FIRST_LINE + LOGO.lines().count() <= lines - KEYS.len());
}

#[test]
fn the_pause_hides_the_board() {
    let mut game = GameState::new(7);
    let frame = game.step(&InputSnapshot::new());
    let text = compose(&frame, true, &Hud::default());
    assert!(text.contains("PAUSE"));
    assert!(text.contains("Q: quit"));
    // No block left on the board (the logo in the side panel stays).
    let mut screen = crate::screen::Screen::new();
    screen.feed(&text);
    let board = 1 + DANGER_GAP.len()..1 + DANGER_GAP.len() + FIELD_COLUMNS;
    for line in screen.text() {
        let field: String = line.chars().skip(board.start).take(board.len()).collect();
        assert!(!field.contains(['█', '▇', '▅', '▂']), "{:?}", line);
    }
    assert_eq!(screen_lines(&frame, true, &Hud::default()).len(), screen_lines(&frame, false, &Hud::default()).len());
}

#[test]
fn a_frame_has_one_line_per_two_row_halves_and_a_ground() {
    let mut game = GameState::new(7);
    let frame = game.step(&InputSnapshot::new());
    let text = compose(&frame, false, &Hud::default());
    // Playfield lines only (no debug line by default).
    assert_eq!(screen_lines(&frame, false, &Hud::default()).len(), (TOP_UNIT / LINE_UNITS + LINES_BELOW_GROUND) as usize);
    assert!(!text.contains("Swap cells"));
    assert_eq!(text.matches(GROUND).count(), 1);
}

#[test]
fn at_game_over_the_logo_turns_into_game_over_then_asks_for_a_key() {
    let mut game = GameState::new(7);
    let frame = game.step(&InputSnapshot::new());
    // Same layout: the words on the same lines, so each line keeps its color.
    let blank = |text: &str| text.lines().position(|l| l.trim().is_empty());
    assert_eq!(blank(LOGO), blank(GAME_OVER));
    assert_eq!(LOGO.lines().count(), GAME_OVER.lines().count());
    let panel = |t: Option<i32>| {
        let mut screen = crate::screen::Screen::new();
        screen.feed(&compose(&frame, false, &Hud { game_over: t, ..Hud::default() }));
        let rows = screen.text();
        let art: Vec<String> = rows[LOGO_FIRST_LINE..LOGO_FIRST_LINE + LOGO.lines().count()]
            .iter()
            .map(|line| line.chars().skip(PANEL_COLUMN).collect::<String>().trim_end().to_string())
            .collect();
        (art, rows.join("\n"))
    };
    let as_blocks = |text: &str| -> Vec<String> {
        text.lines().map(|l| l.replace('#', "█").trim_end().to_string()).collect()
    };
    let (start, _) = panel(Some(0));
    assert_eq!(start, panel(None).0);
    assert_eq!(start, as_blocks(LOGO));
    // Midway, the left columns show GAME OVER, the right ones the logo, and one flips.
    let (middle, text) = panel(Some(game_over_ticks() / 2));
    assert!(middle.iter().any(|l| l.contains(['▂', '▄', '▆'])), "{:?}", middle);
    assert!(!text.contains("Press any key"));
    let (end, text) = panel(Some(game_over_ticks()));
    assert_eq!(end, as_blocks(GAME_OVER));
    assert!(text.contains("Press any key") && !text.contains("quit"));
}
