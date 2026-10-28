//! Grid-playing AI for the VS Computer opponent.
//!
//! A port of a strategic bot (see AUTHORS for its origin). It drives a real
//! `GameState`: every tick it reads the board and the cursor and returns the next input.
//! `planner` is the pure part (cascade evaluator and multi-swap planners); `controller` is
//! the decision loop that reads a `GameState`.

pub(crate) mod planner;
pub(crate) mod controller;
mod candidates;
mod hands;
mod memo;
mod reactive;
mod select;
mod shape;
mod strategic;
pub(crate) mod tuning;
mod transient;
mod view;
#[cfg(test)]
mod arena;

use crate::game_state::GameState;
use crate::ai_difficulty::AIDifficulty;

use controller::{decision_seed, AiController};
use tuning::Adaptive;

/// The grid-playing AI as a stand-alone input driver, for autoplay and measurements: same
/// brain as the VS Computer opponent, plugged on any `GameState` (solo included).
pub struct GridAutoplay {
    controller: AiController,
}

impl GridAutoplay {
    /// `seed` is the seed of the game being played; it only varies equally-valued choices.
    pub fn new(difficulty: AIDifficulty, seed: u32) -> Self {
        Self {
            controller: AiController::with_adaptive(
                Adaptive::for_difficulty(difficulty),
                Some(decision_seed(seed, 1)),
            ),
        }
    }

    /// Reads the game and returns the input flags of this tick (`INPUT_*` bitmask).
    pub fn update(&mut self, game: &GameState) -> u8 {
        self.controller.decide(game)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game_state::GameStatus;
    use crate::score::ScoringMode;
    use crate::input::*;

    /// One solo game played by the autoplay: (score, ticks, FNV-1a of every input issued).
    fn replay(difficulty: AIDifficulty, seed: u32, ticks: i32) -> (i32, i32, u64) {
        let mut game = GameState::with_scoring(seed, ScoringMode::Classic);
        let mut ai = GridAutoplay::new(difficulty, seed);
        let mut input = Input::new();
        let mut hash = 0xcbf2_9ce4_8422_2325_u64;
        let mut played = 0;
        while game.status == GameStatus::Playing && played < ticks {
            let flags = ai.update(&game);
            hash = (hash ^ flags as u64).wrapping_mul(0x0000_0100_0000_01b3);
            for bit in [INPUT_LEFT, INPUT_RIGHT, INPUT_UP, INPUT_DOWN, INPUT_SWAP, INPUT_ADVANCE] {
                input.set_key(bit, flags & bit != 0);
            }
            let snapshot = input.snapshot();
            input.end_frame();
            game.step(&snapshot);
            played += 1;
        }
        (game.score.value(), played, hash)
    }

    /// The autoplay holds no clock and no unseeded randomness: the same seed replays the very
    /// same game, input for input, whatever the level.
    #[test]
    fn the_autoplay_replays_the_same_game_for_a_seed() {
        for difficulty in [AIDifficulty::Easy, AIDifficulty::Medium, AIDifficulty::Hard] {
            let first = replay(difficulty, 4242, 4_000);
            assert_eq!(first, replay(difficulty, 4242, 4_000), "{difficulty:?}");
            assert!(first.0 > 0, "{difficulty:?} did not score");
        }
        assert_ne!(replay(AIDifficulty::Hard, 4242, 4_000).2, replay(AIDifficulty::Hard, 4243, 4_000).2);
    }

    /// Solo game of a seed with `act_when_frozen` on or off: (ticks survived, ticks since the last swap at the end).
    fn survive_with(act_when_frozen: bool, seed: u32) -> (i32, i32) {
        let mut game = GameState::with_scoring(seed, ScoringMode::Classic);
        let mut ai = GridAutoplay::new(AIDifficulty::Hard, seed);
        ai.controller.adaptive.survival.act_when_frozen = act_when_frozen;
        ai.controller.adaptive.aggressive.act_when_frozen = act_when_frozen;
        ai.controller.tuning.act_when_frozen = act_when_frozen;
        let mut input = Input::new();
        let (mut ticks, mut last_swap) = (0, 0);
        while game.status == GameStatus::Playing && ticks < 40_000 {
            let flags = ai.update(&game);
            if flags & INPUT_SWAP != 0 {
                last_swap = ticks;
            }
            for bit in [INPUT_LEFT, INPUT_RIGHT, INPUT_UP, INPUT_DOWN, INPUT_SWAP, INPUT_ADVANCE] {
                input.set_key(bit, flags & bit != 0);
            }
            let snapshot = input.snapshot();
            input.end_frame();
            game.step(&snapshot);
            ticks += 1;
        }
        (ticks, ticks - last_swap)
    }

    /// With the stack at the top and the board frozen (it cannot be raised), the bot used to
    /// stand still until the end although set-ups were left: it survives much longer playing.
    #[test]
    fn the_bot_keeps_playing_on_a_frozen_board() {
        let total = |act: bool| -> i32 { (1..=6).map(|seed| survive_with(act, seed).0).sum() };
        let (off, on) = (total(false), total(true));
        assert!(on > off + off / 5, "{on} ticks played against {off} over 6 games");
    }

    /// A raise latched by the creep (it only clears when the row lands, which never happens
    /// with the creep frozen at the top) must not hold the swaps: the bot was locked out for good.
    #[test]
    fn a_latched_raise_does_not_lock_the_hands() {
        let mut game = GameState::with_scoring(1, ScoringMode::Classic);
        let mut ai = GridAutoplay::new(AIDifficulty::Hard, 1);
        game.creep.advance_latch = true;
        assert_eq!(ai.controller.hold_keys(&game), None);
    }

    /// Swaps that put a row back to what it was two swaps earlier, in the first `ticks` of a game.
    fn flips_in(seed: u32, ticks: i32, guard: bool) -> usize {
        use super::view::{flavor_at, WIDTH};
        let row_text = |game: &GameState, y: i32| -> String {
            (0..WIDTH).map(|x| {
                let c = game.grid.get(x, y);
                if c.is_garbage() { 'G' } else if c.is_block() { char::from(b'0' + flavor_at(game, x, y) as u8) } else { '.' }
            }).collect()
        };
        let mut game = GameState::with_scoring(seed, ScoringMode::Classic);
        let mut ai = GridAutoplay::new(AIDifficulty::Hard, seed);
        ai.controller.adaptive.survival.row_undo_guard = guard;
        ai.controller.adaptive.aggressive.row_undo_guard = guard;
        ai.controller.tuning.row_undo_guard = guard;
        let mut input = Input::new();
        let mut log: Vec<(i32, String, i32)> = Vec::new(); // row, its text before the swap, tick
        let mut flips = 0;
        for tick in 0..ticks {
            if game.status != GameStatus::Playing {
                break;
            }
            let flags = ai.update(&game);
            if flags & INPUT_SWAP != 0 && game.swapper.state == crate::swapper::SwapperState::Idle {
                let y = game.swapper.y;
                let before = row_text(&game, y);
                let n = log.len();
                if n >= 2 && log[n - 1].0 == y && log[n - 2].0 == y && log[n - 2].1 == before && tick - log[n - 2].2 < 400 {
                    flips += 1;
                }
                log.push((y, before, tick));
            }
            for bit in [INPUT_LEFT, INPUT_RIGHT, INPUT_UP, INPUT_DOWN, INPUT_SWAP, INPUT_ADVANCE] {
                input.set_key(bit, flags & bit != 0);
            }
            let snapshot = input.snapshot();
            input.end_frame();
            game.step(&snapshot);
        }
        flips
    }

    /// The bot used to swap the same pair back and forth every few ticks (seed 1 did it four
    /// times in a row during a fall), undoing its own work. The guard refuses a swap that
    /// restores a row a recent swap had just produced.
    #[test]
    fn the_bot_does_not_undo_its_own_swaps() {
        assert_eq!(flips_in(1, 2_000, true), 0);
        let (with, without): (usize, usize) = (1..=10).map(|s| (flips_in(s, 6_000, true), flips_in(s, 6_000, false))).fold((0, 0), |a, b| (a.0 + b.0, a.1 + b.1));
        println!("back-and-forth swaps over 10 games: {with} with the row guard, {without} without");
        assert!(with <= without);
    }

    /// Diagnostic: replays a solo game and dumps the board and the bot's state before the loss.
    /// `DEATH_SEED` (default 4242), `DEATH_LEVEL` (easy|medium|hard).
    #[test]
    #[ignore = "diagnostic"]
    fn death_diagnostic() {
        use super::view::{flavor_at, HEIGHT, WIDTH};
        let seed: u32 = std::env::var("DEATH_SEED").ok().and_then(|v| v.parse().ok()).unwrap_or(4242);
        let level = match std::env::var("DEATH_LEVEL").as_deref() {
            Ok("easy") => AIDifficulty::Easy,
            Ok("medium") => AIDifficulty::Medium,
            _ => AIDifficulty::Hard,
        };
        let mut game = GameState::with_scoring(seed, ScoringMode::Classic);
        let mut ai = GridAutoplay::new(level, seed);
        let mut input = Input::new();
        let mut history: Vec<(i32, String)> = Vec::new();
        let mut last_swap = 0;
        let mut ticks = 0;
        while game.status == GameStatus::Playing && ticks < 60_000 {
            let state = ai.controller.debug_state(&game);
            let flags = ai.update(&game);
            if flags & INPUT_SWAP != 0 {
                last_swap = ticks;
            }
            if ticks % 25 == 0 {
                let mut rows = String::new();
                for y in (1..HEIGHT).rev() {
                    let line: String = (0..WIDTH).map(|x| {
                        let c = game.grid.get(x, y);
                        if c.is_garbage() { 'G' } else if c.is_block() {
                            let f = flavor_at(&game, x, y);
                            if c.is_static() { char::from(b'0' + f as u8) } else { char::from(b'a' + f as u8) }
                        } else { '.' }
                    }).collect();
                    if y <= game.grid.top_occupied_row.max(1) + 1 { rows.push_str(&format!("   {y:2} {line}\n")); }
                }
                history.push((ticks, format!("tick {ticks} cursor=({},{}) top={} flags={flags:#x}\n{state}\n{rows}", game.swapper.x, game.swapper.y, game.grid.top_effective_row)));
                if history.len() > 8 { history.remove(0); }
            }
            for bit in [INPUT_LEFT, INPUT_RIGHT, INPUT_UP, INPUT_DOWN, INPUT_SWAP, INPUT_ADVANCE] {
                input.set_key(bit, flags & bit != 0);
            }
            let snapshot = input.snapshot();
            input.end_frame();
            game.step(&snapshot);
            ticks += 1;
        }
        println!("seed {seed} {level:?}: status {:?} after {ticks} ticks, score {}, last swap at {last_swap}", game.status, game.score.value());
        let from: i32 = std::env::var("DEATH_FROM").ok().and_then(|v| v.parse().ok()).unwrap_or(0);
        for (t, text) in history.iter() {
            if *t >= from { println!("{text}"); }
        }
    }

    /// Diagnostic: how many solo games end with the bot standing still (no swap in the last
    /// 300 ticks), with and without `act_when_frozen`.
    #[test]
    #[ignore = "diagnostic"]
    fn idle_deaths() {
        for act in [false, true] {
            let (mut idle, mut total_ticks, mut score) = (0, 0, 0);
            let games = 60;
            for seed in 1..=games {
                let mut game = GameState::with_scoring(seed, ScoringMode::Classic);
                let mut ai = GridAutoplay::new(AIDifficulty::Hard, seed);
                ai.controller.adaptive.survival.act_when_frozen = act;
                ai.controller.adaptive.aggressive.act_when_frozen = act;
                ai.controller.tuning.act_when_frozen = act;
                let mut input = Input::new();
                let (mut ticks, mut last_swap) = (0, 0);
                while game.status == GameStatus::Playing && ticks < 40_000 {
                    let flags = ai.update(&game);
                    if flags & INPUT_SWAP != 0 { last_swap = ticks; }
                    for bit in [INPUT_LEFT, INPUT_RIGHT, INPUT_UP, INPUT_DOWN, INPUT_SWAP, INPUT_ADVANCE] {
                        input.set_key(bit, flags & bit != 0);
                    }
                    let snapshot = input.snapshot();
                    input.end_frame();
                    game.step(&snapshot);
                    ticks += 1;
                }
                if game.status != GameStatus::Playing && ticks - last_swap > 300 { idle += 1; if act { println!("idle death: seed {seed}, {ticks} ticks, last swap {last_swap}"); } }
                total_ticks += ticks;
                score += game.score.value();
            }
            println!("act_when_frozen={act}: idle deaths {idle}/{games}, mean survival {:.0} s, mean score {}", total_ticks as f64 / games as f64 / 50.0, score / games as i32);
        }
    }

    /// Diagnostic: A -> B -> A on one row. Counts the swaps that bring a row back to what it was
    /// two swaps earlier (the second one undoing the first, possibly through another pair).
    #[test]
    #[ignore = "diagnostic"]
    fn swap_back_and_forth() {
        use super::view::{flavor_at, WIDTH};
        let row_text = |game: &GameState, y: i32| -> String {
            (0..WIDTH).map(|x| {
                let c = game.grid.get(x, y);
                if c.is_garbage() { 'G' } else if c.is_block() { char::from(b'0' + flavor_at(game, x, y) as u8) } else { '.' }
            }).collect()
        };
        let (mut total_swaps, mut flips, mut games_with, mut shown) = (0, 0, 0, 0);
        for seed in 1..=60u32 {
            let mut game = GameState::with_scoring(seed, ScoringMode::Classic);
            let mut ai = GridAutoplay::new(AIDifficulty::Hard, seed);
            let mut input = Input::new();
            let mut ticks = 0;
            let mut log: Vec<(i32, i32, String, i32)> = Vec::new(); // x, y, row before, tick
            let mut game_flips = 0;
            while game.status == GameStatus::Playing && ticks < 20_000 {
                let flags = ai.update(&game);
                if flags & INPUT_SWAP != 0 && game.swapper.state == crate::swapper::SwapperState::Idle {
                    let (x, y) = (game.swapper.x, game.swapper.y);
                    let before = row_text(&game, y);
                    total_swaps += 1;
                    let n = log.len();
                    if n >= 2 && log[n - 1].1 == y && log[n - 2].1 == y && log[n - 2].2 == before && ticks - log[n - 2].3 < 400 {
                        flips += 1;
                        game_flips += 1;
                        if shown < 6 {
                            shown += 1;
                            println!("seed {seed} tick {ticks} row {y}: {} (swap x={}) -> {} (swap x={}) -> back to {} (swap x={x})", log[n - 2].2, log[n - 2].0, log[n - 1].2, log[n - 1].0, before);
                        }
                    }
                    log.push((x, y, before, ticks));
                }
                for bit in [INPUT_LEFT, INPUT_RIGHT, INPUT_UP, INPUT_DOWN, INPUT_SWAP, INPUT_ADVANCE] {
                    input.set_key(bit, flags & bit != 0);
                }
                let snapshot = input.snapshot();
                input.end_frame();
                game.step(&snapshot);
                ticks += 1;
            }
            if game_flips > 0 { games_with += 1; }
        }
        println!("{flips} back-and-forth swaps in {total_swaps} swaps; {games_with}/60 games");
    }

    /// Diagnostic: trace of the decisions around a tick of a game.
    #[test]
    #[ignore = "diagnostic"]
    fn trace_window() {
        use super::view::{flavor_at, HEIGHT, WIDTH};
        let seed: u32 = std::env::var("T_SEED").ok().and_then(|v| v.parse().ok()).unwrap_or(1);
        let (from, to): (i32, i32) = (
            std::env::var("T_FROM").ok().and_then(|v| v.parse().ok()).unwrap_or(1380),
            std::env::var("T_TO").ok().and_then(|v| v.parse().ok()).unwrap_or(1430),
        );
        let mut game = GameState::with_scoring(seed, ScoringMode::Classic);
        let mut ai = GridAutoplay::new(AIDifficulty::Hard, seed);
        let mut input = Input::new();
        for ticks in 0..to {
            if ticks == to - 1 {
                let board = ai.controller.board(&game);
                let cursor = (game.swapper.x, game.swapper.y);
                for c in candidates::trigger_swaps(&board, cursor, |_| false, |p| planner::can_swap(&board, p.x as usize, p.y as usize - 1)) {
                    println!("cand {:?} chain {} cleared {} shatter {} live-legal={}", c.plan, c.cascade.chain_depth, c.cascade.total_cleared, c.cascade.garbage_shattered, view::can_swap_now(&game, c.plan.x, c.plan.y));
                }
                println!("intent {:?} avoid-undo {:?}", ai.controller.plan_strategic(&game, board.clone(), cursor.0, cursor.1, view::Avoid::default(), false), ai.controller.recent_swaps.len());
            }
            let state = if ticks >= from { ai.controller.debug_state(&game) } else { String::new() };
            let flags = ai.update(&game);
            if ticks >= from && (flags != 0 || ticks % 10 == 0) {
                let rows: Vec<String> = (1..8).rev().map(|y| {
                    (0..WIDTH).map(|x| {
                        let c = game.grid.get(x, y);
                        if c.is_garbage() { 'G' } else if c.is_block() { char::from(b'0' + flavor_at(&game, x, y) as u8) } else { '.' }
                    }).collect::<String>()
                }).collect();
                println!("t{ticks} flags={flags:#04x} cursor=({},{}) {state} rows7..1={}", game.swapper.x, game.swapper.y, rows.join("/"));
            }
            for bit in [INPUT_LEFT, INPUT_RIGHT, INPUT_UP, INPUT_DOWN, INPUT_SWAP, INPUT_ADVANCE] {
                input.set_key(bit, flags & bit != 0);
            }
            let snapshot = input.snapshot();
            input.end_frame();
            game.step(&snapshot);
            let _ = HEIGHT;
        }
    }

    /// Diagnostic: back-and-forth swaps over 60 full-length games, with and without the row guard.
    #[test]
    #[ignore = "diagnostic"]
    fn flips_by_guard() {
        for guard in [false, true] {
            let total: usize = (1..=60).map(|s| flips_in(s, 20_000, guard)).sum();
            println!("row_undo_guard={guard}: {total} back-and-forth swaps in 60 games of up to 20000 ticks");
        }
    }

    /// Diagnostic: dead time. Plays many solo games and, whenever the bot does nothing for 75
    /// ticks in a row, classifies the moment: was there something to do (a swap that clears now,
    /// or in two swaps), and could the board be raised.
    #[test]
    #[ignore = "diagnostic"]
    fn dead_time() {
        use super::candidates::trigger_swaps;
        use super::planner::{can_swap, settled};
        use super::view::{board_motion, BoardMotion, WIDTH};
        use std::collections::BTreeMap;
        let games: u32 = std::env::var("DT_GAMES").ok().and_then(|v| v.parse().ok()).unwrap_or(100);
        let level = match std::env::var("DT_LEVEL").as_deref() {
            Ok("easy") => AIDifficulty::Easy,
            Ok("medium") => AIDifficulty::Medium,
            _ => AIDifficulty::Hard,
        };
        // category -> (stretches, dead ticks)
        let mut cats: BTreeMap<String, (u32, i64)> = BTreeMap::new();
        let mut last_cats: BTreeMap<String, (u32, i64)> = BTreeMap::new();
        let mut examples: BTreeMap<String, String> = BTreeMap::new();
        let (mut total_ticks, mut lost) = (0i64, 0);
        for seed in 1..=games {
            let mut game = GameState::with_scoring(seed, ScoringMode::Classic);
            let mut ai = GridAutoplay::new(level, seed);
            let mut input = Input::new();
            let mut idle = 0;
            let mut stretches: Vec<(i32, String, i32)> = Vec::new(); // start tick, category, length
            let mut ticks = 0;
            while game.status == GameStatus::Playing && ticks < 30_000 {
                let flags = ai.update(&game);
                if flags == 0 {
                    idle += 1;
                    if idle == 75 {
                        let motion = board_motion(&game);
                        let board = ai.controller.board(&game);
                        let cursor = (game.swapper.x, game.swapper.y);
                        let one = !trigger_swaps(&board, cursor, |_| false, |p| can_swap(&board, p.x as usize, p.y as usize - 1)).is_empty();
                        let mut two = false;
                        if !one {
                            'outer: for by in 0..board.height {
                                for x in 0..WIDTH as usize - 1 {
                                    if !can_swap(&board, x, by) || board.get(x, by) == board.get(x + 1, by) { continue; }
                                    let mut b2 = board.clone();
                                    let t = b2.get(x, by);
                                    b2.set(x, by, b2.get(x + 1, by));
                                    b2.set(x + 1, by, t);
                                    let b2 = settled(&b2);
                                    if !trigger_swaps(&b2, cursor, |_| false, |p| can_swap(&b2, p.x as usize, p.y as usize - 1)).is_empty() { two = true; break 'outer; }
                                }
                            }
                        }
                        let raise = !game.creep.creep_freeze && game.grid.top_effective_row < 44;
                        let current = format!("{:?} | clear-in-1={} clear-in-2={} | can-raise={} | top={}",
                            motion, one, two, raise, if game.grid.top_effective_row >= 10 { "high" } else { "low" });
                        let _ = BoardMotion::Settled;
                        if !examples.contains_key(&current) {
                            let mut rows = String::new();
                            for y in (1..=(game.grid.top_effective_row + 1).min(20)).rev() {
                                let line: String = (0..WIDTH).map(|x| {
                                    let c = game.grid.get(x, y);
                                    if c.is_garbage() { 'G' } else if c.is_block() { let f = super::view::flavor_at(&game, x, y); if c.is_static() { char::from(b'0' + f as u8) } else { char::from(b'a' + f as u8) } } else { '.' }
                                }).collect();
                                rows.push_str(&format!("      {y:2} {line}\n"));
                            }
                            examples.insert(current.clone(), format!("seed {seed} tick {ticks} cursor=({},{})\n{rows}", game.swapper.x, game.swapper.y));
                        }
                        stretches.push((ticks - 75, current.clone(), 75));
                    } else if idle > 75 {
                        if let Some(last) = stretches.last_mut() { last.2 += 1; }
                    }
                } else {
                    idle = 0;
                }
                for bit in [INPUT_LEFT, INPUT_RIGHT, INPUT_UP, INPUT_DOWN, INPUT_SWAP, INPUT_ADVANCE] {
                    input.set_key(bit, flags & bit != 0);
                }
                let snapshot = input.snapshot();
                input.end_frame();
                game.step(&snapshot);
                ticks += 1;
            }
            total_ticks += ticks as i64;
            let was_lost = game.status != GameStatus::Playing;
            if was_lost { lost += 1; }
            for (start, cat, len) in &stretches {
                let e = cats.entry(cat.clone()).or_default();
                e.0 += 1;
                e.1 += *len as i64;
                if was_lost && *start + *len >= ticks - 1000 {
                    let e = last_cats.entry(cat.clone()).or_default();
                    e.0 += 1;
                    e.1 += *len as i64;
                }
            }
        }
        println!("{level:?}: {games} games, {lost} ended, {total_ticks} ticks played");
        println!("--- all dead time (>= 75 ticks of nothing)");
        for (c, (n, t)) in &cats { println!("{n:5} stretches {t:7} ticks ({:.1}% of play)  {c}", *t as f64 * 100.0 / total_ticks as f64); }
        println!("--- the last 20 s before the end");
        for (c, (n, t)) in &last_cats { println!("{n:5} stretches {t:7} ticks  {c}"); }
        println!("--- one example per category");
        for (c, e) in &examples { println!("{c}\n{e}"); }
    }
}
