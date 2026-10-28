// Crack Attack! Terminal - Standalone Rust binary

mod danger;
mod render;

use crackattack_core::{GameState, InputSnapshot};
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
            "--help" | "-h" => {
                println!("Crack Attack! Terminal (Rust)");
                println!();
                println!("Usage: crackattack-terminal [OPTIONS]");
                println!();
                println!("Options:");
                println!("  --seed N      Use a specific seed (default: a new one each game)");
                println!("  --frames N    Stop after N frames");
                println!("  --debug       Show the state of the cells under the cursor");
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

    let (frame, ending) = play(&mut state, max_frames);
    // The terminal is restored by now: the summary stays in the shell after the game.
    println!("{}", summary(&frame, ending, seed));
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
fn play(state: &mut GameState, max_frames: Option<i32>) -> (crackattack_core::FrameData, Ending) {
    let _guard = TerminalGuard::new();
    render::initialize();
    let _ = io::stdout().flush();

    let frame_duration = Duration::from_millis(1000 / TICKS_PER_SECOND as u64);
    let mut frame_count = 0;
    let mut quit = false;
    let mut prev_state: u8 = 0;
    let mut last = crackattack_core::FrameData::default();
    let mut paused = false;
    let mut danger = danger::DangerAnimator::new();
    // Chain just reached, and ticks left to show it.
    let mut chain: Option<(i32, i32)> = None;

    let ending = loop {
        let frame_start = Instant::now();

        let mut pause_pressed = false;
        let mut input = read_input(&mut quit, &mut pause_pressed);
        if pause_pressed && !last.game_over {
            paused = !paused;
        }
        // Edge detection: compute pressed_this_frame from state transitions.
        input.pressed_this_frame = input.state & !prev_state;
        input.prev_state = prev_state;
        prev_state = input.state;

        if quit {
            break Ending::Quit;
        }

        // Paused: the game does not step, the board is hidden.
        if paused {
            render::render_paused(&last);
            let _ = io::stdout().flush();
            prev_state = 0;
            std::thread::sleep(frame_duration);
            continue;
        }

        let frame = state.step(&input);

        danger.step(frame.ui.creep_freeze, frame.ui.loss_countdown);
        chain = chain.map(|(n, left)| (n, left - 1)).filter(|&(_, left)| left > 0);
        for event in &frame.gameplay_events {
            if let crackattack_core::gameplay_event::GameplayEvent::ChainProgressed(e) = event {
                chain = Some((e.chain_level, CHAIN_SHOW_TICKS));
            }
        }
        let hud = render::Hud { danger: danger.bars(), chain: chain.map(|(n, _)| n) };
        render::render(&frame, &hud);
        let _ = io::stdout().flush();

        frame_count += 1;

        let game_over = frame.game_over;
        last = frame;

        if let Some(max) = max_frames {
            if frame_count >= max {
                break Ending::FrameLimit;
            }
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

fn read_input(quit: &mut bool, pause_pressed: &mut bool) -> InputSnapshot {
    let mut input = InputSnapshot::new();

    while event::poll(Duration::ZERO).unwrap_or(false) {
        if let Ok(Event::Key(KeyEvent { code, kind: KeyEventKind::Press, .. })) = event::read() {
            if is_quit_key(code) {
                *quit = true;
            }
            if is_pause_key(code) {
                *pause_pressed = true;
            }
            input.state |= map_key(code);
        }
    }

    input
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
