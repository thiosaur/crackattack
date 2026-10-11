//! **Glicko-1** math (Glickman, <http://www.glicko.net/glicko/glicko.pdf>).
//!
//! The core has no notion of time. RD inflation during inactivity (`inflate`) is a separate
//! function, applied by the recompute, which is the only code that knows timestamps.
//! `updated` accepts **several results** (one rating period), so it can be checked against the
//! reference vector of the paper. `rate()` is the single-result case: one match is one period.

use crate::{Rating, RatingConfig, RatingSystem};

/// `q = ln(10)/400`, the Glicko-1 scale constant.
fn q() -> f64 {
    std::f64::consts::LN_10 / 400.0
}

/// Weight that reduces the effect of an uncertain opponent: `g(RD) = 1/√(1 + 3q²RD²/π²)`.
fn g(rd: f64) -> f64 {
    let q = q();
    1.0 / (1.0 + 3.0 * q * q * rd * rd / (std::f64::consts::PI * std::f64::consts::PI)).sqrt()
}

/// Expected score of `r` against `(ro, rdo)`: `E = 1/(1 + 10^(−g(RDo)(r−ro)/400))`.
fn expected(r: f64, ro: f64, rdo: f64) -> f64 {
    1.0 / (1.0 + 10f64.powf(-g(rdo) * (r - ro) / 400.0))
}

/// Updates a rating against a **batch** of `(opponent, score)` results from one period.
/// An empty batch returns the rating unchanged. `volatility` is copied as is (kept for Glicko-2).
/// The new RD is clamped to `[cfg.rd_min, cfg.default_rd]`.
pub fn updated(cfg: &RatingConfig, r: &Rating, results: &[(&Rating, f64)]) -> Rating {
    if results.is_empty() {
        return *r;
    }
    let q = q();

    // d² = 1/(q²·Σ g(RDj)²·E·(1−E))
    let mut sum_d = 0.0; // Σ g²E(1−E)
    let mut sum_s = 0.0; // Σ g·(s − E)
    for (opp, score) in results {
        let gj = g(opp.rd);
        let e = expected(r.value, opp.value, opp.rd);
        sum_d += gj * gj * e * (1.0 - e);
        sum_s += gj * (score - e);
    }
    let d_sq_inv = q * q * sum_d; // = 1/d²
    let denom = 1.0 / (r.rd * r.rd) + d_sq_inv; // 1/RD² + 1/d²

    let value = r.value + (q / denom) * sum_s;
    let rd = (1.0 / denom).sqrt().clamp(cfg.rd_min, cfg.default_rd);

    Rating { value, rd, volatility: r.volatility }
}

/// **RD inflation** during inactivity: `RD′ = min(√(RD² + c²·Δt_days), default_rd)`.
/// The recompute applies it before each match, using the time since the player's **last match**.
/// `Δt = 0` returns the rating unchanged.
pub(crate) fn inflate(cfg: &RatingConfig, r: &Rating, dt_days: f64) -> Rating {
    let dt = dt_days.max(0.0); // clock going backwards: clamp
    let rd = (r.rd * r.rd + cfg.rd_inflation_c * cfg.rd_inflation_c * dt)
        .sqrt()
        .min(cfg.default_rd);
    Rating { rd, ..*r }
}

/// Glicko-1 behind the common [`RatingSystem`] trait.
pub(crate) struct Glicko1;

impl RatingSystem for Glicko1 {
    fn rate(&self, cfg: &RatingConfig, a: &Rating, b: &Rating, score_a: f64) -> (Rating, Rating) {
        // Both updates start from the PRE-match ratings (a and b are immutable borrows).
        let new_a = updated(cfg, a, &[(b, score_a)]);
        let new_b = updated(cfg, b, &[(a, 1.0 - score_a)]);
        (new_a, new_b)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> RatingConfig {
        RatingConfig::default()
    }

    fn r(value: f64, rd: f64) -> Rating {
        Rating { value, rd, volatility: 0.06 }
    }

    /// Checks the math against the reference vector of Glickman's paper.
    /// r=1500/RD=200 against (1400/30, win), (1550/100, loss), (1700/300, loss) in one batch
    /// gives r′ ≈ 1464.1, RD′ ≈ 151.4. The paper gives 1464.06 from rounded intermediates,
    /// so the tolerance is ±0.5.
    #[test]
    fn glickman_reference_vector() {
        let player = r(1500.0, 200.0);
        let o1 = r(1400.0, 30.0);
        let o2 = r(1550.0, 100.0);
        let o3 = r(1700.0, 300.0);
        let out = updated(&cfg(), &player, &[(&o1, 1.0), (&o2, 0.0), (&o3, 0.0)]);
        assert!((out.value - 1464.1).abs() < 0.5, "value = {}", out.value);
        assert!((out.rd - 151.4).abs() < 0.5, "rd = {}", out.rd);
    }

    /// E is consistent: E(a→b) + E(b→a) = 1, and E = 0.5 for equal ratings.
    #[test]
    fn expected_outcome_consistency() {
        let e_ab = expected(1500.0, 1700.0, 80.0);
        // E uses the OPPONENT's RD, so the sum is exactly 1 only when both RDs are equal:
        // E(a vs b, RDb) + E(b vs a, RDa) with RDa = RDb.
        let e_ba = expected(1700.0, 1500.0, 80.0);
        assert!((e_ab + e_ba - 1.0).abs() < 1e-12);
        assert!((expected(1500.0, 1500.0, 200.0) - 0.5).abs() < 1e-12);
    }

    /// Direction: a win raises A and lowers B, and both RDs drop. Beating a stronger player
    /// gains more. The favourite loses more points than the underdog when losing.
    #[test]
    fn rate_directional() {
        let c = cfg();
        let a = r(1500.0, 100.0);
        let weak = r(1300.0, 100.0);
        let strong = r(1700.0, 100.0);

        let (a1, w1) = Glicko1.rate(&c, &a, &weak, 1.0);
        assert!(a1.value > a.value && w1.value < weak.value);
        assert!(a1.rd < a.rd && w1.rd < weak.rd);

        let (a2, _) = Glicko1.rate(&c, &a, &strong, 1.0);
        assert!(
            a2.value - a.value > a1.value - a.value,
            "beating a stronger player ({}) must earn more than beating a weaker one ({})",
            a2.value - a.value,
            a1.value - a.value
        );

        // Loss: the favourite (against weak) loses more than the underdog (against strong).
        let (a3, _) = Glicko1.rate(&c, &a, &weak, 0.0);
        let (a4, _) = Glicko1.rate(&c, &a, &strong, 0.0);
        assert!(a.value - a3.value > a.value - a4.value);
    }

    /// Mirror: rate(a, b, s) == swap(rate(b, a, 1−s)). Shows that both updates start from the
    /// pre-match ratings, so order has no effect.
    #[test]
    fn rate_mirror_no_order_effect() {
        let c = cfg();
        let a = r(1450.0, 120.0);
        let b = r(1610.0, 60.0);
        let (a1, b1) = Glicko1.rate(&c, &a, &b, 1.0);
        let (b2, a2) = Glicko1.rate(&c, &b, &a, 0.0);
        assert!((a1.value - a2.value).abs() < 1e-12 && (a1.rd - a2.rd).abs() < 1e-12);
        assert!((b1.value - b2.value).abs() < 1e-12 && (b1.rd - b2.rd).abs() < 1e-12);
    }

    /// Inflation grows with Δt, is capped at the initial RD, is the identity for Δt=0, and
    /// clamps Δt<0.
    #[test]
    fn inflate_behaviour() {
        let c = cfg();
        let fresh = r(1500.0, 50.0);
        assert_eq!(inflate(&c, &fresh, 0.0).rd, 50.0);
        let d10 = inflate(&c, &fresh, 10.0).rd;
        let d50 = inflate(&c, &fresh, 50.0).rd;
        assert!(d10 > 50.0 && d50 > d10);
        assert!(inflate(&c, &fresh, 10_000.0).rd <= c.default_rd);
        assert_eq!(inflate(&c, &fresh, -5.0).rd, 50.0); // clock going backwards
        // About 100 days brings the RD back near its maximum (c = 34.6).
        assert!(inflate(&c, &fresh, 100.0).rd > 340.0);
    }

    /// RD floor: after very many matches, the RD does not drop below `rd_min`.
    #[test]
    fn rd_floor() {
        let c = cfg();
        let mut a = r(1500.0, 350.0);
        let b = r(1500.0, 50.0);
        for _ in 0..500 {
            let (na, _) = Glicko1.rate(&c, &a, &b, 1.0);
            a = na;
        }
        assert!(a.rd >= c.rd_min - 1e-9, "rd = {}", a.rd);
    }

    /// Draw between equal ratings: values stay almost unchanged, RDs still drop.
    #[test]
    fn draw_between_equals() {
        let c = cfg();
        let a = r(1500.0, 150.0);
        let b = r(1500.0, 150.0);
        let (a1, b1) = Glicko1.rate(&c, &a, &b, 0.5);
        assert!((a1.value - 1500.0).abs() < 1e-9 && (b1.value - 1500.0).abs() < 1e-9);
        assert!(a1.rd < 150.0 && b1.rd < 150.0);
    }

    /// σ (volatility) passes through Glicko-1 unchanged.
    #[test]
    fn volatility_passthrough() {
        let c = cfg();
        let a = Rating { value: 1500.0, rd: 200.0, volatility: 0.0417 };
        let b = r(1400.0, 80.0);
        let (a1, _) = Glicko1.rate(&c, &a, &b, 1.0);
        assert_eq!(a1.volatility, 0.0417);
    }

    /// An empty batch returns the rating unchanged.
    #[test]
    fn empty_batch_identity() {
        let c = cfg();
        let a = r(1500.0, 200.0);
        assert_eq!(updated(&c, &a, &[]), a);
    }
}
