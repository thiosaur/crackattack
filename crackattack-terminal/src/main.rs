// Crack Attack! Terminal - Standalone Rust binary

mod render;

use crackattack_core::{GameState, InputSnapshot};
use crackattack_core::input::*;
use std::io::{self, Write};
use std::time::{Duration, Instant};

use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind};
use crossterm::terminal;

fn main() {
    let args: Vec<String> = std::env::args().collect();

    let mut seed: u32 = 0;
    let mut max_frames: Option<i32> = None;

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--seed" => {
                if i + 1 < args.len() {
                    seed = args[i + 1].parse().unwrap_or(0);
                    i += 1;
                }
            }
            "--frames" => {
                if i + 1 < args.len() {
                    max_frames = Some(args[i + 1].parse().unwrap_or(1000));
                    i += 1;
                }
            }
            "--help" | "-h" => {
                println!("Crack Attack! Terminal (Rust)");
                println!();
                println!("Usage: crackattack-terminal [OPTIONS]");
                println!();
                println!("Options:");
                println!("  --seed N      Use specific random seed");
                println!("  --frames N    Stop after N frames");
                println!("  --help        Show this help");
                println!();
                println!("Controls:");
                println!("  Arrow keys / WASD  Move cursor");
                println!("  Space              Swap blocks");
                println!("  Q / Escape         Quit");
                return;
            }
            _ => {}
        }
        i += 1;
    }

    let mut state = GameState::new(seed);

    let _guard = TerminalGuard::new();
    render::initialize();
    let _ = io::stdout().flush();

    let frame_duration = Duration::from_millis(20);  // 50 Hz (matches core game tick rate)
    let mut frame_count = 0;
    let mut quit = false;
    let mut prev_state: u8 = 0;

    loop {
        let frame_start = Instant::now();

        let mut input = read_input(&mut quit);
        // Edge detection: compute pressed_this_frame from state transitions.
        input.pressed_this_frame = input.state & !prev_state;
        input.prev_state = prev_state;
        prev_state = input.state;

        if quit {
            break;
        }

        let frame = state.step(&input);

        render::render(&frame);
        let _ = io::stdout().flush();

        frame_count += 1;

        if let Some(max) = max_frames {
            if frame_count >= max {
                break;
            }
        }

        if frame.game_over {
            std::thread::sleep(Duration::from_secs(2));
            break;
        }

        let elapsed = frame_start.elapsed();
        if elapsed < frame_duration {
            std::thread::sleep(frame_duration - elapsed);
        }
    }

    // The guard's Drop restores the terminal (alternate screen, cursor, raw mode).
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

fn read_input(quit: &mut bool) -> InputSnapshot {
    let mut input = InputSnapshot::new();

    while event::poll(Duration::ZERO).unwrap_or(false) {
        if let Ok(Event::Key(KeyEvent { code, kind: KeyEventKind::Press, .. })) = event::read() {
            if is_quit_key(code) {
                *quit = true;
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
