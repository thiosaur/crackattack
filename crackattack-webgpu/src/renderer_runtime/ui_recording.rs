//! UI batch recording: menu, game HUD, cursor, countdown, postgame.
//!
//! Generates all UI batches and records them into the command buffer.

use crackattack_core::FrameData;

use super::super::SessionState;
use super::RendererRuntime;
use super::frame_plan::FramePlan;
use crate::scheduling::SchedulingState;

impl RendererRuntime {
    /// Generate all UI batches (game HUD, cursor silhouette, pause overlay)
    /// and record them into the command buffer.
    pub(super) fn generate_and_record_ui(&mut self, frame: &FrameData, plan: &FramePlan, _session: &SessionState, scheduling: &SchedulingState) {
        use crate::renderer::gpu::ui_batch::{UIBatch, UIBlendMode};
        use crate::renderer::gpu::types::UIVertex;
        use crate::renderer::ui::layout::UI_Z_FRONT;
        use crate::renderer::scene::game_renderer::{
            play_offset_x, PLAY_OFFSET_Y, GRID_ELEMENT_LENGTH,
        };

        // Read from the frame plan; nothing is derived again here.
        let curtain_alpha = plan.curtain_game_alpha;
        let draw_hud = plan.draw_hud;
        let vs_display = plan.snapshot.as_ref().and_then(|s| s.vs_display.as_ref());
        let curtain_menu_alpha = plan.curtain_menu_alpha;

        // Game-related variables shared by several groups.
        let result_visual = plan.snapshot.as_ref()
            .and_then(|s| crate::renderer::ui::ui_renderer::ResultVisual::from_result(s.result));

        // ================================================================
        // UI batch assembly: render groups, back to front
        //
        //   Group: Menu          — logo + carousel
        //   Group: Game          — HUD, cursor, countdown
        //   Group: Curtain G→P   — black resolving overlay + colored tint
        //   Group: Postgame      — result message
        //   (particles)          — compute sparkles + victory rings
        //   Group: Dialog        — pause overlay
        //   Group: Curtain M↔G   — fade between screens
        // ================================================================

        let mut ui_batches: Vec<crate::renderer::gpu::ui_batch::UIBatch> = Vec::with_capacity(32);

        // "Screen surface" family: reset, then fill with the curtains, veils and
        // tints (full window, clip space) and the result message (drawn above the
        // curtain in the overlay pass, square viewport). `frame_submit` renders
        // them outside the square viewport.
        self.ui.screen_overlay.clear();
        self.ui.screen_result_batch = None;

        // Menu <-> Game curtain (black): full window including margins, under egui.
        if self.render_groups.curtain_menu_game.active && curtain_menu_alpha > 0.0 {
            self.ui.screen_overlay.push_fullscreen([0.0, 0.0, 0.0, curtain_menu_alpha]);
        }

        // The menu is rendered via egui (see `frame_submit.rs`).

        // Group: Game
        if self.render_groups.game.active {
            let mut db_snap = self.scene.danger_bar_animator.snapshot();
            if !draw_hud { db_snap.visible = false; }
            self.ui.ui_renderer.wall_clock = scheduling.wall_clock_seconds;
            self.ui.ui_renderer.logo_shake = self.scene.logo_shake_animator.offset();

            // Layer 1: HUD
            ui_batches.extend(self.ui.ui_renderer.generate_batches(
                frame, vs_display,
                !draw_hud, &db_snap,
                &self.scene.opponent_board_animator,
                {
                    let phase = scheduling.wall_clock_seconds * scheduling.panic_freq * std::f32::consts::TAU;
                    scheduling.panic_smoothed * phase.sin().powi(2)
                },
                curtain_alpha,
            ));

            // Cursor silhouette (part of HUD layer)
            let sw = &frame.swapper;
            let (sdx, sdy) = if sw.move_lockout_countdown > 0 {
                let s = (sw.move_lockout_countdown as f32
                    / crackattack_core::constants::MOVE_DELAY as f32).powi(2);
                match sw.last_move_direction {
                    1 => ( s, 0.0),
                    2 => (-s, 0.0),
                    3 => (0.0, -s),
                    4 => (0.0,  s),
                    _ => (0.0, 0.0),
                }
            } else { (0.0, 0.0) };

            let center_left_x = play_offset_x()
                + (sw.grid_x as f32) * GRID_ELEMENT_LENGTH
                + sdx * GRID_ELEMENT_LENGTH;
            let center_y_w = PLAY_OFFSET_Y
                + frame.play_offset_y * GRID_ELEMENT_LENGTH
                + (sw.grid_y as f32) * GRID_ELEMENT_LENGTH
                + sdy * GRID_ELEMENT_LENGTH;
            let half = 0.5 * GRID_ELEMENT_LENGTH;
            let x_min = center_left_x - half;
            let x_max = center_left_x + 1.5 * GRID_ELEMENT_LENGTH;
            let y_bot = center_y_w - half;
            let y_top = center_y_w + half;
            let z = UI_Z_FRONT;

            let phase = scheduling.wall_clock_seconds * 0.7 * std::f32::consts::TAU;
            let pulse = phase.sin().powi(2);

            let mut batch = UIBatch::new(None, UIBlendMode::Alpha);

            // Clamp animation: pinch brackets inward during swap
            let swap_factor = frame.swapper.swap_factor;
            let swap_active = swap_factor > 0.0;
            let swap_bump = if swap_active {
                ((1.0 - swap_factor) * std::f32::consts::PI).sin()
            } else { 0.0 };
            let clamp_offset = if swap_active { swap_bump * 0.22 } else { 0.0 };

            let body_a = 0.38 + 0.06 * pulse;
            let (br_, bg_, bb_) = (0.88, 0.94, 1.00);

            // Silhouette bracket geometry (10-vertex beveled strip)
            let push_bracket = |batch: &mut UIBatch,
                                cx: f32, cy: f32, sx: f32, sy: f32,
                                r: f32, g: f32, b: f32, a: f32| {
                const SG_A: f32 = 0.1;
                const SG_C: f32 = 0.8;
                const SG_D: f32 = 0.1;
                const SG_F: f32 = 0.4;
                const SG_B: f32 = 2.0 * SG_A / 2.414_214;
                let pts: [(f32, f32); 10] = [
                    (-(SG_C),         0.0),
                    (-(SG_C - SG_D), -SG_A),
                    (-(SG_F - SG_B), -SG_A),
                    ( SG_A,           (SG_F - SG_B)),
                    ( SG_A,           (SG_C - SG_D)),
                    ( 0.0,            SG_C),
                    (-SG_A,           (SG_C - SG_D)),
                    (-SG_A,           SG_F),
                    (-(SG_F),         SG_A),
                    (-(SG_C - SG_D),  SG_A),
                ];
                let base = batch.vertices.len() as u16;
                for (lx, ly) in &pts {
                    batch.vertices.push(UIVertex {
                        x: cx + lx * sx, y: cy + ly * sy, z,
                        u: 0.0, v: 0.0, r, g, b, a,
                    });
                }
                let tris: [[u16; 3]; 8] = [
                    [0, 1, 9], [1, 2, 9], [2, 8, 9], [2, 3, 8],
                    [3, 7, 8], [3, 4, 7], [4, 6, 7], [4, 5, 6],
                ];
                let reverse = sx * sy < 0.0;
                for t in &tris {
                    if reverse {
                        batch.indices.push(base + t[0]);
                        batch.indices.push(base + t[2]);
                        batch.indices.push(base + t[1]);
                    } else {
                        batch.indices.push(base + t[0]);
                        batch.indices.push(base + t[1]);
                        batch.indices.push(base + t[2]);
                    }
                }
            };

            // 4 corners: BL, BR, TL, TR with Clamp offsets
            let corners: [(f32, f32, f32, f32, f32); 4] = [
                (x_min, y_bot, -1.0,  1.0,  clamp_offset),
                (x_max, y_bot,  1.0,  1.0, -clamp_offset),
                (x_min, y_top, -1.0, -1.0,  clamp_offset),
                (x_max, y_top,  1.0, -1.0, -clamp_offset),
            ];
            for &(cx, cy, sx, sy, ox) in &corners {
                push_bracket(&mut batch, cx + ox, cy, sx, sy, br_, bg_, bb_, body_a);
            }

            if !batch.vertices.is_empty() {
                ui_batches.push(batch);
            }

            // Layer 2: game animation.
            // The countdown comes from `SessionSnapshot.countdown`, which is `None` during ReadyPrompt.
            if let Some(ref snap) = plan.snapshot {
                if let Some(ref cd) = snap.countdown {
                    if let Some(batch) = self.ui.ui_renderer.generate_countdown_batch(cd) {
                        ui_batches.push(batch);
                    }
                }
            }
        }

        // Group: Curtain Game->Postgame: black curtain + colored tint, full window.
        // Both are pushed in clip space into the "screen surface" family so they
        // cover the margins and the top of the square viewport.
        if self.render_groups.curtain_game_postgame.active {
            if curtain_alpha > 0.0 {
                self.ui.screen_overlay.push_fullscreen([0.0, 0.0, 0.0, curtain_alpha]);
            }
            // Colored tint (appears once the result is known).
            if let Some(ref snap) = plan.snapshot {
                if snap.result != crackattack_core::frame_data::SessionResult::None {
                    let tint_t = (frame.ui.ticks_since_finished as f32 / 30.0).clamp(0.0, 1.0);
                    let tint_alpha = 0.25 * crate::renderer::ui::helpers::ease_out_quad(tint_t);
                    let is_loss = matches!(snap.result,
                        crackattack_core::frame_data::SessionResult::RoundLose
                        | crackattack_core::frame_data::SessionResult::SessionLose);
                    let (tr, tg, tb) = if is_loss {
                        (0.12, 0.02, 0.02)
                    } else {
                        (0.05, 0.12, 0.02)
                    };
                    self.ui.screen_overlay.push_fullscreen([tr, tg, tb, tint_alpha]);
                }
            }
        }

        // Group: Postgame result message. It is drawn above the curtain, in the
        // overlay pass (square viewport, unchanged world coordinates), not in the
        // main square pass.
        if self.render_groups.postgame.active {
            if let Some(v) = result_visual {
                if let Some(batch) = self.ui.ui_renderer.generate_result_batch(v, frame.ui.ticks_since_finished) {
                    self.ui.screen_result_batch = Some(batch);
                }
            }
        }

        // (particles) — compute sparkles + victory rings (drawn in frame_submit.rs)

        // Group: Dialog veil (black, alpha 0.55), full window. It sits under the egui
        // modal (the egui pass runs after the overlay) and also covers the margins.
        if self.render_groups.dialog.active {
            self.ui.screen_overlay.push_fullscreen([0.0, 0.0, 0.0, 0.55]);
        }

        // Record all batches
        for batch in ui_batches {
            if !batch.vertices.is_empty() {
                self.scene.command_buffer.draw_ui_batch(batch);
            }
        }
    }
}
