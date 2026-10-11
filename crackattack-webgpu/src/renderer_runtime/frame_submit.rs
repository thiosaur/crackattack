//! Frame submission phase: swapchain, render pass, dispatch, egui, present.
//!
//! Consumes a `RecordedFrame` from `scene_recording` and executes
//! all GPU work for the frame.


use crate::renderer::gpu::types::UIVertex;
use crate::app::command::AppCommand;
use crate::app::meta_state::NameInputState;

use super::super::SessionState;
use super::RendererRuntime;
use super::frame_plan::FramePlan;
use crate::renderer::ui::ui_runtime::UiRuntime;
use crate::renderer::gpu::context::GpuContext;
use crate::scheduling::SchedulingState;
use crate::debug_state::DebugState;
use super::scene_recording::RecordedFrame;
use crate::renderer::gpu::targets::DEFAULT_MSAA_SAMPLE_COUNT;

/// Black game background, so the letterbox area outside the square viewport blends in.
const CLEAR_COLOR: wgpu::Color = wgpu::Color { r: 0.0, g: 0.0, b: 0.0, a: 1.0 };

impl RendererRuntime {
    /// Execute the GPU frame: swapchain, render pass, dispatch, egui, present.
    pub(super) fn submit_frame(
        &mut self,
        plan: &FramePlan,
        recorded: &RecordedFrame,
        gpu: &GpuContext,
        session: &SessionState,
        scheduling: &mut SchedulingState,
        debug: &mut DebugState,
        native_scale: f32,
        modal_opt: &mut Option<NameInputState>,
        net_modal_opt: &mut Option<crate::app::meta_state::LanScreenState>,
    ) -> Result<Vec<AppCommand>, wgpu::SurfaceError> {
        self.scene.dump_cursor.begin_frame(&recorded.frame_uniforms, scheduling.frame_counter);

        let output = gpu.surface.get_current_texture()?;
        let view = output
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());

        let mut encoder = gpu.device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("render_encoder"),
            });

        let vertex_stride = std::mem::size_of::<UIVertex>() as u64;
        let index_stride = std::mem::size_of::<u16>() as u64;

        {
            let (color_view, resolve_target) =
                if DEFAULT_MSAA_SAMPLE_COUNT > 1 {
                    (&self.scene.pipelines.render_targets.msaa_color_view, Some(&view))
                } else {
                    (&view, None)
                };

            self.particles.compute_sparkles.flush_clear(&mut encoder);
            if recorded.draw_3d_scene && self.particles.compute_sparkles.has_active() {
                self.particles.compute_sparkles.dispatch(&gpu.queue, &mut encoder);
            }

            let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("main_render_pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: color_view,
                    resolve_target,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(CLEAR_COLOR),
                        // Nothing reads the MSAA texture back: later passes only use the
                        // resolve target (`view`). Discarding saves the memory write of
                        // the multisampled content.
                        store: if resolve_target.is_some() {
                            wgpu::StoreOp::Discard
                        } else {
                            wgpu::StoreOp::Store
                        },
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.scene.pipelines.render_targets.depth_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
            });

            render_pass.set_viewport(
                gpu.viewport_x as f32, gpu.viewport_y as f32,
                gpu.viewport_width as f32, gpu.viewport_height as f32,
                0.0, 1.0,
            );
            render_pass.set_scissor_rect(
                gpu.viewport_x, gpu.viewport_y,
                gpu.viewport_width, gpu.viewport_height,
            );

            // Level lights
            let mut ll_snap = self.scene.level_light_animator.snapshot();
            let panic_phase = scheduling.wall_clock_seconds * scheduling.panic_freq * std::f32::consts::TAU;
            ll_snap.panic_pulse = scheduling.panic_smoothed * panic_phase.sin().powi(2);
            // Specular 0.25 and shininess 2.0 are shader constants, so no material
            // write is needed per frame.
            let ll_vs_display = plan.snapshot.as_ref()
                .and_then(|s| s.vs_display.as_ref());
            let ll_verts = self.ui.ui_renderer.generate_level_light_mesh_vertices(
                ll_vs_display, &ll_snap,
            );

            // Garbage flavor image (before UI batches)
            if let Some((flavor_instance_idx, tex_idx)) = recorded.garbage_flavor_draw {
                if tex_idx < self.scene.pipelines.garbage_flavor_bind_groups.len() {
                    render_pass.set_pipeline(&self.scene.pipelines.garbage_pipeline);
                    render_pass.set_bind_group(0, &self.scene.pipelines.garbage_flavor_bind_groups[tex_idx], &[]);
                    render_pass.set_bind_group(1, &self.scene.pipelines.garbage_instance_bind_group, &[]);
                    render_pass.set_vertex_buffer(0, self.scene.pipelines.garbage_shell_vertex_buffer.slice(..));
                    let fq = &self.scene.pipelines.garbage_shell_ranges.flavor_quad;
                    render_pass.draw(
                        fq.vertex_offset..(fq.vertex_offset + fq.vertex_count),
                        flavor_instance_idx..(flavor_instance_idx + 1),
                    );
                }
            }

            let commands = self.scene.command_buffer.commands();
            super::dispatch_render_commands(
                &mut render_pass,
                commands,
                &mut self.scene.dump_cursor,
                &mut self.scene.material_ring,
                &gpu.queue,
                &gpu.device,
                &self.scene.pipelines,
                &mut self.ui,
                scheduling.frame_counter,
                &recorded.instances,
                vertex_stride,
                index_stride,
            );

            // Compute sparkle draw and level lights — only when the scene is
            // actually visible (not occluded by the game→postgame curtain).
            if plan.draw_game {
                super::draw_compute_sparkles(&mut render_pass, &self.particles);
                super::draw_level_lights_precomputed(&mut render_pass, &gpu.queue, &self.scene.pipelines, &ll_verts);
            }
        }

        // --- Screen overlay pass ---
        // Runs after the scene (resolved into `view`) and before egui. Curtains, veils
        // and tints (clip space, full surface) and the result message (square
        // viewport, above the curtain) were filled in by `generate_and_record_ui`.
        // They are uploaded and drawn here.
        let overlay_verts = self.ui.screen_overlay.upload(&gpu.device, &gpu.queue);

        // Prepare the result outside the pass: upload vertices and create the swapchain UI pipeline.
        let result = self.ui.screen_result_batch.take();
        let result_draw = result.as_ref().map(|b| {
            gpu.queue.write_buffer(&self.ui.screen_ui_vertex_buffer, 0, bytemuck::cast_slice(&b.vertices));
            gpu.queue.write_buffer(&self.ui.screen_ui_index_buffer, 0, bytemuck::cast_slice(&b.indices));
            let additive = b.blend_mode == crate::renderer::gpu::ui_batch::UIBlendMode::Additive;
            let variant = crate::renderer::gpu::pipeline::UIPipelineCache::variant_for(b.texture_mode, additive);
            self.ui.screen_ui_pipeline_cache.get_or_create(&gpu.device, &self.scene.pipelines.shader_modules, variant);
            (variant, b.texture.clone(), b.indices.len() as u32)
        });

        if overlay_verts > 0 || result_draw.is_some() {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("screen_overlay_pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store },
                })],
                depth_stencil_attachment: None,
                ..Default::default()
            });
            // 1) Curtains, veils and tints: default viewport is the full surface (clip-space quads).
            self.ui.screen_overlay.draw(&mut pass, overlay_verts);
            // 2) Result message: square viewport (world coordinates), drawn above the curtain.
            if let Some((variant, tex, idx_count)) = result_draw {
                pass.set_viewport(
                    gpu.viewport_x as f32, gpu.viewport_y as f32,
                    gpu.viewport_width as f32, gpu.viewport_height as f32, 0.0, 1.0,
                );
                pass.set_scissor_rect(gpu.viewport_x, gpu.viewport_y, gpu.viewport_width, gpu.viewport_height);
                if let Some(pipeline) = self.ui.screen_ui_pipeline_cache.get(variant) {
                    let bind_group = tex.as_ref()
                        .and_then(|t| self.ui.ui_bind_groups.get(t.as_ref()))
                        .unwrap_or(&self.ui.solid_bind_group);
                    pass.set_pipeline(pipeline);
                    pass.set_bind_group(0, bind_group, &[]);
                    pass.set_vertex_buffer(0, self.ui.screen_ui_vertex_buffer.slice(..));
                    pass.set_index_buffer(self.ui.screen_ui_index_buffer.slice(..), wgpu::IndexFormat::Uint16);
                    pass.draw_indexed(0..idx_count, 0, 0..1);
                }
            }
        }

        self.finalize_and_present_frame(gpu, session, scheduling, debug, native_scale, plan, output, encoder, &view, modal_opt, net_modal_opt)
    }

    /// Finalize: screenshot, egui overlay, present.
    fn finalize_and_present_frame(
        &mut self,
        gpu: &GpuContext,
        session: &SessionState,
        scheduling: &mut SchedulingState,
        debug: &mut DebugState,
        native_scale: f32,
        plan: &FramePlan,
        output: wgpu::SurfaceTexture,
        encoder: wgpu::CommandEncoder,
        view: &wgpu::TextureView,
        modal_opt: &mut Option<NameInputState>,
        net_modal_opt: &mut Option<crate::app::meta_state::LanScreenState>,
    ) -> Result<Vec<AppCommand>, wgpu::SurfaceError> {
        let mut deferred_commands = Vec::with_capacity(4);

        // Screenshot: the copy is encoded in a dedicated command buffer that is
        // submitted after the egui pass. Menus, options and modals are drawn only by
        // egui, so copying before it would leave them out of every capture.
        let mut capture_buf: Option<wgpu::CommandBuffer> = None;
        if let Some(request) = debug.screenshot_request.take() {
            // Surface without COPY_SRC: fail visibly, never silently. A lost named
            // capture must fail the campaign verdict.
            if !gpu.capture_supported {
                match request {
                    crate::platform::screenshot::ScreenshotRequest::Named(n) => {
                        self.screenshot.record_failure(
                            n, "screenshots unavailable: the surface does not expose COPY_SRC".to_string());
                    }
                    crate::platform::screenshot::ScreenshotRequest::Manual => {
                        log::error!("Cannot capture: the surface does not expose COPY_SRC");
                    }
                }
            } else {
                // `begin_capture` handles the "already in progress" case itself: a warning
                // for a manual capture, a recorded failure (summary and exit code) for a
                // named one.
                let mut cap_enc = gpu.device.create_command_encoder(
                    &wgpu::CommandEncoderDescriptor { label: Some("screenshot_copy") });
                self.screenshot.begin_capture(&gpu.device, &mut cap_enc, &output.texture, request);
                capture_buf = Some(cap_enc.finish());
            }
        }

        // Egui overlay
        if let Some((cmds, egui_buf)) = run_egui_pass(
            &mut self.ui, gpu, native_scale, view, plan, session,
            &self.render_groups, modal_opt, net_modal_opt, debug,
        ) {
            deferred_commands.extend(cmds);
            gpu.queue.submit([encoder.finish(), egui_buf].into_iter().chain(capture_buf));
            output.present();
        } else {
            gpu.queue.submit(std::iter::once(encoder.finish()).chain(capture_buf));
            output.present();
        }

        if !self.screenshot.is_idle() {
            gpu.device.poll(wgpu::Maintain::Poll);
            self.screenshot.poll();
        }

        let total_draws = self.scene.command_buffer.total_draws();
        self.scene.dump_cursor.end_frame(scheduling.frame_counter, total_draws);
        scheduling.frame_counter += 1;

        let max_frames = debug.max_frames;
        if max_frames > 0 && scheduling.frame_counter >= max_frames {
            log::info!("Reached max frames ({}), exiting", max_frames);
            debug.should_exit = true;
        }
        Ok(deferred_commands)
    }
}

/// Reference logical size of the design: a 700 x 700 square.
///
/// The egui zoom is the useful-zone side divided by this value, so the logo,
/// menu, modals, fonts and positions keep a fixed fraction of the square at any
/// window size.
const UI_SCALE_REFERENCE: f32 = 700.0;
/// Lower bound of the egui zoom. Only guards against absurd extremes.
const UI_SCALE_MIN: f32 = 0.3;
/// Upper bound of the egui zoom. Only guards against absurd extremes.
const UI_SCALE_MAX: f32 = 3.0;

/// Renders the "forfeit the game?" confirmation modal and pushes commands for mouse clicks.
///
/// Keyboard and gamepad input goes through `route_abandon_confirm`. Shared by the
/// pause, ready and between-round screens so they all exit the same way.
fn render_abandon_confirm(ctx: &egui::Context, yes: bool, modal_commands: &mut Vec<AppCommand>) {
    use crate::renderer::ui::confirm_modal::{render_confirm_modal, ConfirmOutcome};
    let t = crate::app::locale::t();
    match render_confirm_modal(ctx, t.forfeit_confirm, t.forfeit, t.cancel, yes) {
        ConfirmOutcome::Confirmed => {
            modal_commands.push(AppCommand::PlayUiConfirm);
            modal_commands.push(AppCommand::QuitToMenu);
        }
        ConfirmOutcome::Cancelled => modal_commands.push(AppCommand::SetAbandonConfirm(None)),
        // Mouse hover moves the focus: the modal stays open and only `yes` changes.
        ConfirmOutcome::HoverFocus(y) => modal_commands.push(AppCommand::SetAbandonConfirm(Some(y))),
        ConfirmOutcome::StillOpen => {}
    }
}

/// Run the egui overlay pass: render modals, tessellate, upload, encode.
///
/// Free function to allow split borrows: `ui` mutably, `render_groups` immutably.
fn run_egui_pass(
    ui: &mut UiRuntime,
    gpu: &GpuContext,
    native_scale: f32,
    view: &wgpu::TextureView,
    plan: &FramePlan,
    session: &SessionState,
    render_groups: &crate::app::render_groups::RenderGroups,
    modal_opt: &mut Option<NameInputState>,
    net_modal_opt: &mut Option<crate::app::meta_state::LanScreenState>,
    _debug: &mut DebugState,
) -> Option<(Vec<AppCommand>, wgpu::CommandBuffer)> {
    let presentation = &plan.presentation;
    let has_modal = presentation.as_ref()
        .map_or(false, |p| p.modal.is_some());

    #[allow(unused_mut, unused_assignments)]
    let mut dev_visible = false;
    #[cfg(feature = "dev")]
    { dev_visible = _debug.dev_panel.visible; }

    let needs_egui = modal_opt.is_some()
        || net_modal_opt.is_some()
        || plan.pause_menu_open
        || has_modal
        || plan.post_game_results_visible
        || dev_visible
        || matches!(session.flow_state, crate::app::meta_state::FlowState::Menu);

    if !needs_egui {
        return None;
    }

    // The binary builds `RawInput` from the ordered `PlatformShell` event stream
    // (via `ui_bridge`) and stores it in `ui.pending_raw_input`.
    let mut raw_input = ui.pending_raw_input.take().unwrap_or_default();

    // Debug unit check: the received `screen_rect` must equal window size divided by
    // the effective pixels per point (native scale times responsive zoom). It catches
    // pixel/point mix-ups, for example a platform backend reporting the wrong unit.
    #[cfg(debug_assertions)]
    if let Some(rect) = raw_input.screen_rect {
        let effective = ui.egui_ctx.zoom_factor() * native_scale;
        let expected_pts = gpu.config.width as f32 / effective.max(0.1);
        if (rect.width() - expected_pts).abs() > 1.0 {
            log::warn!(
                "[responsive] unit contract violated? screen_rect={:.1}pt, expected {:.1}pt \
                 (window {}px ÷ effective ppp {:.2})",
                rect.width(), expected_pts, gpu.config.width, effective,
            );
        }
    }
    let mut modal_commands: Vec<AppCommand> = Vec::with_capacity(2);

    // Confine all egui UI (menu and modals) to the useful zone (square in portrait,
    // up to 16:9 in landscape). Restricting egui's `screen_rect` to that sub-rectangle
    // makes every anchor (center, top, bottom) follow it, and egui keeps windows
    // inside it. Hit-testing is unchanged (absolute coordinates in points).
    if let Some(full) = raw_input.screen_rect {
        let (uz_x, uz_y, uz_w, uz_h) =
            crate::renderer::init::compute_useful_zone(gpu.config.width, gpu.config.height);
        let (cw, ch) = (gpu.config.width.max(1) as f32, gpu.config.height.max(1) as f32);
        // Scale by fractions of the window, so the result does not depend on pixels_per_point.
        let min = egui::pos2(
            full.min.x + (uz_x as f32 / cw) * full.width(),
            full.min.y + (uz_y as f32 / ch) * full.height(),
        );
        let size = egui::vec2((uz_w as f32 / cw) * full.width(), (uz_h as f32 / ch) * full.height());
        raw_input.screen_rect = Some(egui::Rect::from_min_size(min, size));
    }

    // egui zoom proportional to the useful size. All egui UI (fonts, margins, modal
    // widths, anchors) scales together and stays sharp. It is computed from the
    // logical size (physical / scale_factor) so DPI is not counted twice, since egui
    // already applies it: effective pixels per point = native scale * zoom.
    // It follows the square side of the viewport (the smaller dimension), so the UI
    // tracks the game square, not the margin.
    let scale_factor = native_scale;
    let side_logical = gpu.viewport_width.min(gpu.viewport_height) as f32 / scale_factor.max(0.1);
    let ui_scale = (side_logical / UI_SCALE_REFERENCE).clamp(UI_SCALE_MIN, UI_SCALE_MAX);

    crate::renderer::ui::icons::sync_fonts(&ui.egui_ctx);
    let full_output = ui.egui_ctx.run(raw_input, |ctx| {
        crate::renderer::ui::egui_theme::apply(ctx);
        ctx.set_zoom_factor(ui_scale);

        // Main menu (egui).
        // Test the modals passed in (`modal_opt` / `net_modal_opt`), not
        // `session.menu_state.*`: they were taken out of the session for rendering, so
        // the session sees `None`. Otherwise the menu would be drawn and read the
        // keyboard behind the modal, causing a double action (Escape would close the
        // modal and also trigger MenuBack).
        if matches!(session.flow_state, crate::app::meta_state::FlowState::Menu)
            && modal_opt.is_none()
            && net_modal_opt.is_none()
            && session.menu_state.info_section.is_none()
        {
            use crate::renderer::ui::egui_menu::{render_egui_menu, MenuOutcome, TickerAction, TickerItem};
            // Footer ticker: items without data are omitted. Records come from
            // `records`; the network rank and the support and update items are added below.
            let ticker = {
                use crate::platform::persist::RecordKey;
                let t = crate::app::locale::t();
                let rec = |key: RecordKey| session.user_config.records.iter()
                    .find(|r| r.key == key).map(|r| r.value);
                let mut items: Vec<TickerItem> = Vec::new();
                if let Some(v) = rec(RecordKey::BestScore) {
                    items.push(TickerItem { top: t.ticker_best_solo.to_string(), bottom: v.to_string(), action: TickerAction::None });
                }
                if let Some(v) = rec(RecordKey::LongestGame) {
                    items.push(TickerItem { top: t.ticker_longest_solo.to_string(), bottom: format!("{}:{:02}", v / 60, v % 60), action: TickerAction::None });
                }
                if let Some(v) = rec(RecordKey::BestChain) {
                    items.push(TickerItem { top: t.ticker_best_combo.to_string(), bottom: format!("x{}", v), action: TickerAction::None });
                }
                if let Some(v) = rec(RecordKey::BestBlocksPerMin) {
                    items.push(TickerItem { top: t.ticker_blocks_per_min.to_string(), bottom: v.to_string(), action: TickerAction::None });
                }
                // Network rank (local ladder): shown only once the player is placed, that is
                // rating deviation under the threshold and enough matches played.
                {
                    let cfg = &crackattack_rating::RatingConfig::default();
                    let me = crackattack_rating::PlayerId(session.user_config.player_uuid_bytes());
                    if let Some(r) = session.ladder_records.get(&me) {
                        if r.rating.rd <= cfg.display_rd_threshold
                            && r.matches_played() >= cfg.min_matches_for_ladder
                        {
                            let league = crackattack_rating::league(&r.rating, cfg);
                            items.push(TickerItem {
                                top: t.ticker_network_rank.to_string(),
                                bottom: format!("{} · {:.0}", t.league_name(league), r.rating.value),
                                action: TickerAction::None,
                            });
                        }
                    }
                }
                // Support/contribute item: only when the distribution channel profile allows
                // it and the project's public identity is set. While it is `None`, no GitHub
                // surface is shown.
                {
                    use crate::platform::distribution::{Distribution, PUBLIC_PROJECT};
                    let profile = Distribution::current().profile();
                    if let Some(url) = profile.contribute_url(PUBLIC_PROJECT.as_ref()) {
                        items.push(TickerItem {
                            top: "GITHUB".to_string(), // already upper case: a brand keeps its plain I in Turkish
                            bottom: t.github_cta.to_string(),
                            action: TickerAction::Support { url },
                        });
                    }
                }
                // Update available (an already validated result from the checker).
                #[cfg(feature = "update-check")]
                if let Some(u) = &session.update_check.available {
                    items.push(TickerItem {
                        top: t.ticker_update.to_string(),
                        bottom: t.update_available_fmt.replace("{}", &u.version),
                        action: TickerAction::Update { url: u.open_url.clone() },
                    });
                }
                items
            };
            // Network zone: a "listening" indicator under the menu tells the user they
            // can receive invites here. The services run in the whole zone (see
            // `sync_network_zone_services`).
            if session.negotiation.tcp_listener.is_some()
                && session.menu_state.lan_screen.is_none()
                && session.menu_state.info_section.is_none()
                && session.menu_state.options_modal.is_none()
            {
                use crate::renderer::ui::egui_theme::Palette;
                egui::Area::new(egui::Id::new("network_zone_listening"))
                    .anchor(egui::Align2::CENTER_BOTTOM, [0.0, -18.0])
                    .order(egui::Order::Foreground)
                    .show(ctx, |ui| {
                        ui.horizontal(|ui| {
                            ui.add(egui::Spinner::new().size(13.0).color(Palette::TEXT_ACCENT));
                            ui.label(egui::RichText::new(
                                crate::app::locale::t().listening_incoming)
                                .size(12.0).color(Palette::TEXT_DIM));
                        });
                    });
            }
            match render_egui_menu(ctx, &session.menu_state, &ticker) {
                // Mouse hover selects the row.
                MenuOutcome::Select(i) => {
                    modal_commands.push(AppCommand::MenuSelectIndex(i));
                }
                // Mouse/touch click on row `i`: select that row first, then run its action
                // through the full Confirm logic (`handle_menu_input`) rather than a partial
                // `confirm()`.
                MenuOutcome::Activate(i) => {
                    use crate::app::input_handler::{handle_menu_input, InputContext};
                    use crate::app::ui_intent::UiIntent;
                    modal_commands.push(AppCommand::MenuSelectIndex(i));
                    let mctx = InputContext {
                        transition_cooldown: session.transition_cooldown,
                        can_go_back: session.menu_state.depth() > 0,
                        menu_confirm_action: session.menu_state.action_at(i),
                        game_seed: session.game_seed,
                        ranking_entry_score: session.user_config.ranking_entry_score(),
                    };
                    modal_commands.extend(handle_menu_input(&mctx, UiIntent::Confirm));
                }
                MenuOutcome::StillOpen => {}
            }
        }

        // Options modal
        if let Some(ref mut modal) = modal_opt {
            use crate::renderer::ui::options_modal::{render_options_modal, ModalOutcome};
            match render_options_modal(ctx, modal) {
                ModalOutcome::Confirmed { player_name, sfx_enabled, music_enabled, sfx_volume, music_volume, language, fullscreen, key_bindings } => {
                    modal_commands.push(AppCommand::ApplyOptionsConfirmed {
                        player_name, sfx_enabled, music_enabled, sfx_volume, music_volume, language, fullscreen, key_bindings,
                    });
                }
                ModalOutcome::StillOpen { name_preview, music_volume_preview, music_enabled_preview, nav_focus_request } => {
                    if let Some(name) = name_preview {
                        modal_commands.push(AppCommand::PreviewNameplate { name });
                    }
                    if let Some(vol) = music_volume_preview {
                        modal_commands.push(AppCommand::PreviewMusicVolume(vol));
                    }
                    if let Some(enabled) = music_enabled_preview {
                        modal_commands.push(AppCommand::PreviewMusicVolume(
                            if enabled { session.user_config.music_volume } else { 0.0 }
                        ));
                    }
                    // Mouse hover sets the focus, as a command like keyboard navigation.
                    if let Some(w) = nav_focus_request {
                        modal_commands.push(AppCommand::OptionsNavFocus(w));
                    }
                }
            }
        }

        // LAN discovery screen
        if let Some(ref mut lan_screen) = net_modal_opt {
            use crate::renderer::ui::network_modal::{render_lan_screen, LanScreenOutcome};
            match render_lan_screen(ctx, lan_screen) {
                LanScreenOutcome::PeerClicked { addr, port } => {
                    modal_commands.push(AppCommand::ConnectToPeer { addr, port });
                }
                LanScreenOutcome::ManualConnect { addr, port } => {
                    modal_commands.push(AppCommand::ConnectToPeer {
                        addr, port,
                    });
                }
                LanScreenOutcome::ReconnectManual => {
                    modal_commands.push(AppCommand::ReconnectToManual);
                }
                LanScreenOutcome::OpenUrl(url) => {
                    modal_commands.push(AppCommand::OpenUrl(url));
                }
                LanScreenOutcome::RequestPaste => {
                    modal_commands.push(AppCommand::LanPasteRequest);
                }
                LanScreenOutcome::CancelInvite => {
                    modal_commands.push(AppCommand::CancelInvite);
                }
                LanScreenOutcome::Cancelled => {
                    modal_commands.push(AppCommand::CloseLanScreen);
                }
                LanScreenOutcome::StillOpen => {}
            }
        }

        // About/Info page (over the menu). Keyboard and gamepad navigation goes through `route_info_intent`.
        if let Some(section) = session.menu_state.info_section {
            use crate::app::menu_tree::InfoSection;
            use crate::renderer::ui::info_modal::{render_info_screen, InfoOutcome};
            let sd = session.menu_state.info_scroll_delta;
            let bf = session.menu_state.info_focus_back;
            let outcome = if matches!(section, InfoSection::NetworkRank | InfoSection::NetworkOpponents) {
                // Network rank and opponents pages: data comes from the ladder journal, not
                // text. Navigation is the same; the opponents page adds activatable rows
                // (quick reconnect).
                use crate::renderer::ui::rank_page::{render_opponents_page, render_rank_page};
                use crate::app::ladder_view::OpponentContact;
                let me = crackattack_rating::PlayerId(session.user_config.player_uuid_bytes());
                let names = |id: &crackattack_rating::PlayerId| {
                    session.ladder.display_name(id).map(str::to_string)
                };
                let contact = |id: &crackattack_rating::PlayerId| {
                    let online = session.negotiation.net_peers.iter()
                        .any(|p| p.uuid_hex.as_deref() == Some(id.to_hex().as_str()));
                    let mut c = session.ladder.player_meta(id).map(|m| OpponentContact {
                        last_rt: m.last_announced_rt,
                        reachable: m.last_addr.is_some() && m.last_port.is_some(),
                        online: false,
                    }).unwrap_or_default();
                    // "Online" (an mDNS announcement carrying its uuid) makes the row
                    // activable even without a stored address: a fresh address is at hand.
                    c.online = online;
                    c.reachable = c.reachable || online;
                    c
                };
                let data = crate::app::ladder_view::build(
                    session.ladder.entries(),
                    &session.ladder_records,
                    &names,
                    &contact,
                    me,
                    &crackattack_rating::RatingConfig::default(),
                );
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_secs())
                    .unwrap_or(0);
                if section == InfoSection::NetworkOpponents {
                    let selected = session.menu_state.rank_selected;
                    // Selection moved by keyboard: recenter the scroll (flag is consumed).
                    let scroll_to_sel = session.menu_state.rank_scroll_pending.replace(false);
                    render_opponents_page(ctx, &data, &crackattack_rating::RatingConfig::default(), now, sd, bf, selected, scroll_to_sel)
                } else {
                    render_rank_page(ctx, &data, &crackattack_rating::RatingConfig::default(), now, sd, bf)
                }
            } else {
                // Credits page surfaces are resolved outside rendering (profile and public
                // identity), by the same function as the keyboard/gamepad router.
                let extras = session.info_extras();
                let selected = session.menu_state.info_action_selected;
                let scroll_to_sel = session.menu_state.info_action_scroll_pending.replace(false);
                render_info_screen(ctx, section, sd, bf, &extras, selected, scroll_to_sel)
            };
            match outcome {
                InfoOutcome::Close => modal_commands.push(AppCommand::CloseInfoScreen),
                // Mouse hover on Back sets the focus, like the other dialogs.
                InfoOutcome::HoverBack => modal_commands.push(AppCommand::SetInfoFocusBack(true)),
                // Opponent rows: hover selects, click quick-reconnects (the runtime
                // validates the stored address).
                InfoOutcome::HoverRow(i) => modal_commands.push(AppCommand::RankSelectSet(i)),
                InfoOutcome::RowClicked(i) => modal_commands.push(AppCommand::ReconnectOpponent(i)),
                // Credits actions: a click sends the same command as Confirm on keyboard/gamepad.
                InfoOutcome::InfoActionActivated(action) => modal_commands.push(action.command()),
                InfoOutcome::HoverInfoAction(i) => modal_commands.push(AppCommand::InfoActionSelectSet(i)),
                InfoOutcome::StillOpen => {}
            }
            if sd != 0.0 {
                modal_commands.push(AppCommand::ScrollInfo(0.0)); // delta consumed: reset
            }
        }

        // Received invite: a session-level overlay drawn over the current screen (menu,
        // network rank, recent opponents, LAN screen...) without changing screens.
        if let Some(ref info) = session.negotiation.pending_invite_info {
            use crate::renderer::ui::network_modal::{render_invite_overlay, InviteOverlayOutcome};
            match render_invite_overlay(ctx, info) {
                InviteOverlayOutcome::Accept => modal_commands.push(AppCommand::AcceptInvite),
                InviteOverlayOutcome::Decline => modal_commands.push(AppCommand::DeclineInvite),
                InviteOverlayOutcome::StillOpen => {}
            }
        }

        // Session modals (from plan.presentation)
        if let Some(ref pres) = presentation {
            use crate::app::presentation::ModalKind;
            match &pres.modal {
                Some(ModalKind::PauseMenu { local_ready, remote_ready, is_network }) => {
                    use crate::renderer::ui::pause_modal::{render_pause_modal, PauseModalContext, PauseOutcome};
                    let seconds_open = session.pause_menu_opened_at
                        .map_or(10.0, |t| t.elapsed().as_secs_f32());
                    let needs_delay = *is_network && !session.pause_initiated_locally;
                    let pctx = PauseModalContext {
                        local_ready: *local_ready,
                        remote_ready: *remote_ready,
                        is_network: *is_network,
                        seconds_open,
                        needs_delay,
                    };
                    if session.post_game_input.abandon_confirm_open {
                        // Quit opened the confirmation: render it over the pause menu.
                        render_abandon_confirm(ctx, session.post_game_input.abandon_confirm_yes, &mut modal_commands);
                    } else {
                        match render_pause_modal(ctx, &pctx, &session.pause_menu_selection) {
                            PauseOutcome::Resume => modal_commands.push(AppCommand::ResumeGame),
                            PauseOutcome::ResumeReady => modal_commands.push(AppCommand::SignalResumeReady),
                            // Quit is resolved by the runtime: confirmation if a round is
                            // in progress, otherwise a direct exit.
                            PauseOutcome::Quit => modal_commands.push(AppCommand::RequestQuitGame),
                            PauseOutcome::StillOpen => {}
                        }
                    }
                }
                Some(ModalKind::ReadyPrompt { round_number, remote_ready, opponent_present }) => {
                    let show_stats = *round_number > 1
                        && session.round_end_stats.is_some()
                        && !session.post_game_input.post_round_advanced;

                    if show_stats {
                        // Post-round summary (curtain closed, panel on top) using the shared
                        // `nav_view` model: result, stats, rating delta and tally. It only
                        // reports the outcome; it has no consent content (no question, no
                        // "X is ready"), which belongs to the ready modal.
                        // Buttons: [Quit <-] and [Next ->]. Enter means Next, which is safe:
                        // it opens the curtain and the ready modal asks the question.
                        // If the opponent left, the left button becomes [Menu] and the
                        // right one is hidden, with no departure message.
                        // Keyboard and gamepad decisions are made by the router
                        // (`route_post_round`); this code only renders and handles mouse
                        // clicks. The router sets the held scroll direction (`scroll_dir`)
                        // and `nav_view` applies the per-frame delta.
                        use crate::renderer::ui::nav_view::{render_nav_view, NavView, NavFocus, NavOutcome, SCROLL_SPEED};
                        use crate::renderer::ui::icons;
                        use crate::renderer::ui::end_screen_panels as esp;
                        use crate::app::post_game::PostGameNavFocus;
                        let t = crate::app::locale::t();
                        let rs = session.round_end_stats.as_ref().unwrap();
                        let live_opp = rs.opponent.as_ref().or_else(||
                            plan.snapshot.as_ref()
                                .and_then(|s| s.vs_display.as_ref())
                                .and_then(|vs| vs.opponent_stats.as_ref())
                        );
                        let focus = match session.post_game_input.nav_focus {
                            PostGameNavFocus::Menu => NavFocus::Left,
                            PostGameNavFocus::Next => NavFocus::Right,
                            PostGameNavFocus::None => NavFocus::Content,
                        };
                        let title = t.round_label(rs.round_number);
                        let view = NavView {
                            id_salt: "round_stats",
                            title: &title,
                            left_icon: icons::CHEVRONS_LEFT,       // Quit (direct exit, the round is scored)
                            left_label: Some(if *opponent_present { t.quit } else { t.menu_label }),
                            // Next is absent if the opponent left: the ready question is
                            // never asked.
                            right_icon: if *opponent_present { Some(icons::ARROW_RIGHT) } else { None },
                            right_label: if *opponent_present { Some(t.next) } else { None },
                            focus,
                            scroll_delta: session.post_game_input.scroll_dir as f32 * SCROLL_SPEED,
                            opaque_bg: false,                      // curtain visible behind
                            center_content: true,                  // stats centered vertically
                        };
                        // Ladder: rating change for the round plus the session total, shown
                        // inside the result banner of the summary.
                        let rating_summary = session.ladder_round_delta.and_then(|delta| {
                            let me = crackattack_rating::PlayerId(session.user_config.player_uuid_bytes());
                            session.ladder_records.get(&me).map(|r| {
                                esp::RatingSummary {
                                    league: crackattack_rating::league(&r.rating, &crackattack_rating::RatingConfig::default()),
                                    value: r.rating.value.round() as i32,
                                    round_delta: delta,
                                    session_delta: session.ladder_match_delta
                                        .filter(|total| (total - delta).abs() > f64::EPSILON),
                                }
                            })
                        });
                        let out = render_nav_view(ctx, &view, |ui| {
                            let opp_name = plan.snapshot.as_ref()
                                .and_then(|sn| sn.vs_display.as_ref())
                                .map(|vs| vs.opponent_name.clone());
                            let names = opp_name.as_deref()
                                .map(|o| (session.player_name.as_str(), o));
                            esp::draw_round_results_body(ui, rs, live_opp, names, rating_summary.as_ref());
                        });

                        // Mouse click on a side button (keyboard/gamepad go through the router).
                        // Quit is direct: the round is over and scored, so nothing to confirm.
                        match out {
                            NavOutcome::LeftActivated => modal_commands.push(AppCommand::RequestQuitGame),
                            NavOutcome::RightActivated => modal_commands.push(AppCommand::AdvanceToReadyPrompt),
                            // Mouse hover focuses the side button, like the main menu.
                            NavOutcome::HoverLeft => modal_commands.push(AppCommand::SetPostGameNavFocus(PostGameNavFocus::Menu)),
                            NavOutcome::HoverRight => modal_commands.push(AppCommand::SetPostGameNavFocus(PostGameNavFocus::Next)),
                            NavOutcome::StillOpen => {}
                        }
                    } else if !render_groups.curtain_menu_game.active {
                        // The "Ready?" question, for round 1 and later rounds: being ready is
                        // the consent. It appears only once the scene is ready: the menu to
                        // game curtain is done (round 1) or the curtain was opened by Next
                        // (later rounds, the new grid is visible behind).
                        // Two navigable buttons: Ready and Quit (a direct exit, nothing is
                        // committed). "X is ready" is shown here, at the moment of answering,
                        // never on the summary. If the opponent left, only Quit remains.
                        // The router handles keyboard/gamepad; this code handles mouse clicks.
                        use crate::renderer::ui::ready_modal::{render_ready_modal, ReadyOutcome, ReadyModalContext};
                        // Opponent status (network only; the AI consents instantly): "X is
                        // ready" or "X is not ready yet", so the player knows whether the
                        // other side has already answered.
                        let tr = crate::app::locale::t();
                        let opp_status = if *opponent_present {
                            plan.snapshot.as_ref()
                                .and_then(|sn| sn.vs_display.as_ref())
                                .filter(|vs| vs.opponent_uuid.is_some())
                                .map(|vs| (
                                    if *remote_ready { tr.player_ready(&vs.opponent_name) }
                                    else { tr.player_not_ready(&vs.opponent_name) },
                                    *remote_ready,
                                ))
                        } else {
                            None
                        };
                        let mctx = ReadyModalContext {
                            round_number: *round_number,
                            opponent_present: *opponent_present,
                            opp_status,
                        };
                        match render_ready_modal(ctx, &mctx, &session.ready_selection) {
                            ReadyOutcome::Ready => modal_commands.push(AppCommand::SignalLocalReady),
                            ReadyOutcome::Quit => modal_commands.push(AppCommand::RequestQuitGame),
                            ReadyOutcome::StillOpen => {}
                        }
                    }
                }
                Some(ModalKind::WaitingBanner { text, quit_timeout_secs }) => {
                    use crate::renderer::ui::waiting_banner::{render_waiting_banner, WaitingOutcome};
                    let elapsed_secs = session.suspension_timer.elapsed_secs();
                    match render_waiting_banner(ctx, text, elapsed_secs, *quit_timeout_secs) {
                        WaitingOutcome::Quit => modal_commands.push(AppCommand::QuitToMenu),
                        WaitingOutcome::Escape => modal_commands.push(AppCommand::OpenPauseMenu),
                        WaitingOutcome::StillWaiting => {}
                    }
                }
                Some(ModalKind::DisconnectedBanner { text }) => {
                    use crate::renderer::ui::waiting_banner::{render_waiting_banner, WaitingOutcome};
                    match render_waiting_banner(ctx, text, 999.0, 0.001) {
                        WaitingOutcome::Quit | WaitingOutcome::Escape => {
                            modal_commands.push(AppCommand::QuitToMenu);
                        }
                        WaitingOutcome::StillWaiting => {}
                    }
                }
                None => {}
            }
        }

        // Results panels
        if plan.post_game_results_visible
            && !render_groups.curtain_menu_game.active
        {
            // Post-game screen using the shared `nav_view` model: pinned title, scrollable
            // body, Menu button on the left and Next button on the right. Content pages
            // use the same model. The router in `main.rs` handles keyboard/gamepad (focus
            // ring); this code renders and handles mouse clicks (`NavOutcome`). The body
            // is a rich panel (tables, stars, accent values), not rendered text.
            use crate::renderer::ui::nav_view::{render_nav_view, NavView, NavFocus, NavOutcome};
            use crate::renderer::ui::icons;
            use crate::renderer::ui::end_screen_panels as esp;
            use crate::app::post_game::{PostGameScreen, PostGameNavFocus};
            use crate::app::meta_state::FlowState;

            let show_score = session.selected_mode.uses_score();
            // VS: `round_end_stats` holds the reliable opponent data (fall back to the snapshot).
            let opp_stats = session.round_end_stats.as_ref()
                .and_then(|rs| rs.opponent.as_ref())
                .or_else(|| plan.snapshot.as_ref()
                    .and_then(|s| s.vs_display.as_ref())
                    .and_then(|vs| vs.opponent_stats.as_ref()));

            let screen = session.post_game_screen;
            let is_final = screen.is_final();
            // VS round stats (tally and comparison) versus solo results (high scores / stats).
            let is_round = !show_score
                && screen == PostGameScreen::Stats
                && session.round_end_stats.is_some();

            let title = if is_round {
                let rs = session.round_end_stats.as_ref().unwrap();
                crate::app::locale::t().round_label(rs.round_number)
            } else {
                esp::results_title(screen).to_string()
            };
            let focus = match session.post_game_input.nav_focus {
                PostGameNavFocus::Menu => NavFocus::Left,
                PostGameNavFocus::Next => NavFocus::Right,
                PostGameNavFocus::None => NavFocus::Content,
            };
            // Each screen has its own scroll state (reset when the screen changes).
            let id_salt = match screen {
                PostGameScreen::HighScores => "pg_high_scores",
                PostGameScreen::Stats => "pg_stats",
                PostGameScreen::ResultMessage => "pg_msg",
            };

            let view = NavView {
                id_salt,
                title: &title,
                left_icon: icons::CHEVRONS_LEFT,      // back to menu
                // On the final screen the right arrow also returns to the menu, and
                // its label says so.
                left_label: Some(crate::app::locale::t().menu_label),
                right_label: Some(if is_final {
                    crate::app::locale::t().menu_label
                } else {
                    crate::app::locale::t().next
                }),
                right_icon: Some(icons::ARROW_RIGHT), // Next (menu on the final screen)
                focus,
                // Held direction (set by `main.rs`) gives a per-frame delta for smooth scrolling.
                scroll_delta: session.post_game_input.scroll_dir as f32
                    * crate::renderer::ui::nav_view::SCROLL_SPEED,
                opaque_bg: false,     // game scene visible behind
                center_content: true, // results centered vertically
            };
            let out = render_nav_view(ctx, &view, |ui| {
                if is_round {
                    let rs = session.round_end_stats.as_ref().unwrap();
                    esp::draw_round_results_body(ui, rs, opp_stats, None, None);
                } else {
                    match screen {
                        PostGameScreen::HighScores => esp::draw_high_scores_body(
                            ui, &session.user_config.scores, session.game_over_rank),
                        PostGameScreen::Stats => esp::draw_final_stats_body(
                            ui, session.game_over_stats.as_ref(), opp_stats,
                            session.game_over_score, show_score,
                            &session.user_config.records, &session.game_over_beaten_records,
                            None),
                        PostGameScreen::ResultMessage => {}
                    }
                }
            });

            match out {
                NavOutcome::LeftActivated => {
                    modal_commands.push(AppCommand::PlayUiConfirm);
                    modal_commands.push(AppCommand::StartTransition(FlowState::Menu, 0.5));
                }
                NavOutcome::RightActivated => {
                    modal_commands.push(AppCommand::PlayUiConfirm);
                    if is_final {
                        modal_commands.push(AppCommand::StartTransition(FlowState::Menu, 0.5));
                    } else {
                        modal_commands.push(AppCommand::AdvancePostGameScreen);
                        modal_commands.push(AppCommand::SetTransitionCooldown(15));
                    }
                }
                // Mouse hover focuses the side button, like the main menu.
                NavOutcome::HoverLeft => modal_commands.push(AppCommand::SetPostGameNavFocus(PostGameNavFocus::Menu)),
                NavOutcome::HoverRight => modal_commands.push(AppCommand::SetPostGameNavFocus(PostGameNavFocus::Next)),
                NavOutcome::StillOpen => {}
            }
        }

        // Dev tuning panel
        #[cfg(feature = "dev")]
        if _debug.dev_panel.visible {
            super::super::dev_ui::render_dev_tuning_window(ctx, &mut _debug.dev_panel);
        }
    });

    // Clear modal_opt if consumed
    if modal_commands.iter().any(|c| matches!(c,
        AppCommand::ApplyOptionsConfirmed { .. } | AppCommand::CloseOptionsModal
    )) {
        *modal_opt = None;
    }

    // egui outputs (cursor, clipboard) are extracted here and applied by the binary
    // through `PlatformShell`.
    let po = full_output.platform_output;
    let copy_text = po.commands.iter().find_map(|c| match c {
        egui::OutputCommand::CopyText(t) if !t.is_empty() => Some(t.clone()),
        _ => None,
    });
    ui.pending_egui_outputs = Some(crate::renderer::ui::ui_runtime::PendingEguiOutputs {
        cursor_icon: po.cursor_icon,
        copy_text,
    });

    // Tessellate and encode egui
    let paint_jobs = ui.egui_ctx.tessellate(full_output.shapes, full_output.pixels_per_point);
    let sd = egui_wgpu::ScreenDescriptor {
        size_in_pixels: [gpu.config.width, gpu.config.height],
        pixels_per_point: full_output.pixels_per_point,
    };
    for (id, delta) in &full_output.textures_delta.set {
        ui.egui_renderer.update_texture(&gpu.device, &gpu.queue, *id, delta);
    }
    let mut egui_enc = gpu.device.create_command_encoder(
        &wgpu::CommandEncoderDescriptor { label: Some("egui") },
    );
    ui.egui_renderer.update_buffers(&gpu.device, &gpu.queue, &mut egui_enc, &paint_jobs, &sd);
    {
        let pass = egui_enc.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("egui_pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view,
                resolve_target: None,
                ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store },
            })],
            depth_stencil_attachment: None,
            ..Default::default()
        });
        let mut pass = pass.forget_lifetime();
        ui.egui_renderer.render(&mut pass, &paint_jobs, &sd);
    }
    for id in &full_output.textures_delta.free {
        ui.egui_renderer.free_texture(id);
    }

    Some((modal_commands, egui_enc.finish()))
}

