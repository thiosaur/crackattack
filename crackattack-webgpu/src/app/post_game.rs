//! Post-game logic: end screen sequencing, inter-round stats, input handling.
//!
//! This module owns the state machine and physics for the post-game flow.
//! It does NOT touch audio, GPU, or persistence — it returns intentions
//! that the caller (main.rs) acts on.

use crackattack_core::frame_data::{SessionLifecycle, SessionSnapshot};

/// Stats captured at the end of a VS round (before the grid resets).
/// Displayed on the post-round screen (open series — no "match" layer).
#[derive(Debug, Clone)]
pub struct RoundEndStats {
    pub local: crackattack_core::stats::GameStats,
    pub opponent: Option<crackattack_core::stats::GameStats>,
    /// Round just played (1-based, unbounded).
    pub round_number: i32,
    /// Session tally including the round just played (for example "3-2").
    pub session_wins: i32,
    pub session_losses: i32,
}

/// Sub-screen of the post-game flow.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PostGameScreen {
    #[default]
    ResultMessage,
    HighScores,
    Stats,
}

impl PostGameScreen {
    pub fn is_results_panel(self) -> bool {
        matches!(self, Self::HighScores | Self::Stats)
    }
    pub fn is_final(self) -> bool {
        matches!(self, Self::Stats)
    }
}

/// Actions that main.rs should perform when post-game state changes.
pub enum PostGameAction {
    /// First detection of game end — submit score, play music, init textures.
    Enter,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PostGameKind {
    Winner,
    GameOver,
}

impl PostGameKind {
    pub fn from_result(result: crackattack_core::frame_data::SessionResult) -> Option<Self> {
        use crackattack_core::frame_data::SessionResult;
        match result {
            SessionResult::SessionWin | SessionResult::RoundWin => Some(Self::Winner),
            SessionResult::SessionLose | SessionResult::RoundLose => Some(Self::GameOver),
            SessionResult::None => None,
        }
    }
}

/// Post-game lifecycle phase.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PostGamePhase {
    /// No game-end detected yet.
    #[default]
    Inactive,
    /// sync() detected game end — waiting for EnterPostGame to capture stats.
    Detected,
    /// EnterPostGame has captured stats/score/rank.
    Entered,
}

#[derive(Debug, Clone)]
pub struct PostGameEffectsState {
    pub last_finished_tick: i32,
    pub last_victory_stage: i32,
    pub phase: PostGamePhase,
}

impl Default for PostGameEffectsState {
    fn default() -> Self {
        Self {
            last_finished_tick: -1,
            last_victory_stage: -1,
            phase: PostGamePhase::Inactive,
        }
    }
}

impl PostGameEffectsState {
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    /// Called each frame. Detects game-end transitions and tracks timing.
    /// Reads lifecycle and result from SessionSnapshot.
    /// ticks_since_finished comes from FrameData (simulation timing).
    pub fn sync(&mut self, ticks_since_finished: i32, snapshot: &SessionSnapshot) -> Vec<PostGameAction> {
        let mut actions = Vec::new();

        // Only active when the game is in a finished state.
        if !matches!(snapshot.lifecycle, SessionLifecycle::ShowResult | SessionLifecycle::Ended) {
            return actions;
        }

        // First detection of game end
        if self.phase == PostGamePhase::Inactive
            && PostGameKind::from_result(snapshot.result).is_some()
        {
            self.phase = PostGamePhase::Detected;
            self.last_finished_tick = ticks_since_finished;
            actions.push(PostGameAction::Enter);
        }

        // Track victory stage progression for texture refresh
        let stage = ticks_since_finished / 50;
        if stage != self.last_victory_stage {
            self.last_victory_stage = stage;
        }

        self.last_finished_tick = ticks_since_finished;

        actions
    }

}

/// Focus of the post-game navigation buttons (Menu / Next, outside dialogs).
///
/// `None` means no button is selected, so Enter/A runs the default action (advance).
/// Left/Right sets the focus, and then Enter/A activates the selected button.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PostGameNavFocus {
    #[default]
    None,
    Menu,
    Next,
}

/// Moves the focus one step in the post-game ring `[Menu, Content(None), Next]`.
///
/// The move stops at the ends (horizontal navigation does not wrap). `ring` lists the
/// positions from left to right; the contracted offer removes `Next`. A `current` that is
/// not in the ring falls back to the content (index 1) before the step.
///
/// This is a pure function shared by the two routers (`route_app_key_keyboard` and
/// `route_post_round` in `main.rs`). The caller compares the result with the current
/// focus and emits a command and a sound only if the focus moved, so hitting an end is
/// silent.
pub fn nav_ring_step(
    ring: &[PostGameNavFocus],
    current: PostGameNavFocus,
    step: i32,
) -> PostGameNavFocus {
    let idx = ring.iter().position(|f| *f == current).unwrap_or(1);
    ring[crate::app::focus::clamp_move(idx, ring.len(), step)]
}

#[cfg(test)]
mod tests {
    use super::*;

    const FULL: [PostGameNavFocus; 3] =
        [PostGameNavFocus::Menu, PostGameNavFocus::None, PostGameNavFocus::Next];

    /// The full ring is walked with Left/Right and stops at the ends (no wrap).
    #[test]
    fn nav_ring_moves_and_clamps() {
        assert_eq!(nav_ring_step(&FULL, PostGameNavFocus::None, -1), PostGameNavFocus::Menu);
        assert_eq!(nav_ring_step(&FULL, PostGameNavFocus::None, 1), PostGameNavFocus::Next);
        // At an end the focus does not move (the caller then emits nothing).
        assert_eq!(nav_ring_step(&FULL, PostGameNavFocus::Menu, -1), PostGameNavFocus::Menu);
        assert_eq!(nav_ring_step(&FULL, PostGameNavFocus::Next, 1), PostGameNavFocus::Next);
    }

    /// Contracted offer (opponent left): the ring shrinks to [Menu, Content].
    /// A focus that is no longer in the ring (`Next`) falls back to the content before the step.
    #[test]
    fn nav_ring_contracted_and_out_of_ring() {
        let contracted = [PostGameNavFocus::Menu, PostGameNavFocus::None];
        assert_eq!(
            nav_ring_step(&contracted, PostGameNavFocus::None, 1),
            PostGameNavFocus::None // no `Next` any more: Right stops at the content
        );
        assert_eq!(
            nav_ring_step(&contracted, PostGameNavFocus::Next, -1),
            PostGameNavFocus::Menu // out of the ring -> content (index 1), then one step left
        );
    }
}

/// Post-game UI input state (separate from gameplay Input).
#[derive(Debug, Clone, Default)]
pub struct PostGameInput {
    /// Focused navigation button (Left/Right). `None` by default.
    pub nav_focus: PostGameNavFocus,
    /// The abandon confirmation modal is open.
    ///
    /// `RequestQuitGame` sets it only when a round is engaged, that is, when leaving
    /// costs a scored round. In practice this means Quit from the pause menu during a
    /// round; leaving from the ready prompt or the post-round screen is direct. Input is
    /// routed by `main.rs::route_abandon_confirm` and drawn by
    /// `frame_submit.rs::render_abandon_confirm`. It is reset at startup and on `QuitToMenu`.
    pub abandon_confirm_open: bool,
    /// Selection in the abandon modal: `true` = "Abandon" focused, `false` = "Cancel".
    /// Cancel is the safe default, so a reflex confirm cancels.
    pub abandon_confirm_yes: bool,
    /// Held scroll direction while the focus is on the content: -1 = up, 1 = down,
    /// 0 = none.
    ///
    /// Set on Up/Down press and cleared on release. `nav_view` applies a delta every frame
    /// (smooth scrolling) and repaints continuously while it is not 0.
    pub scroll_dir: i32,
    /// True when the player has closed the post-round summary ("Next").
    ///
    /// The curtain opens and the "ready for a new round" modal takes over. Reset to false
    /// on each `RoundPrepared` (round 2 and later).
    pub post_round_advanced: bool,
}

