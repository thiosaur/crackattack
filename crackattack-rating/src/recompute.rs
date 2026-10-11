//! **Recompute from the log**: the ladder is never stored as truth. It is fully recomputed
//! from the match journal, so editing a ratings file has no effect: it is overwritten on the
//! next recompute.
//!
//! Entries are replayed **in append order**. The journal order is authoritative; timestamps are
//! only used for **RD inflation** between two matches of the same player.

use std::collections::HashMap;

use crate::glicko1::{inflate, Glicko1};
use crate::{MatchResult, Outcome, PlayerId, Rating, RatingConfig, RatingSystem};

/// Recomputed state of a player, for display (league, wins/losses/draws, `min_matches` filter).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PlayerRecord {
    pub rating: Rating,
    /// Timestamp (Unix seconds) of the last replayed match. Used for the next match's inflation.
    pub(crate) last_match_ts: u64,
    pub wins: u32,
    pub losses: u32,
    pub draws: u32,
}

impl PlayerRecord {
    fn new(cfg: &RatingConfig, ts: u64) -> Self {
        Self {
            rating: Rating::initial(cfg),
            last_match_ts: ts,
            wins: 0,
            losses: 0,
            draws: 0,
        }
    }

    pub fn matches_played(&self) -> u32 {
        self.wins + self.losses + self.draws
    }
}

/// Replays **one** journal entry on the players' state. Returns `None` if the entry is
/// ignored (a self-match means a corrupted journal), otherwise the new ratings `(a, b)`.
///
/// This is the single replay primitive: `recompute` and `trajectory` share it. Their results
/// must agree (the UI delta depends on it), and sharing the code guarantees that.
///
/// - an unknown player is created with the initial rating (1500 / 350) at their first match;
/// - before the match, each player's RD is **inflated** by the time since THEIR last match
///   (`Δt` clamped to ≥ 0 for a clock going backwards);
/// - deterministic: the same journal gives the same ratings.
fn replay_entry(
    players: &mut HashMap<PlayerId, PlayerRecord>,
    entry: &MatchResult,
    cfg: &RatingConfig,
) -> Option<(Rating, Rating)> {
    if entry.a == entry.b {
        return None; // self-match: impossible in play, so the journal is corrupted; ignored
    }

    // Get (or create) both players, with the RD inflated by the elapsed inactivity.
    let rec_a = *players.entry(entry.a).or_insert_with(|| PlayerRecord::new(cfg, entry.ts));
    let rec_b = *players.entry(entry.b).or_insert_with(|| PlayerRecord::new(cfg, entry.ts));
    let dt_days_a = entry.ts.saturating_sub(rec_a.last_match_ts) as f64 / 86_400.0;
    let dt_days_b = entry.ts.saturating_sub(rec_b.last_match_ts) as f64 / 86_400.0;
    let pre_a = inflate(cfg, &rec_a.rating, dt_days_a);
    let pre_b = inflate(cfg, &rec_b.rating, dt_days_b);

    // Simultaneous update from the pre-match ratings (guaranteed by the signature of rate()).
    let (new_a, new_b) = Glicko1.rate(cfg, &pre_a, &pre_b, entry.outcome.score_a());

    let a = players.get_mut(&entry.a).expect("inserted above");
    a.rating = new_a;
    a.last_match_ts = entry.ts;
    match entry.outcome {
        Outcome::WinA => a.wins += 1,
        Outcome::WinB => a.losses += 1,
        Outcome::Draw => a.draws += 1,
    }

    let b = players.get_mut(&entry.b).expect("inserted above");
    b.rating = new_b;
    b.last_match_ts = entry.ts;
    match entry.outcome {
        Outcome::WinA => b.losses += 1,
        Outcome::WinB => b.wins += 1,
        Outcome::Draw => b.draws += 1,
    }

    Some((new_a, new_b))
}

/// Replays the whole journal and returns the state of every player met
/// (see [`replay_entry`] for the semantics).
pub fn recompute(entries: &[MatchResult], cfg: &RatingConfig) -> HashMap<PlayerId, PlayerRecord> {
    let mut players: HashMap<PlayerId, PlayerRecord> = HashMap::new();
    for entry in entries {
        replay_entry(&mut players, entry, cfg);
    }
    players
}

/// **Trajectory** of a player: their rating (`value`) **after each journal entry**, `None` for
/// entries that do not involve them. It replays like `recompute` (both share [`replay_entry`]).
/// The UI uses it to compute the **delta of a game** (value after the last round minus value
/// before the first). `trajectory(...).last()` equals `recompute(...)[player].rating.value`.
pub fn trajectory(entries: &[MatchResult], cfg: &RatingConfig, subject: PlayerId) -> Vec<Option<f64>> {
    let mut players: HashMap<PlayerId, PlayerRecord> = HashMap::new();
    let mut out = Vec::with_capacity(entries.len());

    for entry in entries {
        out.push(match replay_entry(&mut players, entry, cfg) {
            Some((new_a, _)) if entry.a == subject => Some(new_a.value),
            Some((_, new_b)) if entry.b == subject => Some(new_b.value),
            _ => None,
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pid(n: u8) -> PlayerId {
        PlayerId([n; 16])
    }

    fn entry(a: u8, b: u8, outcome: Outcome, ts: u64) -> MatchResult {
        MatchResult { a: pid(a), b: pid(b), outcome, ts }
    }

    #[test]
    fn empty_journal_empty_ladder() {
        assert!(recompute(&[], &RatingConfig::default()).is_empty());
    }

    /// One match: the winner is above 1500, the loser below, and wins/losses are right.
    #[test]
    fn single_match() {
        let cfg = RatingConfig::default();
        let out = recompute(&[entry(1, 2, Outcome::WinA, 1_000)], &cfg);
        let a = &out[&pid(1)];
        let b = &out[&pid(2)];
        assert!(a.rating.value > 1500.0 && b.rating.value < 1500.0);
        assert_eq!((a.wins, a.losses), (1, 0));
        assert_eq!((b.wins, b.losses), (0, 1));
        assert_eq!(a.last_match_ts, 1_000);
    }

    /// Determinism: the same journal gives the same ratings, bit for bit.
    #[test]
    fn deterministic() {
        let cfg = RatingConfig::default();
        let journal = vec![
            entry(1, 2, Outcome::WinA, 100),
            entry(2, 3, Outcome::WinB, 200),
            entry(1, 3, Outcome::Draw, 300),
            entry(1, 2, Outcome::WinB, 400),
        ];
        let r1 = recompute(&journal, &cfg);
        let r2 = recompute(&journal, &cfg);
        assert_eq!(r1, r2);
    }

    /// A self-match is ignored (corrupted journal).
    #[test]
    fn self_match_ignored() {
        let cfg = RatingConfig::default();
        let out = recompute(&[entry(1, 1, Outcome::WinA, 100)], &cfg);
        assert!(out.is_empty());
    }

    /// Inflation: after a long inactivity, the RD of a placed player grows back before their
    /// next match, so their rating moves more than in continuous play.
    #[test]
    fn inactivity_inflates_rd() {
        let cfg = RatingConfig::default();
        // Player 1 gets placed against player 2 (20 matches close together in time)...
        let mut journal: Vec<MatchResult> = (0..20)
            .map(|i| entry(1, 2, if i % 2 == 0 { Outcome::WinA } else { Outcome::WinB }, 1_000 + i))
            .collect();
        let placed = recompute(&journal, &cfg)[&pid(1)].rating.rd;
        assert!(placed < cfg.display_rd_threshold, "placed after 20 matches (rd={placed})");
        // ...then 100 days of inactivity before one last match.
        journal.push(entry(1, 2, Outcome::WinA, 1_020 + 100 * 86_400));
        let after = recompute(&journal, &cfg)[&pid(1)].rating.rd;
        // The post-match RD stays above the placed RD (it grew back before the match).
        assert!(after > placed, "rd must have grown back: {after} vs {placed}");
    }

    /// Negative Δt (clock going backwards between two entries) is clamped: no panic, no NaN.
    #[test]
    fn clock_going_backwards_is_clamped() {
        let cfg = RatingConfig::default();
        let out = recompute(
            &[
                entry(1, 2, Outcome::WinA, 10_000),
                entry(1, 2, Outcome::WinA, 5_000), // ts goes backwards
            ],
            &cfg,
        );
        assert!(out[&pid(1)].rating.value.is_finite());
        assert_eq!(out[&pid(1)].wins, 2);
    }

    /// The trajectory agrees with `recompute` and is `None` for entries that do not involve the player.
    #[test]
    fn trajectory_matches_recompute() {
        let cfg = RatingConfig::default();
        let journal = vec![
            entry(1, 2, Outcome::WinA, 100),
            entry(2, 3, Outcome::WinB, 200), // does not involve player 1
            entry(1, 3, Outcome::WinB, 300),
        ];
        let traj = trajectory(&journal, &cfg, pid(1));
        assert_eq!(traj.len(), 3);
        assert!(traj[0].is_some() && traj[1].is_none() && traj[2].is_some());
        assert!(traj[0].unwrap() > 1500.0, "win -> above 1500");
        let final_value = recompute(&journal, &cfg)[&pid(1)].rating.value;
        assert!((traj[2].unwrap() - final_value).abs() < 1e-9);
    }

    /// Convergence sanity check: 20 wins in a row by a new player give a rating well above 1500
    /// and a clearly lower RD, but the player is NOT placed. Farming a single opponent who
    /// collapses quickly stops being informative (E→1 gives E(1−E)→0, so the RD stalls). This is
    /// a **natural** anti-abuse property of Glicko: placing needs contested matches, not 20 wins
    /// against the same victim.
    #[test]
    fn convergence_sanity() {
        let cfg = RatingConfig::default();
        let journal: Vec<MatchResult> =
            (0..20).map(|i| entry(1, 2, Outcome::WinA, 1_000 + i)).collect();
        let rec = &recompute(&journal, &cfg)[&pid(1)];
        assert!(rec.rating.value > 1700.0, "value = {}", rec.rating.value);
        assert!(rec.rating.rd < 200.0, "rd = {}", rec.rating.rd); // about 178: lower, but not placed
        assert!(rec.rating.rd > cfg.display_rd_threshold, "farming does not place");
        assert_eq!(rec.matches_played(), 20);
    }

    /// In contrast, **contested** matches do place: 20 alternating matches (10 wins, 10 losses)
    /// against the same opponent leave both players placed (RD below the display threshold).
    #[test]
    fn contested_matches_do_place() {
        let cfg = RatingConfig::default();
        let journal: Vec<MatchResult> = (0..20)
            .map(|i| entry(1, 2, if i % 2 == 0 { Outcome::WinA } else { Outcome::WinB }, 1_000 + i))
            .collect();
        let out = recompute(&journal, &cfg);
        assert!(out[&pid(1)].rating.rd < cfg.display_rd_threshold, "rd = {}", out[&pid(1)].rating.rd);
        assert!(out[&pid(2)].rating.rd < cfg.display_rd_threshold);
        // Values stay close to 1500 (equal strength).
        assert!((out[&pid(1)].rating.value - 1500.0).abs() < 60.0);
    }
}
