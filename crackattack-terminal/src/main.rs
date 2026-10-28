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
    let args: Vec<String> = std::env::args().collect();

    let mut seed: Option<u32> = None;
    let mut max_frames: Option<i32> = None;
    let mut autoplay: Option<AIDifficulty> = None;
    let mut headless = false;
    let mut captures = Captures::default();

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--seed" => {
                if i + 1 < args.len() {
                    seed = args[i + 1].parse().ok();
                    i += 1;
                }
            }
            "--frames" => {
                if i + 1 < args.len() {
                    max_frames = Some(args[i + 1].parse().unwrap_or(1000));
                    i += 1;
                }
            }
            "--debug" => {
                render::SHOW_DEBUG.store(true, std::sync::atomic::Ordering::Relaxed);
            }
            "--smooth" => {
                render::SMOOTH_RISE.with(|smooth| smooth.set(true));
            }
            "--autoplay" => {
                // Optional level: easy, medium or hard (the default, as in the GUI).
                let level = args.get(i + 1).and_then(|a| parse_level(a));
                if level.is_some() {
                    i += 1;
                }
                autoplay = Some(level.unwrap_or(AIDifficulty::Hard));
            }
            "--headless" => headless = true,
            "--capture" => {
                if let Some(list) = args.get(i + 1) {
                    match parse_capture_ticks(list) {
                        Some((ticks, at_end)) => {
                            captures.ticks = ticks;
                            captures.at_end = at_end;
                        }
                        None => {
                            eprintln!("--capture: expected ticks such as 100,250,end");
                            std::process::exit(2);
                        }
                    }
                    i += 1;
                }
            }
            "--capture-file" => {
                if let Some(path) = args.get(i + 1) {
                    captures.path = path.clone();
                    i += 1;
                }
            }
            "--help" | "-h" => {
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
                println!("  Arrow keys / WASD  Move cursor");
                println!("  Space              Swap blocks");
                println!("  Enter              Raise the stack");
                println!("  P                  Pause / resume");
                println!("  Q / Escape         Quit");
                return;
            }
            _ => {}
        }
        i += 1;
    }

    let seed = seed.unwrap_or_else(random_seed);
    let mut state = GameState::new(seed);
    let mut driver = Driver::new(autoplay.map(|level| GridAutoplay::new(level, seed)), captures);

    let (frame, ending) = if headless {
        run_headless(&mut state, &mut driver, max_frames)
    } else {
        play(&mut state, &mut driver, max_frames)
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
        self.hud = render::Hud { danger: self.danger.bars(), chain: self.chain.map(|(n, _)| n) };
        if let Some(screen) = &mut self.screen {
            screen.feed(&render::rendered(&frame, &self.hud));
            if self.captures.ticks.binary_search(&self.ticks).is_ok() {
                self.take(&format!("tick {}", self.ticks));
            }
        }
        frame
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
        if frame.game_over {
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

    let frame_duration = Duration::from_millis(1000 / TICKS_PER_SECOND as u64);
    let mut quit = false;
    let mut last = FrameData::default();
    let mut paused = false;

    let ending = loop {
        let frame_start = Instant::now();

        let mut pause_pressed = false;
        let keys = read_input(&mut quit, &mut pause_pressed);
        if pause_pressed && !last.game_over {
            paused = !paused;
        }

        if quit {
            break Ending::Quit;
        }

        // Paused: the game does not step, the board is hidden.
        if paused {
            render::render_paused(&last);
            let _ = io::stdout().flush();
            driver.release_keys();
            std::thread::sleep(frame_duration);
            continue;
        }

        let frame = driver.step(state, keys);
        render::render(&frame, &driver.hud);
        let _ = io::stdout().flush();

        let game_over = frame.game_over;
        last = frame;

        if max_frames.is_some_and(|max| driver.ticks >= max) {
            break Ending::FrameLimit;
        }

        if game_over {
            // Leave the final board on screen a moment before the summary.
            std::thread::sleep(Duration::from_secs(2));
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
    match code {
        KeyCode::Up    | KeyCode::Char('w') | KeyCode::Char('W') => INPUT_UP,
        KeyCode::Down  | KeyCode::Char('s') | KeyCode::Char('S') => INPUT_DOWN,
        KeyCode::Left  | KeyCode::Char('a')                      => INPUT_LEFT,
        KeyCode::Right | KeyCode::Char('d')                      => INPUT_RIGHT,
        KeyCode::Char(' ')                                       => INPUT_SWAP,
        KeyCode::Enter                                           => INPUT_ADVANCE,
        _ => 0,
    }
}

fn is_quit_key(code: KeyCode) -> bool {
    matches!(code, KeyCode::Char('q') | KeyCode::Char('Q') | KeyCode::Esc)
}

fn is_pause_key(code: KeyCode) -> bool {
    matches!(code, KeyCode::Char('p') | KeyCode::Char('P'))
}

/// Keys pressed since the last tick, as `INPUT_*` bits.
fn read_input(quit: &mut bool, pause_pressed: &mut bool) -> u8 {
    let mut keys = 0;

    while event::poll(Duration::ZERO).unwrap_or(false) {
        if let Ok(Event::Key(KeyEvent { code, kind: KeyEventKind::Press, .. })) = event::read() {
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

#[cfg(test)]
mod tests {
    use super::*;

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
