// Crack Attack! Terminal - Standalone Rust binary

mod danger;
mod render;
mod screen;

use crackattack_core::ai_difficulty::AIDifficulty;
use crackattack_core::vs::GridAutoplay;
use crackattack_core::{FrameData, GameState, InputSnapshot};
use crackattack_core::input::*;
use std::io::{self, Write};
use std::time::{Duration, Instant};

use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind};
use crossterm::terminal;

/// Core game ticks per second (one step per frame).
pub(crate) const TICKS_PER_SECOND: i32 = 50;
/// How long the chain just reached stays in the side panel (ticks).
const CHAIN_SHOW_TICKS: i32 = 75;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let options = match parse_args(&args) {
        Ok(Some(options)) => options,
        Ok(None) => {
            print_help();
            return;
        }
        Err(e) => {
            eprintln!("crackattack-terminal: {}", e);
            eprintln!("Try --help.");
            std::process::exit(2);
        }
    };
    render::SHOW_DEBUG.store(options.debug, std::sync::atomic::Ordering::Relaxed);
    render::SMOOTH_RISE.with(|smooth| smooth.set(options.smooth));

    let seed = options.seed.unwrap_or_else(random_seed);
    let mut state = GameState::new(seed);
    let autoplay = options.autoplay.map(|level| GridAutoplay::new(level, seed));
    let mut driver = Driver::new(autoplay, options.captures);

    let (frame, ending) = if options.headless {
        run_headless(&mut state, &mut driver, options.max_frames)
    } else {
        play(&mut state, &mut driver, options.max_frames)
    };
    // The terminal is restored by now: the summary stays in the shell after the game.
    println!("{}", summary(&frame, ending, seed));
    if let Some(written) = driver.finish(&frame) {
        match written {
            Ok((count, path)) => println!("  {} capture(s) written to {}", count, path),
            Err(e) => eprintln!("  Captures not written: {}", e),
        }
    }
}

/// Command-line options.
#[derive(Debug, Default)]
struct Options {
    seed: Option<u32>,
    max_frames: Option<i32>,
    debug: bool,
    smooth: bool,
    autoplay: Option<AIDifficulty>,
    headless: bool,
    captures: Captures,
}

/// The options, `None` for `--help`, or what is wrong with them.
fn parse_args(args: &[String]) -> Result<Option<Options>, String> {
    let mut options = Options::default();
    let mut args = args.iter().map(String::as_str).peekable();
    while let Some(arg) = args.next() {
        let mut value = |what: &str| args.next().ok_or_else(|| format!("{} expects {}", arg, what));
        match arg {
            "--seed" => {
                let n = value("a number")?;
                options.seed = Some(n.parse().map_err(|_| format!("--seed: '{}' is not a number", n))?);
            }
            "--frames" => {
                let n = value("a number of frames")?;
                options.max_frames = Some(
                    n.parse().ok().filter(|&n: &i32| n > 0)
                        .ok_or_else(|| format!("--frames: '{}' is not a positive number", n))?,
                );
            }
            "--debug" => options.debug = true,
            "--smooth" => options.smooth = true,
            "--autoplay" => {
                // Optional level: easy, medium or hard (the default, as in the GUI).
                let level = args.peek().and_then(|a| parse_level(a));
                if level.is_some() {
                    args.next();
                }
                options.autoplay = Some(level.unwrap_or(AIDifficulty::Hard));
            }
            "--headless" => options.headless = true,
            "--capture" => {
                let list = value("ticks such as 100,250,end")?;
                let (ticks, at_end) = parse_capture_ticks(list)
                    .ok_or_else(|| format!("--capture: '{}' is not a list such as 100,250,end", list))?;
                options.captures.ticks = ticks;
                options.captures.at_end = at_end;
            }
            "--capture-file" => options.captures.path = value("a path")?.to_string(),
            "--help" | "-h" => return Ok(None),
            _ => return Err(format!("unknown option '{}'", arg)),
        }
    }
    Ok(Some(options))
}

fn print_help() {
    println!("Crack Attack! Terminal (Rust)");
    println!();
    println!("Usage: crackattack-terminal [OPTIONS]");
    println!();
    println!("Options:");
    println!("  --seed N      Use a specific seed (default: a new one each game)");
    println!("  --frames N    Stop after N frames");
    println!("  --debug       Show the state of the cells under the cursor");
    println!("  --smooth      Move the stack by sixteenths of a row instead of halves");
    println!("                (smoother, but the gaps between blocks flicker)");
    println!("  --autoplay [LEVEL]  The AI plays (easy, medium or hard; default hard)");
    println!("  --headless    No display: the game runs at full speed, for tests");
    println!("                (with --autoplay; stops at game over, --frames or the");
    println!("                last capture)");
    println!("  --capture T1,T2,...  Text captures of the screen at these ticks;");
    println!("                'end' for the last frame");
    println!("  --capture-file PATH  Where captures go (default: {})", DEFAULT_CAPTURE_FILE);
    println!("  --help        Show this help");
    println!();
    println!("Controls:");
    for key in &render::KEYS {
        println!("  {:<19}{}", key.keys, key.action);
    }
}

const DEFAULT_CAPTURE_FILE: &str = "crackattack-captures.txt";

fn parse_level(arg: &str) -> Option<AIDifficulty> {
    match arg {
        "easy" => Some(AIDifficulty::Easy),
        "medium" => Some(AIDifficulty::Medium),
        "hard" => Some(AIDifficulty::Hard),
        _ => None,
    }
}

/// `100,250,end` → ([100, 250], true).
fn parse_capture_ticks(list: &str) -> Option<(Vec<i32>, bool)> {
    let mut ticks = Vec::new();
    let mut at_end = false;
    for item in list.split(',').map(str::trim).filter(|i| !i.is_empty()) {
        if item == "end" {
            at_end = true;
        } else {
            ticks.push(item.parse().ok().filter(|&t: &i32| t > 0)?);
        }
    }
    ticks.sort_unstable();
    ticks.dedup();
    (!ticks.is_empty() || at_end).then_some((ticks, at_end))
}

/// Text captures requested on the command line.
#[derive(Clone, Debug)]
struct Captures {
    /// Ticks to capture, sorted.
    ticks: Vec<i32>,
    /// Also capture the last frame.
    at_end: bool,
    path: String,
}

impl Default for Captures {
    fn default() -> Self {
        Self { ticks: Vec::new(), at_end: false, path: DEFAULT_CAPTURE_FILE.to_string() }
    }
}

impl Captures {
    fn wanted(&self) -> bool {
        !self.ticks.is_empty() || self.at_end
    }
}

/// What runs each tick, with or without a display: the AI (if any), edge detection of
/// the keys, the side panel's state, and the screen captures.
struct Driver {
    ai: Option<GridAutoplay>,
    prev_state: u8,
    danger: danger::DangerAnimator,
    /// Chain just reached, and ticks left to show it.
    chain: Option<(i32, i32)>,
    ticks: i32,
    captures: Captures,
    /// Decodes every frame, so a capture shows exactly what the terminal holds.
    screen: Option<screen::Screen>,
    taken: String,
    taken_count: usize,
    hud: render::Hud,
    renderer: render::Renderer,
}

impl Driver {
    fn new(ai: Option<GridAutoplay>, captures: Captures) -> Self {
        let screen = captures.wanted().then(screen::Screen::new);
        Self {
            ai,
            prev_state: 0,
            danger: danger::DangerAnimator::new(),
            chain: None,
            ticks: 0,
            captures,
            screen,
            taken: String::new(),
            taken_count: 0,
            hud: render::Hud::default(),
            renderer: render::Renderer::new(),
        }
    }

    /// One game tick from the keys held (`keys`, `INPUT_*` bits); the AI adds its own.
    fn step(&mut self, state: &mut GameState, keys: u8) -> FrameData {
        let mut input = InputSnapshot::new();
        input.state = keys;
        if let Some(ai) = &mut self.ai {
            input.state |= ai.update(state);
        }
        // Edge detection: compute pressed_this_frame from state transitions.
        input.pressed_this_frame = input.state & !self.prev_state;
        input.prev_state = self.prev_state;
        self.prev_state = input.state;

        let frame = state.step(&input);
        self.ticks += 1;
        self.danger.step(frame.ui.creep_freeze, frame.ui.loss_countdown);
        self.chain = self.chain.map(|(n, left)| (n, left - 1)).filter(|&(_, left)| left > 0);
        for event in &frame.gameplay_events {
            if let crackattack_core::gameplay_event::GameplayEvent::ChainProgressed(e) = event {
                self.chain = Some((e.chain_level, CHAIN_SHOW_TICKS));
            }
        }
        self.hud = render::Hud { danger: self.danger.bars(), chain: self.chain.map(|(n, _)| n), game_over: None };
        frame
    }

    /// The text that brings the terminal to `frame` (or to the pause screen): only what
    /// changed. Captures decode it as the terminal would.
    fn draw(&mut self, frame: &FrameData, paused: bool) -> String {
        let text = if paused { self.renderer.paused(frame) } else { self.renderer.frame(frame, &self.hud) };
        if let Some(screen) = &mut self.screen {
            screen.feed(&text);
            if !paused && self.captures.ticks.binary_search(&self.ticks).is_ok() {
                self.take(&format!("tick {}", self.ticks));
            }
        }
        text
    }

    /// One tick of the game-over animation: the game itself has stopped.
    fn step_game_over(&mut self) {
        self.ticks += 1;
        self.hud.game_over = Some(self.hud.game_over.map_or(0, |t| t + 1));
    }

    /// GAME OVER is fully shown.
    fn game_over_shown(&self) -> bool {
        self.hud.game_over.is_some_and(|t| t >= render::game_over_ticks())
    }

    /// The keys pressed during the pause must not count as held after it.
    fn release_keys(&mut self) {
        self.prev_state = 0;
    }

    /// Every capture tick is behind: nothing left to wait for.
    fn captures_done(&self) -> bool {
        self.captures.wanted() && !self.captures.at_end
            && self.captures.ticks.last().is_some_and(|&t| self.ticks >= t)
    }

    fn take(&mut self, title: &str) {
        if let Some(screen) = &self.screen {
            self.taken.push_str(&screen.capture(title));
            self.taken.push('\n');
            self.taken_count += 1;
        }
    }

    /// Takes the end capture if asked and writes the file; `None` without captures.
    fn finish(&mut self, last: &FrameData) -> Option<Result<(usize, String), io::Error>> {
        if !self.captures.wanted() {
            return None;
        }
        if self.captures.at_end && self.ticks > 0 {
            let title = format!("end, tick {}{}", self.ticks, if last.game_over { ", game over" } else { "" });
            self.take(&title);
        }
        let content = format!("{}{}", screen::legend(), self.taken);
        Some(std::fs::write(&self.captures.path, content).map(|_| (self.taken_count, self.captures.path.clone())))
    }
}

/// Runs the game without a display, as fast as it goes: for autoplay tests and captures.
fn run_headless(state: &mut GameState, driver: &mut Driver, max_frames: Option<i32>) -> (FrameData, Ending) {
    // Without a limit, a game nobody plays would last until the stack tops out.
    loop {
        let frame = driver.step(state, 0);
        if driver.captures.wanted() {
            driver.draw(&frame, false);
        }
        if frame.game_over {
            // The GAME OVER animation, for captures.
            while driver.captures.wanted() && !driver.game_over_shown() {
                driver.step_game_over();
                driver.draw(&frame, false);
            }
            return (frame, Ending::Lost);
        }
        if max_frames.is_some_and(|max| driver.ticks >= max) || driver.captures_done() {
            return (frame, Ending::FrameLimit);
        }
    }
}

/// A new seed for each game, from the clock: each launch deals a different board. The
/// summary prints it so a game can be replayed with `--seed`.
fn random_seed() -> u32 {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    // Fold the 128-bit count and mix it, so close launches give unrelated seeds.
    let mut x = (nanos as u64) ^ ((nanos >> 64) as u64) ^ u64::from(std::process::id());
    x ^= x >> 33;
    x = x.wrapping_mul(0xff51_afd7_ed55_8ccd);
    x ^= x >> 33;
    x as u32
}

/// Why the game loop stopped.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Ending {
    Lost,
    Quit,
    FrameLimit,
}

/// Runs the game until it ends; returns the last frame and why it stopped.
fn play(state: &mut GameState, driver: &mut Driver, max_frames: Option<i32>) -> (FrameData, Ending) {
    let _guard = TerminalGuard::new();
    render::initialize();
    let _ = io::stdout().flush();
    driver.renderer.set_width(terminal_width());

    let frame_duration = Duration::from_millis(1000 / TICKS_PER_SECOND as u64);
    let mut quit = false;
    let mut last = FrameData::default();
    let mut paused = false;

    let ending = loop {
        let frame_start = Instant::now();

        let (mut pause_pressed, mut resized) = (false, false);
        let keys = read_input(&mut quit, &mut pause_pressed, &mut resized);
        if resized {
            driver.renderer.set_width(terminal_width());
        }
        if pause_pressed && !last.game_over {
            paused = !paused;
        }

        if quit {
            break Ending::Quit;
        }

        // Paused: the game does not step, the board is hidden.
        if paused {
            print!("{}", driver.draw(&last, true));
            let _ = io::stdout().flush();
            driver.release_keys();
            std::thread::sleep(frame_duration);
            continue;
        }

        let frame = driver.step(state, keys);
        print!("{}", driver.draw(&frame, false));
        let _ = io::stdout().flush();

        let game_over = frame.game_over;
        last = frame;

        if max_frames.is_some_and(|max| driver.ticks >= max) {
            break Ending::FrameLimit;
        }

        if game_over {
            show_game_over(driver, &last);
            break Ending::Lost;
        }

        let elapsed = frame_start.elapsed();
        if elapsed < frame_duration {
            std::thread::sleep(frame_duration - elapsed);
        }
    };

    // The guard's Drop restores the terminal (alternate screen, cursor, raw mode).
    (last, ending)
}

/// Game over: the logo turns into GAME OVER, then the final board stays until a key is
/// pressed.
fn show_game_over(driver: &mut Driver, last: &FrameData) {
    let frame_duration = Duration::from_millis(1000 / TICKS_PER_SECOND as u64);
    while !driver.game_over_shown() {
        driver.step_game_over();
        print!("{}", driver.draw(last, false));
        let _ = io::stdout().flush();
        std::thread::sleep(frame_duration);
    }
    // Keys pressed in the rush before the loss, or during the animation, do not count.
    while event::poll(Duration::ZERO).unwrap_or(false) {
        let _ = event::read();
    }
    loop {
        match event::read() {
            Ok(Event::Key(KeyEvent { kind: KeyEventKind::Press, .. })) | Err(_) => break,
            Ok(Event::Resize(..)) => {
                driver.renderer.set_width(terminal_width());
                print!("{}", driver.draw(last, false));
                let _ = io::stdout().flush();
            }
            Ok(_) => {}
        }
    }
}

/// Width of the terminal in columns, if it can be read.
fn terminal_width() -> Option<usize> {
    terminal::size().ok().map(|(columns, _)| columns as usize)
}

/// End-of-game message printed in the shell: how the game ended, score, time, best chain
/// and the seed to replay the same game.
fn summary(frame: &crackattack_core::FrameData, ending: Ending, seed: u32) -> String {
    let title = match ending {
        Ending::Lost => "Game over: the stack reached the top.",
        Ending::Quit => "Game stopped.",
        Ending::FrameLimit => "Frame limit reached.",
    };
    let seconds = frame.ui.clock_time / TICKS_PER_SECOND;
    let mut text = format!(
        "{}\n  Score: {}   Time: {}:{:02}",
        title, frame.score, seconds / 60, seconds % 60,
    );
    if frame.stats.max_chain_level >= 2 {
        text.push_str(&format!("   Best chain: x{}", frame.stats.max_chain_level));
    }
    text.push_str(&format!("\n  Replay this game: crackattack-terminal --seed {}", seed));
    text
}

/// Map a crossterm KeyCode to an input bitmask. Returns 0 for unmapped keys.
fn map_key(code: KeyCode) -> u8 {
    // Letters work in either case (Caps Lock).
    let code = match code {
        KeyCode::Char(c) => KeyCode::Char(c.to_ascii_lowercase()),
        code => code,
    };
    match code {
        KeyCode::Up    | KeyCode::Char('w') => INPUT_UP,
        KeyCode::Down  | KeyCode::Char('s') => INPUT_DOWN,
        KeyCode::Left  | KeyCode::Char('a') => INPUT_LEFT,
        KeyCode::Right | KeyCode::Char('d') => INPUT_RIGHT,
        KeyCode::Char(' ')                  => INPUT_SWAP,
        KeyCode::Enter                      => INPUT_ADVANCE,
        _ => 0,
    }
}

fn is_quit_key(code: KeyCode) -> bool {
    matches!(code, KeyCode::Char('q') | KeyCode::Char('Q') | KeyCode::Esc)
}

fn is_pause_key(code: KeyCode) -> bool {
    matches!(code, KeyCode::Char('p') | KeyCode::Char('P'))
}

/// Keys pressed since the last tick, as `INPUT_*` bits. Also tells whether the terminal
/// was resized (the screen must then be redrawn whole).
fn read_input(quit: &mut bool, pause_pressed: &mut bool, resized: &mut bool) -> u8 {
    let mut keys = 0;

    while event::poll(Duration::ZERO).unwrap_or(false) {
        let event = event::read();
        if let Ok(Event::Resize(..)) = event {
            *resized = true;
        }
        if let Ok(Event::Key(KeyEvent { code, kind: KeyEventKind::Press, .. })) = event {
            if is_quit_key(code) {
                *quit = true;
            }
            if is_pause_key(code) {
                *pause_pressed = true;
            }
            keys |= map_key(code);
        }
    }

    keys
}

/// RAII guard: enables raw mode on construction and restores the FULL terminal state on drop:
/// raw mode, alternate screen and cursor visibility. Restoring in `Drop` means a panic in the
/// main loop still leaves the user's terminal usable.
struct TerminalGuard {
    active: bool,
}

impl TerminalGuard {
    fn new() -> Self {
        match terminal::enable_raw_mode() {
            Ok(()) => TerminalGuard { active: true },
            Err(e) => {
                eprintln!("Warning: could not enable raw mode: {}", e);
                TerminalGuard { active: false }
            }
        }
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        render::cleanup();
        let _ = io::stdout().flush();
        if self.active {
            let _ = terminal::disable_raw_mode();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(line: &str) -> Vec<String> {
        line.split_whitespace().map(String::from).collect()
    }

    #[test]
    fn options_are_parsed() {
        let options = parse_args(&args("--seed 42 --frames 300 --autoplay easy --headless --smooth"))
            .unwrap()
            .unwrap();
        assert_eq!(options.seed, Some(42));
        assert_eq!(options.max_frames, Some(300));
        assert_eq!(options.autoplay, Some(AIDifficulty::Easy));
        assert!(options.headless && options.smooth && !options.debug);
        // A level is optional: the next option is not taken for one.
        let options = parse_args(&args("--autoplay --debug")).unwrap().unwrap();
        assert_eq!(options.autoplay, Some(AIDifficulty::Hard));
        assert!(options.debug);
        assert!(parse_args(&args("--seed 1 --help")).unwrap().is_none());
    }

    #[test]
    fn bad_options_are_errors() {
        for line in ["--seed abc", "--seed", "--frames abc", "--frames 0", "--frames",
                     "--capture x", "--capture-file", "--sede 4", "extra"] {
            assert!(parse_args(&args(line)).is_err(), "{}", line);
        }
    }

    #[test]
    fn help_lists_every_key_of_the_panel() {
        // One table for both: the panel's short forms are among the keys --help gives.
        for key in &render::KEYS {
            assert!(key.keys.contains(key.hint_key), "{}", key.keys);
        }
    }

    #[test]
    fn letters_work_in_either_case() {
        for (lower, upper) in [('w', 'W'), ('a', 'A'), ('s', 'S'), ('d', 'D')] {
            assert_eq!(map_key(KeyCode::Char(lower)), map_key(KeyCode::Char(upper)));
        }
    }

    #[test]
    fn arrows_map_to_directions() {
        assert_eq!(map_key(KeyCode::Up), INPUT_UP);
        assert_eq!(map_key(KeyCode::Down), INPUT_DOWN);
        assert_eq!(map_key(KeyCode::Left), INPUT_LEFT);
        assert_eq!(map_key(KeyCode::Right), INPUT_RIGHT);
    }

    #[test]
    fn wasd_maps_to_directions() {
        assert_eq!(map_key(KeyCode::Char('w')), INPUT_UP);
        assert_eq!(map_key(KeyCode::Char('a')), INPUT_LEFT);
        assert_eq!(map_key(KeyCode::Char('s')), INPUT_DOWN);
        assert_eq!(map_key(KeyCode::Char('d')), INPUT_RIGHT);
    }

    #[test]
    fn action_keys() {
        assert_eq!(map_key(KeyCode::Char(' ')), INPUT_SWAP);
        assert_eq!(map_key(KeyCode::Enter), INPUT_ADVANCE);
    }

    #[test]
    fn unmapped_key_returns_zero() {
        assert_eq!(map_key(KeyCode::Char('z')), 0);
        assert_eq!(map_key(KeyCode::F(1)), 0);
    }

    #[test]
    fn summary_tells_how_it_ended_and_the_score() {
        let mut frame = crackattack_core::FrameData::default();
        frame.score = 1234;
        frame.ui.clock_time = 125 * TICKS_PER_SECOND;
        let text = summary(&frame, Ending::Lost, 42);
        assert!(text.starts_with("Game over"));
        assert!(text.contains("Score: 1234"));
        assert!(text.contains("Time: 2:05"));
        assert!(text.contains("--seed 42"));
        assert!(!text.contains("Best chain"));
        assert!(summary(&frame, Ending::Quit, 42).starts_with("Game stopped"));
    }

    #[test]
    fn capture_ticks_are_parsed_sorted_with_end() {
        assert_eq!(parse_capture_ticks("250,100,end"), Some((vec![100, 250], true)));
        assert_eq!(parse_capture_ticks("end"), Some((vec![], true)));
        assert_eq!(parse_capture_ticks("10, 10"), Some((vec![10], false)));
        assert_eq!(parse_capture_ticks("0"), None);
        assert_eq!(parse_capture_ticks("abc"), None);
        assert_eq!(parse_capture_ticks(""), None);
        assert_eq!(parse_level("medium"), Some(AIDifficulty::Medium));
        assert_eq!(parse_level("--seed"), None);
    }

    #[test]
    fn the_autoplay_plays_headless_and_captures_the_chosen_ticks() {
        let path = std::env::temp_dir().join(format!("crackattack-capture-test-{}.txt", std::process::id()));
        let captures = Captures { ticks: vec![50, 400], at_end: true, path: path.display().to_string() };
        let mut state = GameState::new(3);
        let mut driver = Driver::new(Some(GridAutoplay::new(AIDifficulty::Hard, 3)), captures);
        let (frame, ending) = run_headless(&mut state, &mut driver, Some(600));
        let written = driver.finish(&frame).unwrap();
        let content = std::fs::read_to_string(&path);
        let _ = std::fs::remove_file(&path);
        assert_eq!(ending, Ending::FrameLimit);
        assert_eq!(written.unwrap().0, 3);
        let content = content.unwrap();
        for title in ["=== tick 50 ===", "=== tick 400 ===", "=== end, tick 600 ==="] {
            assert!(content.contains(title), "missing {}", title);
        }
        // The AI did play: alone, the stack would not have scored.
        assert!(frame.score > 0);
        assert!(content.contains("Score"));
    }

    #[test]
    fn random_seeds_differ() {
        let a = random_seed();
        std::thread::sleep(Duration::from_millis(2));
        assert_ne!(a, random_seed());
    }

    #[test]
    fn pause_key() {
        assert!(is_pause_key(KeyCode::Char('p')));
        assert!(is_pause_key(KeyCode::Char('P')));
        assert_eq!(map_key(KeyCode::Char('p')), 0);
    }

    #[test]
    fn quit_keys() {
        assert!(is_quit_key(KeyCode::Char('q')));
        assert!(is_quit_key(KeyCode::Char('Q')));
        assert!(is_quit_key(KeyCode::Esc));
        assert!(!is_quit_key(KeyCode::Char('a')));
    }

    /// What the game writes to the terminal (`--ignored --nocapture` to see it): bytes per
    /// second, lines rewritten and the biggest frame, in a game the AI plays, with
    /// `--smooth`, and paused.
    #[test]
    #[ignore]
    fn output_bandwidth() {
        for (name, smooth, paused) in [("normal", false, false), ("smooth", true, false), ("paused", false, true)] {
            render::SMOOTH_RISE.with(|s| s.set(smooth));
            let (mut bytes, mut lines, mut peak, mut frames) = (0, 0, 0, 0);
            for seed in [1, 2, 3] {
                let mut state = GameState::new(seed);
                let mut driver = Driver::new(Some(GridAutoplay::new(AIDifficulty::Hard, seed)), Captures::default());
                for _ in 0..5000 {
                    let frame = driver.step(&mut state, 0);
                    let text = driver.draw(&frame, paused);
                    bytes += text.len();
                    lines += text.matches(";1H").count();
                    peak = peak.max(text.len());
                    frames += 1;
                    if frame.game_over {
                        break;
                    }
                }
            }
            let per_second = bytes as f64 / frames as f64 * TICKS_PER_SECOND as f64 / 1024.0;
            println!(
                "{:<6} {:>6.1} KB/s  {:>5.0} B/frame  {:.1} lines/frame  peak {} B",
                name, per_second, bytes as f64 / frames as f64, lines as f64 / frames as f64, peak
            );
        }
    }
}
