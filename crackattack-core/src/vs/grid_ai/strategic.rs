//! The strategic play on a settled board, as a priority ladder: the rules below are tried in
//! order and the first one with an answer decides. Each rule is a small method that reads the
//! [`Situation`] (the board, what the scan of one-swap moves found, the stack and garbage
//! state) and answers with a [`Step`]. Adding or reordering a behaviour is editing [`LADDER`].

use crate::game_state::GameState;

use super::controller::AiController;
use super::planner::{
    attack_value, can_swap, hash_plan_board, plan_big_combo_setup, plan_chain_setup, plan_clear_setup, plan_shatter_setup,
    plan_undermine, BigComboSetupPlan, ChainSetupOptions, PlanBoard, PLAN_GARBAGE,
};
use super::candidates::{trigger_swaps, Candidate};
use super::select::Best;
use super::view::{block_count, Avoid, Intent, SwapPlan, HEIGHT, SAFE};

/// What a rule answers.
enum Step {
    /// Play this swap.
    Swap(SwapPlan),
    /// Nothing to play: the caller raises the board if it may.
    Raise,
    /// Not my business: ask the next rule.
    Pass,
}

/// What the chain-enabler search depends on: the board and the options that shape it (the tie
/// seed is fixed for a controller's life, so it is not part of the key).
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) struct ChainSetupKey {
    board: u32,
    min_chain: i32,
    min_run: i32,
    shatter_weight: i32,
    lookahead: bool,
}

/// The best swap that fires something worth it: a chain, a long run or a garbage shatter.
#[derive(Clone, Copy)]
struct Fire {
    plan: SwapPlan,
    score: i32,
    shattered: i32,
}

/// Everything the rules read, computed once per decision.
struct Situation {
    board: PlanBoard,
    cursor: (i32, i32),
    /// A swap to refuse (it would undo the previous one).
    avoid: Avoid,
    garbage_cells: i32,
    /// The stack is within `danger_margin` of the safe height.
    danger: bool,
    /// Best swap worth firing.
    fire: Option<Fire>,
    /// Nearest swap that clears anything.
    clear: Option<SwapPlan>,
    /// The crossed-column x8/x10 construction, when one is on offer (clean, safe board).
    big: Option<BigComboSetupPlan>,
    /// The plan is made for the time the board is in motion (on the board the fall will leave).
    moving: bool,
}

impl Situation {
    /// `plan` unless it is the swap to avoid. Planner rows are grid rows - 1.
    fn allowed(&self, plan: SwapPlan) -> Option<SwapPlan> {
        (!self.avoid.contains(plan)).then_some(plan)
    }
}

type Rule = fn(&mut AiController, &GameState, &Situation) -> Step;

/// The ladder, most urgent first. Two things to know when reading it:
/// - a rule is not exclusive to its name: `rule_fire` may answer with the x8/x10 construction
///   when that is worth more than the small fire on offer;
/// - `rule_flatten_in_danger` and `rule_flatten` are final (they answer `Swap` or `Raise`, never
///   `Pass`), and `rule_raise_first` ends the decision too: nothing below them is reached.
const LADDER: &[Rule] = &[
    AiController::rule_fire,
    AiController::rule_clear_in_danger,
    AiController::rule_shatter_setup,
    AiController::rule_undermine,
    AiController::rule_raise_first,
    AiController::rule_big_combo,
    AiController::rule_flatten_in_danger,
    AiController::rule_chain_setup,
    AiController::rule_plain_clear,
    AiController::rule_build,
    AiController::rule_clear_setup,
    AiController::rule_flatten,
];

impl AiController {
    /// What to do on `board` (the live board when settled, the one the fall will leave otherwise). The last rule of the ladder always answers (a swap or a
    /// raise), so the ladder never falls through; `Raise` is also the answer should it ever.
    pub(super) fn plan_strategic(
        &mut self,
        game: &GameState,
        board: PlanBoard,
        cursor_x: i32,
        cursor_y: i32,
        avoid: Avoid,
        moving: bool,
    ) -> Intent {
        let mut situation = self.situation(game, board, cursor_x, cursor_y, avoid);
        situation.moving = moving;
        for rule in LADDER {
            match rule(self, game, &situation) {
                Step::Swap(plan) => return Intent::Swap(plan),
                Step::Raise => return Intent::Raise,
                Step::Pass => {}
            }
        }
        Intent::Raise
    }

    fn situation(&mut self, game: &GameState, board: PlanBoard, cursor_x: i32, cursor_y: i32, avoid: Avoid) -> Situation {
        let (fire, clear) = self.scan_swaps(&board, cursor_x, cursor_y, avoid);
        let garbage_cells = board.cell.iter().filter(|&&v| v == PLAN_GARBAGE).count() as i32;
        let danger = game.grid.top_effective_row >= SAFE - self.tuning.danger_margin;
        let big = if !danger && garbage_cells == 0 && self.tuning.big_combo_setup_max_cost > 0 {
            self.big_combo(&board, avoid)
        } else {
            None
        };
        Situation {
            board,
            cursor: (cursor_x, cursor_y),
            avoid,
            garbage_cells,
            danger,
            fire,
            clear,
            big,
            moving: false,
        }
    }

    /// Evaluate every legal swap once: the best one worth firing, and the nearest that clears
    /// anything. Equal values are separated by the controller's seeded rank, then by distance.
    fn scan_swaps(
        &self,
        board: &PlanBoard,
        cursor_x: i32,
        cursor_y: i32,
        avoid: Avoid,
    ) -> (Option<Fire>, Option<SwapPlan>) {
        let mut fire = Best::new();
        let mut clear = Best::new();
        let swaps = trigger_swaps(
            board,
            (cursor_x, cursor_y),
            |plan| avoid.contains(plan),
            |plan| can_swap(board, plan.x as usize, plan.y as usize - 1),
        );
        for Candidate { plan, cascade, dist } in swaps {
            let worth_firing = cascade.chain_depth >= self.tuning.fire_min_chain
                || cascade.max_round >= self.tuning.fire_min_run
                || cascade.garbage_shattered > 0;
            if worth_firing {
                let score = attack_value(&cascade) + cascade.garbage_shattered * self.tuning.shatter_weight;
                let rank = self.tie_rank(0x4649_5245, plan.x, plan.y);
                let found = Fire { plan, score, shattered: cascade.garbage_shattered };
                fire.offer(self.biased(score, plan.y), rank, dist, found);
            }
            clear.offer(self.biased(0, plan.y), self.tie_rank(0x434c_4541, plan.x, plan.y), dist, plan);
        }
        (fire.item(), clear.item())
    }

    /// The x8/x10 construction for this board (memoised by board hash: it depends on the board
    /// alone), unless its first swap is the one to avoid.
    fn big_combo(&mut self, board: &PlanBoard, avoid: Avoid) -> Option<BigComboSetupPlan> {
        let (cost, seed) = (self.tuning.big_combo_setup_max_cost, self.decision_seed);
        let plan = self.big_combo_memo.get((hash_plan_board(board), cost), || plan_big_combo_setup(board, cost, seed));
        plan.filter(|b| !avoid.contains(SwapPlan::from_plan(b.x, b.y)))
    }

    // ---- the rules -------------------------------------------------------------------------

    /// Fire a real payoff (attack or defend), unless the larger x8/x10 construction is worth
    /// more than an incidental small fire: that is what lets the bot finish a big combo instead
    /// of abandoning it whenever an x4 appears.
    fn rule_fire(&mut self, _game: &GameState, s: &Situation) -> Step {
        let Some(fire) = s.fire else { return Step::Pass };
        if let Some(b) = s.big {
            if fire.shattered == 0 && fire.score < b.size - 3 {
                return Step::Swap(SwapPlan::from_plan(b.x, b.y));
            }
        }
        Step::Swap(fire.plan)
    }

    /// In danger a plain clear relieves height now (a shatter turns garbage into blocks without
    /// lowering the stack), so it comes before the garbage plans.
    fn rule_clear_in_danger(&mut self, _game: &GameState, s: &Situation) -> Step {
        match s.clear {
            Some(clear) if s.danger => Step::Swap(clear),
            _ => Step::Pass,
        }
    }

    /// No one-swap shatter exists, but garbage may be clearable in a few moves: work toward the
    /// cheapest multi-swap set-up that assembles a match against a slab.
    fn rule_shatter_setup(&mut self, _game: &GameState, s: &Situation) -> Step {
        let pressured = s.danger || s.garbage_cells >= self.tuning.pressure_garbage_cells;
        let cost = if pressured {
            self.tuning.shatter_setup_max_cost.max(self.tuning.pressure_setups)
        } else {
            self.tuning.shatter_setup_max_cost
        };
        if cost <= 0 {
            return Step::Pass;
        }
        match plan_shatter_setup(&s.board, cost).and_then(|p| {
            s.allowed(SwapPlan::from_plan(p.x, p.y))
        }) {
            Some(plan) => Step::Swap(plan),
            None => Step::Pass,
        }
    }

    /// No set-up reaches the garbage (it is perched on a tower): dig the tower out from under
    /// it so the slab descends into range.
    fn rule_undermine(&mut self, _game: &GameState, s: &Situation) -> Step {
        if !self.tuning.undermine {
            return Step::Pass;
        }
        let dig = plan_undermine(&s.board, s.cursor.0, s.cursor.1 - 1, self.decision_seed);
        match dig.and_then(|d| {
            s.allowed(SwapPlan::from_plan(d.0, d.1))
        }) {
            Some(plan) => Step::Swap(plan),
            None => Step::Pass,
        }
    }

    /// Little material left: raise a row before banking.
    fn rule_raise_first(&mut self, game: &GameState, _s: &Situation) -> Step {
        if self.advance_wanted(game) {
            Step::Raise
        } else {
            Step::Pass
        }
    }

    /// With room to think, pursue the human x8/x10 construction: two tall colour columns with
    /// their centre pair crossed.
    fn rule_big_combo(&mut self, _game: &GameState, s: &Situation) -> Step {
        match s.big {
            Some(b) => Step::Swap(SwapPlan::from_plan(b.x, b.y)),
            None => Step::Pass,
        }
    }

    /// In danger with nothing clearable and no garbage plan: dig peaks into gaps (it lowers the
    /// stack and churns up new matches, far better for survival than standing still). With
    /// nothing to dig, raise if the board may be raised; if it may not (the creep is frozen at
    /// the top) pass on to the set-ups below instead of standing still.
    fn rule_flatten_in_danger(&mut self, game: &GameState, s: &Situation) -> Step {
        if !s.danger {
            return Step::Pass;
        }
        let flatten = if self.tuning.flatten { self.find_flatten(game, s.cursor.0, s.cursor.1) } else { None };
        match flatten.and_then(|f| s.allowed(f)) {
            Some(plan) => Step::Swap(plan),
            None if self.advance_allowed(game) || !self.tuning.act_when_frozen => Step::Raise,
            None => Step::Pass,
        }
    }

    /// Safe and nothing worth firing: bank blocks, preferring a chain enabler (one set-up swap
    /// after which a worth-firing cascade is a single trigger swap away).
    fn rule_chain_setup(&mut self, _game: &GameState, s: &Situation) -> Step {
        if !self.tuning.chain_setup {
            return Step::Pass;
        }
        // Board-pure, so memoised by board hash: the lookahead is only paid when the board changes.
        let t = &self.tuning;
        let options = ChainSetupOptions {
            min_chain: t.fire_min_chain,
            min_run: t.fire_min_run,
            shatter_weight: t.shatter_weight,
            lookahead: t.chain_lookahead,
            tie_seed: self.decision_seed,
        };
        let key = ChainSetupKey {
            board: hash_plan_board(&s.board),
            min_chain: options.min_chain,
            min_run: options.min_run,
            shatter_weight: options.shatter_weight,
            lookahead: options.lookahead,
        };
        let found = self.chain_memo.get(key, || plan_chain_setup(&s.board, &options));
        match found.and_then(|c| s.allowed(SwapPlan::from_plan(c.x, c.y))) {
            Some(plan) => Step::Swap(plan),
            None => Step::Pass,
        }
    }

    /// No larger construction: take a productive plain clear before shuffling or raising.
    fn rule_plain_clear(&mut self, _game: &GameState, s: &Situation) -> Step {
        s.clear.map_or(Step::Pass, Step::Swap)
    }

    /// Cluster same-flavour blocks: the strategic tier otherwise stands still for seconds.
    fn rule_build(&mut self, game: &GameState, s: &Situation) -> Step {
        match self.find_build(game, s.cursor.0, s.cursor.1).and_then(|b| s.allowed(b)) {
            Some(plan) => Step::Swap(plan),
            None => Step::Pass,
        }
    }

    /// Nothing to cluster either, and the stack is high, the board cannot be raised or (if
    /// `clear_in_motion`) it is in motion and raising waits: a plain
    /// clear two or three swaps away is better than raising into a full board, or standing still.
    /// (On a low stack it measured worse: it replaces the raise that feeds the bot's chains.)
    fn rule_clear_setup(&mut self, game: &GameState, s: &Situation) -> Step {
        let depth = self.tuning.clear_setup;
        let stuck = s.danger || !self.advance_allowed(game) || (s.moving && self.tuning.clear_in_motion);
        if depth < 2 || !stuck {
            return Step::Pass;
        }
        let seed = self.decision_seed;
        let plan = self.clear_memo.get((hash_plan_board(&s.board), depth), || plan_clear_setup(&s.board, depth, seed));
        match plan.and_then(|p| s.allowed(SwapPlan::from_plan(p.x, p.y))) {
            Some(plan) => Step::Swap(plan),
            None => Step::Pass,
        }
    }

    /// Last resort before advancing: dig a block into a gap. Final.
    fn rule_flatten(&mut self, game: &GameState, s: &Situation) -> Step {
        self.flatten_step(game, s)
    }

    fn flatten_step(&mut self, game: &GameState, s: &Situation) -> Step {
        let flatten = if self.tuning.flatten {
            self.find_flatten(game, s.cursor.0, s.cursor.1)
        } else {
            None
        };
        match flatten.and_then(|f| s.allowed(f)) {
            Some(plan) => Step::Swap(plan),
            None => Step::Raise,
        }
    }

    // ---- raising the board -----------------------------------------------------------------

    /// Whether the board may be raised by hand now: not at the top, and the creep not frozen.
    pub(super) fn advance_allowed(&self, game: &GameState) -> bool {
        game.grid.top_effective_row < HEIGHT && !game.creep.creep_freeze
    }

    /// A proactive raise (before banking): allowed, and little material left on the board.
    pub(super) fn advance_wanted(&self, game: &GameState) -> bool {
        self.advance_allowed(game)
            && self.tuning.advance_below_blocks > 0
            && block_count(game) < self.tuning.advance_below_blocks
    }
}
