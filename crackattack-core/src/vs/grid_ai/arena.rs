//! Experiments on the bot's behaviour parameters. Slow, run by hand:
//!
//! `ARENA_SCENARIO=spire ARENA_VARIANTS='base;flatten_gap=2;advance_below_blocks=30' \
//!    cargo test --release -p crackattack-core arena -- --ignored --nocapture`
//!
//! A variant is a list of `name=value` overrides of the tier's tuning (`AiTuning::with_overrides`),
//! separated by `,`; variants are separated by `;`, and `base` is the tier unchanged. Speed
//! (`cooldown`) is deliberately not a parameter here: it is pacing, not behaviour.
//!
//! Scenarios (a lone bot unless said otherwise, same seeds for every variant):
//!  - `pressure`: a scripted garbage stream that speeds up; survival time and blocks cleared;
//!  - `spire`: a column left 5 to 7 rows above the rest at the start, no garbage; survival time;
//!  - `free`: no garbage received at all (own garbage leaves the board): how much garbage the bot
//!    sends and how many blocks it clears, the two objectives to maximise;
//!  - `solo`: a plain solo game, own garbage landing on itself; survival time and score;
//!  - `duel`: each variant against the unchanged tier, both seats; win rate.
//!
//! Every figure is a mean over `ARENA_SEEDS` seeds (default 100) with its 95% margin.

use super::tuning::{Adaptive, AiTuning};
use crate::constants::STEPS_PER_SECOND;
use crate::game_state::GameStatus;
use crate::grid::{BlockPhase, CellRef};
use crate::vs::grid_opponent::GridOpponent;
use crate::vs::opponent::Opponent;
use crate::ai_difficulty::AIDifficulty;

/// Game length cap in seconds (`ARENA_CAP`, default 300).
fn cap_seconds() -> i32 {
    std::env::var("ARENA_CAP").ok().and_then(|v| v.parse().ok()).unwrap_or(300)
}

fn seeds() -> u32 {
    std::env::var("ARENA_SEEDS").ok().and_then(|v| v.parse().ok()).unwrap_or(100)
}

/// First seed (`ARENA_FIRST_SEED`, default 1): a sweep can be confirmed on fresh seeds.
fn first_seed() -> u32 {
    std::env::var("ARENA_FIRST_SEED").ok().and_then(|v| v.parse().ok()).unwrap_or(1)
}

fn level() -> AIDifficulty {
    match std::env::var("ARENA_LEVEL").as_deref() {
        Ok("medium") => AIDifficulty::Medium,
        Ok("easy") => AIDifficulty::Easy,
        _ => AIDifficulty::Hard,
    }
}

fn variants() -> Vec<String> {
    // `ARENA_SWEEP='advance_limit=4..12'` (optionally `..:2` for a step) tries every value of
    // one parameter; extra fixed overrides can be added with `ARENA_FIXED='zone_above=0'`.
    if let Ok(sweep) = std::env::var("ARENA_SWEEP") {
        let (name, range) = sweep.split_once('=').expect("ARENA_SWEEP=name=lo..hi[:step]");
        let (range, step) = range.split_once(':').map_or((range, 1), |(r, s)| (r, s.parse().expect("step")));
        let (lo, hi) = range.split_once("..").expect("lo..hi");
        let (lo, hi): (i32, i32) = (lo.parse().expect("lo"), hi.parse().expect("hi"));
        let fixed = std::env::var("ARENA_FIXED").unwrap_or_default();
        let sep = if fixed.is_empty() { "" } else { "," };
        let mut out = vec!["base".to_string()];
        out.extend((lo..=hi).step_by(step as usize).map(|v| format!("{fixed}{sep}{name}={v}")));
        return out;
    }
    let spec = std::env::var("ARENA_VARIANTS").unwrap_or_else(|_| "base".to_string());
    spec.split(';').map(|v| v.trim().to_string()).filter(|v| !v.is_empty()).collect()
}

/// Mean and 95% margin of a sample.
fn mean_margin(values: &[f64]) -> (f64, f64) {
    let n = values.len() as f64;
    let mean = values.iter().sum::<f64>() / n;
    let var = values.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / (n - 1.0).max(1.0);
    (mean, 1.96 * (var / n).sqrt())
}

/// Scripted attacker: a slab of 2 to 4 rows every `interval` seconds, shrinking from 12 s to
/// 4 s over the first three minutes, sizes drawn from a seed-only stream.
fn garbage_at(tick: i32, state: &mut u32, next_at: &mut i32) -> Option<(i32, i32, i32)> {
    if tick < *next_at {
        return None;
    }
    *state = state.wrapping_mul(1_103_515_245).wrapping_add(12_345);
    let rows = 2 + ((*state >> 16) % 3) as i32;
    let seconds = (tick / STEPS_PER_SECOND).min(180);
    *next_at = tick + (12 - 8 * seconds / 180) * STEPS_PER_SECOND;
    Some((rows, 6, 0))
}

/// Leaves one column 5 to 7 rows above its neighbours (alternating two flavours so nothing
/// matches by itself).
fn raise_spire(bot: &mut GridOpponent, seed: u32) {
    let game = bot.game_mut_for_experiments();
    let column = (seed % 6) as i32;
    let top = (1..crate::constants::PLAY_HEIGHT as i32)
        .rev()
        .find(|&y| !game.grid.get(column, y).is_empty())
        .unwrap_or(0);
    let (a, b) = ((seed / 6 % 4) as i32, ((seed / 6 + 1 + seed / 24 % 3) % 4) as i32);
    for k in 1..=(5 + (seed / 3 % 3) as i32) {
        let y = top + k;
        let flavor = if k % 2 == 1 { a } else { b };
        if let Some(id) = game.blocks.allocate(column, y, flavor) {
            game.grid.set(column, y, CellRef::block(id, BlockPhase::Static));
            game.grid.notify_impact(y, 1);
        }
    }
}

struct Run {
    seconds: f64,
    blocks: f64,
    score: f64,
    /// Garbage rows sent to the opponent over the game.
    garbage_sent: f64,
}

fn lone_run(scenario: &str, lvl: AIDifficulty, tuning: Adaptive, seed: u32) -> Run {
    let mut bot = GridOpponent::with_adaptive(lvl, tuning, seed);
    for _ in 0..200 {
        bot.countdown_step();
    }
    match scenario {
        "solo" => bot.game_mut_for_experiments().vs_mode = false,
        "spire" => raise_spire(&mut bot, seed),
        _ => {}
    }
    let (mut rng, mut next_at) = (seed ^ 0x9e37_79b9, 10 * STEPS_PER_SECOND);
    let cap = cap_seconds() * STEPS_PER_SECOND;
    let mut tick = 0;
    while tick < cap && bot.game().status == GameStatus::Playing {
        if scenario == "pressure" {
            if let Some((h, w, f)) = garbage_at(tick, &mut rng, &mut next_at) {
                bot.send_garbage(h, w, f);
            }
        }
        bot.play_step();
        bot.recv_garbage();
        tick += 1;
    }
    Run {
        seconds: tick as f64 / STEPS_PER_SECOND as f64,
        blocks: bot.game().stats.blocks_eliminated as f64,
        score: bot.game().score.value() as f64,
        garbage_sent: bot.game().stats.garbage_sent as f64,
    }
}

/// `Some(0)` if `a` wins, `Some(1)` if `b`, `None` for a draw or a timeout.
fn duel(lvl: AIDifficulty, ta: Adaptive, tb: Adaptive, seed: u32) -> Option<usize> {
    let mut bots = [GridOpponent::with_adaptive(lvl, ta, seed), GridOpponent::with_adaptive(lvl, tb, seed)];
    for bot in &mut bots {
        for _ in 0..200 {
            bot.countdown_step();
        }
    }
    for _ in 0..30_000 {
        for i in 0..2 {
            bots[i].play_step();
            for (h, w, f) in bots[i].recv_garbage() {
                bots[1 - i].send_garbage(h, w, f);
            }
        }
        match (bots[0].has_opponent_lost(), bots[1].has_opponent_lost()) {
            (true, false) => return Some(1),
            (false, true) => return Some(0),
            (true, true) => return None,
            _ => {}
        }
    }
    None
}

#[test]
#[ignore = "slow experiment"]
fn arena_run() {
    let scenario = std::env::var("ARENA_SCENARIO").unwrap_or_else(|_| "pressure".to_string());
    let (lvl, n) = (level(), seeds());
    let base_tuning = AiTuning::for_difficulty(lvl);
    // `ARENA_VS=medium|easy`: the duel opponent is another tier's unchanged play.
    let rival = match std::env::var("ARENA_VS").as_deref() {
        Ok("medium") => AIDifficulty::Medium,
        Ok("easy") => AIDifficulty::Easy,
        Ok("hard") => AIDifficulty::Hard,
        _ => lvl,
    };
    // `ARENA_RIVAL='a.advance_below_blocks=55,...'`: the opponent plays that variant instead of
    // the plain tier, so a candidate improvement is measured against the current best.
    let base = match std::env::var("ARENA_RIVAL") {
        Ok(spec) => Adaptive::from_spec(AiTuning::for_difficulty(rival), &spec).expect("ARENA_RIVAL"),
        Err(_) => Adaptive::fixed(AiTuning::for_difficulty(rival)),
    };
    println!("\n{} / {} / {} seeds (mean, 95% margin)", lvl.name(), scenario, n);
    if scenario == "duel" {
        println!("{:<52} {:>14}", "variant", "wins vs base");
    } else {
        println!("{:<44} {:>13} {:>11} {:>11} {:>11}", "variant", "survived (s)", "blocks", "garbage", "score");
    }
    for spec in variants() {
        let tuning = match Adaptive::from_spec(base_tuning, &spec) {
            Ok(t) => t,
            Err(e) => {
                println!("{spec}: {e}");
                continue;
            }
        };
        if scenario == "duel" {
            let (mut wins, mut played) = (0, 0);
            for seed in first_seed()..first_seed() + n {
                for swap in [false, true] {
                    let w = if swap { duel(lvl, base, tuning, seed).map(|i| 1 - i) } else { duel(lvl, tuning, base, seed) };
                    played += 1;
                    wins += (w == Some(0)) as u32;
                }
            }
            // 95% margin of a proportion near one half.
            let margin = 0.98 / (played as f64).sqrt() * 100.0;
            let pct = wins as f64 / played as f64 * 100.0;
            let bar = "#".repeat((pct / 2.0).round() as usize);
            println!("{spec:<40} {wins:>4}/{played} {pct:>5.1}% ±{margin:.1}  {bar}");
            continue;
        }
        let runs: Vec<Run> = (first_seed()..first_seed() + n).map(|s| lone_run(&scenario, lvl, tuning, s)).collect();
        let (t, tm) = mean_margin(&runs.iter().map(|r| r.seconds).collect::<Vec<_>>());
        let (b, bm) = mean_margin(&runs.iter().map(|r| r.blocks).collect::<Vec<_>>());
        let (sc, scm) = mean_margin(&runs.iter().map(|r| r.score).collect::<Vec<_>>());
        let (g, gm) = mean_margin(&runs.iter().map(|r| r.garbage_sent).collect::<Vec<_>>());
        println!("{spec:<44} {t:>7.0} ±{tm:<4.0} {b:>5.0} ±{bm:<3.0} {g:>5.1} ±{gm:<3.1} {sc:>5.0} ±{scm:<3.0}");
    }
}

/// Where the bot dies under `pressure`: a timeline (every 2 s, the last 24 s before the end) of
/// stack row, garbage cells and block cells. `ARENA_VARIANTS` takes one variant.
#[test]
#[ignore = "slow experiment"]
fn arena_death_diagnostic() {
    let lvl = level();
    let base = AiTuning::for_difficulty(lvl);
    let spec = variants().remove(0);
    let adaptive = Adaptive::from_spec(base, &spec).expect("variant");
    for seed in first_seed()..first_seed() + seeds() {
        let mut bot = GridOpponent::with_adaptive(lvl, adaptive, seed);
        for _ in 0..200 {
            bot.countdown_step();
        }
        let (mut rng, mut next_at) = (seed ^ 0x9e37_79b9, 10 * STEPS_PER_SECOND);
        let mut timeline: Vec<(i32, i32, i32, i32)> = Vec::new();
        let mut tick = 0;
        while tick < cap_seconds() * STEPS_PER_SECOND && bot.game().status == GameStatus::Playing {
            if let Some((h, w, f)) = garbage_at(tick, &mut rng, &mut next_at) {
                bot.send_garbage(h, w, f);
            }
            bot.play_step();
            bot.recv_garbage();
            if let Ok(window) = std::env::var("ARENA_TRACE") {
                // `ARENA_TRACE=seed:from:to` prints the bot's state every second in that window.
                let parts: Vec<i32> = window.split(':').filter_map(|v| v.parse().ok()).collect();
                if parts[0] == seed as i32 && tick % STEPS_PER_SECOND == 0 && (parts[1]..parts[2]).contains(&(tick / STEPS_PER_SECOND)) {
                    let row = bot.game().grid.top_effective_row;
                    println!("  t={}s row={} {}", tick / STEPS_PER_SECOND, row, bot.debug_state());
                }
            }
            if tick % (2 * STEPS_PER_SECOND) == 0 {
                let g = &bot.game().grid;
                let (mut garbage, mut blocks) = (0, 0);
                for y in 1..crate::constants::PLAY_HEIGHT as i32 {
                    for x in 0..crate::constants::PLAY_WIDTH as i32 {
                        garbage += g.get(x, y).is_garbage() as i32;
                        blocks += g.get(x, y).is_block() as i32;
                    }
                }
                timeline.push((tick / STEPS_PER_SECOND, g.top_effective_row, garbage, blocks));
            }
            tick += 1;
        }
        let tail: Vec<String> = timeline.iter().rev().take(12).rev().map(|(t, r, g, b)| format!("{t}s:r{r}/g{g}/b{b}")).collect();
        println!("seed {seed} died at {}s | {}", tick / STEPS_PER_SECOND, tail.join(" "));
    }
}

/// Hash of every input the bot issues over `seconds` of play (with the scripted attacker when
/// `attacked`). Behaviour-preserving refactors must leave these unchanged.
fn decision_hash(lvl: AIDifficulty, spec: &str, seed: u32, seconds: i32, attacked: bool) -> u64 {
    let adaptive = Adaptive::from_spec(AiTuning::for_difficulty(lvl), spec).expect("spec");
    let mut bot = GridOpponent::with_adaptive(lvl, adaptive, seed);
    for _ in 0..200 {
        bot.countdown_step();
    }
    let (mut rng, mut next_at) = (seed ^ 0x9e37_79b9, 6 * STEPS_PER_SECOND);
    for tick in 0..seconds * STEPS_PER_SECOND {
        if bot.game().status != GameStatus::Playing {
            break;
        }
        if attacked {
            if let Some((h, w, f)) = garbage_at(tick, &mut rng, &mut next_at) {
                bot.send_garbage(h, w, f);
            }
        }
        bot.play_step();
        bot.recv_garbage();
    }
    bot.decision_hash
}

/// The configurations the characterisation test covers.
const GOLDEN_CASES: &[(&str, &str, bool)] = &[
    ("hard", "base,move_gap=0,move_jitter=0", false),
    ("hard", "base,move_gap=0,move_jitter=0", true),
    ("medium", "base,move_gap=0,move_jitter=0", true),
    ("easy", "base,move_gap=0,move_jitter=0", true),
    ("hard", "a.advance_below_blocks=55,mode_lo=8,mode_hi=12,move_gap=0,move_jitter=0", true),
    ("hard", "a.advance_below_blocks=55,mode_lo=8,mode_hi=12,move_gap=0,move_jitter=0", false),
    ("hard", "opportunism=0,opportunism_linked=0,pressure_garbage_cells=6,fire_min_chain=3,move_gap=0,move_jitter=0", true),
    ("hard", "undermine=0,chain_setup=0,big_combo_setup_max_cost=0,danger_margin=5,move_gap=0,move_jitter=0", true),
    ("hard", "advance_below_blocks=40,fire_min_run=5,flatten=0,move_gap=0,move_jitter=0", false),
    ("medium", "pressure_setups=4,shatter_setup_max_cost=6,move_gap=0,move_jitter=0", true),
];

fn level_named(name: &str) -> AIDifficulty {
    match name {
        "easy" => AIDifficulty::Easy,
        "medium" => AIDifficulty::Medium,
        _ => AIDifficulty::Hard,
    }
}

#[test]
#[ignore = "slow: prints the golden hashes"]
fn print_golden_decisions() {
    for (i, (lvl, spec, attacked)) in GOLDEN_CASES.iter().enumerate() {
        let hashes: Vec<String> = [3u32, 11].iter().map(|&seed| format!("{:016x}", decision_hash(level_named(lvl), spec, seed, 90, *attacked))).collect();
        println!("GOLDEN {i} {}", hashes.join(" "));
    }
}

const GOLDEN_HASHES: [[u64; 2]; 10] = [
    [0xe297ac0410e9f4f1, 0x78c357d13cdaea00],
    [0x7837687098873de9, 0x5f63b86564d855bb],
    [0x5e9e2cae3fe51471, 0x7c381d59a606fe01],
    [0x3357c25cb56dfda3, 0x15131f475f8b885e],
    [0xef6f10d472a001ae, 0x939bd8ba895ff959],
    [0x149265f0ec3d0bcc, 0x5ebdd286a8f8b051],
    [0x20058164c733a32a, 0xc0dfa018d632d589],
    [0x6f7cc062dfd060f5, 0x37277040f86b3e93],
    [0x59dff82124b0a7aa, 0x4e0756bbd68be029],
    [0xc05c59b20c5979c6, 0xd2a764215771137e],
];

/// Characterisation: the bot's inputs on fixed games. A refactor must not move them; a change of
/// behaviour on purpose updates the constants (`print_golden_decisions` prints them).
#[test]
fn decisions_are_unchanged() {
    for (i, (lvl, spec, attacked)) in GOLDEN_CASES.iter().enumerate() {
        for (k, &seed) in [3u32, 11].iter().enumerate() {
            let got = decision_hash(level_named(lvl), spec, seed, 90, *attacked);
            assert_eq!(got, GOLDEN_HASHES[i][k], "case {i} ({lvl} {spec}) seed {seed}: {got:016x}");
        }
    }
}

/// The old autoplay (`crate::ai`) as a duel player: a real game, the same garbage rules and
/// clock as [`GridOpponent`], driven by `AIController` instead.
struct OldAutoplay {
    game: crate::game_state::GameState,
    controller: crate::ai::AIController,
    input: crate::input::Input,
    outgoing: Vec<(i32, i32, i32)>,
    lost: bool,
}

impl OldAutoplay {
    /// `seed` is the session seed, as for `GridOpponent::new` (first round: seed + 1000).
    fn new(seed: u32) -> Self {
        let mut game = crate::game_state::GameState::with_scoring(seed.wrapping_add(1000), crate::score::ScoringMode::Classic);
        game.vs_mode = true;
        Self { game, controller: crate::ai::AIController::new(), input: crate::input::Input::new(), outgoing: Vec::new(), lost: false }
    }

    fn countdown_step(&mut self) {
        self.game.creep.tick_acceleration_only();
        self.game.time_step += 1;
    }

    fn play_step(&mut self) {
        use crate::input::{INPUT_ADVANCE, INPUT_DOWN, INPUT_LEFT, INPUT_RIGHT, INPUT_SWAP, INPUT_UP};
        if self.game.status != GameStatus::Playing {
            return;
        }
        let flags = self.controller.update(&self.game);
        for bit in [INPUT_LEFT, INPUT_RIGHT, INPUT_UP, INPUT_DOWN, INPUT_SWAP, INPUT_ADVANCE] {
            self.input.set_key(bit, flags & bit != 0);
        }
        let snapshot = self.input.snapshot();
        self.input.end_frame();
        self.game.step(&snapshot);
        self.outgoing.extend(self.game.outgoing_garbage.drain(..));
        self.lost |= self.game.status == GameStatus::Lost;
    }

    fn send_garbage(&mut self, height: i32, width: i32, flavor: i32) {
        let time_step = self.game.time_step;
        self.game.garbage_generator.add_to_queue(height, width, flavor, time_step, Some(&mut self.game.rng));
    }
}

/// Cortex (seat `cortex_seat`) against the old autoplay: `Some(true)` if Cortex wins,
/// `Some(false)` if the old autoplay does, `None` for a draw or a timeout.
fn duel_vs_old(cortex: Adaptive, cortex_seat: usize, seed: u32) -> Option<bool> {
    let mut hard = GridOpponent::with_adaptive(AIDifficulty::Hard, cortex, seed);
    let mut old = OldAutoplay::new(seed);
    for _ in 0..200 {
        hard.countdown_step();
        old.countdown_step();
    }
    for tick in 0..30_000 {
        if tick % 3000 == 2999 && std::env::var("ARENA_TRACE").is_ok() {
            println!(
                "TRACE seed {seed} t={}s old: row {} swaps {} garbage_sent {} | cortex: row {} garbage_sent {}",
                tick / 50 + 1,
                old.game.grid.top_effective_row,
                old.game.stats.swaps_performed,
                old.game.stats.garbage_sent,
                hard.game().grid.top_effective_row,
                hard.game().stats.garbage_sent
            );
        }
        // Seat order only decides who steps first within a tick.
        for turn in 0..2 {
            if (turn == 0) == (cortex_seat == 0) {
                hard.play_step();
                for (h, w, f) in hard.recv_garbage() {
                    old.send_garbage(h, w, f);
                }
            } else {
                old.play_step();
                for (h, w, f) in old.outgoing.drain(..) {
                    hard.send_garbage(h, w, f);
                }
            }
        }
        match (hard.has_opponent_lost(), old.lost) {
            (true, false) => return Some(false),
            (false, true) => return Some(true),
            (true, true) => return None,
            _ => {}
        }
    }
    None
}

/// Cortex (the shipped Hard, or `ARENA_VARIANTS`' first variant) against the old autoplay.
#[test]
#[ignore = "slow experiment"]
fn arena_vs_old_autoplay() {
    let cortex = match variants().first().map(String::as_str) {
        Some(spec) if spec != "base" => Adaptive::from_spec(AiTuning::for_difficulty(AIDifficulty::Hard), spec).expect("variant"),
        _ => Adaptive::for_difficulty(AIDifficulty::Hard),
    };
    let (mut cortex_wins, mut old_wins, mut draws) = (0, 0, 0);
    for seed in first_seed()..first_seed() + seeds() {
        for seat in [0, 1] {
            match duel_vs_old(cortex, seat, seed) {
                Some(true) => cortex_wins += 1,
                Some(false) => old_wins += 1,
                None => draws += 1,
            }
        }
    }
    println!("OLDVS cortex {cortex_wins} old {old_wins} draws {draws}");
}

/// The real solo game as `--autoplay` plays it (a plain `GameState`, no garbage), one line per
/// variant: mean score, time and blocks a minute over `ARENA_SEEDS` seeds, capped at `ARENA_CAP`
/// seconds (default 600). `ARENA_LEVEL` and `ARENA_VARIANTS` as usual.
#[test]
#[ignore = "slow: solo score of the autoplay"]
fn solo_score() {
    use crate::game_state::GameState;
    use crate::input::Input;
    use crate::score::ScoringMode;
    use crate::vs::grid_ai::controller::{decision_seed, AiController};
    let cap_ticks = std::env::var("ARENA_CAP").ok().and_then(|v| v.parse::<i32>().ok()).unwrap_or(600) * STEPS_PER_SECOND;
    let first = first_seed();
    for spec in variants() {
        // `shipped` is the tier as it ships (Hard adapts); `shipped,name=v` overrides both sides.
        let adaptive = match spec.strip_prefix("shipped") {
            Some(rest) => {
                let mut a = Adaptive::for_difficulty(level());
                let extra = rest.trim_start_matches(',');
                a.survival = a.survival.with_overrides(extra).expect("variant");
                a.aggressive = a.aggressive.with_overrides(extra).expect("variant");
                a
            }
            None => Adaptive::from_spec(AiTuning::for_difficulty(level()), &spec).expect("variant"),
        };
        let handles: Vec<_> = (first..first + seeds())
            .map(|seed| {
                std::thread::spawn(move || {
                    let mut game = GameState::with_scoring(seed, ScoringMode::Classic);
                    let mut ai = AiController::with_adaptive(adaptive, Some(decision_seed(seed, 1)));
                    let mut input = Input::new();
                    let mut ticks = 0;
                    while game.status == GameStatus::Playing && ticks < cap_ticks {
                        let flags = ai.decide(&game);
                        for bit in [crate::input::INPUT_LEFT, crate::input::INPUT_RIGHT, crate::input::INPUT_UP, crate::input::INPUT_DOWN, crate::input::INPUT_SWAP, crate::input::INPUT_ADVANCE] {
                            input.set_key(bit, flags & bit != 0);
                        }
                        let snapshot = input.snapshot();
                        input.end_frame();
                        game.step(&snapshot);
                        ticks += 1;
                    }
                    let secs = ticks as f64 / STEPS_PER_SECOND as f64;
                    (game.score.value() as f64, secs, game.stats.blocks_eliminated as f64 / (secs / 60.0))
                })
            })
            .collect();
        let runs: Vec<(f64, f64, f64)> = handles.into_iter().map(|h| h.join().unwrap()).collect();
        let col = |f: &dyn Fn(&(f64, f64, f64)) -> f64| mean_margin(&runs.iter().map(f).collect::<Vec<_>>());
        let (s, sm) = col(&|r| r.0);
        let (t, tm) = col(&|r| r.1);
        let (b, bm) = col(&|r| r.2);
        println!("{spec:<44} score {s:6.1}±{sm:<5.1} time {t:6.1}±{tm:<5.1} blk/min {b:5.1}±{bm:<4.1}");
    }
}
