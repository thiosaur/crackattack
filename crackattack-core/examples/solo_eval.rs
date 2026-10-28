//! Solo evaluation of the autoplay drivers: the same seeds, one game each, mean score.
//!
//! `cargo run --release -p crackattack-core --example solo_eval -- [GAMES] [MAX_MINUTES]`
//!
//! Drivers: `classic` (the original test-oriented autoplay) and the grid-playing AI of the VS
//! Computer opponent at its three levels. A game ends when the board tops out or at the time cap.

use crackattack_core::stats::GameStats;
use crackattack_core::vs::GridAutoplay;
use crackattack_core::ai_difficulty::AIDifficulty;
use crackattack_core::{
    AIController, GameState, GameStatus, Input, ScoringMode, INPUT_ADVANCE, INPUT_DOWN,
    INPUT_LEFT, INPUT_RIGHT, INPUT_SWAP, INPUT_UP, STEPS_PER_SECOND,
};

const KEYS: [u8; 6] = [INPUT_LEFT, INPUT_RIGHT, INPUT_UP, INPUT_DOWN, INPUT_SWAP, INPUT_ADVANCE];

enum Driver {
    Classic(AIController),
    Grid(GridAutoplay),
}

struct Outcome {
    score: i32,
    seconds: f64,
    survived: bool,
    /// Cursor steps pressed (one direction key down on a tick).
    cursor_steps: u32,
    stats: GameStats,
}

fn play(driver_name: &str, seed: u32, max_ticks: i32) -> Outcome {
    let mut game = GameState::with_scoring(seed, ScoringMode::Classic);
    let mut input = Input::new();
    let mut driver = match driver_name {
        "classic" => Driver::Classic(AIController::new()),
        "easy" => Driver::Grid(GridAutoplay::new(AIDifficulty::Easy, seed)),
        "medium" => Driver::Grid(GridAutoplay::new(AIDifficulty::Medium, seed)),
        _ => Driver::Grid(GridAutoplay::new(AIDifficulty::Hard, seed)),
    };
    let mut ticks = 0;
    let mut cursor_steps = 0u32;
    while game.status == GameStatus::Playing && ticks < max_ticks {
        let flags = match &mut driver {
            Driver::Classic(ai) => {
                let mut f = ai.update(&game);
                if AIController::should_fast_forward(&game) {
                    f |= INPUT_ADVANCE;
                }
                f
            }
            Driver::Grid(ai) => ai.update(&game),
        };
        if flags & (INPUT_LEFT | INPUT_RIGHT | INPUT_UP | INPUT_DOWN) != 0 {
            cursor_steps += 1;
        }
        for key in KEYS {
            input.set_key(key, flags & key != 0);
        }
        let snapshot = input.snapshot();
        input.end_frame();
        game.step(&snapshot);
        ticks += 1;
    }
    Outcome {
        score: game.score.value(),
        seconds: ticks as f64 / STEPS_PER_SECOND as f64,
        cursor_steps,
        survived: game.status == GameStatus::Playing,
        stats: game.stats.clone(),
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let games: u32 = args.first().and_then(|a| a.parse().ok()).unwrap_or(10);
    let minutes: i32 = args.get(1).and_then(|a| a.parse().ok()).unwrap_or(10);
    let max_ticks = minutes * 60 * STEPS_PER_SECOND;

    println!("{games} solo games per driver, same seeds, cap {minutes} min; means unless noted\n");
    println!(
        "{:<8} {:>6} {:>7} {:>8} {:>7} {:>8} {:>9} {:>8} {:>7} {:>9} {:>7}",
        "driver", "score", "time s", "blocks", "blk/min", "swaps", "swaps/min", "cells/s", "chains", "max chain", "capped"
    );
    for name in ["classic", "easy", "medium", "hard"] {
        let outcomes: Vec<Outcome> = (1..=games).map(|seed| play(name, seed, max_ticks)).collect();
        let n = outcomes.len() as f64;
        let mean = |f: &dyn Fn(&Outcome) -> f64| outcomes.iter().map(f).sum::<f64>() / n;
        let capped = outcomes.iter().filter(|o| o.survived).count();
        let best_chain = outcomes.iter().map(|o| o.stats.max_chain_level).max().unwrap_or(0);
        println!(
            "{name:<8} {:>6.0} {:>7.0} {:>8.0} {:>7.1} {:>8.0} {:>9.1} {:>8.1} {:>7.1} {:>6.1}/{best_chain:<2} {capped:>4}/{games}",
            mean(&|o| o.score as f64),
            mean(&|o| o.seconds),
            mean(&|o| o.stats.blocks_eliminated as f64),
            mean(&|o| o.stats.blocks_per_minute() as f64),
            mean(&|o| o.stats.swaps_performed as f64),
            mean(&|o| o.stats.swaps_performed as f64 / (o.seconds / 60.0)),
            mean(&|o| o.cursor_steps as f64 / o.seconds),
            mean(&|o| o.stats.chains_count as f64),
            mean(&|o| o.stats.max_chain_level as f64),
        );
    }
}
