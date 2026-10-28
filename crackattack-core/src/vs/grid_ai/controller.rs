//! The decision loop of the grid-playing AI (port of `aiController.ts`).
//!
//! Port of aiController.ts from crack-attack-web (https://github.com/mishan/crack-attack-web),
//! original work Copyright (C) 2000 Daniel Nelson, GPL-2.0-or-later.
//!
//! It drives a real `GameState`: each tick it reads the board and the swap cursor and returns
//! the next input flags, so its play is fully visible. It is a pure, deterministic function of
//! the sim state, its own small plan/timer and an optional judgment seed (no clocks, no draws
//! from the gameplay RNG).
//!
//! The strategy is the one of the web version: *hard* evaluates cascades, builds short chains
//! and crossed-column x8/x10 combos and dismantles garbage supports; *medium* fires the chains,
//! combos and shatters it sees; *easy* only reacts to what is one swap away. A per-difficulty
//! think delay paces the visible reactions.
//!
//! Differences from the web version, on purpose:
//! - swap legality is the one of this simulator (`GameState::process_swap`), not a copy of the
//!   TypeScript checks;
//! - the "hold fire" and "garbage-aware danger" knobs, off in every tier of the web version
//!   (measured neutral-to-negative there), are not ported.

use crate::game_state::GameState;
use crate::input::INPUT_ADVANCE;

use super::planner::{BigComboSetupPlan, ChainSetupPlan, SetupPlan};
use super::memo::Memo;
use super::strategic::ChainSetupKey;
use super::tuning::*;
use super::view::*;

/// Derive a controller-only judgment seed for one seat. The gameplay RNG stays shared; this seed
/// only varies equally-valued planner choices.
pub(crate) fn decision_seed(match_seed: u32, seat: u32) -> u32 {
    let mut h = match_seed ^ seat.wrapping_add(1).wrapping_mul(0x9e37_79b1);
    h ^= h >> 16;
    h = h.wrapping_mul(0x85eb_ca6b);
    h ^= h >> 13;
    h = h.wrapping_mul(0xc2b2_ae35);
    h ^= h >> 16;
    h
}

pub(crate) struct AiController {
    /// The live tuning: the adaptive blend for the current threat.
    pub(super) tuning: AiTuning,
    pub(super) adaptive: Adaptive,
    /// Optional seed for reproducible variety among otherwise equal choices.
    pub(super) decision_seed: Option<u32>,
    /// The swapper debounces held keys (a move or swap only triggers on a fresh press), so the
    /// bot alternates a press with a neutral "release" tick. This flag says the next input must
    /// be the release.
    pub(super) release_next: bool,
    /// Ticks to wait after a swap before acting again (difficulty pacing).
    pub(super) cooldown: i32,
    /// Ticks of rest left after a cursor step (hand tempo).
    pub(super) move_rest: i32,
    /// Searches that depend on the board alone (and the options in their key): the chain
    /// enabler and the x8/x10 construction.
    pub(super) chain_memo: Memo<ChainSetupKey, ChainSetupPlan>,
    pub(super) big_combo_memo: Memo<(u32, i32), BigComboSetupPlan>,
    pub(super) clear_memo: Memo<(u32, i32), SetupPlan>,
    /// The last swap issued and the board it was issued on, to refuse an exact undo.
    pub(super) last_swap: Option<(SwapPlan, u32)>,
    /// The last few swaps as the rows they changed, to refuse a swap that restores a row (see
    /// `immediate_undo`).
    pub(super) recent_swaps: Vec<RecentSwap>,
}

/// A swap as it changed its row: the row after it, in plan-board cells.
#[derive(Clone, Debug)]
pub(super) struct RecentSwap {
    pub(super) x: i32,
    pub(super) after: Vec<i16>,
    pub(super) time_step: i32,
}

impl AiController {
    #[cfg(test)]
    pub(crate) fn new(tuning: AiTuning, decision_seed: Option<u32>) -> Self {
        Self::with_adaptive(Adaptive::fixed(tuning), decision_seed)
    }

    pub(crate) fn with_adaptive(adaptive: Adaptive, decision_seed: Option<u32>) -> Self {
        Self {
            tuning: adaptive.survival,
            adaptive,
            decision_seed,
            release_next: false,
            cooldown: 0,
            move_rest: 0,
            chain_memo: Memo::new(),
            big_combo_memo: Memo::new(),
            clear_memo: Memo::new(),
            last_swap: None,
            recent_swaps: Vec::new(),
        }
    }

    /// A candidate's value for comparing close options: the score in eighths of a cell, plus
    /// the height bias per grid row of the swap. Order-preserving while the bias is 0.
    pub(super) fn biased(&self, value: i32, row: i32) -> i32 {
        value * 8 + self.tuning.height_bias * row
    }

    /// Stable pseudo-random rank for an equal choice; 0 preserves scan order when unseeded.
    pub(super) fn tie_rank(&self, kind: u32, x: i32, y: i32) -> u32 {
        let Some(seed) = self.decision_seed else { return 0 };
        let mut h =
            seed ^ kind ^ ((x + 1) as u32).wrapping_mul(0x9e37_79b1) ^ ((y + 1) as u32).wrapping_mul(0x85eb_ca6b);
        h ^= h >> 16;
        h = h.wrapping_mul(0x7feb_352d);
        h ^= h >> 15;
        h = h.wrapping_mul(0x846c_a68b);
        h ^= h >> 16;
        h
    }

    /// The input flags to feed `GameState::step` this tick. Deterministic in `game`. Re-picks the
    /// nearest clearing swap every action tick (rather than committing to a stale plan) so the
    /// ever-rising board never leaves it walking to a target that has shifted away.
    ///
    /// One decision is: hold the keys if the hands are busy; adapt the tuning to the threat;
    /// look at what the board is doing; choose a target; send the hands toward it (or raise the
    /// board when there is nothing to do). The bot never stands still for lack of a cascade to
    /// trigger: while blocks fall or wake up it makes the plays it would make on a settled board
    /// ([`AiController::plan_meanwhile`]).
    pub(crate) fn decide(&mut self, game: &GameState) -> u8 {
        if let Some(held) = self.hold_keys(game) {
            return held;
        }
        self.adapt(game);
        if self.resting() {
            return 0;
        }
        let motion = board_motion(game);
        let mut board = if motion == BoardMotion::Gravity {
            read_gravity_plan_board(game)
        } else {
            self.board(game)
        };
        let avoid_undo = self.immediate_undo(game, &mut board);
        let (cursor_x, cursor_y) = (game.swapper.x, game.swapper.y);
        let intent = match motion {
            BoardMotion::Settled if self.tuning.strategic => self.plan_strategic(game, self.board(game), cursor_x, cursor_y, avoid_undo, false),
            BoardMotion::Settled => {
                self.plan_reactive(game, cursor_x, cursor_y, avoid_undo).map_or(Intent::Raise, Intent::Swap)
            }
            BoardMotion::Gravity => self.plan_in_fall(game, &board, avoid_undo).map_or(Intent::Wait, Intent::Swap),
            // Awaking blocks and moving garbage cannot be projected from a grid copy: take what
            // the live chain offers, if anything, else play meanwhile.
            BoardMotion::Other => self.plan_opportunistic(game, &board, &chain_blocks(game), avoid_undo).map_or(Intent::Wait, Intent::Swap),
        };
        let intent = match (motion, intent) {
            (BoardMotion::Settled, intent) => intent,
            (_, Intent::Swap(target)) => Intent::Swap(target),
            // Nothing to cash in while the board moves (and no live chain to protect): use the
            // time, on the board with its moving blocks as obstacles.
            (_, _) if self.holds_chain(game, motion) => Intent::Wait,
            (_, _) => self.plan_meanwhile(game, motion, avoid_undo),
        };
        match intent {
            Intent::Swap(target) => {
                // The board the undo test reads (not the one the plan may have been made on after
                // the fall): the next decision compares against the same kind of board.
                self.act_on(game, target, &board)
            }
            // A stable position with no useful swap is not a reason to wait for the slow
            // automatic creep: pull in the next row and give the planners fresh material.
            Intent::Raise if (motion == BoardMotion::Settled || self.tuning.raise_in_motion) && self.advance_allowed(game) && !game.creep.advance_latch && !game.creep.advance_held => INPUT_ADVANCE,
            // While the board moves, raise it only if `raise_in_motion` (the position is still supplied).
            Intent::Raise | Intent::Wait => 0,
        }
    }

    /// Why the bot is not acting (diagnostic for `arena`).
    #[cfg(test)]
    pub(crate) fn debug_state(&mut self, game: &GameState) -> String {
        let motion = match board_motion(game) {
            BoardMotion::Settled => "settled",
            BoardMotion::Gravity => "gravity",
            BoardMotion::Other => "other",
        };
        let plan = if motion == "settled" {
            matches!(self.plan_strategic(game, self.board(game), game.swapper.x, game.swapper.y, Avoid::default(), false), Intent::Swap(_))
        } else {
            false
        };
        let mut moving = Vec::new();
        for y in 1..HEIGHT {
            for x in 0..WIDTH {
                let cell = game.grid.get(x, y);
                if !cell.is_empty() && !cell.is_static() && moving.len() < 3 {
                    let left = block_of(game, cell).map(|b| b.animation_countdown - game.time_step);
                    moving.push(format!("{:?}@{x},{y}:p{}:left{:?}", cell.cell_type, cell.phase, left));
                }
            }
        }
        format!(
            "[{}] {motion} plan={plan} cd={} freeze={} latch={} swapper={:?} adv_ok={}",
            moving.join(" "),
            self.cooldown,
            game.creep.creep_freeze,
            game.creep.advance_latch,
            game.swapper.state,
            self.advance_allowed(game)
        )
    }

    /// Refresh the live tuning for the current threat (stack row + garbage on the board).
    pub(super) fn adapt(&mut self, game: &GameState) {
        let a = &self.adaptive;
        if !a.adapts() {
            return;
        }
        let threat_x6 = game.grid.top_effective_row * 6 + garbage_cells(game) * 6 / a.garbage_div.max(1);
        self.tuning = a.blend(a.mode(threat_x6));
    }
}

#[cfg(test)]
mod tests {
    use super::super::tuning::AiTuning;
    use super::*;
    use crate::constants::STEPS_PER_SECOND;
    use crate::input::{Input, INPUT_ADVANCE, INPUT_DOWN, INPUT_LEFT, INPUT_RIGHT, INPUT_SWAP, INPUT_UP};
    use crate::score::ScoringMode;

    /// Swaps issued while the board was in a transient the planners cannot project (awaking
    /// blocks, moving garbage), over a minute of solo play.
    fn swaps_during_transients(opportunism: bool) -> u32 {
        let mut tuning = AiTuning::for_difficulty(crate::ai_difficulty::AIDifficulty::Hard);
        tuning.opportunism = opportunism;
        let mut total = 0;
        for seed in 1..=5 {
            let mut game = GameState::with_scoring(seed, ScoringMode::Classic);
            let mut bot = AiController::new(tuning, Some(decision_seed(seed, 1)));
            let mut input = Input::new();
            for _ in 0..60 * STEPS_PER_SECOND {
                if game.status != crate::game_state::GameStatus::Playing {
                    break;
                }
                let in_transient = board_motion(&game) == BoardMotion::Other;
                let flags = bot.decide(&game);
                if in_transient && flags & INPUT_SWAP != 0 {
                    total += 1;
                }
                for key in [INPUT_LEFT, INPUT_RIGHT, INPUT_UP, INPUT_DOWN, INPUT_SWAP, INPUT_ADVANCE] {
                    input.set_key(key, flags & key != 0);
                }
                let snapshot = input.snapshot();
                input.end_frame();
                game.step(&snapshot);
            }
        }
        total
    }

    #[test]
    fn opportunism_plays_during_transients() {
        // Opportunism or not, the bot does not stand still while the board moves.
        for opportunism in [false, true] {
            assert!(
                swaps_during_transients(opportunism) > 0,
                "it never played while the board was moving (opportunism {opportunism})"
            );
        }
    }

    /// Pace report (ignored: a measurement, not a check): how fast the hands move and how long the
    /// bot stands still while the board is in motion. `PACE_LEVEL=easy|medium|hard`.
    #[test]
    #[ignore]
    fn pace_report() {
        let level = match std::env::var("PACE_LEVEL").as_deref() {
            Ok("easy") => crate::ai_difficulty::AIDifficulty::Easy,
            Ok("medium") => crate::ai_difficulty::AIDifficulty::Medium,
            _ => crate::ai_difficulty::AIDifficulty::Hard,
        };
        let tuning = AiTuning::for_difficulty(level)
            .with_overrides(&std::env::var("PACE_OVERRIDES").unwrap_or_default())
            .unwrap();
        let (mut ticks, mut by_motion, mut idle_by_motion) = (0u64, [0u64; 3], [0u64; 3]);
        let (mut swaps, mut moves, mut runs, mut run_cells, mut longest_idle) = (0u64, 0u64, 0u64, 0u64, 0u64);
        let (mut secs_total, mut idle_streaks, mut idle_streak_ticks) = (0.0f64, 0u64, 0u64);
        let mut motion_streak = 0u64;
        let mut pacing_ticks = 0u64;
        let mut motion_streaks: Vec<u64> = Vec::new();
        for seed in 1..=8u32 {
            let mut game = GameState::with_scoring(seed, ScoringMode::Classic);
            let mut bot = AiController::new(tuning, Some(decision_seed(seed, 1)));
            let mut input = Input::new();
            let (mut run, mut idle) = (0u64, 0u64);
            for _ in 0..120 * STEPS_PER_SECOND {
                if game.status != crate::game_state::GameStatus::Playing {
                    break;
                }
                let motion = board_motion(&game) as usize;
                let busy = bot.release_next
                    || game.creep.advance_latch
                    || game.creep.advance_held
                    || game.swapper.state == crate::swapper::SwapperState::Swapping;
                let pacing = bot.cooldown > 0;
                let flags = bot.decide(&game);
                ticks += 1;
                by_motion[motion] += 1;
                let dir = flags & (INPUT_LEFT | INPUT_RIGHT | INPUT_UP | INPUT_DOWN) != 0;
                let acting = flags != 0;
                if flags & INPUT_SWAP != 0 {
                    swaps += 1;
                }
                if dir {
                    moves += 1;
                    run += 1;
                } else if run > 0 && busy {
                    // release tick inside a walk
                } else if run > 0 {
                    runs += 1;
                    run_cells += run;
                    run = 0;
                }
                if pacing && !acting {
                    pacing_ticks += 1;
                }
                if !acting && !busy && !pacing {
                    idle_by_motion[motion] += 1;
                    idle += 1;
                } else {
                    if idle > 0 {
                        idle_streaks += 1;
                        idle_streak_ticks += idle;
                        longest_idle = longest_idle.max(idle);
                    }
                    idle = 0;
                }
                if motion != 0 {
                    motion_streak += 1;
                } else if motion_streak > 0 {
                    motion_streaks.push(motion_streak);
                    motion_streak = 0;
                }
                for key in [INPUT_LEFT, INPUT_RIGHT, INPUT_UP, INPUT_DOWN, INPUT_SWAP, INPUT_ADVANCE] {
                    input.set_key(key, flags & key != 0);
                }
                let snapshot = input.snapshot();
                input.end_frame();
                game.step(&snapshot);
            }
            secs_total += game.time_step as f64 / STEPS_PER_SECOND as f64;
        }
        let per_s = |n: u64| n as f64 / (ticks as f64 / STEPS_PER_SECOND as f64);
        let pct = |n: u64, d: u64| 100.0 * n as f64 / d.max(1) as f64;
        motion_streaks.sort();
        let median = motion_streaks.get(motion_streaks.len() / 2).copied().unwrap_or(0);
        println!("level {level:?}: {} s played", ticks / STEPS_PER_SECOND as u64);
        println!("cooldown after swaps: {:.0}% of the time (by design)", pct(pacing_ticks, ticks));
        println!("cursor presses/s {:.1} | swaps/s {:.2} | cells walked per action {:.1}", per_s(moves), per_s(swaps), run_cells as f64 / runs.max(1) as f64);
        println!(
            "board settled {:.0}% | gravity {:.0}% | other {:.0}% of the time",
            pct(by_motion[0], ticks), pct(by_motion[1], ticks), pct(by_motion[2], ticks)
        );
        println!(
            "bot idle (no key) in settled {:.0}% | gravity {:.0}% | other {:.0}% of those ticks; idle overall {:.0}%",
            pct(idle_by_motion[0], by_motion[0]), pct(idle_by_motion[1], by_motion[1]), pct(idle_by_motion[2], by_motion[2]),
            pct(idle_by_motion.iter().sum(), ticks)
        );
        println!(
            "motion episodes: {} (median {:.1} s); idle streaks {} (mean {:.2} s, longest {:.1} s)  [{secs_total:.0}]",
            motion_streaks.len(), median as f64 / 50.0,
            idle_streaks, idle_streak_ticks as f64 / idle_streaks.max(1) as f64 / 50.0, longest_idle as f64 / 50.0
        );
    }
}
