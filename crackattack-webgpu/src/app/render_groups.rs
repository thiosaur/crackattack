//! Render layer groups — semantic organization of UI batches.
//!
//! Groups are activated/deactivated based on game state.
//! The rendering loop iterates active groups back-to-front.

use super::meta_state::FlowState;

/// A named group of UI layers.
pub struct RenderGroup {
    pub active: bool,
}

impl RenderGroup {
    fn new(active: bool) -> Self { Self { active } }
}

/// Event produced by [`CurtainState::tick`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum CurtainEvent {
    /// Nothing notable this frame.
    None,
    /// The curtain just reached full opacity — time to apply side-effects.
    /// Used by the menu↔game curtain (has a target FlowState).
    Midpoint,
    /// The curtain finished opening — animation complete, now inactive.
    /// Used by the menu↔game curtain (close→midpoint→open→Complete).
    Complete,
    /// The curtain is now fully closed and stays closed (no reopening).
    /// Used by the game curtain (close only, no midpoint).
    Closed,
    /// The curtain finished opening and deactivated.
    /// Used by the game curtain (open from fully closed to transparent).
    Opened,
}

/// Manages a fade-to-black-and-back curtain animation.
///
/// Lifecycle: `close()` → closing (progress 0→1) → Midpoint
///            → opening (progress 1→0) → Complete → inactive.
pub struct CurtainState {
    pub active: bool,
    /// 0.0 = transparent, 1.0 = fully opaque.
    pub progress: f32,
    /// `true` while fading to black, `false` while fading back in.
    closing: bool,
    /// Progress units per second (= 1.0 / duration).
    speed: f32,
    /// Whether the midpoint callback has already fired.
    midpoint_reached: bool,
    /// The flow state to switch to at the midpoint.
    target: Option<FlowState>,
}

impl CurtainState {
    pub fn new() -> Self {
        Self {
            active: false,
            progress: 0.0,
            closing: false,
            speed: 0.0,
            midpoint_reached: false,
            target: None,
        }
    }

    /// Start closing (fading to black) toward `target`.
    /// At midpoint the curtain re-opens automatically (menu↔game transition).
    pub fn close(&mut self, target: FlowState, duration: f32) {
        self.active = true;
        self.progress = 0.0;
        self.closing = true;
        self.speed = 1.0 / duration;
        self.midpoint_reached = false;
        self.target = Some(target);
    }

    /// Start closing (fading to black) without a target.
    /// The curtain stays fully closed when done (returns `Closed`).
    pub fn close_simple(&mut self, duration: f32) {
        self.active = true;
        self.progress = 0.0;
        self.closing = true;
        self.speed = 1.0 / duration;
        self.midpoint_reached = false;
        self.target = None;
    }

    /// Start opening (fading from black to transparent).
    /// The curtain deactivates when done (returns `Opened`).
    pub fn open(&mut self, duration: f32) {
        self.active = true;
        self.progress = 0.0;
        self.closing = false;
        self.speed = 1.0 / duration;
        self.midpoint_reached = false;
        self.target = None;
    }

    /// Whether the curtain is closed or closing (not open, opening or inactive).
    /// Used by the `EnsureGameCurtainOpen` safety net.
    pub fn is_closing_or_closed(&self) -> bool {
        self.active && self.closing
    }

    /// Current overlay alpha (0 = transparent, 1 = opaque).
    pub fn alpha(&self) -> f32 {
        if !self.active {
            return 0.0;
        }
        if self.closing {
            self.progress
        } else {
            1.0 - self.progress
        }
    }

    /// Take the stored target out (consumed at midpoint).
    pub fn take_target(&mut self) -> Option<FlowState> {
        self.target.take()
    }

    /// Advance the animation by `dt` seconds. Returns an event.
    pub fn tick(&mut self, dt: f32) -> CurtainEvent {
        if !self.active || self.speed == 0.0 {
            return CurtainEvent::None;
        }

        self.progress = (self.progress + self.speed * dt).min(1.0);

        if self.closing {
            if self.progress >= 1.0 {
                if self.target.is_some() {
                    // Menu↔game curtain: midpoint → switch to opening phase
                    if !self.midpoint_reached {
                        self.midpoint_reached = true;
                        self.closing = false;
                        self.progress = 0.0;
                        return CurtainEvent::Midpoint;
                    }
                } else {
                    // Game curtain: stay fully closed
                    self.speed = 0.0;
                    return CurtainEvent::Closed;
                }
            }
        } else {
            // Opening phase
            if self.progress >= 1.0 {
                self.progress = 0.0;
                self.speed = 0.0;
                self.active = false;
                // Menu curtain (has midpoint) → Complete.
                // Game curtain (no midpoint) → Opened.
                return if self.midpoint_reached {
                    CurtainEvent::Complete
                } else {
                    CurtainEvent::Opened
                };
            }
        }

        CurtainEvent::None
    }
}

/// All render groups, ordered back-to-front.
pub struct RenderGroups {
    /// Transition curtain between menu and game.
    pub curtain_menu_game: CurtainState,
    /// Game HUD + cursor + countdown.
    pub game: RenderGroup,
    /// Resolving curtain (black + tint) between game and postgame.
    /// Self-managing: controlled via `close_simple` / `open` commands.
    pub curtain_game_postgame: CurtainState,
    /// Postgame elements (result message, victory rings).
    pub postgame: RenderGroup,
    /// Dialog overlay (pause).
    pub dialog: RenderGroup,
}

impl RenderGroups {
    pub fn new() -> Self {
        Self {
            curtain_menu_game: CurtainState::new(),
            game: RenderGroup::new(false),
            curtain_game_postgame: CurtainState::new(),
            postgame: RenderGroup::new(false),
            dialog: RenderGroup::new(false),
        }
    }

    /// Derive group activity from current game state.
    /// Called each frame before rendering.
    ///
    /// Note: `curtain_menu_game` and `curtain_game_postgame` are
    /// self-managed by their animations — `sync_from_state` does
    /// NOT touch them.
    pub fn sync_from_state(
        &mut self,
        flow_state: &FlowState,
        game_exists: bool,
        pause_menu_open: bool,
        post_game_results_panel: bool,
    ) {
        let in_game = flow_state.is_in_game();
        let is_post_game = flow_state.is_post_game();
        let curtain_alpha = self.curtain_game_postgame.alpha();

        // curtain_menu_game.active is managed by CurtainState — do NOT overwrite here.

        // Game: active when in-game and the game curtain has not fully closed,
        // but NOT during PostGame (the curtain covers it).
        self.game.active = in_game && game_exists
            && curtain_alpha < 1.0
            && !is_post_game;

        // curtain_game_postgame.active is managed by CurtainState — do NOT overwrite here.

        // Postgame: active when the game curtain is visible (closing or closed)
        // or during PostGame, but NOT when showing results panel (egui takes over).
        self.postgame.active = ((in_game && self.curtain_game_postgame.active)
            || is_post_game)
            && !post_game_results_panel;

        // Dialog: active when pause menu is open
        self.dialog.active = pause_menu_open;
    }
}
