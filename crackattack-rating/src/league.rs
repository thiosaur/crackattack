//! **Leagues**: a readable mapping from the numeric rating to named tiers (as in Panel Attack).
//! This crate returns the **enum**; localized labels live in the application locale.

use crate::{Rating, RatingConfig};

/// Tiers, from beginner to top. `Rookie` means the rating is not reliable yet
/// (`rd > display_rd_threshold`) and is still calibrating. This replaces the fixed number of
/// placement matches used by Panel Attack.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum League {
    Rookie,
    Copper,
    Bronze,
    Silver,
    Gold,
    Platinum,
    Diamond,
    Master,
    Grandmaster,
}

/// League of a rating. `Rookie` while the uncertainty is high; otherwise the tier is picked
/// from `league_thresholds` (Bronze up to Grandmaster). `Copper` is below the first threshold.
pub fn league(r: &Rating, cfg: &RatingConfig) -> League {
    if r.rd > cfg.display_rd_threshold {
        return League::Rookie;
    }
    const LADDER: [League; 8] = [
        League::Copper,
        League::Bronze,
        League::Silver,
        League::Gold,
        League::Platinum,
        League::Diamond,
        League::Master,
        League::Grandmaster,
    ];
    let above = cfg.league_thresholds.iter().filter(|t| r.value >= **t).count();
    LADDER[above]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn placed(value: f64) -> Rating {
        Rating { value, rd: 60.0, volatility: 0.06 }
    }

    #[test]
    fn rookie_while_uncertain() {
        let cfg = RatingConfig::default();
        let fresh = Rating { value: 2500.0, rd: 350.0, volatility: 0.06 };
        // Even a huge rating stays `Rookie` while the RD is high.
        assert_eq!(league(&fresh, &cfg), League::Rookie);
    }

    #[test]
    fn thresholds_exact() {
        let cfg = RatingConfig::default();
        assert_eq!(league(&placed(1000.0), &cfg), League::Copper);
        assert_eq!(league(&placed(1124.9), &cfg), League::Copper);
        assert_eq!(league(&placed(1125.0), &cfg), League::Bronze); // threshold is inclusive
        assert_eq!(league(&placed(1275.0), &cfg), League::Silver);
        assert_eq!(league(&placed(1425.0), &cfg), League::Gold);
        assert_eq!(league(&placed(1500.0), &cfg), League::Gold); // the starting rating, 1500, is Gold
        assert_eq!(league(&placed(1575.0), &cfg), League::Platinum);
        assert_eq!(league(&placed(1725.0), &cfg), League::Diamond);
        assert_eq!(league(&placed(1875.0), &cfg), League::Master);
        assert_eq!(league(&placed(2025.0), &cfg), League::Grandmaster);
        assert_eq!(league(&placed(3000.0), &cfg), League::Grandmaster);
    }

    #[test]
    fn leagues_are_ordered() {
        assert!(League::Rookie < League::Copper && League::Copper < League::Grandmaster);
    }
}
