//! Pure look-ahead evaluator and multi-swap planners for the strategic grid AI.
//!
//! The greedy matcher only ever executes a 3-in-a-row that is one swap away, but a plain
//! 3-match sends *no garbage* (normal garbage needs magnitude > 3, and chain garbage needs
//! multiplier > 1). To actually attack, the bot must build **4+ combos** (width garbage)
//! and, above all, **chains**: each chain link ships a full-width garbage row (a completed
//! combo sends `multiplier - 1` full rows).
//!
//! This module evaluates a candidate swap by simulating its *logical cascade* on a
//! lightweight copy of the static-block grid: apply the swap, settle gravity, remove any
//! 3+ runs (shattering adjacent garbage), and repeat, counting the chain depth (about the
//! sim's multiplier), the blocks cleared (about the magnitude) and the garbage shattered.
//! It is a pure, deterministic function of the board (no RNG, no timing), cheap enough to
//! run over every candidate each decision. It deliberately approximates: shattered garbage
//! is removed rather than converted to blocks, and timing windows are ignored. So it
//! *guides* the planner toward combo/chain setups; the real sim is authoritative for what
//! actually fires.
//!
//! Reading a `PlanBoard` from the simulator grid lives in the controller (the reference
//! `readPlanBoard` / `readGravityPlanBoard` are not part of this module).

mod board;
mod cascade;
mod chain;
mod clear;
mod combo;
mod garbage;
#[cfg(test)]
mod tests;

pub(crate) use board::*;
pub(crate) use cascade::*;
pub(crate) use chain::*;
pub(crate) use clear::*;
pub(crate) use combo::*;
pub(crate) use garbage::*;
