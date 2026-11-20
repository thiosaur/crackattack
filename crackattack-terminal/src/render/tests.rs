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
    for line in lowest_unit().div_euclid(LINE_UNITS)..top_unit() / LINE_UNITS {
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
    for step in 0..row_units() {
        frame.play_offset_y = step as f32 / row_units() as f32;
        let canvas = paint(&frame);
        for (bar, tick) in [(cell_columns(), cell_columns() + 1), (3 * cell_columns(), 3 * cell_columns() - 1)] {
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
    for step in 0..row_units() {
        frame.play_offset_y = step as f32 / row_units() as f32;
        let canvas = paint(&frame);
        let bottom = bottom_unit(3, 0, frame.play_offset_y);
        let top = bottom + row_units() - 1;
        for col in [cell_columns() + 1, 3 * cell_columns() - 1] {
            let mut white = 0;
            for line in lowest_unit().div_euclid(LINE_UNITS)..top_unit() / LINE_UNITS {
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
            assert!(white >= 2 * cursor_tick_units() as usize, "rise {}/16: tick lost", step);
        }
    }
}

#[test]
fn by_default_the_gap_between_stacked_blocks_always_shows() {
    let mut game = GameState::new(7);
    let mut frame = game.step(&InputSnapshot::new());
    frame.swapper.grid_y = 12;
    for step in 0..row_units() {
        frame.play_offset_y = step as f32 / row_units() as f32;
        let canvas = paint(&frame);
        for block in frame.blocks.iter().filter(|b| b.grid_y >= 1) {
            let above = frame.blocks.iter().any(|b| b.grid_x == block.grid_x && b.grid_y == block.grid_y + 1);
            if !above {
                continue;
            }
            let col = block.grid_x as usize * cell_columns() + 2;
            let gap = bottom_unit(block.grid_y, 0, frame.play_offset_y) + row_units() - 1;
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
    assert_eq!(text.matches(&purple).count(), 2 * (top_unit() / LINE_UNITS / 2) as usize);
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
        (bottom.div_euclid(LINE_UNITS)..=(bottom + row_units() - 1).div_euclid(LINE_UNITS))
            .filter_map(|line| {
                let row = (top_unit() / LINE_UNITS - 1 - line) as usize;
                lines[row].chars().nth(DANGER_GAP.len() + 1 + cell * cell_columns() + cell_columns() / 2)
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
    let panel = panel_column();
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
    let lines = (top_unit() / LINE_UNITS + lines_below_ground()) as usize;
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
    let board = 1 + DANGER_GAP.len()..1 + DANGER_GAP.len() + field_columns();
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
    assert_eq!(screen_lines(&frame, false, &Hud::default()).len(), (top_unit() / LINE_UNITS + lines_below_ground()) as usize);
    assert!(!text.contains("Swap cells"));
    // What a terminal needs to show it, below which the screen goes compact.
    assert_eq!(tall_lines(), screen_lines(&frame, false, &Hud::default()).len());
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
            .map(|line| line.chars().skip(panel_column()).collect::<String>().trim_end().to_string())
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

#[test]
fn compact_fits_the_shellcade_canvas_with_a_line_per_row() {
    COMPACT.with(|compact| compact.set(true));
    let mut game = GameState::new(7);
    let frame = game.step(&InputSnapshot::new());
    let mut screen = crate::screen::Screen::new();
    screen.feed(&compose(&frame, false, &Hud::default()));
    let lines = screen.text();
    assert_eq!(lines.len(), COMPACT_LINES);
    assert!(lines.iter().all(|l| l.chars().count() <= 80), "{:#?}", lines);
    // Every block is a ▇: the eighth above it is the gap to the next row.
    for block in frame.blocks.iter().filter(|b| b.grid_y >= 1) {
        let line = bottom_unit(block.grid_y, 0, frame.play_offset_y) / LINE_UNITS;
        let row = (COMPACT_LINES - (top_unit() / LINE_UNITS + lines_below_ground()) as usize) / 2
            + (top_unit() / LINE_UNITS - 1 - line) as usize;
        let col = DANGER_GAP.len() + 1 + block.grid_x as usize * cell_columns() + 1;
        let shown: String = lines[row].chars().skip(col).take(block_columns()).collect();
        assert!(shown == "▇▇" || shown.contains(['[', ']']), "block ({}, {}): {:?}", block.grid_x, block.grid_y, shown);
    }
    // The cursor brackets are glyphs, on the line of its row.
    assert!(lines.iter().any(|l| l.contains('[') && l.contains(']')));
}

// ---- Placement: the zone, its two heights, the floor ----

#[test]
fn the_zone_is_80_wide_and_24_or_28_tall_and_a_terminal_under_80x24_gets_a_message() {
    let tall = tall_lines();
    // Exactly the floor: compact, no margin.
    assert_eq!(place(Some((80, 24)), false), Placement::Zone { origin: (0, 0), tall: false });
    // Tall enough for the normal screen.
    assert_eq!(place(Some((80, tall)), false), Placement::Zone { origin: (0, 0), tall: true });
    // Between the two heights: compact, centered vertically.
    assert_eq!(place(Some((80, 26)), false), Placement::Zone { origin: (0, 1), tall: false });
    // --compact wins over a tall terminal; the zone is centered in both directions.
    assert_eq!(place(Some((100, 40)), true), Placement::Zone { origin: (10, 8), tall: false });
    assert_eq!(place(Some((100, 40)), false), Placement::Zone { origin: (10, (40 - tall) / 2), tall: true });
    // Narrow but tall: never the normal screen cut on the right, a message.
    assert_eq!(place(Some((79, 40)), false), Placement::TooSmall { columns: 79, rows: 40 });
    assert_eq!(place(Some((120, 23)), false), Placement::TooSmall { columns: 120, rows: 23 });
    // Unknown size (no terminal): 80x24 with no centering.
    assert_eq!(place(None, false), Placement::Zone { origin: (0, 0), tall: false });
}

#[test]
fn under_80x24_a_message_replaces_the_screen_and_a_new_size_redraws_everything() {
    let frame = FrameData::default();
    let mut renderer = Renderer::new();
    renderer.set_placement(Placement::TooSmall { columns: 62, rows: 20 });
    assert!(renderer.too_small());
    let text = renderer.frame(&frame, &Hud::default());
    assert!(text.contains("Terminal too small") && text.contains("80x24 needed") && text.contains("now 62x20"));
    // The pause screen and the menu give the same message.
    assert!(renderer.paused(&frame, &Hud::default()).is_empty() || renderer.paused(&frame, &Hud::default()).contains("too small"));
    assert!(renderer.menu(&MENU_TOP, 0, None).is_empty() || renderer.menu(&MENU_TOP, 0, None).contains("too small"));
    // Growing back: the game comes back, drawn whole (the screen is cleared first).
    renderer.set_placement(Placement::Zone { origin: (0, 0), tall: false });
    assert!(!renderer.too_small());
    let game = renderer.frame(&frame, &Hud::default());
    assert!(game.contains(CLEAR) && game.contains("Score") && !game.contains("too small"));
    // A change of height alone (same width) moves the zone: everything is drawn again.
    renderer.set_placement(Placement::Zone { origin: (0, 3), tall: false });
    let moved = renderer.frame(&frame, &Hud::default());
    assert!(moved.contains(CLEAR) && moved.contains("\x1b[4;1H"), "not redrawn at the new row");
}

#[test]
fn the_menu_is_centered_in_the_same_zone_as_the_games_and_fits_both_heights() {
    for (tall, rows) in [(false, 24), (true, tall_lines())] {
        let mut renderer = Renderer::new();
        renderer.set_placement(Placement::Zone { origin: (7, 2), tall });
        let text = renderer.menu(&MENU_COMPUTER, 0, None);
        assert!(text.contains("> Logic (easy)") && text.contains("Vs computer"), "menu missing");
        // Written inside the zone: from row 3 on, from column 8 on, never above or left of it.
        let starts: Vec<(usize, usize)> = text.split("\x1b[").skip(1).filter_map(|part| {
            let (pos, _) = part.split_once('H')?;
            let (r, c) = pos.split_once(';')?;
            Some((r.parse().ok()?, c.parse().ok()?))
        }).collect();
        assert!(!starts.is_empty());
        assert!(starts.iter().all(|&(r, c)| c == 8 && (3..3 + rows).contains(&r)), "{:?}", starts);
    }
}

// ---- Banners: big words and text over the board ----

fn banner_screen(banner: Banner, compact: bool) -> (Vec<String>, String) {
    COMPACT.with(|c| c.set(compact));
    let mut game = GameState::new(7);
    let frame = game.step(&InputSnapshot::new());
    let hud = Hud { banner: Some(banner), ..Hud::default() };
    let lines = screen_lines(&frame, false, &hud);
    let normal = screen_lines(&frame, false, &Hud::default()).len();
    assert_eq!(lines.len(), normal, "a banner changes the height of the screen");
    let text = lines.join("\n");
    COMPACT.with(|c| c.set(false));
    (lines, text)
}

fn ready(round: i32) -> Banner {
    Banner {
        above: vec!["Round".to_string()],
        words: vec![round.to_string()],
        color: 220,
        below: vec!["You 1 - 0 Sigma".to_string(), "Enter: play again".to_string(), "Q: menu".to_string()],
    }
}

#[test]
fn the_round_prompt_is_a_big_number_with_text_around_it_in_both_sizes() {
    for compact in [false, true] {
        for round in [1, 2, 12] {
            let (lines, text) = banner_screen(ready(round), compact);
            let mut screen = crate::screen::Screen::new();
            screen.feed(&output::Output::new().update(lines));
            let shown = screen.text().join("\n");
            for expected in ["Round", "You 1 - 0 Sigma", "Enter: play again", "Q: menu"] {
                assert!(shown.contains(expected), "compact={} round={} lacks {:?}\n{}", compact, round, expected, shown);
            }
            // The number is painted in its color, and the board is gone (no block of any other color).
            assert!(text.contains("38;5;220"), "no big number");
            assert!(!text.contains(&format!("38;5;{}m", COLORS[2])), "the board shows through");
        }
    }
}

#[test]
fn a_round_result_is_two_big_words_that_fit_even_the_compact_field() {
    for compact in [false, true] {
        for (words, color) in [(["YOU", "WIN"], 46), (["YOU", "LOSE"], 196)] {
            let banner = Banner { words: words.iter().map(|w| w.to_string()).collect(), color, ..Banner::default() };
            let (_, text) = banner_screen(banner, compact);
            assert!(text.contains(&format!("38;5;{}", color)), "{:?} compact={} not drawn", words, compact);
        }
    }
}

#[test]
fn a_text_only_banner_still_replaces_the_board_line_by_line() {
    let banner = Banner { above: vec!["Connection lost".to_string(), "Q: menu".to_string()], ..Banner::default() };
    let (_, text) = banner_screen(banner, false);
    assert!(text.contains("Connection lost") && text.contains("Q: menu"));
    assert!(!text.contains(&format!("38;5;{}m", COLORS[2])));
}

#[test]
fn a_long_round_number_keeps_its_height_and_never_gets_cut() {
    let shown = |round: i32, compact: bool| {
        let (lines, _) = banner_screen(ready(round), compact);
        let mut screen = crate::screen::Screen::new();
        screen.feed(&output::Output::new().update(lines));
        screen.text().join("\n")
    };
    // 99 and 999 fit as big blocks in both fields; the number is not written as text.
    for compact in [false, true] {
        for round in [9, 10, 99, 100, 999] {
            assert!(!shown(round, compact).contains(&round.to_string()), "round {} compact={}", round, compact);
        }
    }
    // 1000 does not fit the compact field: written as text, whole, rather than cut.
    assert!(shown(1000, true).contains("1000"));
    assert!(!shown(1000, false).contains("1000"));
    // Two digits are as tall as one in the normal field (the pixels get narrower, not shorter).
    let height = |round: i32| shown(round, false).lines().filter(|l| l.contains(['█', '▄', '▂', '▆'])).count();
    assert!(height(10) + 2 >= height(9), "round 10 is {} lines tall, round 9 {}", height(10), height(9));
}

fn lobby_view(others: usize) -> LobbyView {
    LobbyView {
        name: "Pixel".to_string(),
        others: (0..others).map(|i| (format!("Rival{}", i), if i == 0 { "playing" } else { "menu" })).collect(),
        notice: Some("Zed joined".to_string()),
        invite: None,
    }
}

#[test]
fn the_lobby_takes_the_logos_place_in_the_panel_and_gives_it_back_at_game_over() {
    let frame = FrameData::default();
    for compact in [false, true] {
        COMPACT.with(|c| c.set(compact));
        let lobby = Some(lobby_view(3));
        let solo = compose(&frame, false, &Hud { lobby: lobby.clone(), ..Hud::default() });
        assert!(solo.contains("Online: ") && solo.contains("Pixel") && solo.contains("Rival0") && solo.contains("Zed joined"), "compact={}", compact);
        // Without a lobby, nothing of it, and the logo is there (its first row is blocks).
        assert!(!compose(&frame, false, &Hud::default()).contains("Online"));
        // The game is lost: the logo comes back to turn into GAME OVER.
        let lost = compose(&frame, false, &Hud { lobby, game_over: Some(0), ..Hud::default() });
        assert!(!lost.contains("Online"), "compact={}", compact);
        // A duel keeps its preview; the notice goes under it, the list is not shown.
        let duel = Hud { lobby: Some(lobby_view(3)), versus: Some(Versus::default()), ..Hud::default() };
        let text = compose(&frame, false, &duel);
        assert!(text.contains("Zed joined") && !text.contains("Rival0"), "compact={}", compact);
    }
    COMPACT.with(|c| c.set(false));
}

#[test]
fn the_menu_ends_with_the_number_of_players_and_fits_both_heights() {
    for rows in [24, tall_lines()] {
        let lines = menu::lines(&menu::TOP, 0, 80, rows, Some(&lobby_view(8)));
        assert_eq!(lines.len(), rows, "the count is the last row of the zone");
        assert!(lines.last().unwrap().contains("9 players online"), "{:?}", lines.last());
        // The pseudo greets the player just above.
        assert!(lines[rows - 2].contains("Welcome") && lines[rows - 2].contains("Pixel"), "{:?}", lines[rows - 2]);
        let text = lines.join("\n");
        assert!(text.contains("> Solo") && text.contains("Quit") && !text.contains("Rival0"));
        assert!(lines.iter().all(|l| visible_width(l) <= 80));
        // Without a lobby, no count.
        assert!(!menu::lines(&menu::TOP, 0, 80, rows, None).join("\n").contains("online"));
    }
}

#[test]
fn the_logo_and_the_items_stay_where_they_are_whatever_the_page_or_the_lobby() {
    // Rows of the logo (blocks), of the first item and of the keys.
    let places = |page: &menu::Page, lobby: Option<&LobbyView>, rows: usize| {
        let lines = menu::lines(page, 0, 80, rows, lobby);
        let logo: Vec<usize> = (0..lines.len()).filter(|&i| lines[i].contains('█')).collect();
        let hint = lines.iter().position(|l| l.contains("Arrows: choose")).unwrap();
        let first = lines.iter().position(|l| l.contains("> ")).unwrap();
        (logo, first, hint)
    };
    for rows in [24, tall_lines()] {
        let reference = places(&menu::TOP, None, rows);
        assert!(!reference.0.is_empty(), "the logo is there at {} rows", rows);
        for page in [&menu::TOP, &menu::COMPUTER] {
            for lobby in [None, Some(lobby_view(0)), Some(lobby_view(2)), Some(lobby_view(9))] {
                let got = places(page, lobby.as_ref(), rows);
                assert_eq!((&got.0, got.2), (&reference.0, reference.2), "moved at {} rows", rows);
            }
        }
        // The first item is on the same row on every page.
        assert_eq!(places(&menu::TOP, None, rows).1, places(&menu::COMPUTER, None, rows).1);
    }
}

#[test]
fn the_held_result_shows_its_way_out_in_both_sizes() {
    let frame = FrameData::default();
    for compact in [false, true] {
        COMPACT.with(|c| c.set(compact));
        for words in [["YOU", "WIN"], ["YOU", "LOSE"]] {
            let banner = Banner {
                words: words.iter().map(|w| w.to_string()).collect(),
                color: 46,
                below: vec![String::new(), "Enter: continue".to_string()],
                ..Banner::default()
            };
            let text = compose(&frame, false, &Hud { banner: Some(banner), ..Hud::default() });
            assert!(text.contains("Enter: continue"), "compact={} {:?}", compact, words);
            // The big words are there too (blocks in the color of the result).
            assert!(text.contains(&format!("38;5;{}m", 46)), "compact={}", compact);
        }
    }
    COMPACT.with(|c| c.set(false));
}

fn names(n: usize) -> Vec<String> {
    (0..n).map(|i| format!("Player{}", i)).collect()
}

#[test]
fn a_long_list_scrolls_by_itself_and_always_shows_the_choice() {
    use super::menu::{list_lines, window_start};
    // Short lists do not scroll; long ones keep the choice on the second row, except at the ends.
    assert!((0..3).all(|s| window_start(3, s) == 0));
    assert_eq!([0, 1, 2, 4, 7, 8].map(|s| window_start(9, s)), [0, 0, 1, 3, 5, 5]);
    let items = names(9);
    for rows in [24, tall_lines()] {
        for selected in 0..9 {
            let lines = list_lines("Vs player", &items, selected, "hint", 80, rows, None);
            assert!(lines.len() <= rows - 2, "{} lines at {} rows: the last two are the lobby's", lines.len(), rows);
            let text = lines.join("\n");
            assert!(text.contains(&format!("> Player{}", selected)), "the choice is not shown: {}", selected);
            assert!(text.contains(&format!("{}/9", selected + 1)), "no position at {}", selected);
            let shown = items.iter().filter(|i| text.contains(i.as_str())).count();
            assert_eq!(shown, 4, "four items at a time");
            // The arrows say where the list goes on.
            assert_eq!(text.contains('\u{25b2}'), selected > 1, "up arrow at {}", selected);
            // The last items are shown from the choice on the sixth (index 5) at the latest.
            assert_eq!(text.contains('\u{25bc}'), selected < 6, "down arrow at {}", selected);
        }
    }
    // Nothing to scroll: no arrows, no position.
    let text = list_lines("Vs player", &names(3), 1, "hint", 80, 24, None).join("\n");
    assert!(!text.contains('\u{25b2}') && !text.contains('\u{25bc}') && !text.contains("/3"));
}

#[test]
fn the_list_page_keeps_the_logo_and_the_keys_where_the_other_pages_have_them() {
    let rows_of = |lines: &Vec<String>| -> (Vec<usize>, usize) {
        ((0..lines.len()).filter(|&i| lines[i].contains('█')).collect(), lines.iter().position(|l| l.contains("hint") || l.contains("Arrows: choose")).unwrap())
    };
    for rows in [24, tall_lines()] {
        let page = rows_of(&menu::lines(&menu::COMPUTER, 0, 80, rows, None));
        let list = rows_of(&menu::list_lines("Vs player", &names(9), 5, "hint", 80, rows, None));
        assert_eq!(page.0, list.0, "logo moved at {} rows", rows);
        assert_eq!(page.1, list.1, "keys moved at {} rows", rows);
    }
}

#[test]
fn the_invitation_replaces_the_greeting_on_the_players_page_only() {
    let mut lobby = lobby_view(2);
    lobby.invite = Some("ssh -p 2222 invite-abcdefghij@example.org".to_string());
    for rows in [24, tall_lines()] {
        let page = menu::list_lines("Vs player", &["Back".to_string()], 0, "hint", 80, rows, Some(&lobby)).join("\n");
        assert!(page.contains("ssh -p 2222 invite-abcdefghij@example.org") && page.contains("To invite a friend"), "{}", page);
        assert!(!page.contains("Welcome"), "{}", page);
        // The other pages keep the greeting, whatever the lobby holds.
        let menu_page = menu::lines(&menu::TOP_ONLINE, 0, 80, rows, Some(&lobby)).join("\n");
        assert!(menu_page.contains("Welcome") && !menu_page.contains("invite"), "{}", menu_page);
        // No command: the greeting, as before.
        let plain = menu::list_lines("Vs player", &["Back".to_string()], 0, "hint", 80, rows, Some(&lobby_view(2))).join("\n");
        assert!(plain.contains("Welcome"));
    }
}
