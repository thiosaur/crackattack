// Score tracking: classic scoring rules.

use std::collections::HashMap;
use crate::constants::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
// Extension point: there is a single variant, and `ScoringSystem::new` ignores
// the mode. Add variants only when a second scoring mode exists.
pub enum ScoringMode {
    #[default]
    Classic,
}

#[derive(Debug, Clone)]
pub struct ScoringSystem {
    score: Score,
}

impl ScoringSystem {
    pub fn new(_mode: ScoringMode) -> Self {
        ScoringSystem { score: Score::new() }
    }

    pub fn reset(&mut self) { self.score.reset(); }
    pub fn value(&self) -> i32 { self.score.value }
    pub fn backlog(&self) -> i32 { self.score.backlog }

    pub(crate) fn on_elimination(&mut self, combo_id: i32, magnitude: i32,
                          special_magnitude: i32, time_step: i32) -> i32 {
        self.score.on_elimination(combo_id, magnitude, special_magnitude, time_step)
    }

    pub(crate) fn on_multiplier(&mut self, combo_id: i32, new_multiplier: i32) -> i32 {
        self.score.on_multiplier(combo_id, new_multiplier)
    }

    pub(crate) fn on_combo_completed(&mut self, combo_id: i32) {
        self.score.on_combo_completed(combo_id);
    }

    pub(crate) fn flush_backlog(&mut self) { self.score.flush_backlog(); }
    #[cfg(test)]
    pub fn flush_all(&mut self) { self.score.flush_all(); }

}

impl Default for ScoringSystem {
    fn default() -> Self {
        ScoringSystem::new(ScoringMode::default())
    }
}

/// Per-combo score tracking state.
#[derive(Debug, Clone, Default)]
pub(crate) struct ComboScoreState {
    pub base_score_this_step: i32,
    pub base_accumulated_score: i32,
    pub n_multipliers_this_step: i32,
    pub last_time_step: i32,
}

/// Score state tracking.
#[derive(Debug, Clone)]
pub(crate) struct Score {
    pub value: i32,
    pub backlog: i32,
    combo_states: HashMap<i32, ComboScoreState>,
    pub fade_timer: i32,
}

impl Score {
    pub(crate) fn new() -> Self {
        Score {
            value: 0,
            backlog: 0,
            combo_states: HashMap::new(),
            fade_timer: 0,
        }
    }

    pub(crate) fn reset(&mut self) {
        self.value = 0;
        self.backlog = 0;
        self.combo_states.clear();
        self.fade_timer = 0;
    }

    /// Calculate elimination score.
    fn calculate_elimination_score(magnitude: i32, special_magnitude: i32) -> i32 {
        if special_magnitude > 0 {
            GRAY_SCORE * if special_magnitude == MIN_PATTERN_LENGTH {
                MIN_PATTERN_SCORE
            } else {
                special_magnitude
            }
        } else {
            if magnitude == MIN_PATTERN_LENGTH {
                MIN_PATTERN_SCORE
            } else {
                magnitude
            }
        }
    }

    /// Called when blocks are eliminated.
    pub(crate) fn on_elimination(&mut self, combo_id: i32, magnitude: i32,
                          special_magnitude: i32, time_step: i32) -> i32 {
        let points = Self::calculate_elimination_score(magnitude, special_magnitude);

        let combo_state = self.combo_states.entry(combo_id).or_default();
        if combo_state.last_time_step != time_step {
            combo_state.base_score_this_step = 0;
            combo_state.n_multipliers_this_step = 0;
            combo_state.last_time_step = time_step;
        }
        combo_state.base_score_this_step += points;
        combo_state.base_accumulated_score += points;

        self.backlog += points;
        points
    }

    /// Called when multiplier increases.
    pub(crate) fn on_multiplier(&mut self, combo_id: i32, new_multiplier: i32) -> i32 {
        let mut bonus_points = 0;
        if new_multiplier > 1 {
            if let Some(combo_state) = self.combo_states.get_mut(&combo_id) {
                // The bonus separates this-step and accumulated base scores so that
                // repeated multiplier bumps within the same tick don't double-count:
                // this-step score is scaled by the new multiplier slots, while
                // previously accumulated score is scaled by the slots already counted.
                bonus_points = combo_state.base_score_this_step
                    * (new_multiplier - combo_state.n_multipliers_this_step - 1)
                    + combo_state.base_accumulated_score
                    * combo_state.n_multipliers_this_step;
                combo_state.n_multipliers_this_step += 1;
            }
        }

        self.backlog += bonus_points;
        bonus_points
    }

    /// Called when a combo is completed.
    pub(crate) fn on_combo_completed(&mut self, combo_id: i32) {
        self.combo_states.remove(&combo_id);
    }

    /// Progressive backlog flush with fade-in timing.
    pub(crate) fn flush_backlog(&mut self) {
        if self.fade_timer > 0 {
            self.fade_timer -= 1;
            return;
        }
        if self.backlog > 0 {
            self.backlog -= 1;
            self.value += 1;
            self.fade_timer = (12 - 2 * self.backlog).max(1);
        }
    }

    /// Flush entire backlog immediately (for tests and non-animated contexts)
    #[cfg(test)]
    pub(crate) fn flush_all(&mut self) {
        self.value += self.backlog;
        self.backlog = 0;
        self.fade_timer = 0;
    }

}

impl Default for Score {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_elimination_score_normal() {
        assert_eq!(Score::calculate_elimination_score(3, 0), 2);
        assert_eq!(Score::calculate_elimination_score(4, 0), 4);
        assert_eq!(Score::calculate_elimination_score(5, 0), 5);
    }

    #[test]
    fn test_elimination_score_gray() {
        assert_eq!(Score::calculate_elimination_score(3, 3), 6);
        assert_eq!(Score::calculate_elimination_score(4, 4), 12);
    }

    #[test]
    fn test_on_elimination() {
        let mut score = Score::new();
        let points = score.on_elimination(1, 3, 0, 100);
        assert_eq!(points, 2);
        assert_eq!(score.backlog, 2);

        let points = score.on_elimination(1, 4, 0, 100);
        assert_eq!(points, 4);
        assert_eq!(score.backlog, 6);

        let state = score.combo_states.get(&1).unwrap();
        assert_eq!(state.base_score_this_step, 6);
        assert_eq!(state.base_accumulated_score, 6);
    }

    #[test]
    fn test_flush_all() {
        let mut score = Score::new();
        score.backlog = 100;
        score.flush_all();
        assert_eq!(score.value, 100);
        assert_eq!(score.backlog, 0);
    }

    #[test]
    fn test_flush_backlog_progressive() {
        let mut score = Score::new();
        score.backlog = 3;

        score.flush_backlog();
        assert_eq!(score.value, 1);
        assert_eq!(score.backlog, 2);
        assert_eq!(score.fade_timer, 8);

        for _ in 0..8 {
            score.flush_backlog();
        }
        assert_eq!(score.value, 1);
        assert_eq!(score.fade_timer, 0);

        score.flush_backlog();
        assert_eq!(score.value, 2);
        assert_eq!(score.backlog, 1);
        assert_eq!(score.fade_timer, 10);
    }
}
