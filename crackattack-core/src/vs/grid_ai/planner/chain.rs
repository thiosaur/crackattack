//! Chain preparation: one swap after which a worth-firing cascade is a trigger swap away.

use super::board::*;
use super::cascade::*;

/// What a chain-enabling setup swap buys: the swap plus the enabled payoff.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ChainSetupPlan {
    /// The setup swap `(x,y)`<->`(x+1,y)`, in plan coordinates.
    pub x: usize,
    pub y: usize,
    /// Score of the best cascade this setup enables (attack value + shatter bonus).
    pub score: i32,
}

/// Options of [`plan_chain_setup`].
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct ChainSetupOptions {
    /// Minimum chain depth for an enabled cascade to count.
    pub min_chain: i32,
    /// Minimum single-round run for an enabled cascade to count.
    pub min_run: i32,
    /// Score bonus per garbage cell shattered.
    pub shatter_weight: i32,
    /// Also search three-swap constructions (setup, setup, trigger).
    pub lookahead: bool,
    /// Optional seed ranking otherwise equal choices.
    pub tie_seed: Option<u32>,
}

/// The offensive counterpart of [`plan_shatter_setup`]: a bounded two-ply search for one
/// **setup swap** that fires nothing itself but *enables* a worth-firing cascade one swap
/// later. This is how chains are *built* rather than merely noticed: arrange the board so
/// that a single trigger swap starts a cascade whose falls feed further matches.
///
/// Setup swaps are lateral block<->block exchanges (gravity-neutral, so the board stays
/// settled and plans survive re-planning) that complete no run of their own: a swap that
/// clears is a clear, owned by the fire/survival branches. For each candidate setup,
/// trigger swaps are scanned with the static [`makes_run3`] prefilter (exact on a settled
/// board); a block dropped into a fall-through gap can also trigger via gravity, so those
/// few candidates get the full cascade evaluation directly. An enabled cascade counts when
/// it meets the caller's fire thresholds (chain depth / run length), the same bar the fire
/// branch uses, so the enabled trigger is guaranteed to be taken on a later action tick
/// once the setup lands.
///
/// Incremental depth for free: after the setup executes, re-planning either fires the
/// enabled trigger or discovers a *further* enabler, so multi-swap constructions emerge
/// from repeated one-swap planning without a deeper search. With `opts.lookahead`, a
/// **second level** kicks in only when no single enabler exists (about 87% of bank
/// positions, measured): scan setup swaps whose result *contains* a single enabler, a
/// three-swap construction (setup, setup, trigger). The same monotone ladder guarantees
/// progress: a 2-level plan becomes a 1-level plan after its first swap, then a fire.
/// Deterministic: best score wins; an optional seed ranks ties without drawing gameplay
/// RNG. Pure function of the board and options, with no timing.
pub(crate) fn plan_chain_setup(board: &PlanBoard, opts: &ChainSetupOptions) -> Option<ChainSetupPlan> {
    let direct = search_enabler(board, opts);
    if direct.is_some() || !opts.lookahead {
        return direct;
    }

    // No single enabler exists: look one level deeper. The candidate move is a setup swap
    // after which a single enabler *does* exist; it inherits that enabler's eventual
    // score. Only runs in enabler-less positions, and the run3 prefilter keeps the inner
    // searches cheap.
    let mut best: Option<ChainSetupPlan> = None;
    let mut best_rank = 0u32;
    let mut b2 = board.clone();
    for sy in 0..board.height {
        for sx in 0..board.width - 1 {
            let a = board.get(sx, sy);
            let c = board.get(sx + 1, sy);
            if a < 0 || c < 0 {
                continue; // setups are block<->block only
            }
            if map_flavor_to_base_flavor(a) == map_flavor_to_base_flavor(c) {
                continue; // no-op
            }
            if makes_run3(board, sx, sy) {
                continue; // that's a clear, not a setup
            }
            b2.cell.copy_from_slice(&board.cell);
            b2.set(sx, sy, c);
            b2.set(sx + 1, sy, a);
            let enabled = search_enabler(&b2, opts);
            let rank = seeded_rank(opts.tie_seed, &[0x4c4f4f4b, sx, sy]);
            if let Some(enabled) = enabled {
                let better = match &best {
                    None => true,
                    Some(b) => {
                        enabled.score > b.score
                            || (enabled.score == b.score && opts.tie_seed.is_some() && rank > best_rank)
                    }
                };
                if better {
                    best = Some(ChainSetupPlan {
                        x: sx,
                        y: sy,
                        score: enabled.score,
                    });
                    best_rank = rank;
                }
            }
        }
    }
    best
}

/// The single-enabler search behind [`plan_chain_setup`] (one setup + trigger).
pub(super) fn search_enabler(board: &PlanBoard, opts: &ChainSetupOptions) -> Option<ChainSetupPlan> {
    let mut best: Option<ChainSetupPlan> = None;
    let mut best_rank = 0u32;
    let mut b2 = board.clone();

    for sy in 0..board.height {
        for sx in 0..board.width - 1 {
            let a = board.get(sx, sy);
            let c = board.get(sx + 1, sy);
            if a < 0 || c < 0 {
                continue; // setups are block<->block only
            }
            if map_flavor_to_base_flavor(a) == map_flavor_to_base_flavor(c) {
                continue; // no-op
            }
            if makes_run3(board, sx, sy) {
                continue; // that's a clear, not a setup
            }
            // Apply the setup on the scratch board (restored from `board` each time).
            b2.cell.copy_from_slice(&board.cell);
            b2.set(sx, sy, c);
            b2.set(sx + 1, sy, a);

            for ty in 0..b2.height {
                for tx in 0..b2.width - 1 {
                    if tx == sx && ty == sy {
                        continue; // undoing the setup is pointless
                    }
                    if !can_swap(&b2, tx, ty) {
                        continue;
                    }
                    let ta = b2.get(tx, ty);
                    let tc = b2.get(tx + 1, ty);
                    if ta >= 0 && tc >= 0 && map_flavor_to_base_flavor(ta) == map_flavor_to_base_flavor(tc) {
                        continue; // no-op
                    }
                    // A block swapped over a fall-through gap triggers via gravity, which
                    // the static prefilter can't see: evaluate those in full.
                    let fall_through = (ta >= 0 && tc < 0 && ty > 0 && b2.get(tx + 1, ty - 1) < 0)
                        || (tc >= 0 && ta < 0 && ty > 0 && b2.get(tx, ty - 1) < 0);
                    if !fall_through && !makes_run3(&b2, tx, ty) {
                        continue;
                    }
                    let cas = evaluate_swap(&b2, tx, ty);
                    if cas.chain_depth < opts.min_chain && cas.max_round < opts.min_run {
                        continue;
                    }
                    let score = attack_value(&cas) + cas.garbage_shattered * opts.shatter_weight;
                    let rank = seeded_rank(opts.tie_seed, &[0x43484149, sx, sy]);
                    let better = match &best {
                        None => true,
                        Some(b) => score > b.score || (score == b.score && opts.tie_seed.is_some() && rank > best_rank),
                    };
                    if better {
                        best = Some(ChainSetupPlan { x: sx, y: sy, score });
                        best_rank = rank;
                    }
                }
            }
        }
    }
    best
}
