//! Local ladder: a **pure** Glicko-1 core that can move to Glicko-2.
//!
//! No I/O and no clock: timestamps are passed in as parameters. Persistence (the `ladder.json`
//! journal) lives in the application.
//!
//! Principles:
//! - a rating is **never** stored as truth. It is **recomputed** from the match journal
//!   (`recompute`).
//! - the algorithm sits behind a **trait** (`RatingSystem`). Today it is `Glicko1`; a `Glicko2`
//!   can replace it (the `volatility` field is already stored and ignored by Glicko-1).
//! - **RD inflation** (uncertainty growing back with inactivity) uses the real time elapsed
//!   between matches. It is applied in `recompute`, NOT in the math core, which has no notion
//!   of time.

use serde::{Deserialize, Serialize};

pub mod glicko1;
pub mod league;
pub mod recompute;

pub use league::{league, League};
pub use recompute::{recompute, trajectory, PlayerRecord};

/// **Persistent** player identity (16 bytes, the same value as `UserConfig.player_uuid`).
///
/// Serialized as **32 hex chars** (manual serde impl, not an array of 16 numbers). The
/// `ladder.json` journal must stay **human-readable** and consistent with the `player_uuid` in
/// `state.json` and with `protocol::uuid_hex`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PlayerId(pub [u8; 16]);

impl PlayerId {
    /// Encodes as lowercase hex (32 chars).
    pub fn to_hex(&self) -> String {
        use std::fmt::Write;
        let mut out = String::with_capacity(32);
        for b in self.0 {
            let _ = write!(out, "{b:02x}");
        }
        out
    }

    /// Decodes from hex (32 chars). Returns `None` if malformed.
    pub fn from_hex(s: &str) -> Option<Self> {
        if s.len() != 32 || !s.bytes().all(|b| b.is_ascii_hexdigit()) {
            return None;
        }
        let mut out = [0u8; 16];
        for (i, chunk) in s.as_bytes().chunks(2).enumerate() {
            let hi = (chunk[0] as char).to_digit(16)?;
            let lo = (chunk[1] as char).to_digit(16)?;
            out[i] = ((hi << 4) | lo) as u8;
        }
        Some(Self(out))
    }
}

impl Serialize for PlayerId {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_hex())
    }
}

impl<'de> Deserialize<'de> for PlayerId {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        PlayerId::from_hex(&s)
            .ok_or_else(|| serde::de::Error::custom(format!("invalid PlayerId hex: {s:?}")))
    }
}

/// A player's rating, with all **3 Glicko-2 fields**. `volatility` is stored but **ignored by
/// Glicko-1** (copied as is); it becomes active with Glicko-2. Uses the **public** scale
/// (about 1500, RD about 350).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Rating {
    pub value: f64,
    pub rd: f64,
    pub volatility: f64,
}

impl Rating {
    /// Initial rating of an unknown player.
    pub fn initial(cfg: &RatingConfig) -> Self {
        Self {
            value: cfg.default_value,
            rd: cfg.default_rd,
            volatility: cfg.default_volatility,
        }
    }
}

/// Outcome of a match, from the point of view of the ordered pair `(a, b)` in the journal.
/// `Draw` is handled by the formula (score 0.5) but is never produced today: the network round
/// verdict always breaks ties (loss_tick, swaps, role).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    WinA,
    WinB,
    Draw,
}

impl Outcome {
    /// Score of A (1 win, 0 loss, 0.5 draw). The score of B is `1 − score_a`.
    pub(crate) fn score_a(self) -> f64 {
        match self {
            Outcome::WinA => 1.0,
            Outcome::WinB => 0.0,
            Outcome::Draw => 0.5,
        }
    }
}

/// One entry of the **match journal**, the source of truth.
/// `ts` is in Unix seconds. It is only used for RD inflation, never for sorting: the journal
/// order is authoritative.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct MatchResult {
    pub a: PlayerId,
    pub b: PlayerId,
    pub outcome: Outcome,
    pub ts: u64,
}

/// System parameters. All can be overridden from the JSON config.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct RatingConfig {
    pub default_value: f64,
    pub default_rd: f64,
    /// Initial σ. Only Glicko-2 uses it.
    pub default_volatility: f64,
    /// RD floor: a rating never freezes completely.
    pub rd_min: f64,
    /// Inflation constant `c` (points/√day). The RD grows back with inactivity:
    /// `RD′ = min(√(RD² + c²·Δt_days), default_rd)`. 34.6 takes RD 50 to 350 in about 100 days.
    pub(crate) rd_inflation_c: f64,
    /// RD below which the player is placed (otherwise the league is `Rookie`).
    /// 150 means placed after about 8 played rounds (about 3 best-of-3 matches). This favours
    /// speed: early ranks are volatile (±73 points per round, about half a league). Other
    /// measured values: 125 gives 11 rounds and ±44; 100 gives 16 rounds and ±30.
    pub display_rd_threshold: f64,
    /// Minimum number of matches to appear on the ladder (anti-abuse, display side).
    pub min_matches_for_ladder: u32,
    /// Thresholds of the leagues Bronze to Grandmaster (`Copper` is below the first threshold).
    pub(crate) league_thresholds: [f64; 7],
}

impl Default for RatingConfig {
    fn default() -> Self {
        Self {
            default_value: 1500.0,
            default_rd: 350.0,
            default_volatility: 0.06,
            rd_min: 30.0,
            rd_inflation_c: 34.6,
            display_rd_threshold: 150.0,
            min_matches_for_ladder: 5,
            league_thresholds: [1125.0, 1275.0, 1425.0, 1575.0, 1725.0, 1875.0, 2025.0],
        }
    }
}

/// A rating algorithm. Moving to Glicko-2 only needs another implementation.
pub(crate) trait RatingSystem {
    /// **Pure**: returns the new ratings `(a′, b′)`, computed from the **pre-match** ratings of
    /// BOTH players. B never sees the already-updated A; returning values makes that bug
    /// impossible by construction. `score_a` is one of 0, 0.5, 1.
    fn rate(&self, cfg: &RatingConfig, a: &Rating, b: &Rating, score_a: f64) -> (Rating, Rating);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `PlayerId` serializes as 32 hex chars (readable journal), not as an array of numbers.
    /// The round trip is exact, and invalid hex is rejected on deserialization.
    #[test]
    fn player_id_serde_hex() {
        let id = PlayerId([0xAB; 16]);
        let json = serde_json::to_string(&id).unwrap();
        assert_eq!(json, format!("\"{}\"", "ab".repeat(16)));
        let back: PlayerId = serde_json::from_str(&json).unwrap();
        assert_eq!(back, id);
        assert!(serde_json::from_str::<PlayerId>("\"zz\"").is_err());
        assert!(serde_json::from_str::<PlayerId>("[1,2,3]").is_err());
        // A full journal entry stays readable.
        let entry = MatchResult { a: id, b: PlayerId([1; 16]), outcome: Outcome::WinA, ts: 42 };
        let json = serde_json::to_string(&entry).unwrap();
        assert!(json.contains("\"win_a\"") && json.contains(&"ab".repeat(16)));
        let back: MatchResult = serde_json::from_str(&json).unwrap();
        assert_eq!(back, entry);
    }
}
