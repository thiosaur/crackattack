//! UI display constants for positioning, sizing, and coloring of all HUD elements.

/// Play area Y offset. The X offset is `game_renderer::PLAY_OFFSET_X`.
pub const PLAY_OFFSET_Y: f32 = -7.0;

// Danger bar (edge glow): width of the horizontal gradient and exponent of
// the falloff curve.
pub const DANGER_BAR_FADE_WIDTH: f32 = 0.75;
pub const DANGER_BAR_FALLOFF: f32 = 1.5;
pub const PLAY_OFFSET_Z: f32 = -42.5;

/// External elements position (left side of screen)
pub const LEFT_EXTERNAL_CENTER: f32 = -10.5;

/// Clock position
pub const CLOCK_OFFSET_X: f32 = LEFT_EXTERNAL_CENTER;
pub const CLOCK_OFFSET_Y: f32 = -2.5;
pub const CLOCK_DIGIT_LENGTH: f32 = 0.6;
pub const CLOCK_DIGIT_OFFSET: f32 = 1.0;
pub const CLOCK_CENTER_OFFSET: f32 = 0.7;

/// Name/Nameplate position
pub const NAME_OFFSET_Y: f32 = 7.0;
pub const NAME_SCALE: f32 = 4.4;

/// Score position Y (shared with clock; solo mode shows score, VS mode shows clock)
pub const SCORE_OFFSET_Y: f32 = -2.5;

/// Message display
pub const MESSAGE_SCALE: f32 = 6.0;

/// Countdown display
pub const COUNTDOWN_SCALE: f32 = 4.0;
pub const COUNTDOWN_HEIGHT_RATIO: f32 = 0.4;

/// Session score ("3 - 2", open-ended series).
/// Placed below the nameplate, at a reduced clock scale.
pub const SESSION_SCORE_OFFSET_Y: f32 = NAME_OFFSET_Y - NAME_SCALE / 2.0 - 1.5;
pub const SESSION_SCORE_SCALE: f32 = 0.6;
/// Session score position when the opponent board preview takes the space below the nameplate:
/// between the preview and the clock.
pub const SESSION_SCORE_WITH_BOARD_OFFSET_Y: f32 = -0.7;

/// Gradient of the session score digits (same as the clock), as vertex colors: bottom left,
/// bottom right, top left, top right.
pub const SESSION_SCORE_GRADIENT: [[f32; 4]; 4] = [
    [0.3, 0.3, 1.0, 1.0],
    [0.5, 0.5, 1.0, 1.0],
    [0.5, 0.5, 1.0, 1.0],
    [1.0, 1.0, 1.0, 1.0],
];

/// Opponent board preview, below the nameplate: 6 × 13 cells.
pub const OPPONENT_BOARD_CELL: f32 = 0.3;
/// Top edge of the preview.
pub const OPPONENT_BOARD_TOP_Y: f32 = 4.4;
/// Frame around the preview.
pub const OPPONENT_BOARD_FRAME: f32 = 0.05;
/// Colors (sRGB): background, frame, garbage, and garbage that just appeared. Blocks take
/// the gradient of the score digits (`SESSION_SCORE_GRADIENT`), stretched over the preview.
pub const OPPONENT_BOARD_BACKGROUND: u32 = 0x0f1218;
pub const OPPONENT_BOARD_FRAME_COLOR: u32 = 0x292e38;
pub const OPPONENT_BOARD_GARBAGE: u32 = 0x825155;
pub const OPPONENT_BOARD_GARBAGE_FLASH: u32 = 0xe8343f;

/// Safe height (13 rows)
pub const GC_SAFE_HEIGHT: i32 = 13;

/// Steps per second (game tick rate)
pub const GC_STEPS_PER_SECOND: i32 = 50;

/// Number of level lights (GC_SAFE_HEIGHT - 1)
pub const LEVEL_LIGHT_COUNT: i32 = GC_SAFE_HEIGHT - 1;

/// Triangle prism mesh data from triangle.glb (post -90deg X rotation for Y-up)
/// 9 unique vertices with smooth normals, 24 indices (8 triangles)
pub(crate) const PRISM_POSITIONS: [[f32; 3]; 9] = [
    [ 0.00,  0.00, -0.12],   // v0: tip back
    [ 0.96, -0.36, -0.12],   // v1: base-bottom back
    [ 0.96,  0.36, -0.12],   // v2: base-top back
    [ 0.00,  0.00,  0.12],   // v3: tip front
    [ 0.00,  0.00,  0.12],   // v4: tip front (dup, different normal)
    [ 0.96, -0.36,  0.12],   // v5: base-bottom front
    [ 0.96, -0.36,  0.12],   // v6: base-bottom front (dup)
    [ 0.96,  0.36,  0.12],   // v7: base-top front
    [ 0.96,  0.36,  0.12],   // v8: base-top front (dup)
];

pub(crate) const PRISM_NORMALS: [[f32; 3]; 9] = [
    [-0.8382,  0.0000, -0.5453],
    [ 0.4716, -0.6805, -0.5608],
    [ 0.4716,  0.6805, -0.5608],
    [-0.8382,  0.0000,  0.5453],
    [-0.8382,  0.0000,  0.5453],
    [ 0.4716, -0.6805,  0.5608],
    [ 0.4716, -0.6805,  0.5608],
    [ 0.4716,  0.6805,  0.5608],
    [ 0.4716,  0.6805,  0.5608],
];

pub(crate) const PRISM_INDICES: [usize; 24] = [
    0, 2, 1,    // back face
    4, 6, 8,    // front face
    1, 3, 0,    // bottom-left side
    7, 0, 3,    // top-left side
    2, 5, 1,    // bottom-right side
    1, 5, 3,    // bottom-center side
    7, 2, 0,    // top-center side
    2, 7, 5,    // base (back face of base)
];

/// Vertices per light (8 triangles x 3 verts = 24)
pub const LEVEL_LIGHT_VERTS_PER_LIGHT: usize = 24;
/// Max total level light vertices (24 lights x 24 verts)
pub const LEVEL_LIGHT_MAX_VERTICES: usize = LEVEL_LIGHT_COUNT as usize * 2 * LEVEL_LIGHT_VERTS_PER_LIGHT;

/// Level light colors (VS mode):
pub const LEVEL_LIGHT_COLOR_BLUE: [f32; 3] = [0.0, 0.0, 0.7];    // Safe zone (neither)
pub const LEVEL_LIGHT_COLOR_ORANGE: [f32; 3] = [0.9, 0.5, 0.0];  // Both have blocks (common zone)
pub const LEVEL_LIGHT_COLOR_RED: [f32; 3] = [0.8, 0.0, 0.0];     // Player disadvantage (losing)
pub const LEVEL_LIGHT_COLOR_GREEN: [f32; 3] = [0.0, 0.8, 0.0];   // Player advantage (winning)

/// External elements Z offset
pub const EXTERNAL_OFFSET_Z: f32 = -42.5;


/// Reference plane: visible area at z = -42.5, NOT at the near plane.
/// distance = camera.positionZ - referencePlaneZ = 0 - (-42.5) = 42.5
/// sceneZOffset is NOT used in this calculation (only in view matrix).
pub(crate) const FRUSTUM_LEFT: f32 = -15.6;
pub(crate) const FRUSTUM_RIGHT: f32 = 8.4;
pub(crate) const FRUSTUM_BOTTOM: f32 = -6.0;
pub(crate) const FRUSTUM_TOP: f32 = 18.0;
pub(crate) const FRUSTUM_NEAR: f32 = 38.0;
pub(crate) const FRUSTUM_WIDTH: f32 = FRUSTUM_RIGHT - FRUSTUM_LEFT;   // 24.0 (at near plane)
pub(crate) const FRUSTUM_HEIGHT: f32 = FRUSTUM_TOP - FRUSTUM_BOTTOM;  // 24.0 (at near plane)

pub(crate) const REFERENCE_PLANE_Z: f32 = -42.5;
pub(crate) const CAMERA_POSITION_Z: f32 = 0.0;
pub(crate) const REFERENCE_DISTANCE: f32 = CAMERA_POSITION_Z - REFERENCE_PLANE_Z;  // 42.5

// tanHalfFov = (frustumHeight / 2) / near = 12.0 / 38.0
pub(crate) const TAN_HALF_FOV: f32 = (FRUSTUM_HEIGHT / 2.0) / FRUSTUM_NEAR;  // 0.31579

// Reference plane dimensions (visible area at z = -42.5)
pub(crate) const REF_HEIGHT: f32 = 2.0 * REFERENCE_DISTANCE * TAN_HALF_FOV;  // 26.84
pub(crate) const REF_WIDTH: f32 = REF_HEIGHT;  // aspect = 1.0

// Off-center ratios from frustum
pub(crate) const OFF_CENTER_X: f32 = (FRUSTUM_LEFT + FRUSTUM_RIGHT) / FRUSTUM_WIDTH;   // -0.3
pub(crate) const OFF_CENTER_Y: f32 = (FRUSTUM_BOTTOM + FRUSTUM_TOP) / FRUSTUM_HEIGHT;  //  0.5

// Reference plane center
pub(crate) const REF_CENTER_X: f32 = OFF_CENTER_X * REF_WIDTH * 0.5;   // -4.03
pub(crate) const REF_CENTER_Y: f32 = OFF_CENTER_Y * REF_HEIGHT * 0.5;  //  6.71

// Reference plane edges: the single source of the visible rectangle at z = -42.5.
// `layout.rs` uses these edges, so danger bars, backdrop and HUD line up.
pub(crate) const REF_LEFT: f32 = REF_CENTER_X - REF_WIDTH / 2.0;    // -17.45
pub(crate) const REF_RIGHT: f32 = REF_CENTER_X + REF_WIDTH / 2.0;   //   9.39
pub(crate) const REF_BOTTOM: f32 = REF_CENTER_Y - REF_HEIGHT / 2.0; //  -6.71
pub(crate) const REF_TOP: f32 = REF_CENTER_Y + REF_HEIGHT / 2.0;    //  20.13

/// Level light positions, computed from reference plane edges (NOT near plane)
pub(crate) const LIGHT_HALF_WIDTH: f32 = 1.0;
pub(crate) const EDGE_MARGIN: f32 = 0.5;

pub const LEVEL_LIGHT_RIGHT_X: f32 = REF_RIGHT - LIGHT_HALF_WIDTH - EDGE_MARGIN;  // 7.89
pub const LEVEL_LIGHT_LEFT_X: f32 = REF_LEFT + LIGHT_HALF_WIDTH + EDGE_MARGIN;    // -15.95
