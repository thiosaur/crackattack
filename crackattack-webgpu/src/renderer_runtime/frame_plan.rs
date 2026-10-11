//! Per-frame visual plan: pure decision data, derived once before rendering.

/// All visual decisions for one frame, derived once before rendering.
///
/// Built at the top of `render_frame()` from session snapshot +
/// render groups. Consumed by all subsequent rendering phases.
/// No wgpu types — this is pure decision data.
pub(super) struct FramePlan {
    /// Session snapshot, read once (avoids 4 redundant constructions per frame).
    pub(super) snapshot: Option<crackattack_core::frame_data::SessionSnapshot>,
    /// Presentation derived once from snapshot (modals, grid visibility, result).
    pub(super) presentation: Option<crate::app::presentation::StatusPresentation>,
    /// Flow state is in-game (Playing or PostGame).
    pub(super) in_game: bool,
    /// A game session exists.
    pub(super) game_exists: bool,
    /// 3D scene should be drawn (in_game && game_exists && curtain not fully closed).
    pub(super) draw_game: bool,
    /// HUD elements should be drawn (in_game && scene not hidden by curtain).
    pub(super) draw_hud: bool,
    /// Game→postgame curtain alpha (0=transparent, 1=opaque).
    pub(super) curtain_game_alpha: f32,
    /// Menu↔game curtain alpha.
    pub(super) curtain_menu_alpha: f32,
    /// Post-game results panel is visible (egui takes over rendering).
    pub(super) post_game_results_visible: bool,
    /// Current flow state (Menu, Playing, PostGame).
    pub(super) flow_state: crate::app::meta_state::FlowState,
    /// Pause menu is currently visible (snapshot-derived presentation state).
    pub(super) pause_menu_open: bool,
}
