//! Frame rendering logic for `RendererRuntime`.
//!
//! Split into sub-modules by frame phase:
//! - `scene_recording` — block instances, garbage shells, command buffer
//! - `frame_submit` — swapchain, render pass, dispatch, egui, present
//!
//! This module (`mod.rs`) contains:
//! - `render_frame()` — the orchestrator
//! - Lighting, particles, nameplate texture helpers
//! - Free render helpers: `dispatch_render_commands`, `draw_compute_sparkles`,
//!   `draw_level_lights_precomputed`

mod frame_plan;
mod scene_recording;
mod frame_submit;
mod ui_recording;


use crackattack_core::FrameData;
use crate::renderer::scene::transforms::prepare_lighting_uniforms;
use crate::renderer::gpu::mesh::BuiltinMesh;
use crate::renderer::gpu::backend::RenderDumpCursor;
use crate::renderer::gpu::pipeline::UIPipelineCache;
use crate::renderer::gpu::render_dump;
use crate::app::command::AppCommand;
use crate::app::meta_state::NameInputState;
use crate::app::meta_state::FlowState;

use super::SessionState;
use frame_plan::FramePlan;
use crate::renderer::scene::pipelines::ScenePipelines;
use crate::renderer::gpu::context::GpuContext;
use crate::scheduling::SchedulingState;
use crate::debug_state::DebugState;
use crate::renderer::scene::scene_renderer::SceneRenderer;
use crate::renderer::ui::ui_runtime::UiRuntime;
use crate::renderer::effects::particle_runtime::ParticleRuntime;
use crate::platform::screenshot::ScreenshotCapture;
use crate::app::render_groups::RenderGroups;

/// Renderer runtime: scene pipelines, UI, particles, screenshot.
/// Can be borrowed independently from SessionRuntime.
pub(crate) struct RendererRuntime {
    pub(crate) scene: SceneRenderer,
    pub(crate) ui: UiRuntime,
    pub(crate) particles: ParticleRuntime,
    pub(crate) screenshot: ScreenshotCapture,
    pub(crate) render_groups: RenderGroups,
}

pub(crate) use crate::renderer::scene::garbage_flavor::{GarbageFlavorImageState, GARBAGE_FLAVOR_COUNT};

/// Fixed capacity of the consolidated UI buffers (`ui_vertex_buffer` and
/// `ui_index_buffer`). They are created once and never resized.
/// `dispatch_render_commands` checks every batch against these limits.
pub(crate) const UI_VERTEX_CAPACITY: usize = 4096;
pub(crate) const UI_INDEX_CAPACITY: usize = 8192;

/// Offsets of one UI batch in the consolidated buffers. The first pass of
/// `dispatch_render_commands` computes them; the second pass consumes them by position.
struct PreparedUIBatch {
    index_offset: u64,
    index_count: u32,
    base_vertex: i32,
}

/// Merges UI batches into two large uploads and guards the fixed capacities.
///
/// Invariant: `prepared` holds exactly ONE entry per pushed non-empty batch, in order.
/// A dropped batch (overflow) is stored as `None`. The second pass consumes the entries
/// by position, so a missing entry would shift all following offsets and overrun the
/// index buffer.
struct UiBatchConsolidator {
    vertices: Vec<crate::renderer::gpu::types::UIVertex>,
    indices: Vec<u16>,
    prepared: Vec<Option<PreparedUIBatch>>,
    index_stride: u64,
}

impl UiBatchConsolidator {
    fn new(index_stride: u64) -> Self {
        Self { vertices: Vec::new(), indices: Vec::new(), prepared: Vec::new(), index_stride }
    }

    /// Accepts or drops a (non-empty) batch.
    ///
    /// A batch that would overflow either capacity is logged and recorded as `None`.
    /// It never panics: this replaces a wgpu validation error. In practice the real
    /// vertex count stays below 4096.
    fn push(&mut self, batch_vertices: &[crate::renderer::gpu::types::UIVertex], batch_indices: &[u16]) {
        let pad = batch_indices.len() % 2;
        if self.vertices.len() + batch_vertices.len() > UI_VERTEX_CAPACITY
            || self.indices.len() + batch_indices.len() + pad > UI_INDEX_CAPACITY
        {
            log::error!(
                "DrawUIBatch ignored: fixed UI buffer capacity exceeded \
                 ({}/{} vertices, {}/{} indices)",
                self.vertices.len() + batch_vertices.len(), UI_VERTEX_CAPACITY,
                self.indices.len() + batch_indices.len() + pad, UI_INDEX_CAPACITY,
            );
            self.prepared.push(None);
            return;
        }
        self.prepared.push(Some(PreparedUIBatch {
            index_offset: (self.indices.len() as u64) * self.index_stride,
            index_count: batch_indices.len() as u32,
            base_vertex: self.vertices.len() as i32,
        }));
        self.vertices.extend_from_slice(batch_vertices);
        self.indices.extend_from_slice(batch_indices);
        // Pad to 4-byte alignment (u16 indices: pad if the count is odd).
        if batch_indices.len() % 2 == 1 {
            self.indices.push(0);
        }
    }
}

use crate::renderer::init::find_assets_dir;

impl RendererRuntime {
    /// Top-level render entry point. Called by AppState::render() after
    /// test-mode early-exit. Orchestrates grid dump, lighting upload,
    /// command recording, and GPU execution.
    pub(crate) fn render_frame(
        &mut self,
        gpu: &GpuContext,
        session: &SessionState,
        scheduling: &mut SchedulingState,
        debug: &mut DebugState,
        native_scale: f32,
        frame: &FrameData,
        flow_state: &FlowState,
        game_exists: bool,
        modal_opt: &mut Option<NameInputState>,
        net_modal_opt: &mut Option<crate::app::meta_state::LanScreenState>,
    ) -> Result<Vec<AppCommand>, wgpu::SurfaceError> {
        // Build the frame plan once: it is the single source of truth for all
        // rendering decisions. `snapshot()` and `derive_presentation()` are
        // called here and nowhere else in the render path.
        let plan = {
            let snapshot = session.game.as_ref().map(|s| s.snapshot());
            let presentation = snapshot.as_ref().map(|snap|
                crate::app::presentation::derive_presentation(snap, session.pause_menu_open)
            );
            let in_game = flow_state.is_in_game();
            let curtain_game_alpha = self.render_groups.curtain_game_postgame.alpha();
            let curtain_menu_alpha = self.render_groups.curtain_menu_game.alpha();
            let scene_hidden = curtain_game_alpha >= 1.0;
            FramePlan {
                in_game,
                game_exists,
                draw_game: in_game && game_exists && curtain_game_alpha < 1.0,
                draw_hud: in_game && !scene_hidden,
                curtain_game_alpha,
                curtain_menu_alpha,
                post_game_results_visible: session.post_game_screen.is_results_panel(),
                flow_state: flow_state.clone(),
                pause_menu_open: session.pause_menu_open,
                snapshot,
                presentation,
            }
        };

        let render_dt = scheduling.last_render_dt;
        self.prepare_and_upload_lighting(gpu, &plan, frame, render_dt, scheduling);
        let recorded = self.record_scene(&plan, frame, gpu, session, scheduling, render_dt, debug);
        self.submit_frame(&plan, &recorded, gpu, session, scheduling, debug, native_scale, modal_opt, net_modal_opt)
    }


    /// Refresh the player nameplate + all opponent nameplates.
    /// Called after name change or network connection.
    pub(crate) fn refresh_nameplates(&mut self, gpu: &GpuContext, session: &SessionState) {
        let fonts_dir = find_assets_dir().join("textures/fonts");
        // Player nameplate
        if let Ok((_, _, d)) = crate::renderer::ui::text_texture::generate_nameplate_texture_from_assets(
            &fonts_dir, &session.player_name
        ) {
            let _ = self.ui.texture_cache.update_raw(&gpu.queue, "nameplate", &d);
        }
        // AI opponent nameplates
        for dn in &["Logic", "Sigma", "Cortex"] {
            let tn = format!("nameplate_vs_{}", dn.to_lowercase());
            if let Ok((_, _, d)) = crate::renderer::ui::text_texture::generate_vs_nameplate_from_assets(
                &fonts_dir, &session.player_name, dn
            ) {
                let _ = self.ui.texture_cache.update_raw(&gpu.queue, &tn, &d);
            }
        }
        // Network opponent nameplate (if applicable)
        if let Some(ref game) = session.game {
            let snap = game.snapshot();
            let opp_name = snap.vs_display
                .as_ref().map(|v| v.opponent_name.as_str()).unwrap_or("");
            if !opp_name.is_empty() && opp_name != "Logic" && opp_name != "Sigma" && opp_name != "Cortex" {
                let tn = format!("nameplate_vs_{}", opp_name.to_lowercase());
                // Evict stale network nameplates: remove every `nameplate_vs_*` key that
                // is neither one of the 3 preloaded AI names nor the current opponent.
                // Otherwise each distinct name would keep a 256x128 texture and a bind
                // group alive until exit. The game is 1v1, so only one network
                // nameplate exists at a time.
                let stale: Vec<String> = self.ui.texture_cache.names()
                    .filter(|k| k.starts_with("nameplate_vs_")
                        && k.as_str() != tn
                        && k.as_str() != "nameplate_vs_logic"
                        && k.as_str() != "nameplate_vs_sigma"
                        && k.as_str() != "nameplate_vs_cortex")
                    .cloned()
                    .collect();
                for k in stale {
                    self.ui.texture_cache.remove(&k);
                    self.ui.ui_bind_groups.remove(&k);
                }
                if let Ok((w, h, d)) = crate::renderer::ui::text_texture::generate_vs_nameplate_from_assets(
                    &fonts_dir, &session.player_name, opp_name
                ) {
                    let _ = self.ui.texture_cache.load_raw(&gpu.device, &gpu.queue, &tn, w, h, &d);
                    self.rebuild_ui_bind_groups(gpu);
                    log::info!("Generated network opponent nameplate: {}", tn);
                }
            }
        }
    }

    /// Preview a nameplate texture (live update during name editing).
    pub(crate) fn preview_nameplate(&mut self, gpu: &GpuContext, name: &str) {
        let fonts_dir = find_assets_dir().join("textures/fonts");
        if let Ok((_, _, d)) = crate::renderer::ui::text_texture::generate_nameplate_texture_from_assets(
            &fonts_dir, name
        ) {
            let _ = self.ui.texture_cache.update_raw(&gpu.queue, "nameplate", &d);
        }
    }

    /// Rebuild UI bind groups for all textures in the cache.
    /// Call after adding a new texture via load_raw.
    pub(crate) fn rebuild_ui_bind_groups(&mut self, gpu: &GpuContext) {
        for tex_name in self.ui.texture_cache.names() {
            if self.ui.ui_bind_groups.contains_key(tex_name) {
                continue;
            }
            if let Some(view) = self.ui.texture_cache.get_view(tex_name) {
                let bg = crate::renderer::init::create_ui_bind_group(
                    &gpu.device, &self.ui.ui_bind_group_layout,
                    &self.ui.ui_uniform_buffer, &self.ui.ui_sampler, view,
                    &format!("ui_bind_group_{}", tex_name),
                );
                self.ui.ui_bind_groups.insert(tex_name.to_string(), bg);
            }
        }
    }

    /// Prepare and upload lighting uniforms (headlight, panic pulse, dev tuning).
    fn prepare_and_upload_lighting(&mut self, gpu: &GpuContext, plan: &FramePlan, frame: &FrameData, render_dt: f32, scheduling: &mut SchedulingState) {
        // Derive countdown progress from the plan's snapshot (read once)
        let countdown_progress = plan.snapshot.as_ref().and_then(|snap| {
            snap.countdown.as_ref().map(|cd| {
                use crackattack_core::frame_data::CountdownDigit;
                // Overall progress 0.0→1.0 across all 200 ticks
                let total_elapsed = match cd.digit {
                    CountdownDigit::Three => cd.ticks_into_digit,
                    CountdownDigit::Two => 50 + cd.ticks_into_digit,
                    CountdownDigit::One => 100 + cd.ticks_into_digit,
                    CountdownDigit::Go => 150 + cd.ticks_into_digit,
                };
                total_elapsed as f32 / 200.0
            })
        });
        #[allow(unused_mut)] // `mut` is only used under `--features dev` below
        let mut lighting_uniforms = if plan.flow_state.is_menu() {
            // Menu backdrop: steady light, whatever state the last game left behind.
            scheduling.panic_smoothed = 0.0;
            crate::renderer::gpu::uniforms::LightingUniformsGPU::default()
        } else {
            prepare_lighting_uniforms(frame, countdown_progress)
        };

        // Low-pass filter `panic_intensity` so sudden jumps (e.g. garbage
        // landing and dropping free_rows by several rows at once) do not
        // trigger a red flash. Exponential smoothing with a 0.4 s time constant:
        //   smoothed += (target - smoothed) * (1 - exp(-dt / tau))
        {
            let target = lighting_uniforms.panic_intensity;
            let tau = 0.4_f32;
            let alpha = 1.0 - (-render_dt / tau).exp();
            scheduling.panic_smoothed += (target - scheduling.panic_smoothed) * alpha;
            lighting_uniforms.panic_intensity = scheduling.panic_smoothed;
        }

        // Panic blink: when `panic_intensity > 0`, pulse the key light toward
        // warm red on a sin² curve (0.8 Hz, so about 1.6 pulses per second).
        // The blend is capped at 0.55 and the target colour is a soft warm
        // rose so the scene stays readable.
        let freq_hz = 0.8_f32;

        scheduling.panic_freq = freq_hz;
        let panic = lighting_uniforms.panic_intensity;
        if panic > 0.0 {
            let phase = scheduling.wall_clock_seconds * freq_hz * std::f32::consts::TAU;
            let pulse = phase.sin().powi(2);
            // At most 55% blend toward the warm-red target, scaled by
            // pulse and panic_intensity.
            let k = (panic * pulse * 0.55).min(0.55);
            let target = [1.8_f32, 0.35, 0.25];
            for i in 0..3 {
                lighting_uniforms.diffuse[i] =
                    lighting_uniforms.diffuse[i] * (1.0 - k) + target[i] * k;
            }
        }
        // Garbage warm gradient (fixed values).
        lighting_uniforms.garbage_warm = 0.25;
        lighting_uniforms.specular[3] = 0.25;
        gpu.queue.write_buffer(
            &self.scene.pipelines.lighting_buffer,
            0,
            bytemuck::bytes_of(&lighting_uniforms),
        );
    }

    /// The logo trembles when garbage first lands on the board (not when it falls again).
    /// Called with the events of the frame's steps, like `spawn_nucleons_and_chain_rings`.
    pub(crate) fn shake_logo_on_garbage_landing(&mut self, frame: &FrameData) {
        for event in &frame.gameplay_events {
            if let crackattack_core::gameplay_event::GameplayEvent::GarbageLanded(g) = event {
                if !g.initial {
                    continue;
                }
                log::debug!("Logo shake: garbage {}x{} landed", g.width, g.height);
                self.scene.logo_shake_animator.garbage_landed(g.width, g.height);
            }
        }
    }

    pub(crate) fn spawn_nucleons_and_chain_rings(&mut self, frame: &FrameData) {
            use crackattack_core::BlockState;
            use crackattack_core::constants::{STEPS_PER_GRID, DYING_DELAY};
            use crate::renderer::scene::game_renderer::{play_offset_x, PLAY_OFFSET_Y, GRID_ELEMENT_LENGTH};

            // Bohr-atom look: three different orbits intersect at the cell
            // centre, each tilted 60 degrees from the next. Flattened
            // ellipses (a=0.45, b=0.18) look "seen from an angle" even in 2D.
            const NUCLEON_A: f32 = 0.45;      // ellipse semi-major
            const NUCLEON_B: f32 = 0.18;      // ellipse semi-minor
            const NUCLEON_ANGULAR_SPEED: f32 = 0.14; // rad/tick (~45 ticks/revolution)
            const NUCLEON_FADE_THRESHOLD: f32 = 0.90;
            const NUCLEON_SPARKLE_SIZE: f32 = 0.55;
            const NUCLEON_SPARKLE_LIFETIME: f32 = 22.0;
            const NUCLEON_FLAVOR: u32 = 6;    // nucleon blue (matches shader)
            let shrink_time = DYING_DELAY as f32 - 5.0; // matches game_renderer DyingShrink
            let tick_f = frame.time_step as f32;
            for block in &frame.blocks {
                let st = BlockState::from_bits_truncate(block.state);
                if !st.contains(BlockState::DYING) { continue; }
                // Same as the `anim_progress` computation in `generate_block_instance`
                // (DyingShrink branch): only spawn during the shrink phase,
                // not during the 5-frame flash that precedes it.
                let time_in_shrink = shrink_time - (block.animation_countdown as f32);
                if time_in_shrink <= 0.0 { continue; }
                let p = (time_in_shrink / shrink_time).clamp(0.0, 1.0);
                if p >= NUCLEON_FADE_THRESHOLD { continue; }
                let chain_level: i32 = block.chain_level.max(1);
                // Ramp in: ease the spawn size from 0 to full over the first
                // 20% of the shrink, so nucleons appear smoothly.
                let ramp_in = (p / 0.20).clamp(0.0, 1.0);
                let ramp_in_smooth = ramp_in * ramp_in * (3.0 - 2.0 * ramp_in);
                let size = NUCLEON_SPARKLE_SIZE * ramp_in_smooth;
                if size < 0.01 { continue; }
                // Block world position (same as `generate_block_instance`).
                let bx = play_offset_x() + (block.grid_x as f32) * GRID_ELEMENT_LENGTH;
                let by = PLAY_OFFSET_Y
                    + frame.play_offset_y * GRID_ELEMENT_LENGTH
                    + (block.grid_y as f32) * GRID_ELEMENT_LENGTH
                    + (block.sub_grid_y as f32 / STEPS_PER_GRID as f32) * GRID_ELEMENT_LENGTH;
                let bz = crate::renderer::scene::game_renderer::SPARK_Z;
                // Encode (6 + block_flavor) so the shader can blend in a faint
                // tint of the dying block's own colour.
                let block_flavor_u = (block.flavor.max(0) as u32) % 6;
                let flavor = NUCLEON_FLAVOR + block_flavor_u;
                // Nucleon count equals the chain level (x1 gives 1, x2 gives 2, ...).
                let nucleon_count = chain_level.max(1) as u32;

                // Animated phase: each block gets a deterministic offset so
                // adjacent dying blocks do not move in sync.
                let phase = tick_f * NUCLEON_ANGULAR_SPEED
                    + (block.id as f32) * 0.6;

                // Orbit positions (pure geometry).
                let positions = crate::renderer::scene::game_renderer::nucleon_orbit_positions(
                    nucleon_count,
                    NUCLEON_A,
                    NUCLEON_B,
                    phase,
                    (block.id as f32) * 0.3, // per-block orientation offset
                );

                // Radial fade-out: nucleons drift outward during the last 40% of the shrink.
                let radial_ramp = ((p - 0.60) / 0.40).clamp(0.0, 1.0);
                let radial_speed = 0.045 * radial_ramp;

                for pos in &positions {
                    let sx = bx + pos.dx;
                    let sy = by + pos.dy;
                    let (vx, vy) = if radial_speed > 0.0 {
                        let dist = (pos.dx * pos.dx + pos.dy * pos.dy).sqrt().max(0.001);
                        (pos.dx / dist * radial_speed, pos.dy / dist * radial_speed)
                    } else {
                        (0.0, 0.0)
                    };
                    let slot = self.particles.compute_sparkles.next_slot();
                    self.particles.compute_sparkles.push_spawn(
                        crate::renderer::gpu::compute_sparkles::SparkleSpawnGpu {
                            position: [sx, sy, bz],
                            initial_angle: phase,
                            velocity: [vx, vy],
                            angular_v: 0.05,
                            size,
                            lifetime: NUCLEON_SPARKLE_LIFETIME,
                            flavor,
                            target_slot: slot,
                            _pad: 0,
                        },
                    );
                }

            }

        // Trace gameplay events.
        for event in &frame.gameplay_events {
            match event {
                crackattack_core::gameplay_event::GameplayEvent::BlockLanded(e) => {
                    log::trace!("[event] BlockLanded id={} at ({},{})", e.block_id, e.grid_x, e.grid_y);
                }
                crackattack_core::gameplay_event::GameplayEvent::EliminationStarted(e) => {
                    log::trace!("[event] EliminationStarted combo={} mag={} cells={:?}", e.combo_id, e.magnitude, e.cells);
                }
                crackattack_core::gameplay_event::GameplayEvent::ChainProgressed(e) => {
                    log::trace!("[event] ChainProgressed combo={} x{} cells={:?}", e.combo_id, e.chain_level, e.trigger_cells);
                }
                _ => {}
            }
        }

        { // Chain rings
            use crackattack_core::gameplay_event::GameplayEvent;
            use crate::renderer::scene::game_renderer::{
                play_offset_x, PLAY_OFFSET_Y, GRID_ELEMENT_LENGTH,
            };
            let chain_count = frame.gameplay_events.iter()
                .filter(|e| matches!(e, GameplayEvent::ChainProgressed(_)))
                .count();
            if chain_count > 0 {
                log::trace!("[ring] render frame sees {} ChainProgressed events, trigger_cells: {:?}",
                    chain_count,
                    frame.gameplay_events.iter().filter_map(|e| match e {
                        GameplayEvent::ChainProgressed(cp) => Some(cp.trigger_cells.len()),
                        _ => None,
                    }).collect::<Vec<_>>());
            }
            for event in &frame.gameplay_events {
                if let GameplayEvent::ChainProgressed(e) = event {
                    let chain_level = e.chain_level.clamp(2, 10);
                    let ring_count = chain_level as u32;
                    const SPARKS_PER_RING: u32 = 22;

                    // Barycenter of trigger cells
                    let (cx, cy) = if e.trigger_cells.is_empty() {
                        (3.0, 6.0) // fallback center
                    } else {
                        let sum_x: f32 = e.trigger_cells.iter().map(|&(x, _)| x as f32).sum();
                        let sum_y: f32 = e.trigger_cells.iter().map(|&(_, y)| y as f32).sum();
                        let n = e.trigger_cells.len() as f32;
                        (sum_x / n, sum_y / n)
                    };
                    // The X offset must follow the responsive layout, like blocks and
                    // nucleons. Otherwise the rings drift away from the board in a wide window.
                    let bx = play_offset_x() + cx * GRID_ELEMENT_LENGTH;
                    let by = PLAY_OFFSET_Y
                        + frame.play_offset_y * GRID_ELEMENT_LENGTH
                        + cy * GRID_ELEMENT_LENGTH;
                    let sample_flavor = (e.flavor.max(0) as u32) % 6;

                    for r in 0..ring_count {
                        let t = if ring_count > 1 {
                            r as f32 / (ring_count - 1) as f32
                        } else { 0.0 };
                        let speed = 0.12 + t * 0.10;
                        let phase_offset = (r as f32) * 0.12;
                        for i in 0..SPARKS_PER_RING {
                            let a = (i as f32 / SPARKS_PER_RING as f32)
                                * std::f32::consts::TAU
                                + phase_offset;
                            let slot = self.particles.compute_sparkles.next_slot();
                            self.particles.compute_sparkles.push_spawn(
                                crate::renderer::gpu::compute_sparkles::SparkleSpawnGpu {
                                    position: [bx, by, crate::renderer::scene::game_renderer::SPARK_Z],
                                    initial_angle: a,
                                    velocity: [a.cos() * speed, a.sin() * speed],
                                    angular_v: 0.15,
                                    size: 0.85,
                                    lifetime: 130.0,
                                    flavor: sample_flavor,
                                    target_slot: slot,
                                    _pad: 0,
                                },
                            );
                        }
                    }
                }
            }
        }
    }

}

/// Dispatch all recorded render commands into the active render pass.
///
/// Walks `commands` and translates each `RenderCommand` variant into the
/// matching pipeline bind and draw call. `RenderDump` logging happens inline
/// for each draw.
fn dispatch_render_commands<'pass>(
    render_pass: &mut wgpu::RenderPass<'pass>,
    commands: &[crate::renderer::gpu::render_command::RenderCommand],
    dump_cursor: &mut RenderDumpCursor,
    material_ring: &mut crate::renderer::gpu::ring_buffer::UniformRingBuffer,
    queue: &wgpu::Queue,
    device: &wgpu::Device,
    scene: &'pass ScenePipelines,
    ui: &'pass mut UiRuntime,
    frame_counter: u32,
    instances: &[crate::renderer::gpu::instance::BlockInstance],
    _vertex_stride: u64,
    index_stride: u64,
) {
    use crate::renderer::gpu::render_command::RenderCommand;

    let mut consolidation = UiBatchConsolidator::new(index_stride);
    for cmd in commands {
        if let RenderCommand::DrawUIBatch(batch) = cmd {
            if batch.vertices.is_empty() { continue; }
            consolidation.push(&batch.vertices, &batch.indices);
        }
    }
    let UiBatchConsolidator { vertices: all_vertices, indices: all_indices, prepared: ui_prepared, .. } =
        consolidation;

    // Two consolidated uploads instead of one write per batch.
    if !all_vertices.is_empty() {
        queue.write_buffer(&ui.ui_vertex_buffer, 0, bytemuck::cast_slice(&all_vertices));
        queue.write_buffer(&ui.ui_index_buffer, 0, bytemuck::cast_slice(&all_indices));
    }

    // Pre-upload materials into the ring buffer with dynamic offsets.
    // The block material is a constant. It is still written once per draw call
    // so that the dynamic offsets stay one per draw.
    let block_material = crate::renderer::gpu::uniforms::block_material_uniforms();
    material_ring.reset();
    let mut material_offsets: Vec<u32> = Vec::with_capacity(4);
    for cmd in commands {
        match cmd {
            RenderCommand::DrawInstanced { .. } => {
                material_offsets.push(material_ring.write(queue, &block_material));
            }
            _ => {}
        }
    }
    let mut material_idx: usize = 0;

    let mut ui_batch_idx: usize = 0;

    for cmd in commands {
        match cmd {
            RenderCommand::BeginFrame => {
                // Already handled before the render pass opens.
            }

            RenderCommand::DrawInstanced { instance_count } => {
                // RenderDump logging: hashes the real transform (the CPU copy of the
                // GPU buffer) and the constant material that was actually uploaded.
                if render_dump::is_enabled() {
                    let instance_data = &instances[..*instance_count as usize];
                    render_dump::draw_instanced(
                        frame_counter, dump_cursor.draw_idx(), "Block",
                        *instance_count,
                        bytemuck::bytes_of(&scene.transform_uniforms),
                        bytemuck::bytes_of(&block_material),
                        bytemuck::cast_slice(instance_data),
                    );
                }
                dump_cursor.increment_draw_idx();

                let mat_offset = material_offsets[material_idx];
                material_idx += 1;

                render_pass.set_pipeline(&scene.block_pipeline);
                render_pass.set_bind_group(0, &scene.frame_bind_group, &[]);
                render_pass.set_bind_group(1, &scene.instance_bind_group, &[]);
                render_pass.set_bind_group(2, &scene.block_material_bind_group, &[mat_offset]);
                render_pass.set_vertex_buffer(0, scene.block_vertex_buffer.slice(..));
                render_pass.draw(0..scene.block_vertex_count, 0..*instance_count);
            }

            RenderCommand::DrawUIBatch(batch) => {
                if batch.vertices.is_empty() {
                    continue;
                }

                // Consume the entry before any skip: there is one entry per non-empty
                // batch, mirroring the first pass. Otherwise later offsets shift.
                let prepared = &ui_prepared[ui_batch_idx];
                ui_batch_idx += 1;
                let Some(prepared) = prepared else {
                    // The first pass dropped this batch (capacity): nothing was
                    // uploaded, so there is nothing to draw or to log in the dump.
                    continue;
                };

                // RenderDump logging (uses original batch data for hashing)
                if render_dump::is_enabled() {
                    let texture_mode = match &batch.texture {
                        Some(_) => "Modulate",
                        None => "None",
                    };
                    render_dump::draw_ui_batch(
                        frame_counter,
                        dump_cursor.draw_idx(),
                        batch.vertices.len() as u32,
                        batch.indices.len() as u32,
                        texture_mode,
                        bytemuck::cast_slice(&batch.vertices),
                        bytemuck::cast_slice(&batch.indices),
                    );
                }
                dump_cursor.increment_draw_idx();

                // Pipeline + bind group selection
                let bind_group = if let Some(ref tex_name) = batch.texture {
                    let bg = ui.ui_bind_groups.get(tex_name.as_ref());
                    if bg.is_none() {
                        log::error!("UI texture '{}' not found, using solid fallback", tex_name);
                    }
                    bg.unwrap_or(&ui.solid_bind_group)
                } else {
                    &ui.solid_bind_group
                };

                let additive = batch.blend_mode == crate::renderer::gpu::ui_batch::UIBlendMode::Additive;
                let variant = UIPipelineCache::variant_for(batch.texture_mode, additive);
                let pipeline = ui.ui_pipeline_cache.get_or_create(
                    device,
                    &scene.shader_modules,
                    variant,
                );

                // GPU draw: no `write_buffer` here, only bind and draw with offsets.
                render_pass.set_pipeline(pipeline);
                render_pass.set_bind_group(0, bind_group, &[]);
                render_pass.set_vertex_buffer(0, ui.ui_vertex_buffer.slice(..));
                render_pass.set_index_buffer(
                    ui.ui_index_buffer.slice(prepared.index_offset..),
                    wgpu::IndexFormat::Uint16,
                );
                render_pass.draw_indexed(0..prepared.index_count, prepared.base_vertex, 0..1);
            }

            RenderCommand::DrawGarbageInstanced { mesh, instance_count, first_instance, texture } => {
                if *instance_count > 0 {
                    // RenderDump logging: garbage draws appear in the traces like
                    // blocks. `total_draws` counts them, so their records must exist.
                    if render_dump::is_enabled() {
                        render_dump::draw_garbage_instanced(
                            frame_counter, dump_cursor.draw_idx(), &format!("{:?}", mesh),
                            *instance_count, *first_instance, *texture,
                        );
                    }
                    dump_cursor.increment_draw_idx();

                    render_pass.set_pipeline(&scene.garbage_pipeline);
                    render_pass.set_bind_group(0, &scene.garbage_uniform_bind_group, &[]);
                    render_pass.set_bind_group(1, &scene.garbage_instance_bind_group, &[]);

                    let range = match mesh {
                        BuiltinMesh::GarbageThickCorner => &scene.garbage_shell_ranges.thick_corner,
                        BuiltinMesh::GarbageThickEdge => &scene.garbage_shell_ranges.thick_edge,
                        BuiltinMesh::GarbageThickMiddle => &scene.garbage_shell_ranges.thick_middle,
                        BuiltinMesh::GarbageThinCap => &scene.garbage_shell_ranges.thin_cap,
                        BuiltinMesh::GarbageThinMiddle => &scene.garbage_shell_ranges.thin_middle,
                        BuiltinMesh::GarbageSmall => &scene.garbage_shell_ranges.small,
                        _ => &scene.garbage_shell_ranges.small,
                    };
                    render_pass.set_vertex_buffer(0, scene.garbage_shell_vertex_buffer.slice(..));
                    let v_start = range.vertex_offset;
                    let v_end = v_start + range.vertex_count;
                    render_pass.draw(v_start..v_end, *first_instance..(*first_instance + *instance_count));
                }
            }

            RenderCommand::EndFrame => {
                // Handled after loop.
            }
        }
    }
}

/// Draw compute sparkles into the active render pass.
///
/// When the compute sparkle path is active, the CPU sparkle system emits
/// no `DrawParticleInstanced` for sparkles. This function binds the
/// compute render buffer and draws all `SPARKLE_SLOT_COUNT` slots;
/// inactive slots have `color.a = 0.0` and contribute nothing.
fn draw_compute_sparkles<'pass>(
    render_pass: &mut wgpu::RenderPass<'pass>,
    particles: &'pass ParticleRuntime,
) {
    if particles.compute_sparkles.has_active() {
        use crate::renderer::gpu::compute_sparkles::SPARKLE_SLOT_COUNT;
        render_pass.set_pipeline(&particles.particle_pipeline);
        render_pass.set_bind_group(0, &particles.particle_uniform_bind_group, &[]);
        render_pass.set_bind_group(1, particles.compute_sparkles.render_bind_group(), &[]);
        render_pass.draw(0..6, 0..SPARKLE_SLOT_COUNT);
    }
}

/// Draw pre-computed level light vertices into the active render pass.
///
/// Vertex generation happens before the borrow split (see the call site in
/// `submit_frame`), so this function receives the generated vertices and
/// only uploads and draws them.
fn draw_level_lights_precomputed<'pass>(
    render_pass: &mut wgpu::RenderPass<'pass>,
    queue: &wgpu::Queue,
    scene: &'pass ScenePipelines,
    ll_verts: &[crate::renderer::gpu::types::MeshVertex],
) {
    if !ll_verts.is_empty() {
        queue.write_buffer(
            &scene.level_light_vertex_buffer,
            0,
            bytemuck::cast_slice(ll_verts),
        );
        render_pass.set_pipeline(&scene.level_light_pipeline);
        // Level lights share the blocks' `frame_bind_group` (transform + lighting).
        render_pass.set_bind_group(0, &scene.frame_bind_group, &[]);
        render_pass.set_vertex_buffer(0, scene.level_light_vertex_buffer.slice(..));
        render_pass.draw(0..ll_verts.len() as u32, 0..1);
    }
}

#[cfg(test)]
mod tests {
    use super::{UiBatchConsolidator, UI_INDEX_CAPACITY, UI_VERTEX_CAPACITY};
    use crate::renderer::gpu::types::UIVertex;

    const STRIDE: u64 = std::mem::size_of::<u16>() as u64;

    fn verts(n: usize) -> Vec<UIVertex> {
        vec![bytemuck::Zeroable::zeroed(); n]
    }

    /// A batch dropped in the middle must leave a `None` entry and must shift
    /// neither the offsets nor the count of the following batches.
    #[test]
    fn overflow_batch_keeps_prepared_in_sync() {
        let mut c = UiBatchConsolidator::new(STRIDE);
        c.push(&verts(10), &[0u16; 6]);                      // accepted
        c.push(&verts(UI_VERTEX_CAPACITY), &[0u16; 2]);      // dropped (would overflow)
        c.push(&verts(7), &[0u16; 3]);                       // accepted

        assert_eq!(c.prepared.len(), 3, "one entry per non-empty batch, drops included");
        assert!(c.prepared[0].is_some());
        assert!(c.prepared[1].is_none(), "the overflowing batch must be None, not absent");
        let third = c.prepared[2].as_ref().expect("the 3rd batch is accepted");
        assert_eq!(third.base_vertex, 10, "the dropped batch's vertices do not count");
        assert_eq!(third.index_offset, 6 * STRIDE);
        assert_eq!(third.index_count, 3);
        assert_eq!(c.vertices.len(), 17);
        assert_eq!(c.indices.len(), 6 + 3 + 1, "3 indices (odd) → +1 padding");
    }

    #[test]
    fn odd_index_count_pads_next_offset() {
        let mut c = UiBatchConsolidator::new(STRIDE);
        c.push(&verts(4), &[0u16; 5]); // odd count: padded to 6
        c.push(&verts(4), &[0u16; 4]);
        assert_eq!(c.indices.len(), 6 + 4);
        assert_eq!(c.prepared[1].as_ref().unwrap().index_offset, 6 * STRIDE);
    }

    #[test]
    fn exact_capacity_is_accepted_then_next_batch_dropped() {
        let mut c = UiBatchConsolidator::new(STRIDE);
        c.push(&verts(UI_VERTEX_CAPACITY), &[0u16; 4]);
        assert!(c.prepared[0].is_some(), "exact capacity fits");
        c.push(&verts(1), &[0u16; 2]);
        assert!(c.prepared[1].is_none(), "one more vertex overflows");
    }

    #[test]
    fn index_padding_counts_toward_capacity() {
        let mut c = UiBatchConsolidator::new(STRIDE);
        // 8191 indices (odd) + 1 padding = exact capacity: accepted, buffer full.
        c.push(&verts(8), &vec![0u16; UI_INDEX_CAPACITY - 1]);
        assert!(c.prepared[0].is_some(), "odd + padding = exact capacity → fits");
        assert_eq!(c.indices.len(), UI_INDEX_CAPACITY);
        // Any further batch overflows the index buffer (vertices still have room).
        c.push(&verts(8), &[0u16; 2]);
        assert!(c.prepared[1].is_none(), "index capacity full → dropped");
    }
}
