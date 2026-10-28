//! Grid opponent: the VS Computer opponent that really plays.
//!
//! Instead of the gridless `VirtualPlayer` (a garbage queue and a timer), it owns a real
//! `GameState` driven by the grid-playing AI of [`super::grid_ai`]. The player's garbage lands
//! on its board, its attacks come from the chains and combos it actually builds, and it loses
//! when its stack tops out, exactly as in a network game. The session sees its name, its level,
//! its garbage, and the simplified picture of its board it publishes for the HUD preview.
//!
//! The simulation advances only while the round is played (`play_step`), plus the countdown
//! bookkeeping the player's own game does (`countdown_step`), so both boards stay on the same
//! clock. The other phases (`time_step`) leave it frozen.

use std::collections::VecDeque;

use crate::game_state::{GameState, GameStatus};
use crate::input::{
    Input, InputSnapshot, INPUT_ADVANCE, INPUT_DOWN, INPUT_LEFT, INPUT_RIGHT, INPUT_SWAP, INPUT_UP,
};
use crate::score::ScoringMode;
use crate::ai_difficulty::AIDifficulty;

use super::board_view::{BoardViewPublisher, OpponentBoardView};
use super::grid_ai::controller::{decision_seed, AiController};
use super::grid_ai::tuning::Adaptive;
use super::opponent::Opponent;

/// The seat of the bot in the judgment seed (the player is seat 0).
const BOT_SEAT: u32 = 1;

/// AI opponent that plays a real game on its own `GameState`.
pub struct GridOpponent {
    difficulty: AIDifficulty,
    adaptive: Adaptive,
    /// Seed of the session; each round derives its own, like the player's game.
    seed_base: u32,
    /// Rounds prepared so far (1-based once the first round is prepared).
    round: u32,
    /// True from construction until the session's first `game_start`: the constructor has
    /// already prepared round 1, and the session preparing that same round must not move on to 2.
    fresh: bool,
    game: GameState,
    controller: AiController,
    /// Edge detection for the controller's presses, as for a human keyboard.
    input: Input,
    /// Garbage the bot produced this tick, waiting for the session to collect it.
    outgoing: VecDeque<(i32, i32, i32)>,
    /// Decides when a new picture of the board goes out, as a network peer will.
    board_publisher: BoardViewPublisher,
    /// Last published picture of the board.
    board_view: OpponentBoardView,
    loss_timestamp: Option<i32>,
    /// Round ready signal: the bot is always immediately ready.
    round_ready: u32,
    /// FNV-1a over every input the controller issued (characterisation tests).
    #[cfg(test)]
    pub(crate) decision_hash: u64,
}

impl GridOpponent {
    /// `seed` is the session seed (the one given to the player's game).
    pub fn new(difficulty: AIDifficulty, seed: u32) -> Self {
        Self::with_adaptive(difficulty, Adaptive::for_difficulty(difficulty), seed)
    }

    #[cfg(test)]
    pub(crate) fn debug_state(&mut self) -> String {
        self.controller.debug_state(&self.game)
    }

    pub(crate) fn with_adaptive(difficulty: AIDifficulty, adaptive: Adaptive, seed: u32) -> Self {
        let mut opponent = Self {
            difficulty,
            adaptive,
            seed_base: seed,
            round: 0,
            fresh: false,
            game: GameState::with_scoring(seed, ScoringMode::Classic),
            controller: AiController::with_adaptive(adaptive, None),
            input: Input::new(),
            outgoing: VecDeque::new(),
            board_publisher: BoardViewPublisher::new(),
            board_view: OpponentBoardView::empty(),
            loss_timestamp: None,
            round_ready: 0,
            #[cfg(test)]
            decision_hash: 0xcbf2_9ce4_8422_2325,
        };
        opponent.game_start();
        opponent.fresh = true;
        opponent
    }

    /// Experiments only: the bot's game, to shape a scenario or to play it alone.
    #[cfg(test)]
    pub(crate) fn game_mut_for_experiments(&mut self) -> &mut GameState {
        &mut self.game
    }

    pub fn difficulty(&self) -> AIDifficulty {
        self.difficulty
    }

    /// The bot's game, read-only (tests).
    pub fn game(&self) -> &GameState {
        &self.game
    }

    fn input_snapshot(&mut self, flags: u8) -> InputSnapshot {
        for bit in [INPUT_LEFT, INPUT_RIGHT, INPUT_UP, INPUT_DOWN, INPUT_SWAP, INPUT_ADVANCE] {
            self.input.set_key(bit, flags & bit != 0);
        }
        let snapshot = self.input.snapshot();
        self.input.end_frame();
        snapshot
    }
}

impl Opponent for GridOpponent {
    fn game_start(&mut self) {
        // The constructor prepared round 1; the session's own preparation of round 1 repeats
        // it (same seed, a clean board) instead of skipping to round 2, whose seed the
        // player's game does not have.
        if std::mem::take(&mut self.fresh) {
            // round stays 1
        } else {
            self.round += 1;
        }
        // Same seed as the player's game of this round (`VsSession::prepare_round`): both
        // boards start identical.
        let seed = self.seed_base.wrapping_add(self.round.wrapping_mul(1000));
        self.game = GameState::with_scoring(seed, ScoringMode::Classic);
        self.game.vs_mode = true;
        self.controller = AiController::with_adaptive(
            self.adaptive,
            Some(decision_seed(seed, BOT_SEAT)),
        );
        self.input.reset();
        self.outgoing.clear();
        self.board_view = self.board_publisher.start(&self.game);
        self.loss_timestamp = None;
    }

    /// Phases where the round is not played (ready prompt, result, pause): nothing moves.
    fn time_step(&mut self) {}

    /// Mirrors what the session does to the player's game during the countdown: the creep's
    /// acceleration timer and the tick counter advance, the board does not.
    fn countdown_step(&mut self) {
        self.game.creep.tick_acceleration_only();
        self.game.time_step += 1;
    }

    fn play_step(&mut self) {
        if self.game.status != GameStatus::Playing {
            return;
        }
        let flags = self.controller.decide(&self.game);
        #[cfg(test)]
        {
            self.decision_hash = (self.decision_hash ^ u64::from(flags)).wrapping_mul(0x0100_0000_01b3);
        }
        let input = self.input_snapshot(flags);
        self.game.step(&input);
        self.outgoing.extend(self.game.outgoing_garbage.drain(..));
        if let Some(view) = self.board_publisher.step(&self.game) {
            self.board_view = view;
        }
        if self.game.status == GameStatus::Lost && self.loss_timestamp.is_none() {
            self.loss_timestamp = Some(self.game.time_step);
        }
    }

    fn final_stats(&self) -> Option<&crate::stats::GameStats> {
        Some(&self.game.stats)
    }

    fn send_garbage(&mut self, height: i32, width: i32, flavor: i32) {
        // Same landing rule as the player's own game (`VsSession::time_step`): an initial drop
        // delay of the average plus a random spread drawn from the simulation RNG.
        let time_step = self.game.time_step;
        self.game.garbage_generator.add_to_queue(height, width, flavor, time_step, Some(&mut self.game.rng));
    }

    fn recv_garbage(&mut self) -> Vec<(i32, i32, i32)> {
        self.outgoing.drain(..).collect()
    }

    /// Level lights, as for a network opponent: the height of its stack.
    fn grid_level(&mut self) -> i32 {
        self.game.grid.top_effective_row.clamp(0, 12)
    }

    fn has_opponent_lost(&self) -> bool {
        self.loss_timestamp.is_some()
    }

    fn opponent_loss_timestamp(&self) -> Option<i32> {
        self.loss_timestamp
    }

    fn opponent_name(&self) -> &str {
        self.difficulty_name()
    }

    fn board_view(&self) -> Option<&OpponentBoardView> {
        Some(&self.board_view)
    }

    fn set_local_round_ready(&mut self, round: u32) {
        self.round_ready = round;
    }

    fn opponent_round_ready(&self) -> u32 {
        self.round_ready
    }
}

impl GridOpponent {
    fn difficulty_name(&self) -> &'static str {
        self.difficulty.name()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::constants::STEPS_PER_SECOND;

    /// Runs the bot alone through the countdown and `ticks` played ticks, with no opponent
    /// garbage, and returns it.
    fn run_alone(difficulty: AIDifficulty, seed: u32, ticks: i32) -> GridOpponent {
        let mut bot = GridOpponent::new(difficulty, seed);
        for _ in 0..200 {
            bot.countdown_step();
        }
        for _ in 0..ticks {
            bot.play_step();
        }
        bot
    }

    /// Two bots on one clock, cross-wired like the session does: each one's outgoing garbage
    /// lands on the other. Returns `(winner, ticks)`, `winner` being 0 for `a`, 1 for `b`,
    /// `None` on a draw or when `max_ticks` ran out.
    fn duel(a: AIDifficulty, b: AIDifficulty, seed: u32, max_ticks: i32) -> (Option<usize>, i32) {
        let mut bots = [GridOpponent::new(a, seed), GridOpponent::new(b, seed)];
        for bot in &mut bots {
            for _ in 0..200 {
                bot.countdown_step();
            }
        }
        for tick in 0..max_ticks {
            for i in 0..2 {
                bots[i].play_step();
                for (h, w, f) in bots[i].recv_garbage() {
                    bots[1 - i].send_garbage(h, w, f);
                }
            }
            match (bots[0].has_opponent_lost(), bots[1].has_opponent_lost()) {
                (true, false) => return (Some(1), tick),
                (false, true) => return (Some(0), tick),
                (true, true) => return (None, tick),
                _ => {}
            }
        }
        (None, max_ticks)
    }

    #[test]
    fn bot_plays_real_moves() {
        // One minute of play: it swaps, clears blocks and scores.
        let bot = run_alone(AIDifficulty::Hard, 7, 60 * STEPS_PER_SECOND);
        assert!(bot.game().score.value() > 0, "the bot never cleared anything");
        assert!(bot.game().stats.swaps_performed > 10, "the bot barely moved");
    }

    #[test]
    fn bot_is_deterministic() {
        // Same seed, same moves: the same board and score tick for tick.
        let a = run_alone(AIDifficulty::Medium, 11, 40 * STEPS_PER_SECOND);
        let b = run_alone(AIDifficulty::Medium, 11, 40 * STEPS_PER_SECOND);
        assert_eq!(a.game().score.value(), b.game().score.value());
        assert_eq!(a.game().time_step, b.game().time_step);
        assert_eq!(a.game().grid.top_effective_row, b.game().grid.top_effective_row);
    }

    #[test]
    fn frozen_outside_play() {
        // `time_step` (ready prompt, result, pause) leaves the board untouched.
        let mut bot = GridOpponent::new(AIDifficulty::Easy, 3);
        let before = bot.game().time_step;
        for _ in 0..500 {
            bot.time_step();
        }
        assert_eq!(bot.game().time_step, before);
        for _ in 0..200 {
            bot.countdown_step();
        }
        assert_eq!(bot.game().time_step, before + 200);
    }

    #[test]
    fn incoming_garbage_lands_on_its_board() {
        let mut bot = GridOpponent::new(AIDifficulty::Easy, 5);
        for _ in 0..200 {
            bot.countdown_step();
        }
        bot.send_garbage(1, 6, crate::garbage::GARBAGE_FLAVOR_NORMAL);
        let mut seen = false;
        for _ in 0..(STEPS_PER_SECOND * 30) {
            bot.play_step();
            seen |= bot.game().garbage.iter_active().next().is_some();
        }
        assert!(seen, "the garbage sent to the bot never reached its board");
    }

    /// The cells of the board's lowest rows, as (block or garbage, flavour): what the player
    /// sees of a starting grid.
    fn layout(game: &GameState) -> Vec<(bool, i32)> {
        let mut cells = Vec::new();
        for y in 0..13 {
            for x in 0..6 {
                let cell = game.grid.get(x, y);
                let flavor = cell.block_id().and_then(|id| game.blocks.get(id)).map_or(-1, |b| b.flavor);
                cells.push((cell.is_empty(), flavor));
            }
        }
        cells
    }

    /// Against the computer the bot starts every round on the same grid as the player, as a
    /// network opponent does (both seeds follow `seed_base + round * 1000`).
    #[test]
    fn the_bot_starts_each_round_on_the_players_grid() {
        use crate::vs::VsSession;
        let mut session = VsSession::new(42, GridOpponent::new(AIDifficulty::Hard, 42));
        assert!(layout(&session.player_game).iter().any(|c| !c.0), "no blocks to compare");
        assert_eq!(layout(&session.player_game), layout(session.opponent.game()), "round 1");
        for round in 2..=3 {
            session.round_number = round;
            session.prepare_round();
            assert_eq!(layout(&session.player_game), layout(session.opponent.game()), "round {round}");
        }
    }

    #[test]
    fn a_new_round_starts_clean() {
        let mut bot = run_alone(AIDifficulty::Hard, 9, 30 * STEPS_PER_SECOND);
        bot.game_start();
        assert!(!bot.has_opponent_lost());
        assert_eq!(bot.game().score.value(), 0);
        assert!(bot.recv_garbage().is_empty());
        let start = OpponentBoardView::from_game(bot.game());
        assert_eq!(bot.board_view(), Some(&start), "the starting board is not published");
    }

    #[test]
    fn board_view_follows_the_board_and_shows_landed_garbage_soon() {
        use super::super::board_view::{BoardCell, BOARD_VIEW_MIN_GAP, BOARD_VIEW_ROWS};
        use crate::constants::PLAY_WIDTH;
        use crate::gameplay_event::GameplayEvent;

        let mut bot = GridOpponent::new(AIDifficulty::Easy, 5);
        assert_eq!(bot.board_view(), Some(&OpponentBoardView::from_game(bot.game())));
        for _ in 0..200 {
            bot.countdown_step();
        }
        bot.send_garbage(1, 6, crate::garbage::GARBAGE_FLAVOR_NORMAL);
        let mut previous = bot.board_view().cloned();
        let mut landed_at = None;
        for tick in 0..(STEPS_PER_SECOND * 30) {
            bot.play_step();
            let view = bot.board_view().cloned();
            if view != previous {
                // A view is only ever the board as it is at the tick it goes out.
                assert_eq!(view.as_ref(), Some(&OpponentBoardView::from_game(bot.game())));
                previous = view;
            }
            let landed = bot.game().gameplay_events.iter().any(|e| matches!(e, GameplayEvent::GarbageLanded(_)));
            if landed && landed_at.is_none() {
                landed_at = Some(tick);
            }
            if let Some(t) = landed_at {
                if tick == t + BOARD_VIEW_MIN_GAP {
                    let view = bot.board_view().unwrap();
                    let shows_garbage = (0..PLAY_WIDTH).any(|x| {
                        (0..BOARD_VIEW_ROWS).any(|r| matches!(view.cell(x, r), BoardCell::Garbage(_)))
                    });
                    assert!(shows_garbage, "landed garbage still not shown {BOARD_VIEW_MIN_GAP} ticks later");
                    return;
                }
            }
        }
        panic!("the garbage sent to the bot never landed");
    }

    /// A whole VS Computer session: the autoplay plays the human seat against the grid
    /// opponent, so garbage really flows both ways through the session routing and the round
    /// ends on a real loss.
    #[test]
    fn session_with_a_playing_opponent_exchanges_garbage_and_ends() {
        use crate::vs::{round::SessionPhase, session::VsSession};

        // Which seed ends within the time depends on how both sides play, so a few are tried:
        // the point is that a session with a playing opponent can end, and garbage crosses.
        let mut ended_runs = 0;
        for seed in [42u32, 7, 13, 99] {
            let mut session = VsSession::new(seed, GridOpponent::new(AIDifficulty::Hard, seed));
            let mut human = crate::vs::GridAutoplay::new(AIDifficulty::Hard, seed);
            let mut input = Input::new();
            let mut ended = false;
            for _ in 0..(STEPS_PER_SECOND * 60 * 6) {
                let flags = human.update(&session.player_game);
                for bit in [INPUT_LEFT, INPUT_RIGHT, INPUT_UP, INPUT_DOWN, INPUT_SWAP, INPUT_ADVANCE] {
                    input.set_key(bit, flags & bit != 0);
                }
                session.time_step(&input.snapshot());
                input.end_frame();
                if matches!(session.phase(), SessionPhase::Resolving | SessionPhase::ShowResult) {
                    ended = true;
                    break;
                }
            }
            let bot = &session.opponent;
            assert!(bot.game().stats.swaps_performed > 20, "the bot did not play (seed {seed})");
            assert!(
                session.player_game.stats.garbage_received + bot.game().stats.garbage_received > 0,
                "no garbage crossed between the two boards (seed {seed})"
            );
            if ended {
                ended_runs += 1;
            }
        }
        assert!(ended_runs > 0, "no session ended in six minutes, whatever the seed");
    }

    /// The tiers of the web version are decisive but beatable: hard > medium > easy. A rough
    /// sanity check of the port (the web arena measured 77% and 93% over 60 seeds); slow, so
    /// run by hand with `cargo test -p crackattack-core --release -- --ignored ladder`.
    #[test]
    #[ignore = "slow measurement"]
    fn ladder_hard_beats_medium_beats_easy() {
        let seeds = 1..=30u32;
        let mut hard_wins = 0;
        let mut medium_wins = 0;
        let mut played = 0;
        for seed in seeds.clone() {
            for swap in [false, true] {
                let (hard_vs_medium, _) = if swap {
                    let (w, t) = duel(AIDifficulty::Medium, AIDifficulty::Hard, seed, 30_000);
                    (w.map(|i| 1 - i), t)
                } else {
                    duel(AIDifficulty::Hard, AIDifficulty::Medium, seed, 30_000)
                };
                let (medium_vs_easy, _) = if swap {
                    let (w, t) = duel(AIDifficulty::Easy, AIDifficulty::Medium, seed, 30_000);
                    (w.map(|i| 1 - i), t)
                } else {
                    duel(AIDifficulty::Medium, AIDifficulty::Easy, seed, 30_000)
                };
                played += 1;
                if hard_vs_medium == Some(0) {
                    hard_wins += 1;
                }
                if medium_vs_easy == Some(0) {
                    medium_wins += 1;
                }
            }
        }
        eprintln!("hard beats medium: {hard_wins}/{played}; medium beats easy: {medium_wins}/{played}");
        assert!(hard_wins * 100 > played * 55, "hard should beat medium most of the time");
        assert!(medium_wins * 100 > played * 70, "medium should beat easy most of the time");
    }
}
