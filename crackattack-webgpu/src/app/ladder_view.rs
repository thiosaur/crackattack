//! Data of the network rank page, built **purely** (testable) from the match journal.
//!
//! The page is **not** a ladder of opponents. Their local rating is frozen at our last
//! meetings, and a ranked table would suggest a consistency that cannot exist without a
//! server. The page only shows what is **locally true**:
//!  - **my card**: my rank, since my whole history is in my journal;
//!  - **my latest games**: consecutive rounds against the same opponent, grouped;
//!  - **opponents met**: a factual win/loss record, with no rating or rank.

use crackattack_rating::{
    league, trajectory, League, MatchResult, Outcome, PlayerId, PlayerRecord, Rating,
    RatingConfig,
};
use std::collections::HashMap;

/// My card (page header): the only data that is always exact.
pub struct MyCard {
    pub rating_value: f64,
    pub rd: f64,
    pub league: League,
    pub wins: u32,
    pub losses: u32,
}

/// A "game" is a run of **consecutive** rounds against the same opponent (a best-of-3 is one row).
pub struct PartyRow {
    pub opponent: String,
    pub wins: u32,
    pub losses: u32,
    /// Change of MY rating over the game (value after the last round minus value before the first).
    pub delta: f64,
    /// Timestamp of the last round of the game (shown as a relative date).
    pub last_ts: u64,
}

/// Factual record per opponent, plus the last **self-announced** rating (labeled as such;
/// the page still ranks nobody but me) and quick reconnect.
pub struct OpponentRow {
    pub id: PlayerId,
    pub name: String,
    /// MY wins against this opponent (the UI labels this as my record).
    pub wins: u32,
    pub losses: u32,
    pub last_ts: u64,
    /// Last self-announced rating (mDNS `rt` captured in the journal). `None` = never seen placed.
    pub last_rt: Option<i32>,
    /// A reconnect address is stored, so the row can be activated (reconnect).
    pub reachable: bool,
    /// Currently seen over mDNS ("online" badge).
    pub online: bool,
}

/// Contact info of an opponent, read from the journal (`players` meta).
#[derive(Default, Clone, Copy)]
pub struct OpponentContact {
    pub last_rt: Option<i32>,
    pub reachable: bool,
    /// An mDNS announcement carrying its uuid is currently visible (active zone services).
    pub online: bool,
}

/// League derived from a **bare value** (self-announced rating). A peer only announces once
/// it is placed, so its RD is below the threshold and 0 is used.
pub fn league_of_value(value: f64, cfg: &RatingConfig) -> League {
    league(&Rating { value, rd: 0.0, volatility: cfg.default_volatility }, cfg)
}

pub struct RankPageData {
    /// `None` = no rated match (empty page).
    pub me: Option<MyCard>,
    /// Games, most recent first (10 max).
    pub parties: Vec<PartyRow>,
    /// Opponents, most recently seen first (10 max).
    pub opponents: Vec<OpponentRow>,
}

const MAX_ROWS: usize = 10;

/// Builds the page data from the journal (plus names) and the recomputed ratings.
pub fn build(
    entries: &[MatchResult],
    records: &HashMap<PlayerId, PlayerRecord>,
    names: &dyn Fn(&PlayerId) -> Option<String>,
    contact: &dyn Fn(&PlayerId) -> OpponentContact,
    me: PlayerId,
    cfg: &RatingConfig,
) -> RankPageData {
    let my_record = records.get(&me);
    let card = my_record.map(|r| MyCard {
        rating_value: r.rating.value,
        rd: r.rating.rd,
        league: league(&r.rating, cfg),
        wins: r.wins,
        losses: r.losses,
    });

    // My rating after each entry (for the per-game deltas).
    let traj = trajectory(entries, cfg, me);
    let name_of = |id: &PlayerId| names(id).unwrap_or_else(|| id.to_hex()[..8].to_string());

    // Games: group my consecutive rounds by opponent.
    // `my_value_before` tracks my value BEFORE the current entry (initially the default rating).
    let mut parties: Vec<PartyRow> = Vec::new();
    let mut current: Option<(PlayerId, PartyRow, f64)> = None; // (opponent, row, value before the game)
    let mut my_value_before = cfg.default_value;
    for (i, e) in entries.iter().enumerate() {
        // A self-match (a == b) is ignored by `recompute` and `trajectory` (traj[i] = None).
        // Skip it here too: a forged or corrupt journal could contain one, and the write-time
        // guard (`record_rated_round`) does not protect reads, so `expect` below would panic.
        if e.a == e.b {
            continue;
        }
        let (opp, won) = if e.a == me {
            (e.b, e.outcome == Outcome::WinA)
        } else if e.b == me {
            (e.a, e.outcome == Outcome::WinB)
        } else {
            continue; // not my game (generic journal)
        };
        let after = traj[i].expect("entry involving me -> trajectory filled in");

        match current.as_mut() {
            Some((cur_opp, row, before)) if *cur_opp == opp => {
                if won { row.wins += 1 } else { row.losses += 1 }
                row.delta = after - *before;
                row.last_ts = e.ts;
            }
            _ => {
                if let Some((_, row, _)) = current.take() {
                    parties.push(row);
                }
                current = Some((
                    opp,
                    PartyRow {
                        opponent: name_of(&opp),
                        wins: won as u32,
                        losses: !won as u32,
                        delta: after - my_value_before,
                        last_ts: e.ts,
                    },
                    my_value_before,
                ));
            }
        }
        my_value_before = after;
    }
    if let Some((_, row, _)) = current.take() {
        parties.push(row);
    }
    parties.reverse(); // most recent first
    parties.truncate(MAX_ROWS);

    // Opponents: cumulative record, ordered by last meeting.
    let mut opp_map: HashMap<PlayerId, OpponentRow> = HashMap::new();
    for e in entries {
        let (opp, won) = if e.a == me {
            (e.b, e.outcome == Outcome::WinA)
        } else if e.b == me {
            (e.a, e.outcome == Outcome::WinB)
        } else {
            continue;
        };
        let row = opp_map.entry(opp).or_insert_with(|| {
            let c = contact(&opp);
            OpponentRow {
                id: opp,
                name: name_of(&opp),
                wins: 0,
                losses: 0,
                last_ts: 0,
                last_rt: c.last_rt,
                reachable: c.reachable,
                online: c.online,
            }
        });
        if won { row.wins += 1 } else { row.losses += 1 }
        row.last_ts = row.last_ts.max(e.ts);
    }
    let mut opponents: Vec<OpponentRow> = opp_map.into_values().collect();
    opponents.sort_by(|a, b| b.last_ts.cmp(&a.last_ts));
    opponents.truncate(MAX_ROWS);

    RankPageData { me: card, parties, opponents }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crackattack_rating::recompute;

    fn pid(n: u8) -> PlayerId {
        PlayerId([n; 16])
    }

    fn entry(a: u8, b: u8, outcome: Outcome, ts: u64) -> MatchResult {
        MatchResult { a: pid(a), b: pid(b), outcome, ts }
    }

    fn names(id: &PlayerId) -> Option<String> {
        Some(match id.0[0] {
            1 => "Moi".into(),
            2 => "Marie".into(),
            3 => "Paul".into(),
            _ => return None,
        })
    }

    /// A best-of-3 won 2-1 against Marie, one round against Paul, then a rematch against Marie
    /// gives 3 games in reverse chronological order, with deltas consistent with the trajectory.
    #[test]
    fn parties_grouped_by_consecutive_opponent() {
        let cfg = RatingConfig::default();
        let journal = vec![
            entry(1, 2, Outcome::WinA, 100), // best-of-3 vs Marie...
            entry(1, 2, Outcome::WinB, 200),
            entry(1, 2, Outcome::WinA, 300), // ... 2-1
            entry(1, 3, Outcome::WinA, 400), // vs Paul 1-0
            entry(1, 2, Outcome::WinB, 500), // rematch vs Marie 0-1
        ];
        let records = recompute(&journal, &cfg);
        let data = build(&journal, &records, &names, &|_| Default::default(), pid(1), &cfg);

        assert_eq!(data.parties.len(), 3);
        assert_eq!(data.parties[0].opponent, "Marie"); // most recent first
        assert_eq!((data.parties[0].wins, data.parties[0].losses), (0, 1));
        assert_eq!(data.parties[1].opponent, "Paul");
        assert_eq!(data.parties[2].opponent, "Marie");
        assert_eq!((data.parties[2].wins, data.parties[2].losses), (2, 1));

        // The sum of the game deltas equals my total delta since the default rating.
        let total: f64 = data.parties.iter().map(|p| p.delta).sum();
        let me = &records[&pid(1)];
        assert!((total - (me.rating.value - cfg.default_value)).abs() < 1e-9);

        // My card reflects the recompute.
        let card = data.me.as_ref().unwrap();
        assert_eq!((card.wins, card.losses), (3, 2));
        assert!((card.rating_value - me.rating.value).abs() < 1e-9);
    }

    /// Opponents: cumulative record, ordered by last meeting, no duplicates.
    #[test]
    fn opponents_summary() {
        let cfg = RatingConfig::default();
        let journal = vec![
            entry(1, 2, Outcome::WinA, 100),
            entry(1, 3, Outcome::WinB, 200),
            entry(1, 2, Outcome::WinA, 300),
        ];
        let records = recompute(&journal, &cfg);
        let data = build(&journal, &records, &names, &|id| OpponentContact {
            last_rt: if id.0[0] == 2 { Some(1512) } else { None },
            reachable: id.0[0] == 2,
            online: false,
        }, pid(1), &cfg);

        assert_eq!(data.opponents.len(), 2);
        assert_eq!(data.opponents[0].last_rt, Some(1512));
        assert!(data.opponents[0].reachable);
        assert_eq!(data.opponents[1].last_rt, None);
        assert!(!data.opponents[1].reachable);
        assert_eq!(data.opponents[0].name, "Marie"); // seen last (ts 300)
        assert_eq!((data.opponents[0].wins, data.opponents[0].losses), (2, 0));
        assert_eq!(data.opponents[1].name, "Paul");
        assert_eq!((data.opponents[1].wins, data.opponents[1].losses), (0, 1));
    }

    /// An empty journal gives an empty page (`me` = `None`) and no panic.
    #[test]
    fn empty_journal() {
        let cfg = RatingConfig::default();
        let records = HashMap::new();
        let data = build(&[], &records, &names, &|_| Default::default(), pid(1), &cfg);
        assert!(data.me.is_none() && data.parties.is_empty() && data.opponents.is_empty());
    }
}
