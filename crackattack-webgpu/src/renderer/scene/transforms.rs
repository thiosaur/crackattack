//! Transform matrices, material uniforms, and lighting uniforms.
//!
//! Pure math functions that produce GPU-ready uniform data.

use crate::renderer::gpu::uniforms::{TransformUniformsGPU, LightingUniformsGPU};

/// Frustum parameters derived from viewport layout:
///   playPixelOffsetX = 0.4, playPixelWidth = 0.5
///   playWidth = 12.0, playHeight = 24.0, cameraHeightRatio = 0.25
///   left  = -(0.4/0.5 + 0.5) * 12 = -15.6
///   right = ((1-0.4-0.5)/0.5 + 0.5) * 12 = 8.4
///   bottom = -24 * 0.25 = -6.0
///   top = 24 * 0.75 = 18.0
const FRUSTUM_NEAR: f32 = 38.0;
const FRUSTUM_FAR: f32 = 65.0;
const FRUSTUM_LEFT: f32 = -15.6;
const FRUSTUM_RIGHT: f32 = 8.4;
const FRUSTUM_BOTTOM: f32 = -6.0;
const FRUSTUM_TOP: f32 = 18.0;

/// Create projection matrix using frustum (NOT FOV-based!).
///
/// The frustum widens horizontally with the aspect ratio. The vertical bounds (`t`/`b`)
/// stay fixed and `l`/`r` grow symmetrically, so the half-width is the half-height times
/// `aspect_eff`. The scale per pixel is the same on both axes: nothing is distorted and
/// the play field stays square. A wider window shows more world at the sides.
/// `aspect_eff = clamp(width/height, 1, 16/9)`. At aspect 1, `l`/`r` are `-15.6`/`8.4`.
pub fn create_projection_matrix(width: f32, height: f32) -> [f32; 16] {
    let n = FRUSTUM_NEAR;
    let f = FRUSTUM_FAR;
    let aspect_eff = (width / height.max(1.0)).clamp(1.0, crate::renderer::init::USEFUL_ZONE_MAX_ASPECT);
    let center_x = (FRUSTUM_LEFT + FRUSTUM_RIGHT) * 0.5;
    let half_w = (FRUSTUM_TOP - FRUSTUM_BOTTOM) * 0.5 * aspect_eff;
    let l = center_x - half_w;
    let r = center_x + half_w;
    let b = FRUSTUM_BOTTOM;
    let t = FRUSTUM_TOP;

    // Frustum projection matrix (WebGPU Z [0,1] range)
    // Column-major order for WebGPU
    [
        2.0 * n / (r - l),      0.0,                    0.0,                        0.0,
        0.0,                    2.0 * n / (t - b),      0.0,                        0.0,
        (r + l) / (r - l),      (t + b) / (t - b),      f / (n - f),                -1.0,
        0.0,                    0.0,                    n * f / (n - f),            0.0,
    ]
}

/// Create view matrix (identity — camera at origin looking down -Z).
///
/// The view matrix is identity because all scene geometry is pre-positioned
/// in world space; no camera transform is applied.
pub fn create_view_matrix() -> [f32; 16] {
    [
        1.0, 0.0, 0.0, 0.0,
        0.0, 1.0, 0.0, 0.0,
        0.0, 0.0, 1.0, 0.0,
        0.0, 0.0, 0.0, 1.0,
    ]
}

/// Convert [f32; 16] to [[f32; 4]; 4]
fn mat16_to_mat4x4(m: &[f32; 16]) -> [[f32; 4]; 4] {
    [
        [m[0], m[1], m[2], m[3]],
        [m[4], m[5], m[6], m[7]],
        [m[8], m[9], m[10], m[11]],
        [m[12], m[13], m[14], m[15]],
    ]
}

/// Create transform uniforms for a draw with a custom model matrix.
///
/// Shared implementation used by:
/// `create_transform_uniforms` calls this with identity for the frame-level transform
/// uniform written at setup and resize time.
///
/// Takes the model as `[[f32; 4]; 4]` (column-major nested arrays) and flattens it
/// internally, because `multiply_mat4` and `normal_matrix_from_mat16` work on `[f32; 16]`.
pub fn create_transform_uniforms_with_model(
    width: u32,
    height: u32,
    model: &[[f32; 4]; 4],
) -> TransformUniformsGPU {
    use crate::matrix::normal_matrix_from_mat16;

    let projection = create_projection_matrix(width as f32, height as f32);
    let view = create_view_matrix();

    // Flatten the nested column-major array into a flat column-major array.
    // The outer index of the nested form is the column.
    let model_flat: [f32; 16] = [
        model[0][0], model[0][1], model[0][2], model[0][3],  // column 0
        model[1][0], model[1][1], model[1][2], model[1][3],  // column 1
        model[2][0], model[2][1], model[2][2], model[2][3],  // column 2
        model[3][0], model[3][1], model[3][2], model[3][3],  // column 3
    ];

    // MVP = projection * view * model
    let mv = multiply_mat4(&view, &model_flat);
    let mvp = multiply_mat4(&projection, &mv);

    // Normal matrix: upper-left 3x3 of modelView, padded to 4x4, no transpose.
    let normal_matrix = normal_matrix_from_mat16(&mv);

    // Identity for texture matrices
    let identity = [
        [1.0, 0.0, 0.0, 0.0],
        [0.0, 1.0, 0.0, 0.0],
        [0.0, 0.0, 1.0, 0.0],
        [0.0, 0.0, 0.0, 1.0],
    ];

    TransformUniformsGPU {
        mvp: mat16_to_mat4x4(&mvp),
        model_view: mat16_to_mat4x4(&mv),
        normal_matrix,
        tex_matrix: identity,
        tex_matrix1: identity,
        clip_plane: [0.0, 0.0, 0.0, 0.0],
        clip_enabled: [0.0, 0.0, 0.0, 0.0],
    }
}

/// Create transform uniforms for the game scene (frame-level, model = identity).
pub fn create_transform_uniforms(width: u32, height: u32) -> TransformUniformsGPU {
    const IDENTITY: [[f32; 4]; 4] = [
        [1.0, 0.0, 0.0, 0.0],
        [0.0, 1.0, 0.0, 0.0],
        [0.0, 0.0, 1.0, 0.0],
        [0.0, 0.0, 0.0, 1.0],
    ];
    create_transform_uniforms_with_model(width, height, &IDENTITY)
}

/// Headlight Y offset from camera (2.1 grid elements).
pub const HEADLIGHT_Y_OFFSET: f32 = 4.2;

/// Headlight Z offset from camera.
pub const HEADLIGHT_Z_OFFSET: f32 = -35.0;

/// Create lighting uniforms with headlight at (0, 4.2, -35) relative to camera.
pub fn create_lighting_uniforms() -> LightingUniformsGPU {
    LightingUniformsGPU {
        ambient: [0.2, 0.2, 0.2, 1.0],
        diffuse: [1.0, 1.0, 1.0, 1.0],
        specular: [1.0, 1.0, 1.0, 1.0],
        light_pos: [0.0, HEADLIGHT_Y_OFFSET, HEADLIGHT_Z_OFFSET, 1.0],
        shininess: 10.0,
        panic_intensity: 0.0,
        game_time: 0.0,
        garbage_warm: 0.25,
        // Neutral dying zone defaults.
        dying_zone: [-0.5, 0.5, -0.5, 0.5],
    }
}

/// Prepare per-frame lighting uniforms with headlight modulation.
///
/// `countdown_progress`: 0.0 at countdown start, 1.0 at countdown end.
/// None if no countdown is active.
pub fn prepare_lighting_uniforms(
    frame: &crackattack_core::FrameData,
    countdown_progress: Option<f32>,
) -> LightingUniformsGPU {
    use crackattack_core::GamePhase;

    let headlight_level = if let Some(progress) = countdown_progress {
        // During countdown: ramp from 0→1 as countdown progresses
        progress.sqrt()
    } else {
        match frame.ui.phase {
            // Fade light from 1.0 to 0.0 over 200 ticks after game ends.
            GamePhase::Finished => (1.0 - frame.ui.ticks_since_finished as f32 / 200.0).max(0.0),
            _ => 1.0,
        }
    };

    // panic_intensity depends only on free_rows, not on danger_bar.is_high_alert
    // (which drives its own UI readout and does not change the scene-wide panic tint).
    //
    //   free_rows = 0  -> panic_intensity = 1.0
    //   free_rows = 1  -> 0.5
    //   free_rows >= 2 -> 0.0
    let free_rows = crackattack_core::constants::SAFE_HEIGHT as i32 - frame.ui.level;
    // Panic starts at 1 free row remaining, full at 0.
    let panic_intensity = if free_rows <= 1 {
        1.0 - (free_rows as f32 * 0.5) // free=1 -> 0.5, free=0 -> 1.0
    } else {
        0.0
    };

    let d = LightingUniformsGPU::default();
    LightingUniformsGPU {
        diffuse: [d.diffuse[0]*headlight_level, d.diffuse[1]*headlight_level, d.diffuse[2]*headlight_level, d.diffuse[3]],
        // specular.rgb is read by no shader, so it is not modulated by the headlight.
        // specular.w is overwritten every frame (it carries `wave_amp`).
        // game_time is read by nothing, so it stays 0.
        panic_intensity,
        ..d
    }
}

fn multiply_mat4(a: &[f32; 16], b: &[f32; 16]) -> [f32; 16] {
    let mut result = [0.0f32; 16];
    for row in 0..4 {
        for col in 0..4 {
            for k in 0..4 {
                result[col * 4 + row] += a[k * 4 + row] * b[col * 4 + k];
            }
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_projection_matrix() {
        // Aspect 1:1: the base frustum is unchanged (r-l = 24).
        let proj = create_projection_matrix(480.0, 480.0);
        assert!(proj[0] != 0.0); // x scale: 2*near/(right-left)
        assert!(proj[5] != 0.0); // y scale: 2*near/(top-bottom)
        assert!(proj[10] != 0.0); // f/(n-f)
        assert_eq!(proj[11], -1.0); // perspective divide
        assert!((proj[0] - (2.0 * 38.0 / 24.0)).abs() < 0.001); // 2*near/(right-left), r-l=24

        // Landscape aspect: the horizontal extent grows, the vertical one does not.
        let wide = create_projection_matrix(1600.0, 900.0); // 16:9
        assert!((wide[5] - proj[5]).abs() < 1e-6, "vertical unchanged");
        assert!(wide[0] < proj[0], "horizontal widened (2n/(r-l) smaller)");
        // r-l = 24 * 16/9
        assert!((wide[0] - (2.0 * 38.0 / (24.0 * 16.0 / 9.0))).abs() < 0.001);
    }

    #[test]
    fn test_matrix_multiply() {
        let identity = [
            1.0, 0.0, 0.0, 0.0,
            0.0, 1.0, 0.0, 0.0,
            0.0, 0.0, 1.0, 0.0,
            0.0, 0.0, 0.0, 1.0,
        ];
        let result = multiply_mat4(&identity, &identity);
        assert_eq!(result, identity);

        // Column-major, `a * b` (b applied first). Translation × scale is not commutative:
        // the translation column must stay (1, 2, 3); `b * a` would give (2, 6, 12).
        let translation = [
            1.0, 0.0, 0.0, 0.0,
            0.0, 1.0, 0.0, 0.0,
            0.0, 0.0, 1.0, 0.0,
            1.0, 2.0, 3.0, 1.0,
        ];
        let scale = [
            2.0, 0.0, 0.0, 0.0,
            0.0, 3.0, 0.0, 0.0,
            0.0, 0.0, 4.0, 0.0,
            0.0, 0.0, 0.0, 1.0,
        ];
        assert_eq!(
            multiply_mat4(&translation, &scale),
            [
                2.0, 0.0, 0.0, 0.0,
                0.0, 3.0, 0.0, 0.0,
                0.0, 0.0, 4.0, 0.0,
                1.0, 2.0, 3.0, 1.0,
            ]
        );
    }

    /// Checks structural properties of `create_transform_uniforms_with_model` on a
    /// non-trivial model matrix (rotation, scale and translation).
    #[test]
    fn test_create_transform_uniforms_with_model_properties() {
        let model: [[f32; 4]; 4] = [
            [0.5196152, 0.0,        -0.3,       0.0],
            [0.0776457, 0.5796362,   0.1344973, 0.0],
            [0.3,       -0.1554841,  0.5196152, 0.0],
            [-5.6,      4.5,        -42.5,      1.0],
        ];
        let t = create_transform_uniforms_with_model(700, 700, &model);

        // The model appears unchanged in model_view (the view is identity).
        assert_eq!(t.model_view, model);
        // The MVP differs from the model (the projection is not identity)...
        assert_ne!(t.mvp, model);
        // ...and every component is finite.
        for row in &t.mvp {
            for v in row {
                assert!(v.is_finite());
            }
        }
        // The normal matrix keeps a homogeneous 4th row and column.
        assert_eq!(t.normal_matrix[3][3], 1.0);

        // Static fields.
        let identity = [
            [1.0, 0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
            [0.0, 0.0, 0.0, 1.0],
        ];
        assert_eq!(t.tex_matrix, identity);
        assert_eq!(t.tex_matrix1, identity);
        assert_eq!(t.clip_plane, [0.0, 0.0, 0.0, 0.0]);
        assert_eq!(t.clip_enabled, [0.0, 0.0, 0.0, 0.0]);
    }
}
