//! Scene recording phase: block instances, garbage shells, command buffer.
//!
//! Fills the `RenderCommandBuffer` and returns a `RecordedFrame`
//! for the submit phase.

use crackattack_core::FrameData;
use crate::renderer::scene::game_renderer::{
    generate_garbage_shell_instances, GarbageShellInstances,
    assemble_garbage_draws,
    GRID_ELEMENT_LENGTH, PLAY_OFFSET_Y,
};
use crate::renderer::gpu::uniforms::FrameUniforms;

use super::super::SessionState;
use super::RendererRuntime;
use super::frame_plan::FramePlan;
use crate::renderer::gpu::context::GpuContext;
use crate::scheduling::SchedulingState;
use crate::debug_state::DebugState;

/// Data produced by `record_scene()`, consumed by `submit_frame()`.
///
/// Transfers ownership of GPU instance data and draw metadata from
/// the recording phase to the submission phase.
pub(super) struct RecordedFrame {
    /// Block instances for the instanced draw call.
    pub(super) instances: Vec<crate::renderer::gpu::instance::BlockInstance>,
    /// Frame uniforms (time_step, play_offset_y, etc.).
    pub(super) frame_uniforms: crate::renderer::gpu::uniforms::FrameUniforms,
    /// Whether 3D scene should be drawn.
    pub(super) draw_3d_scene: bool,
    /// Garbage flavor image draw info: (instance_idx, texture_idx).
    pub(super) garbage_flavor_draw: Option<(u32, usize)>,
}

impl RendererRuntime {
    /// Record all draw commands into the command buffer. Returns a
    /// `RecordedFrame` with the instance data for the submit phase.
    pub(super) fn record_scene(&mut self, plan: &FramePlan, frame: &FrameData, gpu: &GpuContext, session: &SessionState, scheduling: &SchedulingState, render_dt: f32, debug: &mut DebugState) -> RecordedFrame {
        // The menu shows its drifting blocks in place of the game.
        let draw_menu_backdrop = plan.flow_state.is_menu();
        let draw_3d_scene = plan.in_game && plan.game_exists;

        self.render_groups.sync_from_state(
            &plan.flow_state, plan.game_exists,
            plan.pause_menu_open, plan.post_game_results_visible,
        );

        self.scene.command_buffer.clear();

        let play_offset_y_world = PLAY_OFFSET_Y
            + frame.play_offset_y * GRID_ELEMENT_LENGTH;
        let frame_uniforms = FrameUniforms {
            time_step: frame.time_step,
            play_offset_y: play_offset_y_world,
        };
        self.scene.command_buffer.begin_frame();

        let draw_game = plan.draw_game;

        self.process_frame_events(plan, frame, render_dt, debug);

        let instances = if draw_menu_backdrop {
            self.scene.menu_backdrop.step(render_dt);
            let aspect = gpu.viewport_width as f32 / gpu.viewport_height.max(1) as f32;
            self.scene.menu_backdrop.instances(aspect)
        } else {
            crate::renderer::scene::game_renderer::generate_frame_instances(
                frame,
                &self.scene.block_impact_phases,
            )
        };

        let block_count = instances.len() as u32;
        if block_count > 0 {
            gpu.queue.write_buffer(
                &self.scene.pipelines.instance_buffer,
                0,
                bytemuck::cast_slice(&instances),
            );
        }

        if (draw_game || draw_menu_backdrop) && block_count > 0 {
            self.scene.command_buffer.draw_instanced(block_count);
        }

        // "Pop" phase: the first three quarters of the awaking, when neither the
        // shell nor the flavor quad is drawn. Both sites must use this single
        // predicate; if they disagreed, a flavor index could exist without a
        // matching push.
        let is_popping = |g: &crackattack_core::frame_data::GarbageSnapshot| {
            g.state == crackattack_core::garbage::GarbageState::Awaking as u8
                && g.awaking_progress == 0.0
        };

        let mut shell = GarbageShellInstances::new();
        for garbage in &frame.garbage {
            if is_popping(garbage) { continue; }
            generate_garbage_shell_instances(garbage, frame.play_offset_y, &mut shell);
        }
        let (mut garbage_instances, garbage_draws) = assemble_garbage_draws(&shell);

        // The flavor instance index is set at the push site and nowhere else.
        // With no push (active garbage in the pop phase) there is no index.
        self.scene.garbage_flavor_instance_idx = None;
        if draw_game {
            self.scene.garbage_flavor_state.update(&frame.garbage);
            if let Some(flavor_id) = self.scene.garbage_flavor_state.active_garbage_id {
                if let Some(g) = frame.garbage.iter()
                    .find(|g| g.id == flavor_id && !is_popping(g))
                {
                    use crate::renderer::scene::game_renderer::{play_offset_x, PLAY_OFFSET_Y, PLAY_OFFSET_Z, GRID_ELEMENT_LENGTH};
                    use crate::renderer::gpu::instance::GarbagePieceInstance;

                    let fx = self.scene.garbage_flavor_state.flavor_x as f32;
                    let fy = self.scene.garbage_flavor_state.flavor_y as f32;
                    let base_y = PLAY_OFFSET_Y
                        + frame.play_offset_y * GRID_ELEMENT_LENGTH
                        + (g.grid_y as f32) * GRID_ELEMENT_LENGTH
                        + (g.sub_grid_y as f32 / crackattack_core::constants::STEPS_PER_GRID as f32) * GRID_ELEMENT_LENGTH;

                    let is_shattering = g.shatter_progress > 0.0;
                    let clip_plane = if is_shattering {
                        let start_w = GRID_ELEMENT_LENGTH * 0.5;
                        let end_w = -(g.height as f32) * GRID_ELEMENT_LENGTH;
                        let sp = g.shatter_progress;
                        let eased = sp * sp * (3.0 - 2.0 * sp);
                        let w_local = start_w + (end_w - start_w) * eased;
                        let w_world = w_local - base_y;
                        [0.0_f32, 1.0, 0.0, w_world]
                    } else {
                        [0.0_f32, 0.0, 0.0, 0.0]
                    };

                    let pos_x = play_offset_x() + (g.grid_x as f32 + fx + 0.5) * GRID_ELEMENT_LENGTH;
                    let pos_y = base_y + (fy + 0.5) * GRID_ELEMENT_LENGTH;
                    let pos_z = PLAY_OFFSET_Z + 1.01;

                    let flags: u32 = if is_shattering { 0b11 } else { 0b10 };

                    garbage_instances.push(GarbagePieceInstance {
                        position: [pos_x, pos_y, pos_z],
                        flags,
                        color: [1.0, 1.0, 1.0, 1.0],
                        uv_transform: [1.0, 1.0, 0.0, 0.0],
                        clip_plane_eye: clip_plane,
                    });
                    self.scene.garbage_flavor_instance_idx =
                        Some((garbage_instances.len() - 1) as u32);
                }
            }
        }

        if draw_game && !garbage_instances.is_empty() {
            gpu.queue.write_buffer(
                &self.scene.pipelines.garbage_instance_buffer,
                0,
                bytemuck::cast_slice(&garbage_instances),
            );

            for (mesh, count, first) in &garbage_draws {
                self.scene.command_buffer.draw_garbage_instanced(
                    *mesh, *count, *first, 0,
                );
            }
        }

        self.generate_and_record_ui(frame, plan, session, scheduling);

        self.scene.command_buffer.end_frame();

        let garbage_flavor_draw = self.scene.garbage_flavor_instance_idx
            .map(|idx| (idx, self.scene.garbage_flavor_state.flavor_tex_idx));

        RecordedFrame {
            instances,
            frame_uniforms,
            draw_3d_scene,
            garbage_flavor_draw,
        }
    }

    /// Process gameplay events: update animators, track block impacts.
    fn process_frame_events(&mut self, plan: &FramePlan, frame: &FrameData, render_dt: f32, debug: &mut DebugState) {
        self.scene.danger_bar_animator.time_step(frame.ui.creep_freeze, frame.ui.loss_countdown);
        let opponent_board = plan.snapshot.as_ref()
            .and_then(|snap| snap.vs_display.as_ref())
            .and_then(|vs| vs.opponent_board.as_ref());
        self.scene.opponent_board_animator.time_step(opponent_board, render_dt);
        let is_countdown = plan.snapshot.as_ref()
            .map_or(false, |snap| snap.countdown.is_some());
        if is_countdown && self.scene.level_light_animator.slide_out_progress > 0.0 {
            self.scene.level_light_animator = crate::renderer::effects::game_animators::LevelLightAnimator::new();
        }
        if frame.ui.phase == crackattack_core::GamePhase::Playing || is_countdown {
            self.scene.level_light_animator.time_step(
                frame.ui.top_effective_row,
            );
        } else if frame.ui.phase == crackattack_core::GamePhase::Finished {
            self.scene.level_light_animator.slide_out_step();
        }

        // Logo tremor: started by `shake_logo_on_garbage_landing` (the render frame
        // carries no events), advanced here.
        self.scene.logo_shake_animator.time_step(render_dt);

        let _ = debug;
    }
}
