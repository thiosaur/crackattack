//! UI renderer: generates UI batches for the HUD.
//!
//! This module generates `UIBatch` data for:
//! - Clock and score display (digit textures)
//! - Result messages (GAME OVER, WINNER, etc.)
//! - Countdown (3, 2, 1, GO)
//! - Level lights (3D prisms)
//! - Danger bars (glow on the sides)

use crate::renderer::gpu::ui_batch::{UIBatch, UIBlendMode};
use crate::renderer::scene::game_renderer::GRID_ELEMENT_LENGTH;
use crate::renderer::gpu::types::{UIVertex, UITextureMode};
use crate::renderer::ui::layout::SCREEN;
use super::constants::*;
use super::helpers::*;
use crackattack_core::{FrameData, UIState, CountdownDigit, CountdownInfo};

/// Visual type for game result texture rendering.
/// Renderer-internal — not a transport type. Mapped from SessionResult.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ResultVisual {
    GameOver,
    Winner,
    RoundWin,
    RoundLose,
}

impl ResultVisual {
    /// Map from SessionResult to the visual type for texture rendering.
    pub fn from_result(result: crackattack_core::frame_data::SessionResult) -> Option<Self> {
        use crackattack_core::frame_data::SessionResult;
        match result {
            SessionResult::SessionWin => Some(Self::Winner),
            SessionResult::SessionLose => Some(Self::GameOver),
            SessionResult::RoundWin => Some(Self::RoundWin),
            SessionResult::RoundLose => Some(Self::RoundLose),
            SessionResult::None => None,
        }
    }
}


/// UI Renderer generates batches from game state
pub struct UIRenderer {
    /// Cached digit texture names
    digit_textures: [&'static str; 10],
    /// Extra clock texture (colon, etc.)
    clock_extra_texture: &'static str,
    /// Logo texture name
    logo_texture: &'static str,
    /// Nameplate texture name (generated "PLAYER 1")
    nameplate_texture: &'static str,
    /// Message textures by type
    message_textures: std::collections::HashMap<ResultVisual, &'static str>,
    /// Countdown textures
    countdown_textures: std::collections::HashMap<CountdownDigit, &'static str>,

    /// Wall-clock seconds (advances even during pause)
    pub wall_clock: f32,
    /// Logo tremor (x, y offsets in world units, tilt in radians counterclockwise),
    /// from `LogoShakeAnimator`.
    pub logo_shake: (f32, f32, f32),
    /// Reflow offsets set on resize (see `init::reflow_offsets`). All are 0 for a square window.
    /// Info column: horizontal offset (negative, toward the left).
    pub reflow_info_dx: f32,
    /// Level lights: offset toward the edge, inset by the outer margin.
    pub reflow_light_dx: f32,
    /// Danger bar: full offset to the edge (edge glow, no outer margin).
    pub reflow_edge_dx: f32,
}

impl UIRenderer {
    /// Create a new UI renderer
    pub fn new() -> Self {
        const DIGIT_TEXTURES: [&str; 10] = [
            "clock_0", "clock_1", "clock_2", "clock_3", "clock_4",
            "clock_5", "clock_6", "clock_7", "clock_8", "clock_9",
        ];
        let digit_textures = DIGIT_TEXTURES;
        let clock_extra_texture = "clock_extra";
        let logo_texture = "logo";
        let nameplate_texture = "nameplate";
        let mut message_textures = std::collections::HashMap::new();
        message_textures.insert(ResultVisual::GameOver, "message_game_over");
        message_textures.insert(ResultVisual::Winner, "message_winner");
        message_textures.insert(ResultVisual::RoundWin, "message_round_win");
        message_textures.insert(ResultVisual::RoundLose, "message_round_lose");

        let mut countdown_textures = std::collections::HashMap::new();
        countdown_textures.insert(CountdownDigit::Three, "count_down_3");
        countdown_textures.insert(CountdownDigit::Two, "count_down_2");
        countdown_textures.insert(CountdownDigit::One, "count_down_1");
        countdown_textures.insert(CountdownDigit::Go, "count_down_go");

        Self {
            digit_textures,
            clock_extra_texture,
            logo_texture,
            nameplate_texture,
            message_textures,
            countdown_textures,
            wall_clock: 0.0,
            logo_shake: (0.0, 0.0, 0.0),
            reflow_info_dx: 0.0,
            reflow_light_dx: 0.0,
            reflow_edge_dx: 0.0,
        }
    }

    /// Horizontal offset of the info column (logo, nameplate, clock/score).
    #[inline]
    fn info_dx(&self) -> f32 { self.reflow_info_dx }

    /// Generate all UI batches for a frame.
    ///
    /// `curtain_alpha` (0.0..=1.0): current opacity of the game curtain.
    /// When >= 1.0 the HUD is fully hidden — only the curtain overlay and
    /// result message are rendered (by generate_and_record_ui).
    pub fn generate_batches(
        &mut self,
        frame: &FrameData,
        vs_display: Option<&crackattack_core::frame_data::VsDisplay>,
        is_game_over_wait: bool,
        danger_bar: &crate::renderer::effects::game_animators::DangerBarSnapshot,
        opponent_board: &crate::renderer::effects::game_animators::OpponentBoardAnimator,
        panic_pulse: f32,
        curtain_alpha: f32,
    ) -> Vec<UIBatch> {
        let mut batches = Vec::with_capacity(16);
        let has_opponent = vs_display.is_some();

        // Curtain fully closed: HUD hidden, only overlays and result
        // message will be added by generate_and_record_ui.
        if curtain_alpha >= 1.0 {
            return batches;
        }

        // Always render logo
        batches.push(self.generate_logo_batch());

        // Nameplate: always shown. VS mode uses "PLAYER VS <opponent>" texture.
        {
            let tex_name: std::borrow::Cow<'static, str> = if let Some(vs) = vs_display {
                format!("nameplate_vs_{}", vs.opponent_name.to_lowercase()).into()
            } else {
                self.nameplate_texture.into()
            };
            batches.push(self.generate_nameplate_batch_with_texture(&tex_name));
        }

        // VS mode: clock + session score. Solo mode: score digits.
        if has_opponent {
            batches.extend(self.generate_clock_batches(&frame.ui));
            batches.extend(self.generate_session_score_batches(vs_display));
            if let Some(view) = opponent_board.view() {
                batches.push(self.generate_opponent_board_batch(view, opponent_board));
            }
        } else {
            batches.extend(self.generate_score_digit_batches(&frame.ui));
        }

        // The countdown is drawn by `generate_and_record_ui`, after the cursor, so it
        // appears in front.

        // Danger bars — not during GameOverWait (already faded)
        if !is_game_over_wait {
            if let Some(batch) = self.generate_danger_bar_batch(danger_bar, panic_pulse) {
                batches.push(batch);
            }
        }

        // The result message is drawn by `generate_and_record_ui`, after the game
        // curtain overlay.

        batches
    }

    /// Generate clock display batches (MM:SS format)
    /// Digit layout uses simplified centering. Not pixel-identical to the original.
    fn generate_clock_batches(&self, ui: &UIState) -> Vec<UIBatch> {
        let mut batches = Vec::with_capacity(6);

        // Convert clock time (in game ticks) to seconds
        let total_seconds = ui.clock_time / GC_STEPS_PER_SECOND;
        let minutes = (total_seconds / 60) % 100;
        let seconds = total_seconds % 60;

        // Colon at center, digits placed outward from the colon
        // Seconds to the right, minutes to the left

        let z = EXTERNAL_OFFSET_Z;
        let l = CLOCK_DIGIT_LENGTH;
        let base_y = CLOCK_OFFSET_Y;

        // Gradient: dark blue (BL) → medium blue (BR, TL) → white (TR)
        let color_bl = [0.3, 0.3, 1.0, 1.0];
        let color_br = [0.5, 0.5, 1.0, 1.0];
        let color_tl = [0.5, 0.5, 1.0, 1.0];
        let color_tr = [1.0, 1.0, 1.0, 1.0];

        let vertex = |vx: f32, vy: f32, tu: f32, tv: f32, color: [f32; 4]| -> UIVertex {
            UIVertex {
                x: vx, y: vy, z,
                u: tu, v: tv,
                r: color[0], g: color[1], b: color[2], a: color[3],
            }
        };

        // Colon position (plus the reflow offset of the info column)
        let colon_x = CLOCK_OFFSET_X + self.info_dx() - CLOCK_DIGIT_OFFSET / 2.0;

        // Colon
        let mut colon_batch = UIBatch::alpha_text(self.clock_extra_texture);
        colon_batch.push_quad(
            vertex(colon_x - l, base_y - l, 0.0, 1.0, color_bl),
            vertex(colon_x + l, base_y - l, 1.0, 1.0, color_br),
            vertex(colon_x - l, base_y + l, 0.0, 0.0, color_tl),
            vertex(colon_x + l, base_y + l, 1.0, 0.0, color_tr),
        );
        batches.push(colon_batch);

        // Seconds (right of colon): s0 is rightmost, s1 is left of it
        let sec_start = colon_x + CLOCK_CENTER_OFFSET / 2.0 + 3.0 * CLOCK_DIGIT_OFFSET / 2.0;
        let s0 = (seconds % 10) as usize;
        let s1 = (seconds / 10) as usize;
        for (i, &d) in [s0, s1].iter().enumerate() {
            let dx = sec_start - (i as f32) * CLOCK_DIGIT_OFFSET;
            let mut batch = UIBatch::alpha_text(self.digit_textures[d]);
            batch.push_quad(
                vertex(dx - l, base_y - l, 0.0, 1.0, color_bl),
                vertex(dx + l, base_y - l, 1.0, 1.0, color_br),
                vertex(dx - l, base_y + l, 0.0, 0.0, color_tl),
                vertex(dx + l, base_y + l, 1.0, 0.0, color_tr),
            );
            batches.push(batch);
        }

        // Minutes (left of colon)
        let min_start = sec_start - CLOCK_DIGIT_OFFSET - CLOCK_DIGIT_OFFSET - CLOCK_CENTER_OFFSET;
        let m0 = (minutes % 10) as usize;
        let mut batch = UIBatch::alpha_text(self.digit_textures[m0]);
        batch.push_quad(
            vertex(min_start - l, base_y - l, 0.0, 1.0, color_bl),
            vertex(min_start + l, base_y - l, 1.0, 1.0, color_br),
            vertex(min_start - l, base_y + l, 0.0, 0.0, color_tl),
            vertex(min_start + l, base_y + l, 1.0, 0.0, color_tr),
        );
        batches.push(batch);

        if minutes >= 10 {
            let m1 = (minutes / 10) as usize;
            let dx = min_start - CLOCK_DIGIT_OFFSET;
            let mut batch = UIBatch::alpha_text(self.digit_textures[m1]);
            batch.push_quad(
                vertex(dx - l, base_y - l, 0.0, 1.0, color_bl),
                vertex(dx + l, base_y - l, 1.0, 1.0, color_br),
                vertex(dx - l, base_y + l, 0.0, 0.0, color_tl),
                vertex(dx + l, base_y + l, 1.0, 0.0, color_tr),
            );
            batches.push(batch);
        }

        batches
    }

    /// Generate level light mesh vertices for the 3D lit pipeline.
    ///
    /// The lights are triangles on both sides that show the grid fill level. On the right
    /// side the tip points left (toward the playfield); the left side is mirrored.
    ///
    /// Returns pre-positioned MeshVertex data with emission color in UV attribute.
    /// Each light is a 3D triangular prism (8 triangles = 24 verts, from triangle.glb).
    /// Total up to 576 vertices (24 lights × 24 verts).
    /// The vertices are in world space — the shader applies the game's view-projection.
    pub fn generate_level_light_mesh_vertices(&self, vs_display: Option<&crackattack_core::frame_data::VsDisplay>, ll_snapshot: &crate::renderer::effects::game_animators::LevelLightsSnapshot) -> Vec<crate::renderer::gpu::types::MeshVertex> {
        let mut verts = Vec::with_capacity(LEVEL_LIGHT_MAX_VERTICES);

        if ll_snapshot.slide_progress <= 0.0 {
            return verts;
        }

        let z = EXTERNAL_OFFSET_Z;
        let base_y = PLAY_OFFSET_Y + GRID_ELEMENT_LENGTH / 2.0;
        let spacing = GRID_ELEMENT_LENGTH;

        let local_level = ll_snapshot.local_level;
        let opponent_level = vs_display.map_or(-1, |v| v.opponent_level);
        let has_opponent_data = opponent_level >= 0;
        let panic_pulse = ll_snapshot.panic_pulse;
        let slide_offset_right = ll_snapshot.slide_offset_right;
        let slide_offset_left = ll_snapshot.slide_offset_left;

        // Left and right lights move toward the widened edges, inset by the outer margin
        // (see `reflow_offsets`).
        let light_dx = self.reflow_light_dx;
        for pos in 0..2 {
            let is_left = pos == 0;
            let slide_offset = if is_left { slide_offset_left } else { slide_offset_right };
            let base_x = if is_left { LEVEL_LIGHT_LEFT_X - light_dx } else { LEVEL_LIGHT_RIGHT_X + light_dx };

            for n in 0..LEVEL_LIGHT_COUNT as usize {
                let y = base_y + ((n + 1) as f32) * spacing;
                let x = base_x + slide_offset;

                // Color: blue = neither player has this level, orange = both (or solo),
                // red = only the local player, green = only the opponent.
                let local_has = (n as i32) < local_level;
                let opp_has = has_opponent_data && (n as i32) < opponent_level;

                let mut color = if !local_has && !opp_has {
                    LEVEL_LIGHT_COLOR_BLUE
                } else if local_has && opp_has {
                    LEVEL_LIGHT_COLOR_ORANGE
                } else if !has_opponent_data {
                    if local_has { LEVEL_LIGHT_COLOR_ORANGE } else { LEVEL_LIGHT_COLOR_BLUE }
                } else {
                    if local_has { LEVEL_LIGHT_COLOR_RED } else { LEVEL_LIGHT_COLOR_GREEN }
                };

                // Panic effect on level lights — dim mode: darken color at pulse peak.
                if panic_pulse > 0.0 {
                    let dim = 1.0 - panic_pulse;
                    color[0] *= dim;
                    color[1] *= dim;
                    color[2] *= dim;
                }

                color[0] = color[0].min(1.0);
                color[1] = color[1].min(1.0);
                color[2] = color[2].min(1.0);

                let alpha = 1.0_f32;

                let sign = if is_left { -1.0_f32 } else { 1.0_f32 };

                for &idx in PRISM_INDICES.iter() {
                    let p = PRISM_POSITIONS[idx];
                    let n = PRISM_NORMALS[idx];
                    verts.push(crate::renderer::gpu::types::MeshVertex {
                        pos: [x + sign * p[0], y + p[1], z + p[2]],
                        normal: [sign * n[0], n[1], n[2]],
                        uv: color,
                        uv_diffuse: [alpha, 0.0],
                    });
                }
            }
        }

        verts
    }

    /// Generate logo batch. The logo floats like the menu one (`logo_float`).
    pub fn generate_logo_batch(&self) -> UIBatch {
        use crate::renderer::ui::layout::{logo_float, UI_Z, LEFT_COL_X, LOGO_Y, LOGO_HW, LOGO_HH};
        let (lift, tilt) = logo_float(self.wall_clock as f64);
        // World Y points up and a positive angle turns counterclockwise: both signs flip
        // compared with the screen convention of `logo_float`.
        let (shake_x, shake_y, shake_tilt) = self.logo_shake;
        let (cx, cy) = (LEFT_COL_X + self.info_dx() + shake_x, LOGO_Y - lift * 2.0 * LOGO_HW + shake_y);
        let (sin, cos) = (shake_tilt - tilt).sin_cos();
        let mut batch = UIBatch::with_texture_mode(
            Some(std::borrow::Cow::Borrowed(self.logo_texture)), UITextureMode::Modulate, UIBlendMode::Alpha);
        let v = |dx: f32, dy: f32, tu: f32, tv: f32| UIVertex {
            x: cx + dx * cos - dy * sin, y: cy + dx * sin + dy * cos, z: UI_Z,
            u: tu, v: tv, r: 1.0, g: 1.0, b: 1.0, a: 1.0,
        };
        batch.push_quad(
            v(-LOGO_HW, -LOGO_HH, 0.0, 1.0), v(LOGO_HW, -LOGO_HH, 1.0, 1.0),
            v(-LOGO_HW, LOGO_HH, 0.0, 0.0), v(LOGO_HW, LOGO_HH, 1.0, 0.0),
        );
        batch
    }

    /// Generate nameplate batch with a specific texture name (may be dynamic for VS mode).
    fn generate_nameplate_batch_with_texture(&self, tex_name: &str) -> UIBatch {
        use crate::renderer::ui::layout::{UI_Z, LEFT_COL_X, NAME_Y, NAME_HW, NAME_HH};
        let mut batch = UIBatch::with_texture_mode(
            Some(std::borrow::Cow::Owned(tex_name.to_string())),
            UITextureMode::AlphaText, UIBlendMode::Alpha,
        );
        let cx = LEFT_COL_X + self.info_dx();
        let v = |vx: f32, vy: f32, tu: f32, tv: f32| -> UIVertex {
            UIVertex { x: vx, y: vy, z: UI_Z, u: tu, v: tv, r: 1.0, g: 1.0, b: 1.0, a: 1.0 }
        };
        batch.push_quad(
            v(cx - NAME_HW, NAME_Y - NAME_HH, 0.0, 1.0),
            v(cx + NAME_HW, NAME_Y - NAME_HH, 1.0, 1.0),
            v(cx - NAME_HW, NAME_Y + NAME_HH, 0.0, 0.0),
            v(cx + NAME_HW, NAME_Y + NAME_HH, 1.0, 0.0),
        );
        batch
    }


    /// Generate batches for all score digits
    ///
    /// The score is centered around CLOCK_OFFSET_X, digits spaced consistently with the clock.
    /// Per-vertex gradient: dark blue (BL) → medium blue (BR,TL) → white (TR).
    pub fn generate_score_digit_batches(&self, ui: &UIState) -> Vec<UIBatch> {
        let score = ui.score.max(0);
        let digits = Self::number_to_digits(score, 4); // minimum 4 digits displayed

        let num_digits = digits.len();
        let spacing = CLOCK_DIGIT_OFFSET;
        // Center digit group around CLOCK_OFFSET_X (digits are MSB-first → left to right)
        let start_x = CLOCK_OFFSET_X + self.info_dx() - ((num_digits - 1) as f32 * spacing) / 2.0;
        let base_y = SCORE_OFFSET_Y;
        let z = EXTERNAL_OFFSET_Z;
        let l = CLOCK_DIGIT_LENGTH;

        digits.iter().enumerate().map(|(i, &digit)| {
            let dx = start_x + (i as f32) * spacing;

            // clock_N.png are alpha-only (RGB=0, shape in alpha)
            let mut batch = UIBatch::alpha_text(self.digit_textures[digit as usize]);

            // Gradient: BL=(0.3,0.3,1.0), BR/TL=(0.5,0.5,1.0), TR=(1.0,1.0,1.0)
            batch.push_quad(
                UIVertex { x: dx - l, y: base_y - l, z, u: 0.0, v: 1.0, r: 0.3, g: 0.3, b: 1.0, a: 1.0 },
                UIVertex { x: dx + l, y: base_y - l, z, u: 1.0, v: 1.0, r: 0.5, g: 0.5, b: 1.0, a: 1.0 },
                UIVertex { x: dx - l, y: base_y + l, z, u: 0.0, v: 0.0, r: 0.5, g: 0.5, b: 1.0, a: 1.0 },
                UIVertex { x: dx + l, y: base_y + l, z, u: 1.0, v: 0.0, r: 1.0, g: 1.0, b: 1.0, a: 1.0 },
            );

            batch
        }).collect()
    }


    /// Generate result display batch from a ResultVisual.
    /// Called with the message derived from SessionResult.
    ///
    /// Unified Stamp animation: same behaviour for win and lose.
    pub fn generate_result_batch(&mut self, message_type: ResultVisual, ticks_since_finished: i32) -> Option<UIBatch> {
        let texture_name = *self.message_textures.get(&message_type)?;
        let mut batch = UIBatch::modulate(texture_name);

        let t = ticks_since_finished as f32;
        let x = SCREEN.center_x();
        let z = PLAY_OFFSET_Z + 0.3;

        let base_scale = MESSAGE_SCALE;

        // Unified Stamp animation (same for win and lose)
        let stamp_scale = if t < 20.0 {
            // First 20 ticks: appear, scale 3x to 1x
            let p = (t / 20.0).clamp(0.0, 1.0);
            3.0 - 2.0 * ease_out_quad(p)
        } else if t < 45.0 {
            // Next 25 ticks: settle, overshoot 1.0 to 1.06 and back
            let p = ((t - 20.0) / 25.0).clamp(0.0, 1.0);
            1.0 + 0.06 * (1.0 - p) * (std::f32::consts::PI * p).sin()
        } else {
            // After that: idle, subtle breathing
            1.0 + 0.015 * (self.wall_clock * 1.5 * std::f32::consts::TAU).sin()
        };

        let alpha = (t / 10.0).clamp(0.0, 1.0);

        let w = base_scale * stamp_scale;
        let h = w; // messages are square
        let y = SCREEN.center_y() + 2.0;

        let vertex = |vx: f32, vy: f32, tu: f32, tv: f32| -> UIVertex {
            UIVertex { x: vx, y: vy, z, u: tu, v: tv, r: 1.0, g: 1.0, b: 1.0, a: alpha }
        };
        batch.push_quad(
            vertex(x - w, y - h, 0.0, 1.0),
            vertex(x + w, y - h, 1.0, 1.0),
            vertex(x - w, y + h, 0.0, 0.0),
            vertex(x + w, y + h, 1.0, 0.0),
        );
        Some(batch)
    }

    /// Generate countdown display batch (Stamp animation).
    pub fn generate_countdown_batch(&self, info: &CountdownInfo) -> Option<UIBatch> {
        let countdown = info.digit;
        let texture_name = *self.countdown_textures.get(&countdown)?;

        let mut batch = UIBatch::modulate(texture_name);

        let base_y = PLAY_OFFSET_Y + GRID_ELEMENT_LENGTH * GC_SAFE_HEIGHT as f32 * COUNTDOWN_HEIGHT_RATIO;
        let z = PLAY_OFFSET_Z + 0.6;
        let t = info.ticks_into_digit as f32 / info.ticks_per_digit as f32;
        let is_go = matches!(countdown, CountdownDigit::Go);
        let base_scale = if is_go { COUNTDOWN_SCALE * 1.5 } else { COUNTDOWN_SCALE };

        // Stamp: starts big (3×), slams down to 1×
        let slam = if t < 0.1 { 3.0 - 2.0 * (t / 0.1) } else { 1.0 };
        let alpha = if t < 0.05 { t / 0.05 } else { 1.0 };
        let (mut sx, mut sy) = (base_scale * slam, base_scale * slam);

        // Exit animation for GO (zoom-out + fade in last 30%)
        let alpha = if is_go && t > 0.7 {
            let exit_t = (t - 0.7) / 0.3;
            let grow = 1.0 + 0.4 * exit_t;
            sx *= grow;
            sy *= grow;
            alpha * (1.0 - exit_t)
        } else {
            alpha
        };

        let vertex = |vx: f32, vy: f32, tu: f32, tv: f32| -> UIVertex {
            UIVertex {
                x: vx, y: vy, z,
                u: tu, v: tv,
                r: 1.0, g: 1.0, b: 1.0, a: alpha,
            }
        };

        batch.push_quad(
            vertex(-sx, base_y - sy, 0.0, 1.0),
            vertex(sx, base_y - sy, 1.0, 1.0),
            vertex(-sx, base_y + sy, 0.0, 0.0),
            vertex(sx, base_y + sy, 1.0, 0.0),
        );

        Some(batch)
    }

    /// Danger bars: an untextured glow along both screen edges, over the full height.
    /// The color depends on the alert level. The base alpha is 0.4, modulated by the
    /// `DangerBar` state (top fade, falloff, panic pulse).
    fn generate_danger_bar_batch(&self, db: &crate::renderer::effects::game_animators::DangerBarSnapshot, panic_pulse: f32) -> Option<UIBatch> {
        let width = super::constants::DANGER_BAR_FADE_WIDTH;
        let falloff = super::constants::DANGER_BAR_FALLOFF;
        if !db.visible {
            return None;
        }

        let mut batch = UIBatch::new(None, UIBlendMode::Alpha);

        // Full reference plane height
        let bar_bottom = REF_BOTTOM;
        let bar_total_height = REF_HEIGHT;
        // The danger bar is an edge glow, so it stays on the widened edge (full E/2, no outer
        // margin, unlike the level lights).
        let ref_left = REF_LEFT - self.reflow_edge_dx;
        let ref_right = REF_RIGHT + self.reflow_edge_dx;
        let bar_height = bar_total_height * db.bar;
        let z = EXTERNAL_OFFSET_Z;

        // Danger bar color gradient:
        //   Inactive:   [0.0, 0.0, 1.0] blue
        //   Low alert:  [0.8, 0.0, 0.8] purple
        //   High alert: [1.0, 0.0, 0.0] red
        let (cr, cg, cb) = if db.is_high_alert {
            (1.0_f32, 0.0_f32, 0.0_f32)
        } else if db.is_low_alert {
            (0.8_f32, 0.0_f32, 0.8_f32)
        } else {
            (0.0_f32, 0.0_f32, 1.0_f32)
        };

        // Base alpha 0.4, multiplied by fade alpha
        // Base alpha + panic breathing (pulse boosts opacity by up to 50%)
        let ca = 0.4 * db.alpha * (1.0 + panic_pulse * 0.5);

        if ca <= 0.0 || bar_height <= 0.0 {
            return None;
        }

        let v = |vx: f32, vy: f32, a: f32| -> UIVertex {
            UIVertex { x: vx, y: vy, z, u: 0.0, v: 0.0, r: cr, g: cg, b: cb, a }
        };

        // Render as N vertical strips with alpha following a power curve.
        const STRIPS: u32 = 6;

        // Vertical soft top: the top ~15% of bar height fades out
        let top_fade_ratio = 0.15;
        let solid_height = bar_height * (1.0 - top_fade_ratio);
        let fade_top_start = bar_bottom + solid_height;
        let fade_top_end = bar_bottom + bar_height;

        for i in 0..STRIPS {
            let t0 = i as f32 / STRIPS as f32;
            let t1 = (i + 1) as f32 / STRIPS as f32;
            let a0 = ca * (1.0 - t0).powf(falloff);
            let a1 = ca * (1.0 - t1).powf(falloff);

            // Left bar
            let lx0 = ref_left + t0 * width;
            let lx1 = ref_left + t1 * width;
            batch.push_quad(v(lx0, bar_bottom, a0), v(lx1, bar_bottom, a1),
                           v(lx0, fade_top_start, a0), v(lx1, fade_top_start, a1));
            batch.push_quad(v(lx0, fade_top_start, a0), v(lx1, fade_top_start, a1),
                           v(lx0, fade_top_end, 0.0), v(lx1, fade_top_end, 0.0));

            // Right bar (mirrored)
            let rx0 = ref_right - t0 * width;
            let rx1 = ref_right - t1 * width;
            batch.push_quad(v(rx0, bar_bottom, a0), v(rx1, bar_bottom, a1),
                           v(rx0, fade_top_start, a0), v(rx1, fade_top_start, a1));
            batch.push_quad(v(rx0, fade_top_start, a0), v(rx1, fade_top_start, a1),
                           v(rx0, fade_top_end, 0.0), v(rx1, fade_top_end, 0.0));
        }

        Some(batch)
    }

    /// Session score such as "3 - 2" (rounds won - rounds lost) below the nameplate.
    /// The series is open-ended, so the tally is unbounded. It uses the clock digit
    /// textures at a reduced scale.
    fn generate_session_score_batches(&self, vs_display: Option<&crackattack_core::frame_data::VsDisplay>) -> Vec<UIBatch> {
        let Some(vs) = vs_display else { return Vec::new() };

        let z = EXTERNAL_OFFSET_Z;
        let l = CLOCK_DIGIT_LENGTH * SESSION_SCORE_SCALE;
        let step = CLOCK_DIGIT_OFFSET * SESSION_SCORE_SCALE;
        // The opponent board preview, when there is one, takes the place below the nameplate.
        let base_y = if vs.opponent_board.is_some() { SESSION_SCORE_WITH_BOARD_OFFSET_Y } else { SESSION_SCORE_OFFSET_Y };
        let center_x = LEFT_EXTERNAL_CENTER + self.info_dx();

        // Same gradient as the clock, for a consistent info column.
        let [color_bl, color_br, color_tl, color_tr] = SESSION_SCORE_GRADIENT;
        let vertex = |vx: f32, vy: f32, tu: f32, tv: f32, color: [f32; 4]| -> UIVertex {
            UIVertex {
                x: vx, y: vy, z,
                u: tu, v: tv,
                r: color[0], g: color[1], b: color[2], a: color[3],
            }
        };

        let mut batches = Vec::with_capacity(4);

        // Dash separator at the center: a solid quad (the clock textures have no dash
        // glyph), with the same gradient as the digits.
        let dash_w = l * 0.55;
        let dash_h = l * 0.14;
        let mut dash_batch = UIBatch::new(None, UIBlendMode::Alpha);
        dash_batch.push_quad(
            vertex(center_x - dash_w, base_y - dash_h, 0.0, 1.0, color_bl),
            vertex(center_x + dash_w, base_y - dash_h, 1.0, 1.0, color_br),
            vertex(center_x - dash_w, base_y + dash_h, 0.0, 0.0, color_tl),
            vertex(center_x + dash_w, base_y + dash_h, 1.0, 0.0, color_tr),
        );
        batches.push(dash_batch);

        // Local wins on the left (units digit next to the separator, growing leftward),
        // local losses on the right (growing rightward).
        let wins = Self::number_to_digits(vs.session_wins.max(0), 1);
        for (i, &d) in wins.iter().rev().enumerate() {
            let dx = center_x - step * (1.0 + i as f32);
            let mut batch = UIBatch::alpha_text(self.digit_textures[d as usize]);
            batch.push_quad(
                vertex(dx - l, base_y - l, 0.0, 1.0, color_bl),
                vertex(dx + l, base_y - l, 1.0, 1.0, color_br),
                vertex(dx - l, base_y + l, 0.0, 0.0, color_tl),
                vertex(dx + l, base_y + l, 1.0, 0.0, color_tr),
            );
            batches.push(batch);
        }
        let losses = Self::number_to_digits(vs.session_losses.max(0), 1);
        for (i, &d) in losses.iter().enumerate() {
            let dx = center_x + step * (1.0 + i as f32);
            let mut batch = UIBatch::alpha_text(self.digit_textures[d as usize]);
            batch.push_quad(
                vertex(dx - l, base_y - l, 0.0, 1.0, color_bl),
                vertex(dx + l, base_y - l, 1.0, 1.0, color_br),
                vertex(dx - l, base_y + l, 0.0, 0.0, color_tl),
                vertex(dx + l, base_y + l, 1.0, 0.0, color_tr),
            );
            batches.push(batch);
        }

        batches
    }

    /// Opponent board preview below the nameplate: blocks as one grey mass, garbage as a
    /// red-brown mass, cells drawn edge to edge. A slab that just appeared is bright red and fades
    /// (see `OpponentBoardAnimator`).
    fn generate_opponent_board_batch(
        &self,
        view: &crackattack_core::vs::OpponentBoardView,
        anim: &crate::renderer::effects::game_animators::OpponentBoardAnimator,
    ) -> UIBatch {
        use crackattack_core::constants::PLAY_WIDTH;
        use crackattack_core::vs::board_view::BOARD_VIEW_ROWS;
        use crackattack_core::vs::BoardCell;

        let z = EXTERNAL_OFFSET_Z;
        let c = OPPONENT_BOARD_CELL;
        let left = LEFT_EXTERNAL_CENTER + self.info_dx() - c * PLAY_WIDTH as f32 / 2.0;
        let top = OPPONENT_BOARD_TOP_Y;
        let bottom = top - c * BOARD_VIEW_ROWS as f32;
        let right = left + c * PLAY_WIDTH as f32;

        let mut batch = UIBatch::new(None, UIBlendMode::Alpha);
        // `color` gives each vertex its color from its position.
        let mut rect = |x0: f32, y0: f32, x1: f32, y1: f32, color: &dyn Fn(f32, f32) -> [f32; 4]| {
            let v = |x: f32, y: f32| {
                let [r, g, b, a] = color(x, y);
                UIVertex { x, y, z, u: 0.0, v: 0.0, r, g, b, a }
            };
            batch.push_quad(v(x0, y0), v(x1, y0), v(x0, y1), v(x1, y1));
        };

        let f = OPPONENT_BOARD_FRAME;
        let frame_color = srgb(OPPONENT_BOARD_FRAME_COLOR);
        let background = srgb(OPPONENT_BOARD_BACKGROUND);
        rect(left - f, bottom - f, right + f, top + f, &|_, _| frame_color);
        rect(left, bottom, right, top, &|_, _| background);

        // Blocks: the score digits' gradient, interpolated over the whole preview so the
        // masses read as one surface.
        let [bl, br, tl, tr] = SESSION_SCORE_GRADIENT;
        let block = |x: f32, y: f32| -> [f32; 4] {
            let u = (x - left) / (right - left);
            let v = (y - bottom) / (top - bottom);
            std::array::from_fn(|i| {
                let lower = bl[i] + (br[i] - bl[i]) * u;
                let upper = tl[i] + (tr[i] - tl[i]) * u;
                lower + (upper - lower) * v
            })
        };
        let garbage = srgb(OPPONENT_BOARD_GARBAGE);
        let flash = srgb(OPPONENT_BOARD_GARBAGE_FLASH);
        for row in 0..BOARD_VIEW_ROWS {
            let y0 = bottom + c * row as f32;
            for x in 0..PLAY_WIDTH {
                let x0 = left + c * x as f32;
                match view.cell(x, row) {
                    BoardCell::Empty => {}
                    BoardCell::Block => rect(x0, y0, x0 + c, y0 + c, &block),
                    BoardCell::Garbage(id) => {
                        let t = anim.flash(id);
                        let color: [f32; 4] =
                            std::array::from_fn(|i| garbage[i] + (flash[i] - garbage[i]) * t);
                        rect(x0, y0, x0 + c, y0 + c, &|_, _| color);
                    }
                }
            }
        }
        batch
    }

    /// Convert a number to individual digits
    fn number_to_digits(mut n: i32, min_digits: usize) -> Vec<u8> {
        let mut digits = Vec::with_capacity(7);

        if n == 0 {
            digits.push(0);
        } else {
            while n > 0 {
                digits.push((n % 10) as u8);
                n /= 10;
            }
        }

        // Pad with leading zeros
        while digits.len() < min_digits {
            digits.push(0);
        }

        digits.reverse();
        digits
    }
}

impl Default for UIRenderer {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_number_to_digits() {
        assert_eq!(UIRenderer::number_to_digits(0, 1), vec![0]);
        assert_eq!(UIRenderer::number_to_digits(0, 3), vec![0, 0, 0]);
        assert_eq!(UIRenderer::number_to_digits(123, 3), vec![1, 2, 3]);
        assert_eq!(UIRenderer::number_to_digits(42, 4), vec![0, 0, 4, 2]);
        assert_eq!(UIRenderer::number_to_digits(999999, 6), vec![9, 9, 9, 9, 9, 9]);
    }

    #[test]
    fn test_ui_batch_push_quad() {
        let mut batch = UIBatch::new(None, UIBlendMode::Opaque);

        let v = |vx: f32, vy: f32| UIVertex {
            x: vx, y: vy, z: 0.0,
            u: 0.0, v: 0.0,
            r: 1.0, g: 1.0, b: 1.0, a: 1.0,
        };

        batch.push_quad(v(0.0, 0.0), v(1.0, 0.0), v(0.0, 1.0), v(1.0, 1.0));

        assert_eq!(batch.vertices.len(), 4);
        assert_eq!(batch.indices.len(), 6);
        assert_eq!(batch.indices, vec![0, 1, 2, 2, 1, 3]);
    }

    #[test]
    fn test_ui_renderer_creation() {
        let renderer = UIRenderer::new();
        assert_eq!(renderer.digit_textures.len(), 10);
        assert!(renderer.message_textures.contains_key(&ResultVisual::GameOver));
        assert!(renderer.countdown_textures.contains_key(&CountdownDigit::Go));
    }
}

/// An sRGB `0xRRGGBB` color as the linear RGBA the UI vertices take (the surface is sRGB).
fn srgb(hex: u32) -> [f32; 4] {
    let channel = |shift: u32| {
        let c = ((hex >> shift) & 0xff) as f32 / 255.0;
        if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
    };
    [channel(16), channel(8), channel(0), 1.0]
}
