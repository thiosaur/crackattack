// Windows release builds have no console window. The log also goes to a file on every platform
// (see `platform::diagnostics`). Debug builds keep the console.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

//! Crack Attack! — native application shell.
//!
//! This file is the orchestration layer between the subsystems:
//!
//! - `app/`      — pure decision logic (input handler, commands, flow state)
//! - `platform/` — native services (audio, persistence, CLI)
//! - `renderer/` — GPU rendering (scene, UI, effects, GPU infrastructure)
//!
//! ## Architecture
//!
//! `AppState` owns the subsystems and wires them together:
//! - `GpuContext`      — device, queue, surface, viewport
//! - `SessionRuntime`  — session state + audio; owns `apply_app_command`
//! - `SceneRenderer`   — pipelines (ScenePipelines) + executor + visual state
//! - `UiRuntime` — UI pipeline, texture cache, egui
//! - `ParticleRuntime`   — compute sparkles
//!
//! The SDL3 event loop (`run_sdl_backend`) drives `AppState`:
//! - `tick()`   — fixed 50Hz game step, AI, post-game sync
//! - `update()` — wall-clock transitions (before render)
//! - `render()` — read-only on session state
//!
//! Input goes through `app/input_handler` → `AppCommand` → `apply_app_command`.
//! The render path never mutates session or flow state.
//!
//! ## Init
//!
//! `AppState::new()` lives in `app_bootstrap.rs` — it creates all GPU
//! resources sequentially. Mesh, particle and UI infrastructure are
//! delegated to `build_mesh_resources`, `build_particle_resources` and
//! `build_ui_resources`. The block atlas and texture cache are loaded
//! early so bind groups use real textures (no placeholder/rebuild).
//!
//! ## Session Runtime
//!
//! `SessionRuntime` (session state + audio) lives in `session_runtime.rs`.
//! All session mutations go through `SessionRuntime::apply_app_command`.
//! Commands that need renderer/UI/debug are returned as residuals.

use std::time::{Duration, Instant};

use crate::renderer::scene::transforms::create_transform_uniforms;
use crate::app::command::{AppCommand, AppKey, AppMusic};

/// Window dimensions in logical pixels.
const WINDOW_WIDTH: u32 = 700;
const WINDOW_HEIGHT: u32 = 700;

/// Game tick rate: 50 Hz fixed step.
const TICKS_PER_SECOND: u32 = 50;
const TICK_DURATION: Duration = Duration::from_micros(1_000_000 / TICKS_PER_SECOND as u64);

/// Wake-up interval for static states (solo pause): about 15 Hz.
/// The game logic still runs at 50 Hz (the accumulator in `tick()` catches up with
/// real time). Only wake-ups and rendering are less frequent, which saves about
/// 70% of the frames in these states.
const IDLE_TICK_DURATION: Duration = Duration::from_micros(1_000_000 / 15);

use crate::app::meta_state::{NameInputState, FlowState};
use crate::platform::sound::{AudioOutput, MusicTrack};

use crackattack_core::FrameData;
// GameSession trait is used via Box<dyn GameSession> in SessionState

// ── Modules (a single binary crate, so the `dead_code` lint sees the whole tree) ──
mod sync_ext;
mod debug_state;
mod scheduling;
mod app;
mod platform;
mod renderer;
#[cfg(feature = "dev")]
mod dev_tuning;
mod input_source;
// Remote part of the update system. It can be removed at build time with
// `--no-default-features --features native`.
#[cfg(all(feature = "native", feature = "update-check"))]
mod update;
mod matrix;

/// Short game name, shown in the window title and the UI.
pub const GAME_NAME: &str = "Crack Attack!";
/// Full edition name, shown where the product is identified (Info screen, metadata).
/// The subtitle "Back From The Stack" distinguishes this Rust 2.x port from the
/// original game.
pub const GAME_NAME_FULL: &str = "Crack Attack! Back From The Stack";


const BLOCK_SHADER: &str = include_str!("wgsl/block.wgsl");
use renderer_runtime::{GarbageFlavorImageState, GARBAGE_FLAVOR_COUNT, RendererRuntime};
use renderer::gpu::context::GpuContext;
use scheduling::SchedulingState;
use debug_state::DebugState;

/// Resolve P/Escape key into an AppCommand based on the current escape policy.
/// Single source of truth for pause/resume/menu-transition behavior.
fn resolve_escape_action(
    action: &crate::app::presentation::EscapeAction,
    key: AppKey,
) -> Option<AppCommand> {
    use crate::app::presentation::EscapeAction;
    match (action, key) {
        (EscapeAction::OpenPauseMenu, AppKey::Cancel | AppKey::Pause) => {
            Some(AppCommand::OpenPauseMenu)
        }
        (EscapeAction::Resume, AppKey::Cancel | AppKey::Pause) => {
            Some(AppCommand::ResumeGame)
        }
        (EscapeAction::SignalResumeReady, AppKey::Cancel | AppKey::Pause) => {
            Some(AppCommand::SignalResumeReady)
        }
        (EscapeAction::TransitionMenu, AppKey::Cancel) => {
            Some(AppCommand::StartTransition(FlowState::Menu, 0.5))
        }
        _ => None,
    }
}



// impl RendererRuntime, impl SceneRenderer, and rendering free functions
// are in renderer_runtime.rs (binary submodule).

/// Application state — owns all subsystems, wired by the event loop.
///
/// Subsystems are separated by ownership so the borrow checker
/// enforces boundaries (e.g. renderer can't mutate session state).
struct AppState<P: platform_shell::PlatformShell> {
    /// Owns the wgpu surface, which is built from the raw handles of the platform window.
    gpu: GpuContext,
    renderer: RendererRuntime,
    runtime: SessionRuntime,
    scheduling: SchedulingState,
    debug: DebugState,
    /// Cached per-tick presentation derivation. Updated once in tick(),
    /// consumed by has_session_modal() and grid visibility. Avoids
    /// redundant snapshot() + derive_presentation() calls.
    cached_has_modal: bool,
    cached_grid_cleared: bool,
    /// Compiled input bindings for O(1) lookup. Rebuilt on config change.
    key_bindings: keycodes::CompiledBindings,
    /// Window is occluded (hidden or minimized), tracked from the event stream
    /// (`InputEvent::Occluded`). The SDL loop skips rendering while it is set.
    occluded: bool,
    /// Test miniscript being replayed (`--script` / `session:qa.script`); `None` in normal
    /// play. Ticked once per frame in `drain_inputs`, which injects `InputEvent`s at the
    /// `poll_events` seam.
    script: Option<script::ScriptRunner>,
    /// Input recorder (`--record` / `session:qa.record`); `None` otherwise. Captures the
    /// real `InputEvent::Key` events in `drain_inputs` and writes the `.mini` file on exit.
    recorder: Option<script::Recorder>,
    /// Platform text input currently on (see `sync_text_input`).
    text_input_on: bool,
    /// Platform layer: window, event stream and outputs (cursor, clipboard, fullscreen).
    /// Generic over `PlatformShell`; `Sdl3Platform` is the only implementation, and the
    /// generic keeps the layer replaceable in tests.
    ///
    /// Declared LAST on purpose: fields drop in declaration order, and the window must
    /// outlive the surface held by `gpu`.
    platform: P,
}

impl<P: platform_shell::PlatformShell> AppState<P> {
    // AppState::new() is in app_bootstrap.rs (binary submodule).

    /// Handle a window resize.
    /// Any size and aspect ratio is accepted. The game renders into the useful zone
    /// (`compute_useful_zone`): a centered rectangle with an aspect ratio between 1:1 and
    /// 16:9. The rest of the window is a black margin that blends with the black clear
    /// color of `frame_submit`. `ui_scale` follows the shorter side of the useful zone,
    /// so egui scales with the game, not with the margin.
    fn handle_resize(&mut self, new_size: (u32, u32)) {
        self.resize(new_size);
    }

    /// Is there an active egui modal from the session (pause, ready, waiting)?
    /// Reads the cached value updated once per tick.
    fn has_session_modal(&self) -> bool {
        self.cached_has_modal
    }

    /// Is the options screen waiting for a key or button to (re)bind an action?
    /// While capturing, keyboard events go to the input router (which assigns the
    /// binding), not to egui, even if the options overlay is open.
    fn is_capturing_binding(&self) -> bool {
        self.runtime.session.menu_state.options_modal.as_ref()
            .map_or(false, |m| m.capture_binding.is_some())
    }

    /// Active input context. This is the single decision point: it gathers the session
    /// state and delegates to the pure `resolve_context`.
    fn active_context(&self) -> crate::app::input_context::Context {
        use crate::app::input_context::{ContextFlow, ContextInputs, resolve_context};
        let ms = &self.runtime.session.menu_state;
        let (flow, gameplay_active) = match self.runtime.session.flow_state {
            FlowState::Menu => (ContextFlow::Menu, false),
            _ => {
                let pgs = if matches!(self.runtime.session.flow_state, FlowState::PostGame) {
                    Some(self.runtime.session.post_game_screen)
                } else {
                    None
                };
                let gameplay = self.runtime.session.game.as_ref().map(|s| {
                    let snap = s.snapshot();
                    crate::app::presentation::derive_input(
                        &snap, self.runtime.session.pause_menu_open, pgs,
                    ).gameplay
                }).unwrap_or(false);
                (ContextFlow::InGame, gameplay)
            }
        };
        resolve_context(&ContextInputs {
            capture: ms.options_modal.as_ref().and_then(|m| m.capture_binding),
            flow,
            options_open: ms.options_modal.is_some(),
            options_osk_open: ms.options_modal.as_ref().map(|m| m.keyboard_open).unwrap_or(false),
            lan_open: ms.lan_screen.is_some(),
            pause_menu_open: self.runtime.session.pause_menu_open,
            gameplay_active,
        })
    }

    /// Should keyboard events go to **egui** instead of the `dispatch_input` router?
    /// - binding capture: no (router);
    /// - pause menu: no (router);
    /// - options: only while the on-screen keyboard edits text (dedicated context);
    ///   otherwise navigation goes through `dispatch_input` and `route_options_intent`;
    /// - other modals: egui.
    fn keyboard_goes_to_egui(&self) -> bool {
        use crate::app::input_context::Context;
        if self.is_capturing_binding() {
            return false;
        }
        // An invite overlay is showing: the keyboard goes to the router (`route_invite_intent`),
        // even over the manual connect screen (otherwise IP entry would capture the keys).
        if self.runtime.session.negotiation.pending_invite_info.is_some() {
            return false;
        }
        match self.active_context() {
            Context::PauseMenu => false,
            // Name editor (on-screen keyboard) open: physical keys go to egui so that
            // `egui::Event::Text` is produced (AZERTY and shift are handled natively) and
            // collected by `render_onscreen_keyboard`. The gamepad does not produce window
            // events, so it keeps navigating the virtual keys through the router.
            Context::MenuOptionsTextEdit => true,
            // Options panel without an open editor: the keyboard always belongs to the router.
            // Giving it to egui whenever a widget has focus (for example after a click on a
            // checkbox) would break keyboard navigation. Name entry uses the on-screen keyboard,
            // which has its own context above.
            Context::MenuOptions => false,
            // LAN screen: navigation goes to the router (single pipeline, same as menu and
            // options), including the manual connect screen. Exception: while a field is edited
            // with the on-screen keyboard, physical keys go to egui so that `egui::Event::Text`
            // is produced (same model as `MenuOptionsTextEdit`).
            Context::MenuLan => self.runtime.session.menu_state.lan_screen.as_ref()
                .map_or(false, |l| l.manual_open && l.manual.editing.is_some()),
            _ => {
                // `ReadyPrompt` (ready phase and between-round stats): the router handles all
                // interaction (keyboard and gamepad), so the keyboard does not go to egui.
                // `ready_modal` and the stats only render. Waiting and Disconnected still use egui,
                // because their Quit button has no router equivalent.
                if self.is_ready_prompt() {
                    return false;
                }
                // The Info page has no text input, so its keyboard goes to the router, like the
                // gamepad. It is not counted here. Options and LAN have their own arms above.
                self.runtime.session.pause_menu_open || self.has_session_modal()
            }
        }
    }

    /// Debug mode (global F10 hotkey): logs a snapshot of what matters for diagnosis.
    /// It covers the input context, flow, overlays and above all the network state (LAN
    /// phase, discovered peers with address, port and status, negotiation in progress,
    /// active discovery and listening). It works everywhere (menu, network mode, in
    /// game) and writes to the log (`RUST_LOG=info`).
    fn log_debug_snapshot(&self) {
        let s = &self.runtime.session;
        log::info!("===== DEBUG SNAPSHOT =====");
        log::info!("flow={:?}  context={:?}", s.flow_state, self.active_context());
        log::info!("mode={:?}  seed={}  name={:?}", s.selected_mode, s.game_seed, s.player_name);
        log::info!(
            "overlays: options_modal={}  lan_screen={}  pause_menu={}",
            s.menu_state.options_modal.is_some(),
            s.menu_state.lan_screen.is_some(),
            s.pause_menu_open,
        );
        log::info!(
            "net: discovery={}  tcp_listener={}  pending_invite={}  pending_connection={}",
            s.negotiation.discovery.is_some(),
            s.negotiation.tcp_listener.is_some(),
            s.negotiation.pending_invite.is_some(),
            !s.negotiation.phase.is_idle(),
        );
        if let Some(ref lan) = s.menu_state.lan_screen {
            log::info!(
                "LAN: phase={:?}  selected={}  manual_open={}  peers={}",
                lan.phase, lan.selected, lan.manual_open, lan.peers.len(),
            );
            if lan.manual_open {
                log::info!(
                    "  manual: focus={:?}  editing={:?}  ipv4={:?}  ipv6={:?}  port={:?}",
                    lan.manual.nav_focus, lan.manual.editing,
                    lan.manual.ipv4, lan.manual.ipv6, lan.manual.port,
                );
            }
            for p in &lan.peers {
                log::info!(
                    "  peer name={:?} addr={} port={} status={:?} sid={}",
                    p.name, p.addr, p.port, p.status, p.session_id,
                );
            }
        }
        log::info!("==========================");
    }

    /// A text field is being edited: the name editor of the options, or a field of the
    /// manual connect screen.
    fn text_entry_open(&self) -> bool {
        let menu = &self.runtime.session.menu_state;
        menu.options_modal.as_ref().is_some_and(|m| m.keyboard_open)
            || menu.lan_screen.as_ref().is_some_and(|l| l.manual_open && l.manual.editing.is_some())
    }

    /// Turns the platform text input on while a text field is edited, off otherwise. SDL3
    /// delivers typed characters (`InputEvent::Text`) only while it is on.
    fn sync_text_input(&mut self) {
        let want = self.text_entry_open();
        if want != self.text_input_on {
            self.platform.set_text_input(want);
            self.text_input_on = want;
            log::debug!("Text input {}", if want { "on" } else { "off" });
        }
    }

    /// Dispatch the frame's input events between egui and the exclusive input router.
    /// The split is per event: `keyboard_goes_to_egui()` is re-evaluated between keys,
    /// because dispatching can open or close the on-screen keyboard mid-frame. Returns
    /// the events meant for egui and dispatches the router keys on the way (including the
    /// F12 and backquote hotkeys, on the neutral `Key` type).
    fn dispatch_frame_inputs(&mut self, events: Vec<platform_shell::InputEvent>) -> Vec<platform_shell::InputEvent> {
        use platform_shell::{gameplay_bridge, InputEvent};
        let mut egui_events = Vec::with_capacity(events.len());
        for ev in events {
            // Shell events (resize, focus, close, occlusion) are handled from the stream.
            match &ev {
                InputEvent::Resized { pixels, .. } => {
                    self.handle_resize(*pixels);
                }
                InputEvent::FocusChanged(f) => { self.renderer.ui.focused = *f; }
                InputEvent::CloseRequested => { self.debug.should_exit = true; }
                InputEvent::Occluded(o) => { self.occluded = *o; }
                // Gamepad: same entry point as the keyboard. The router picks capture, menu or
                // in-game by context. Never goes to egui.
                InputEvent::Gamepad(g) => {
                    crate::input_source::set(crate::input_source::InputSource::Gamepad);
                    let cmds = self.dispatch_input(g.input, g.pressed, false);
                    for cmd in cmds {
                        self.apply_app_command(cmd);
                    }
                    continue;
                }
                _ => {}
            }
            let is_kbd = matches!(ev, InputEvent::Key { .. } | InputEvent::Text(_));
            if !is_kbd || self.keyboard_goes_to_egui() {
                egui_events.push(ev);
                continue;
            }
            // Text that is not for egui: the router does not consume text, so it is ignored.
            let Some(k) = gameplay_bridge::to_key_event(&ev) else { continue; };
            // Input source = keyboard.
            crate::input_source::set(crate::input_source::InputSource::Keyboard);
            // Global F10 debug hotkey (diagnostic snapshot, mostly network), whatever the
            // context.
            if k.key == keycodes::Key::F10 && k.pressed && !k.repeat {
                self.log_debug_snapshot();
                continue;
            }
            // Global debug hotkeys (on the neutral key type).
            if k.key == keycodes::Key::F12 && k.pressed && !k.repeat {
                self.debug.screenshot_request =
                    Some(crate::platform::screenshot::ScreenshotRequest::Manual);
                self.platform.request_redraw();
                continue;
            }
            #[cfg(feature = "dev")]
            if k.key == keycodes::Key::Backquote && k.pressed && !k.repeat {
                self.debug.dev_panel.visible = !self.debug.dev_panel.visible;
                self.platform.request_redraw();
                continue;
            }
            // Single exclusive router (capture / menu / in-game).
            let cmds = self.dispatch_input(keycodes::PhysicalInput::Key(k.key), k.pressed, k.repeat);
            for cmd in cmds {
                self.apply_app_command(cmd);
            }
        }
        self.sync_text_input();
        egui_events
    }

    /// Single entry point for physical inputs (keyboard outside egui, and gamepad).
    /// Picks the active context, then applies the matching resolution mode: raw capture,
    /// menu conventions or in-game bindings.
    fn dispatch_input(&mut self, physical: keycodes::PhysicalInput, pressed: bool, repeat: bool) -> Vec<AppCommand> {
        use crate::app::input_context::Context;
        match self.active_context() {
            Context::Capture { action, slot } => {
                self.handle_capture(physical, pressed, repeat, action, slot)
            }
            Context::MenuRoot
            | Context::MenuOptions
            | Context::MenuOptionsTextEdit
            | Context::MenuLan => self.route_menu_input(physical, pressed, repeat),
            Context::PauseMenu => self.handle_pause_menu(physical, pressed, repeat),
            Context::Gameplay | Context::InGameUi => {
                let app_key = self.key_bindings.get(physical);
                self.route_app_key_keyboard(physical, app_key, pressed, repeat)
            }
        }
    }

    /// Pause menu: keyboard and gamepad navigation goes through `pause_menu_step` (pure
    /// logic). Updates the selection and emits the resulting command (resume / ready / quit).
    fn handle_pause_menu(&mut self, physical: keycodes::PhysicalInput, pressed: bool, repeat: bool) -> Vec<AppCommand> {
        use crate::app::ui_intent::{UiContext, UiIntent};
        use crate::renderer::ui::pause_modal::{
            in_protection, pause_menu_step, PauseMenuInput, PauseOutcome,
        };
        if !pressed || repeat {
            return vec![];
        }
        // Abandon confirmation open (Quit during a round): shared routing, takes priority
        // over pause menu navigation.
        if let Some(cmds) = self.route_abandon_confirm(physical) {
            return cmds;
        }
        // Intent: the configured pause key (binding) resumes; otherwise the navigation convention applies.
        let intent = if self.key_bindings.get(physical) == Some(AppKey::Pause) {
            UiIntent::Pause
        } else {
            match ui_conventions::resolve_ui_intent(physical, UiContext::Navigation) {
                Some(i) => i,
                None => return vec![],
            }
        };
        // Input state derived from the session (matches `PauseModalContext` in `frame_submit`).
        let (is_network, local_ready) = self.runtime.session.game.as_ref()
            .map(|s| {
                use crackattack_core::frame_data::SessionInterruption as SI;
                match s.snapshot().interruption {
                    SI::Paused { local_ready, is_network, .. } => (is_network, local_ready),
                    SI::ConnectionLost { .. } | SI::Disconnected => (true, false),
                    _ => (false, false),
                }
            })
            .unwrap_or((false, false));
        let seconds_open = self.runtime.session.pause_menu_opened_at
            .map_or(10.0, |t| t.elapsed().as_secs_f32());
        let needs_delay = is_network && !self.runtime.session.pause_initiated_locally;
        let input = PauseMenuInput {
            is_network,
            local_ready,
            in_protection: in_protection(needs_delay, seconds_open),
        };
        let sel = self.runtime.session.pause_menu_selection.get();
        let (new_sel, outcome) = pause_menu_step(&input, sel, intent);
        self.runtime.session.pause_menu_selection.set(new_sel);
        let mut cmds = vec![];
        match outcome {
            PauseOutcome::Resume => cmds.push(AppCommand::ResumeGame),
            PauseOutcome::ResumeReady => cmds.push(AppCommand::SignalResumeReady),
            // Quit: resolved by the runtime. A committed round asks for confirmation ("Abandon
            // the game?", drawn over the pause menu). Otherwise (pause opened at the ready prompt
            // or after a round) it exits directly.
            PauseOutcome::Quit => cmds.push(AppCommand::RequestQuitGame),
            PauseOutcome::StillOpen => {}
        }
        if new_sel != sel {
            cmds.push(AppCommand::PlayUiNavigate);
        }
        if new_sel != sel || outcome != PauseOutcome::StillOpen {
            cmds.push(AppCommand::RequestRedraw);
        }
        cmds
    }

    /// Capture mode: the next press assigns the physical input to the armed binding.
    /// Escape cancels (keyboard).
    fn handle_capture(
        &mut self,
        physical: keycodes::PhysicalInput,
        pressed: bool,
        repeat: bool,
        action: crate::platform::persist::BindingAction,
        slot: usize,
    ) -> Vec<AppCommand> {
        if !pressed || repeat {
            return vec![];
        }
        if let Some(modal) = self.runtime.session.menu_state.options_modal.as_mut() {
            if physical == keycodes::PhysicalInput::Key(keycodes::Key::Escape) {
                modal.capture_binding = None; // keyboard cancel
            } else if !keycodes::input_matches_options_tab(modal.tab, physical) {
                // Wrong input type for the tab (a key in the Gamepad tab, or a button in the
                // Keyboard tab): ignored, and the capture stays armed until the right type arrives.
            } else if let Some(name) = keycodes::physical_input_to_name(physical) {
                modal.key_bindings.assign(action, slot, name.to_string());
                modal.capture_binding = None;
                // Warn when `assign` (last wins) took the key from another action and left it
                // without any key, which would make it unplayable.
                if let Some(orphan) = modal.key_bindings.validate() {
                    log::warn!(
                        "Rebinding left action {:?} without any key — use Reset if a control becomes unusable",
                        orphan
                    );
                }
            }
        }
        vec![AppCommand::RequestRedraw]
    }

    /// Route a physical input with the keyboard repeat flag.
    /// `app_key` is the bound gameplay action (layer 1) if the input is bound, otherwise
    /// `None`: then only the UI conventions layer can resolve it.
    fn route_app_key_keyboard(&self, physical: keycodes::PhysicalInput, app_key: Option<AppKey>, pressed: bool, repeat: bool) -> Vec<AppCommand> {
        use crate::app::input_handler::*;
        let ctx = InputContext {
            transition_cooldown: self.runtime.session.transition_cooldown,
            can_go_back: self.runtime.session.menu_state.depth() > 0,
            menu_confirm_action: self.runtime.session.menu_state.peek_confirm_action(),
            game_seed: self.runtime.session.game_seed,
            ranking_entry_score: self.runtime.session.user_config.ranking_entry_score(),
        };

        match &self.runtime.session.flow_state {
            // `FlowState::Menu` is routed earlier by `route_menu_input`, so this arm is not
            // reached through the `AppKey` path.
            FlowState::Menu => vec![],
            FlowState::Playing | FlowState::PostGame => {
                // Post-round screen: dedicated sub-state (keyboard and gamepad unified), handled
                // before the generic policy derivation, which is too coarse for `ReadyPrompt`.
                // See `route_post_round`.
                if self.post_round_screen_active() {
                    return self.route_post_round(physical, app_key, pressed, repeat);
                }
                // Ready phase (the between-round stats are handled above): two navigable buttons
                // (Ready / Quit), same for keyboard and gamepad.
                if self.is_ready_prompt() {
                    return self.route_ready_prompt(physical, pressed, repeat);
                }
                if repeat { return vec![]; }
                // Derive input policy from session snapshot.
                let post_game_screen = if matches!(self.runtime.session.flow_state, FlowState::PostGame) {
                    Some(self.runtime.session.post_game_screen)
                } else {
                    None
                };
                let derived = self.runtime.session.game.as_ref().map(|s| {
                    let snap = s.snapshot();
                    let policy = crate::app::presentation::derive_input(
                        &snap, self.runtime.session.pause_menu_open,
                        post_game_screen,
                    );
                    (policy, snap.lifecycle)
                });
                if let Some((policy, lifecycle)) = derived {
                    use crate::app::presentation::{ConfirmAction, EscapeAction};
                    use crate::app::ui_intent::{UiContext, UiDir, UiIntent};
                    use crackattack_core::frame_data::SessionLifecycle;

                    if policy.gameplay {
                        // === GAMEPLAY context (layer 1, bindings) ===
                        // Pause = fixed conventions {Start, Escape} or the configurable keyboard binding
                        // (`app_key == Pause`, default P). B (PadEast) never pauses. Everything else goes
                        // to the gameplay bindings.
                        if pressed
                            && (ui_conventions::resolve_ui_intent(physical, UiContext::Gameplay)
                                == Some(UiIntent::Pause)
                                || app_key == Some(AppKey::Pause))
                        {
                            vec![AppCommand::OpenPauseMenu]
                        } else {
                            app_key.map_or_else(Vec::new, |ak| handle_playing_input(&ctx, ak, pressed))
                        }
                    } else if pressed {
                        // === UI sub-state (pause menu / ready / post-game) ===
                        // Resolved by CONVENTION on the physical input (layer 2), not by binding:
                        // Confirm = A/Enter/Space, Cancel = B/Escape, Pause = Start/P,
                        // Navigate = arrows/D-pad/stick.
                        let nav = ui_conventions::resolve_ui_intent(physical, UiContext::Navigation);
                        let mut cmds = vec![];

                        // Navigable sub-state (post-game: escape = TransitionMenu, not pause): Left/Right
                        // set the focus on the footer buttons (Menu / Next). Up/Down scroll the list.
                        let navigable_substate = !matches!(policy.escape_action, EscapeAction::OpenPauseMenu);

                        // Post-game footer (Menu / Next buttons, outside dialogs), focus model: Left/Right
                        // trigger nothing, they only move the focus to a button, and Enter/A activates.
                        // Without focus, Enter/A runs the default action (Next). With focus, the focused
                        // button decides (Menu: go to the menu, Next: default action).
                        // The focus ring has 3 positions: [Menu (left) <-> Content <-> Next (right)].
                        // `None` means the content is focused (scrollable). Left/Right move along the ring
                        // and stop at the ends.
                        use crate::app::post_game::PostGameNavFocus;
                        let nav_focus = self.runtime.session.post_game_input.nav_focus;
                        if navigable_substate {
                            let ring = [PostGameNavFocus::Menu, PostGameNavFocus::None, PostGameNavFocus::Next];
                            let step = match nav {
                                Some(UiIntent::Navigate(UiDir::Left)) => -1,
                                Some(UiIntent::Navigate(UiDir::Right)) => 1,
                                _ => 0,
                            };
                            if step != 0 {
                                let new_focus = crate::app::post_game::nav_ring_step(&ring, nav_focus, step);
                                if new_focus != nav_focus {
                                    cmds.push(AppCommand::SetPostGameNavFocus(new_focus));
                                    cmds.push(AppCommand::PlayUiNavigate);
                                }
                            }
                        }

                        // Confirm: activates the focused button, otherwise runs the default action.
                        // Ignored while `transition_cooldown > 0` (set on entering post-game and between
                        // screens), so that a key mashed at the end of a game (for example Space = swap)
                        // does not skip the stats screen.
                        if nav == Some(UiIntent::Confirm) && ctx.transition_cooldown <= 0 {
                            if navigable_substate && nav_focus == PostGameNavFocus::Menu {
                                // "Menu" button focused: escape to the menu.
                                if let Some(cmd) =
                                    resolve_escape_action(&policy.escape_action, AppKey::Cancel)
                                {
                                    cmds.push(AppCommand::PlayUiConfirm);
                                    cmds.push(cmd);
                                }
                            } else {
                                match &policy.confirm_action {
                                    Some(ConfirmAction::SignalReady) => {
                                        // Never reached: every `ReadyPrompt` is intercepted earlier (`route_post_round` /
                                        // `route_ready_prompt`). Ready is deliberately not signaled here, because consent
                                        // must go through the dedicated sub-states.
                                    }
                                    Some(ConfirmAction::EnterPostGame) => {
                                        cmds.push(AppCommand::EnterPostGame);
                                    }
                                    Some(ConfirmAction::AdvancePostGameScreen) => {
                                        cmds.push(AppCommand::PlayUiConfirm);
                                        cmds.push(AppCommand::AdvancePostGameScreen);
                                        cmds.push(AppCommand::SetTransitionCooldown(15));
                                    }
                                    Some(ConfirmAction::ReturnToMenu) => {
                                        cmds.push(AppCommand::PlayUiConfirm);
                                        cmds.push(AppCommand::StartTransition(FlowState::Menu, 0.5));
                                    }
                                    None => {}
                                }
                            }
                        }

                        // Pause or go back, according to the sub-state's `escape_action`.
                        if matches!(policy.escape_action, EscapeAction::OpenPauseMenu) {
                            // "Pausable" prompt (ready / waiting / countdown): pauses with the fixed convention
                            // {Start, Escape} or the keyboard binding (default P). B does not pause.
                            if ui_conventions::resolve_ui_intent(physical, UiContext::Gameplay)
                                == Some(UiIntent::Pause)
                                || app_key == Some(AppKey::Pause)
                            {
                                cmds.push(AppCommand::OpenPauseMenu);
                            }
                        } else {
                            // Navigable sub-state: Cancel (B/Escape) and Pause (Start/P) trigger the escape
                            // action (resume / back to menu). `resolve_escape_action` is reused through the
                            // equivalent logical key. TransitionMenu only accepts Cancel, so Pause does not
                            // go back there.
                            let escape_key = match nav {
                                Some(UiIntent::Cancel) => Some(AppKey::Cancel),
                                Some(UiIntent::Pause) => Some(AppKey::Pause),
                                _ => None,
                            };
                            if let Some(k) = escape_key {
                                if let Some(cmd) = resolve_escape_action(&policy.escape_action, k) {
                                    cmds.push(cmd);
                                }
                            }
                        }

                        // Content scroll (Up/Down), only when the content has the focus (ring = None):
                        // sets the held direction. The post-game view applies one delta per frame while it
                        // is held, and the release clears it (see the "key released" branch). Nothing
                        // happens when a side button is focused.
                        if navigable_substate
                            && nav_focus == PostGameNavFocus::None
                            && matches!(
                                nav,
                                Some(UiIntent::Navigate(UiDir::Up)) | Some(UiIntent::Navigate(UiDir::Down))
                            )
                        {
                            let up = nav == Some(UiIntent::Navigate(UiDir::Up));
                            cmds.push(AppCommand::SetPostGameScrollDir(if up { -1 } else { 1 }));
                            cmds.push(AppCommand::RequestRedraw);
                        }
                        // Input carry-over during the countdown: the simulation is frozen (the session
                        // returns `frame_data()` without `step(input)`), so recording the press has no
                        // visible effect during the countdown. But the held state is already present at the
                        // first `Playing` tick, so the press (for example raising the grid) takes effect
                        // when the countdown ends, without releasing and pressing again. The release is
                        // forwarded in the "key released" branch below.
                        if lifecycle == SessionLifecycle::Countdown {
                            if let Some(ak) = app_key {
                                cmds.extend(handle_playing_input(&ctx, ak, pressed));
                            }
                        }
                        cmds
                    } else {
                        // === UI sub-state: key released ===
                        let nav = ui_conventions::resolve_ui_intent(physical, UiContext::Navigation);
                        let mut cmds = Vec::new();
                        // Releasing Up/Down stops the held scroll of the post-game content.
                        if matches!(
                            nav,
                            Some(UiIntent::Navigate(UiDir::Up)) | Some(UiIntent::Navigate(UiDir::Down))
                        ) {
                            cmds.push(AppCommand::SetPostGameScrollDir(0));
                        }
                        if let Some(ak) = app_key {
                            cmds.extend(handle_playing_input(&ctx, ak, pressed));
                        }
                        cmds
                    }
                } else {
                    app_key.map_or_else(Vec::new, |ak| handle_playing_input(&ctx, ak, pressed))
                }
            }
        }
    }

    /// Keyboard and gamepad navigation of the LAN screen (peer list, manual screen,
    /// accept/decline), consuming `UiIntent`s (resolved by convention, see
    /// `ui_conventions`). Touch and mouse go through egui clicks and mutate the same
    /// session state. One exception: on the manual screen with the on-screen keyboard
    /// open, physical keys go to egui (`keyboard_goes_to_egui`) to produce text, while
    /// the gamepad still arrives here (`ManualKb*`).
    fn route_lan_intent(&self, intent: crate::app::ui_intent::UiIntent) -> Vec<AppCommand> {
        use crate::app::meta_state::LanScreenPhase;
        use crate::app::ui_intent::{UiDir, UiIntent};
        use crackattack_net::discovery::PeerStatus;
        let lan = match self.runtime.session.menu_state.lan_screen.as_ref() {
            Some(l) => l,
            None => return vec![],
        };
        // Manual IP dialog open: focus lives in the session state (`lan.manual`), and
        // keyboard and gamepad navigation are unified here. IP entry with the gamepad goes
        // through the on-screen keyboard.
        if lan.manual_open {
            // On-screen keyboard open: it captures all navigation (same model as the options,
            // see `route_options_intent`). Physical keys go to egui (`keyboard_goes_to_egui`)
            // and `render_osk_grid` consumes them.
            if lan.manual.editing.is_some() {
                return match intent {
                    UiIntent::Navigate(UiDir::Up) => vec![AppCommand::ManualKbMove(0, -1)],
                    UiIntent::Navigate(UiDir::Down) => vec![AppCommand::ManualKbMove(0, 1)],
                    UiIntent::Navigate(UiDir::Left) => vec![AppCommand::ManualKbMove(-1, 0)],
                    UiIntent::Navigate(UiDir::Right) => vec![AppCommand::ManualKbMove(1, 0)],
                    UiIntent::Confirm => vec![AppCommand::ManualKbActivate, AppCommand::PlayUiConfirm],
                    UiIntent::Cancel => vec![AppCommand::ManualKbClose],
                    _ => vec![],
                };
            }
            return match intent {
                // Vertical ring (wraps). All four arrows navigate (Left = Up, Right = Down), as on
                // the auto screen; no element adjusts a value.
                UiIntent::Navigate(UiDir::Up) | UiIntent::Navigate(UiDir::Left) =>
                    vec![AppCommand::ManualNavMove(-1)],
                UiIntent::Navigate(UiDir::Down) | UiIntent::Navigate(UiDir::Right) =>
                    vec![AppCommand::ManualNavMove(1)],
                UiIntent::Confirm => vec![AppCommand::ManualNavActivate, AppCommand::PlayUiConfirm],
                UiIntent::Cancel => vec![AppCommand::CloseLanScreen],
                _ => vec![],
            };
        }
        match &lan.phase {
            LanScreenPhase::Scanning | LanScreenPhase::Error { .. } => match intent {
                // Linear navigation: the four arrows move to previous/next (Left = Up, Right = Down).
                // This makes the right-hand footer button reachable.
                UiIntent::Navigate(UiDir::Up) | UiIntent::Navigate(UiDir::Left) =>
                    vec![AppCommand::LanSelectMove(-1)],
                UiIntent::Navigate(UiDir::Down) | UiIntent::Navigate(UiDir::Right) =>
                    vec![AppCommand::LanSelectMove(1)],
                UiIntent::Confirm => {
                    // Navigable footer: Back (to the menu).
                    if lan.selected == lan.retour_index() {
                        vec![AppCommand::CloseLanScreen]
                    } else if let LanScreenPhase::Error { hint, .. } = &lan.phase {
                        // Error ring: an optional [View download] button, then Back. Never a peer (the list
                        // is not shown on error).
                        match hint {
                            Some(crate::app::meta_state::LanErrorHint::Download(url)) =>
                                vec![AppCommand::PlayUiConfirm, AppCommand::OpenUrl(url.clone())],
                            _ => vec![],
                        }
                    } else {
                        match lan.peers.get(lan.selected) {
                            Some(peer) if peer.status == PeerStatus::Available && peer.compatible => vec![
                                AppCommand::PlayUiConfirm,
                                AppCommand::ConnectToPeer {
                                    addr: peer.addr,
                                    port: peer.port,
                                },
                            ],
                            _ => vec![],
                        }
                    }
                }
                UiIntent::Cancel => vec![AppCommand::CloseLanScreen],
                _ => vec![],
            },
            LanScreenPhase::Inviting { .. } => match intent {
                // Cancel the sent invite: back to the list (the screen stays open).
                UiIntent::Cancel => vec![AppCommand::CancelInvite],
                _ => vec![],
            },
            // Reconnecting screen: [Manual connect] / [Back] are navigable; Escape = Back (stops
            // the attempts).
            LanScreenPhase::Reconnecting { .. } => match intent {
                UiIntent::Navigate(UiDir::Up) | UiIntent::Navigate(UiDir::Left) =>
                    vec![AppCommand::LanSelectMove(-1)],
                UiIntent::Navigate(UiDir::Down) | UiIntent::Navigate(UiDir::Right) =>
                    vec![AppCommand::LanSelectMove(1)],
                UiIntent::Confirm => if lan.selected == 0 {
                    vec![AppCommand::PlayUiConfirm, AppCommand::ReconnectToManual]
                } else {
                    vec![AppCommand::PlayUiConfirm, AppCommand::CancelInvite]
                },
                UiIntent::Cancel => vec![AppCommand::CancelInvite],
                _ => vec![],
            },
        }
    }

    /// Keyboard and gamepad routing of the **invite overlay** (session level):
    /// Left/Right toggle Accept (0) / Decline (1); Confirm validates the focus; Cancel
    /// declines. During the connection handshake the overlay takes no input.
    fn route_invite_intent(&self, intent: crate::app::ui_intent::UiIntent) -> Vec<AppCommand> {
        use crate::app::ui_intent::{UiDir, UiIntent};
        let Some(ref info) = self.runtime.session.negotiation.pending_invite_info else { return vec![] };
        if info.connecting {
            return vec![];
        }
        match intent {
            // Sound only on movement: nothing if the choice is already selected.
            UiIntent::Navigate(UiDir::Left) | UiIntent::Navigate(UiDir::Up) => {
                if info.selection.replace(0) == 0 { vec![] } else { vec![AppCommand::PlayUiNavigate] }
            }
            UiIntent::Navigate(UiDir::Right) | UiIntent::Navigate(UiDir::Down) => {
                if info.selection.replace(1) == 1 { vec![] } else { vec![AppCommand::PlayUiNavigate] }
            }
            UiIntent::Confirm => if info.selection.get() == 0 {
                vec![AppCommand::PlayUiConfirm, AppCommand::AcceptInvite]
            } else {
                vec![AppCommand::DeclineInvite]
            },
            UiIntent::Cancel => vec![AppCommand::DeclineInvite],
            _ => vec![],
        }
    }

    /// Gamepad navigation of the options modal (Player/Sound tabs and on-screen
    /// keyboard), consuming `UiIntent`s. The physical keyboard is read in
    /// `options_modal::render_options_modal` (egui path) and mutates the same state.
    /// Binding capture (Controls tab) is intercepted earlier.
    fn route_options_intent(&self, intent: crate::app::ui_intent::UiIntent) -> Vec<AppCommand> {
        use crate::app::ui_intent::{UiDir, UiIntent};
        let modal = match self.runtime.session.menu_state.options_modal.as_ref() {
            Some(m) => m,
            None => return vec![],
        };
        // On-screen keyboard open: it captures all navigation.
        if modal.keyboard_open {
            return match intent {
                UiIntent::Navigate(UiDir::Up) => vec![AppCommand::OptionsKbMove(0, -1)],
                UiIntent::Navigate(UiDir::Down) => vec![AppCommand::OptionsKbMove(0, 1)],
                UiIntent::Navigate(UiDir::Left) => vec![AppCommand::OptionsKbMove(-1, 0)],
                UiIntent::Navigate(UiDir::Right) => vec![AppCommand::OptionsKbMove(1, 0)],
                UiIntent::Confirm => vec![AppCommand::OptionsKbActivate, AppCommand::PlayUiConfirm],
                UiIntent::Cancel => vec![AppCommand::OptionsKbClose],
                _ => vec![],
            };
        }
        // Tabs without a row layout (for example Controls): only closing (B) is allowed.
        // Row navigation applies as soon as a layout exists (Player, Sound).
        if crate::app::options_nav::layout(modal.tab).is_empty() {
            return match intent {
                UiIntent::Cancel => vec![AppCommand::CloseOptionsModal],
                _ => vec![],
            };
        }
        // Row navigation: Up/Down change row; Left/Right adjust a value widget or move
        // within a row of buttons.
        match intent {
            UiIntent::Navigate(UiDir::Up) => vec![AppCommand::OptionsNavMove(0, -1)],
            UiIntent::Navigate(UiDir::Down) => vec![AppCommand::OptionsNavMove(0, 1)],
            UiIntent::Navigate(UiDir::Left) => vec![AppCommand::OptionsNavMove(-1, 0)],
            UiIntent::Navigate(UiDir::Right) => vec![AppCommand::OptionsNavMove(1, 0)],
            UiIntent::Confirm => vec![AppCommand::OptionsNavActivate, AppCommand::PlayUiConfirm],
            UiIntent::Cancel => vec![AppCommand::CloseOptionsModal],
            _ => vec![],
        }
    }

    /// Navigation of the **Info page** (Rules / Credits), keyboard and gamepad through
    /// `UiIntent`. Policy: scrollable content plus a footer. Left/Right move the focus
    /// between the content and the Back button (a ring of two with silent stops at the
    /// ends: no command and no sound there, like the post-game ring). Up/Down scroll when
    /// the content is focused. Confirm (on the button) or Cancel go back.
    /// Opponents page: Up/Down move the selection among the opponent rows (rendering
    /// scrolls to keep it visible). Confirm (on the content) quickly reconnects to the
    /// selected row (no-op without a stored address, checked by the runtime).
    fn route_info_intent(&self, intent: crate::app::ui_intent::UiIntent) -> Vec<AppCommand> {
        use crate::app::menu_tree::InfoSection;
        use crate::app::ui_intent::{UiDir, UiIntent};
        const STEP: f32 = 90.0; // scroll per key press (px)
        let back = self.runtime.session.menu_state.info_focus_back;
        let rank_page = self.runtime.session.menu_state.info_section
            == Some(InfoSection::NetworkOpponents);
        // Page with actions (Credits, Updates): Up/Down walk through [text, actions...] with
        // stops at the ends. Up on the text scrolls it. Confirm activates the selected action.
        let info_actions = self.runtime.session.info_actions();
        let action_sel = self.runtime.session.menu_state.info_action_selected;
        let actions_page = !info_actions.is_empty();
        match intent {
            UiIntent::Cancel => vec![AppCommand::CloseInfoScreen, AppCommand::PlayUiConfirm],
            UiIntent::Confirm if back => vec![AppCommand::CloseInfoScreen, AppCommand::PlayUiConfirm],
            UiIntent::Confirm if actions_page && action_sel > 0 => info_actions
                .get(action_sel - 1)
                .map(|a| vec![a.command(), AppCommand::PlayUiConfirm])
                .unwrap_or_default(),
            UiIntent::Navigate(UiDir::Up) if !back && actions_page && action_sel == 0 =>
                vec![AppCommand::ScrollInfo(-STEP)],
            UiIntent::Navigate(UiDir::Up) if !back && actions_page =>
                vec![AppCommand::InfoActionSelectMove(-1)],
            UiIntent::Navigate(UiDir::Down) if !back && actions_page =>
                vec![AppCommand::InfoActionSelectMove(1)],
            UiIntent::Confirm if rank_page => {
                vec![AppCommand::ReconnectOpponent(self.runtime.session.menu_state.rank_selected)]
            }
            UiIntent::Navigate(UiDir::Left) if !back =>
                vec![AppCommand::SetInfoFocusBack(true), AppCommand::PlayUiNavigate],
            UiIntent::Navigate(UiDir::Right) if back =>
                vec![AppCommand::SetInfoFocusBack(false), AppCommand::PlayUiNavigate],
            UiIntent::Navigate(UiDir::Up) if !back && rank_page =>
                vec![AppCommand::RankSelectMove(-1)],
            UiIntent::Navigate(UiDir::Down) if !back && rank_page =>
                vec![AppCommand::RankSelectMove(1)],
            UiIntent::Navigate(UiDir::Up) if !back => vec![AppCommand::ScrollInfo(-STEP)],
            UiIntent::Navigate(UiDir::Down) if !back => vec![AppCommand::ScrollInfo(STEP)],
            _ => vec![],
        }
    }

    /// Is the **post-round summary** active (result, stats and tally, curtain closed)?
    /// The summary only reports and commits nothing: "Next" (default) opens the curtain
    /// and lets the "Ready for a new round?" modal ask the question. The router
    /// (`route_post_round`) handles it for keyboard and gamepad, hence the branch in
    /// `keyboard_goes_to_egui` and the interception in `route_app_key_keyboard`.
    /// Mirrors the `show_stats` condition of `frame_submit`.
    fn post_round_screen_active(&self) -> bool {
        use crackattack_core::frame_data::SessionInterruption;
        let s = match self.runtime.session.game.as_ref() {
            Some(s) => s,
            None => return false,
        };
        matches!(
            s.snapshot().interruption,
            SessionInterruption::ReadyPrompt { round_number, .. } if round_number > 1
        ) && self.runtime.session.round_end_stats.is_some()
            && !self.runtime.session.post_game_input.post_round_advanced
    }

    /// Is the post-round offer **contracted** ([Menu] only)? True when the opponent has
    /// left: no message, the screen only offers what is still possible.
    fn post_round_offer_contracted(&self) -> bool {
        use crackattack_core::frame_data::SessionInterruption;
        self.runtime.session.game.as_ref().is_some_and(|s| matches!(
            s.snapshot().interruption,
            SessionInterruption::ReadyPrompt { opponent_present: false, .. }
        ))
    }

    /// Is the current interruption a `ReadyPrompt` (ready phase or post-round screen)?
    /// The router handles both (keyboard and gamepad), so their keyboard does not go to
    /// egui (see `keyboard_goes_to_egui`). Superset of `post_round_screen_active`.
    fn is_ready_prompt(&self) -> bool {
        use crackattack_core::frame_data::SessionInterruption;
        self.runtime.session.game.as_ref().is_some_and(|s| {
            matches!(s.snapshot().interruption, SessionInterruption::ReadyPrompt { .. })
        })
    }

    /// Ready-phase sub-state ("Ready?", curtain open): two navigable buttons, **Ready /
    /// Quit**, keyboard and gamepad unified (same as the pause menu). Up/Down toggle the
    /// selection (wraps). Confirm activates the focused button: **Ready** sends
    /// `SignalLocalReady`; **Quit** sends `RequestQuitGame`, resolved directly because
    /// nothing is committed at the prompt (the confirmation only exists for leaving
    /// during a round). Pause/Escape open the pause menu (a "suspend" gesture, distinct
    /// from quitting). Rendering only draws and handles mouse clicks. Confirm is ignored
    /// while `transition_cooldown > 0`.
    fn route_ready_prompt(&self, physical: keycodes::PhysicalInput, pressed: bool, repeat: bool) -> Vec<AppCommand> {
        use crate::app::ui_intent::{UiContext, UiDir, UiIntent};
        if !pressed || repeat {
            return vec![];
        }
        // Contracted offer (the opponent left during the question): the modal only offers
        // "Quit". Any confirmation exits; navigation does nothing.
        let contracted = self.post_round_offer_contracted();
        let nav = ui_conventions::resolve_ui_intent(physical, UiContext::Navigation);
        let sel = self.runtime.session.ready_selection.get();
        match nav {
            Some(UiIntent::Navigate(UiDir::Up)) | Some(UiIntent::Navigate(UiDir::Down)) => {
                if contracted {
                    return vec![];
                }
                self.runtime.session.ready_selection.set((sel + 1) % 2);
                vec![AppCommand::PlayUiNavigate]
            }
            Some(UiIntent::Confirm) => {
                if self.runtime.session.transition_cooldown > 0 {
                    vec![]
                } else if contracted {
                    vec![AppCommand::PlayUiConfirm, AppCommand::RequestQuitGame]
                } else if sel == 0 {
                    vec![AppCommand::SignalLocalReady]
                } else {
                    vec![AppCommand::PlayUiConfirm, AppCommand::RequestQuitGame]
                }
            }
            // Pause/Escape open the pause menu, which is distinct from the explicit "Quit"
            // button.
            Some(UiIntent::Pause) | Some(UiIntent::Cancel) => vec![AppCommand::OpenPauseMenu],
            _ => vec![],
        }
    }

    /// Keyboard and gamepad routing of the **abandon confirmation modal** ("Abandon the
    /// game?"). It only exists for leaving a committed round (Quit from the pause menu
    /// during a round); leaving from "Ready?" or after a round is direct. Returns
    /// `Some(cmds)` when the confirmation is open (input consumed), `None` otherwise.
    /// Left = Cancel, Right = Abandon (focus); Confirm validates the focused button;
    /// Cancel cancels (safe default).
    fn route_abandon_confirm(&self, physical: keycodes::PhysicalInput) -> Option<Vec<AppCommand>> {
        use crate::app::ui_intent::{UiContext, UiDir, UiIntent};
        if !self.runtime.session.post_game_input.abandon_confirm_open {
            return None;
        }
        let nav = ui_conventions::resolve_ui_intent(physical, UiContext::Navigation);
        let yes = self.runtime.session.post_game_input.abandon_confirm_yes;
        Some(match nav {
            // Sound only on movement: nothing if the button is already focused.
            Some(UiIntent::Navigate(UiDir::Left)) if yes =>
                vec![AppCommand::SetAbandonConfirm(Some(false)), AppCommand::PlayUiNavigate],
            Some(UiIntent::Navigate(UiDir::Right)) if !yes =>
                vec![AppCommand::SetAbandonConfirm(Some(true)), AppCommand::PlayUiNavigate],
            Some(UiIntent::Confirm) => if yes {
                vec![AppCommand::PlayUiConfirm, AppCommand::QuitToMenu]
            } else {
                vec![AppCommand::SetAbandonConfirm(None)]
            },
            Some(UiIntent::Cancel) => vec![AppCommand::SetAbandonConfirm(None)],
            _ => vec![],
        })
    }

    /// Post-round summary sub-state (keyboard and gamepad through the router): focus
    /// ring [Menu <-> Content <-> Next] and **held** scrolling of the content. Menu quits
    /// directly (the round is finished and scored, nothing to confirm). Next (default)
    /// closes the summary and lets the "Ready?" modal ask the question (nothing is
    /// committed here). With a **contracted** offer (opponent left) the ring shrinks to
    /// [Menu <-> Content] and Confirm = Menu. `frame_submit` only draws and handles mouse
    /// clicks. Pause (Start/P) stays available.
    fn route_post_round(&self, physical: keycodes::PhysicalInput, app_key: Option<AppKey>, pressed: bool, repeat: bool) -> Vec<AppCommand> {
        use crate::app::ui_intent::{UiContext, UiDir, UiIntent};
        use crate::app::post_game::PostGameNavFocus;
        let nav = ui_conventions::resolve_ui_intent(physical, UiContext::Navigation);
        // Release: the only thing to do is stop the held scroll of the content.
        if !pressed {
            if matches!(nav, Some(UiIntent::Navigate(UiDir::Up)) | Some(UiIntent::Navigate(UiDir::Down))) {
                return vec![AppCommand::SetPostGameScrollDir(0)];
            }
            return vec![];
        }
        // The ring acts on press; scrolling uses press and release (the direction stays set
        // during repeats and is cleared on release).
        if repeat {
            return vec![];
        }
        let contracted = self.post_round_offer_contracted();
        let nav_focus = self.runtime.session.post_game_input.nav_focus;
        // Pause: fixed convention (Start/P, via `UiIntent::Pause`) OR a keyboard binding
        // **rebound** to Pause (`app_key`). This route bypasses the generic logic of
        // `route_app_key_keyboard`, so the test is repeated here; otherwise a custom Pause
        // key would not pause on this screen.
        if nav == Some(UiIntent::Pause) || app_key == Some(AppKey::Pause) {
            return vec![AppCommand::OpenPauseMenu];
        }
        match nav {
            Some(UiIntent::Navigate(dir @ (UiDir::Left | UiDir::Right))) => {
                // Contracted offer: no right-hand button ("Next").
                let ring: &[PostGameNavFocus] = if contracted {
                    &[PostGameNavFocus::Menu, PostGameNavFocus::None]
                } else {
                    &[PostGameNavFocus::Menu, PostGameNavFocus::None, PostGameNavFocus::Next]
                };
                let step = if dir == UiDir::Left { -1 } else { 1 };
                let new_focus = crate::app::post_game::nav_ring_step(ring, nav_focus, step);
                if new_focus != nav_focus {
                    vec![AppCommand::SetPostGameNavFocus(new_focus), AppCommand::PlayUiNavigate]
                } else {
                    vec![]
                }
            }
            Some(UiIntent::Navigate(dir @ (UiDir::Up | UiDir::Down))) if nav_focus == PostGameNavFocus::None => {
                let up = dir == UiDir::Up;
                vec![AppCommand::SetPostGameScrollDir(if up { -1 } else { 1 }), AppCommand::RequestRedraw]
            }
            Some(UiIntent::Confirm) => {
                // The summary commits NOTHING: "Next" (default without focus) only opens the
                // curtain. The question ("Ready for a new round?") is asked afterwards by the
                // modal. Contracted offer: Enter = Menu (the only way out).
                if self.runtime.session.transition_cooldown > 0 {
                    vec![]
                } else if nav_focus == PostGameNavFocus::Menu || contracted {
                    vec![AppCommand::PlayUiConfirm, AppCommand::RequestQuitGame]
                } else {
                    // Content or "Next": close the summary (safe, commits nothing).
                    vec![AppCommand::PlayUiConfirm, AppCommand::AdvanceToReadyPrompt]
                }
            }
            _ => vec![],
        }
    }

    /// Unified **menu** router (`FlowState::Menu`): resolves the physical input to a
    /// `UiIntent` by **convention** (`ui_conventions`), then dispatches to the active
    /// overlay (LAN / options) or the main menu. Keyboard and gamepad both come here in
    /// the menu; gameplay (Playing/PostGame) keeps the bindings -> `AppKey` path.
    fn route_menu_input(&self, input: keycodes::PhysicalInput, pressed: bool, repeat: bool) -> Vec<AppCommand> {
        use crate::app::ui_intent::UiContext;
        if !pressed || repeat {
            return vec![]; // the menu acts on press only
        }
        let ctx = if self.text_entry_open() { UiContext::TextEdit } else { UiContext::Navigation };
        let intent = match ui_conventions::resolve_ui_intent(input, ctx) {
            Some(i) => i,
            None => return vec![],
        };
        // Invite received (session-level overlay): takes priority over any menu screen.
        if self.runtime.session.negotiation.pending_invite_info.is_some() {
            return self.route_invite_intent(intent);
        }
        if self.runtime.session.menu_state.info_section.is_some() {
            self.route_info_intent(intent)
        } else if self.runtime.session.menu_state.lan_screen.is_some() {
            self.route_lan_intent(intent)
        } else if self.runtime.session.menu_state.options_modal.is_some() {
            self.route_options_intent(intent)
        } else {
            use crate::app::input_handler::InputContext;
            let mctx = InputContext {
                transition_cooldown: self.runtime.session.transition_cooldown,
                can_go_back: self.runtime.session.menu_state.depth() > 0,
                menu_confirm_action: self.runtime.session.menu_state.peek_confirm_action(),
                game_seed: self.runtime.session.game_seed,
                ranking_entry_score: self.runtime.session.user_config.ranking_entry_score(),
            };
            crate::app::input_handler::handle_menu_input(&mctx, intent)
        }
    }

    /// Drain the ordered `PlatformShell` event stream, split it between egui and the
    /// router, and build the `RawInput`. Pixels are converted to egui points at the
    /// **effective** pixels-per-point = native x `zoom_factor`.
    /// Must run before `handle_redraw` takes the modals out: while they are taken, the
    /// effects of dispatching would be overwritten. The SDL driver also calls it alone
    /// when the window is occluded, to pump events (including Restored) without paying
    /// for rendering.
    fn drain_inputs(&mut self) {
        let mut events = self.platform.poll_events();
        // Test miniscript: inject the actions due this frame at the same seam as a real key
        // press (so the egui/router path is identical). With a fixed step, one render frame
        // is one sim tick.
        if self.script.is_some() {
            let frame = self.scheduling.frame_counter;
            let flow = self.runtime.session.flow_state;
            let effects = self.script.as_mut().unwrap().tick(frame, flow);
            for effect in effects {
                match effect {
                    script::Effect::Input(ev) => events.push(ev),
                    script::Effect::Capture(name) => {
                        // Final name = <script>-<name>.png. The prefix is the .mini file name, which avoids
                        // collisions between scripts of the same campaign.
                        let stem = self.debug.script_path.as_ref()
                            .map(|p| std::path::Path::new(p)
                                .file_stem().map(|s| s.to_string_lossy().into_owned())
                                .unwrap_or_else(|| "script".to_string()))
                            .unwrap_or_else(|| "script".to_string());
                        let filename = format!("{stem}-{name}.png");
                        self.debug.script_captures_demanded += 1;
                        // Two captures in the same frame: the first would be lost silently. This is a
                        // campaign failure, never a coalescence.
                        if let Some(crate::platform::screenshot::ScreenshotRequest::Named(f)) =
                            self.debug.screenshot_request.replace(
                                crate::platform::screenshot::ScreenshotRequest::Named(filename),
                            )
                        {
                            self.renderer.screenshot.record_failure(
                                f, "overwritten by a capture requested on the same frame".to_string());
                        }
                    }
                    script::Effect::Quit => {
                        log::info!("script: quit (frame {frame})");
                        self.debug.should_exit = true;
                    }
                    script::Effect::AssertFailed(msg) => {
                        log::error!("{msg}");
                        self.debug.script_assert_failed = true;
                        self.debug.should_exit = true;
                    }
                }
            }
        }
        // Recording: capture the input stream that drove the session this frame (the real
        // keyboard in normal play; with `--script` + `--record`, the injected events, which
        // re-exports the script in absolute form). Runs after injection, before dispatch.
        if let Some(rec) = self.recorder.as_mut() {
            let frame = self.scheduling.frame_counter;
            for ev in &events { rec.capture(frame, ev); }
        }
        let egui_events = self.dispatch_frame_inputs(events);
        let native_ppp = self.platform.scale_factor().max(0.1);
        let effective_ppp = native_ppp * self.renderer.ui.egui_ctx.zoom_factor();
        let screen_px = (self.gpu.config.width as f32, self.gpu.config.height as f32);
        let focused = self.renderer.ui.focused;
        self.renderer.ui.pending_raw_input = Some(platform_shell::ui_bridge::build_raw_input(
            &egui_events, screen_px, native_ppp, effective_ppp, focused,
        ));
    }

    /// Handle a redraw request: tick timing, update transitions, render frame.
    /// Returns Err(true) for fatal errors, Err(false) for recoverable ones.
    fn handle_redraw(&mut self) -> Result<(), bool> {
        // Drain the event stream and build the `RawInput`. Must run before the modals are taken (see `drain_inputs`).
        self.drain_inputs();

        // Render dt: real time in normal mode. In QA fixed-step mode it is constant (= tick
        // period, 20 ms) so that animations timed in seconds (menu <-> game curtains) advance
        // deterministically: 1 frame == 1 tick == 20 ms. Otherwise the 0.5 s curtain would
        // take a variable number of frames and shift the whole timeline.
        let now = Instant::now();
        let render_dt = if self.scheduling.fixed_step {
            TICK_DURATION.as_secs_f32()
        } else {
            (now - self.scheduling.last_render_instant).as_secs_f32().min(0.1)
        };
        self.scheduling.last_render_instant = now;
        self.scheduling.wall_clock_seconds += render_dt;
        self.scheduling.last_render_dt = render_dt;

        // Pre-render update: transitions
        self.update(render_dt);

        // Render frame: `cached_frame` is moved out of the session during rendering instead
        // of being cloned every frame (a clone of several Vecs at 50-60 Hz, only to split
        // the session borrow from the `&mut self` of `render()`). The render path never
        // reads `session.cached_frame`, so taking it has no effect. It is put back right
        // after, in the same take/restore window as the modals. It is only restored if it
        // was `Some`; in the `None` case (before the first tick) a temporary frame is built
        // and not stored.
        let taken_frame = self.runtime.session.cached_frame.take();
        let frame_from_cache = taken_frame.is_some();
        let frame = taken_frame.unwrap_or_else(|| match self.runtime.session.game.as_ref() {
            Some(active) => active.frame_data(),
            None => FrameData::default(),
        });

        // Take modals out before render (render needs &mut for egui input)
        // Take/restore window: `menu_state.options_modal` and `lan_screen` are EMPTY during
        // `render()`, so any mutation there would be lost on restore. Commands produced by
        // rendering MUST be deferred (`render_cmds`). A debug guard below panics on an
        // immediate `apply_app_command` inside this window.
        #[cfg(debug_assertions)]
        { self.debug.modals_taken = true; }
        let mut modal_opt = self.runtime.session.menu_state.options_modal.take();
        let mut net_modal_opt = self.runtime.session.menu_state.lan_screen.take();

        // Render
        let render_cmds = match self.render(&frame, &mut modal_opt, &mut net_modal_opt) {
            Ok(cmds) => cmds,
            // `Outdated` needs a reconfiguration just like `Lost` (wgpu: the swap chain must be
            // updated). Otherwise a surface invalidated without a matching `Resized` (wake after
            // minimizing, monitor change) stays black on every frame. `Timeout` and `Other` are
            // transient: skip and retry.
            Err(wgpu::SurfaceError::Lost | wgpu::SurfaceError::Outdated) => {
                let size = self.platform.pixel_size();
                self.resize(size);
                Vec::new()
            }
            Err(wgpu::SurfaceError::OutOfMemory) => {
                log::error!("Out of memory");
                return Err(true);
            }
            Err(e) => {
                log::warn!("Surface error: {:?}", e);
                Vec::new()
            }
        };

        // Put modals back
        self.runtime.session.menu_state.options_modal = modal_opt;
        self.runtime.session.menu_state.lan_screen = net_modal_opt;
        // Put the frame back in the session, only if it came from there (see above).
        if frame_from_cache {
            self.runtime.session.cached_frame = Some(frame);
        }
        #[cfg(debug_assertions)]
        { self.debug.modals_taken = false; }

        // Apply deferred commands
        for cmd in render_cmds {
            self.apply_app_command(cmd);
        }

        Ok(())
    }

    fn resize(&mut self, new_size: (u32, u32)) {
        let new_size = (new_size.0, new_size.1);
        if new_size.0 > 0 && new_size.1 > 0 {
            self.gpu.config.width = new_size.0;
            self.gpu.config.height = new_size.1;
            self.gpu.surface.configure(&self.gpu.device, &self.gpu.config);
            self.renderer.scene.pipelines.render_targets.resize(&self.gpu.device, (new_size.0, new_size.1));

            // Viewport = useful zone (a square in portrait, up to 16:9 in landscape). The 3D
            // scene renders into it with an aspect-aware horizontal frustum (see
            // `create_projection_matrix`), so there is no distortion and the side space is
            // available for the layout reflow.
            let (vp_x, vp_y, vp_w, vp_h) = compute_useful_zone(new_size.0, new_size.1);
            self.gpu.viewport_x = vp_x;
            self.gpu.viewport_y = vp_y;
            self.gpu.viewport_width = vp_w;
            self.gpu.viewport_height = vp_h;

            // Reflow: info panel, board and light offsets depending on the aspect (margins [outer, 2, 1, 2, outer]).
            let r = crate::renderer::init::reflow_offsets(new_size.0, new_size.1);
            self.renderer.ui.ui_renderer.reflow_info_dx = r.info_dx;
            self.renderer.ui.ui_renderer.reflow_light_dx = r.light_dx;
            self.renderer.ui.ui_renderer.reflow_edge_dx = r.edge_dx;
            crate::renderer::scene::game_renderer::set_play_reflow_dx(r.play_dx);

            // Update 3D uniforms with new aspect ratio
            let transform_uniforms = create_transform_uniforms(new_size.0, new_size.1);
            self.gpu.queue.write_buffer(&self.renderer.scene.pipelines.uniform_buffer, 0, bytemuck::bytes_of(&transform_uniforms));
            self.renderer.scene.pipelines.transform_uniforms = transform_uniforms;

            // Update UI uniforms
            let ui_uniforms = create_ui_projection(new_size.0, new_size.1);
            self.gpu.queue.write_buffer(&self.renderer.ui.ui_uniform_buffer, 0, bytemuck::bytes_of(&ui_uniforms));
        }
    }

    fn tick(&mut self) -> bool {
        // STARTUP commands (CLI shortcuts): applied once, through the same path as the
        // menu actions.
        if !self.runtime.session.pending_startup.is_empty() {
            let startup = std::mem::take(&mut self.runtime.session.pending_startup);
            for cmd in startup {
                self.apply_app_command(cmd);
            }
        }

        // How many sim ticks to run this frame?
        //  - Normal mode: 50 Hz catch-up driven by real time (accumulator).
        //  - QA fixed-step mode (`--set session:qa.fixed_step=true`): EXACTLY 1 tick per
        //    frame, independent of real time, so render frame == sim tick and runs are
        //    reproducible. `last_tick` is kept current to avoid an accumulator spike if the
        //    mode is turned off mid-run.
        let now = Instant::now();
        let ticks_to_run = if self.scheduling.fixed_step {
            self.scheduling.last_tick = now;
            self.scheduling.tick_accumulator = std::time::Duration::ZERO;
            1
        } else {
            let elapsed = now - self.scheduling.last_tick;
            self.scheduling.last_tick = now;
            self.scheduling.tick_accumulator += elapsed;
            let mut n = 0u32;
            while self.scheduling.tick_accumulator >= TICK_DURATION {
                self.scheduling.tick_accumulator -= TICK_DURATION;
                n += 1;
            }
            n
        };

        // Fixed 50 Hz game step.
        let mut tick_count = 0;
        for _ in 0..ticks_to_run {
            tick_count += 1;

            // Dev-only: freeze simulation
            let freeze = {
                #[cfg(feature = "dev")]
                { self.debug.dev_panel.tuning.freeze_simulation }
                #[cfg(not(feature = "dev"))]
                { false }
            };

            // Session logic: game step, AI, sound, post-game effects
            let tick_cmds = self.runtime.tick_session(freeze);
            for cmd in tick_cmds {
                self.apply_app_command(cmd);
            }

            // Dev-only: drop test garbage (needs debug state, stays here)
            #[cfg(feature = "dev")]
            {
                let h = self.debug.dev_panel.tuning.debug_drop_garbage_height;
                if h > 0 {
                    if let Some(ref mut session) = self.runtime.session.game {
                        if let Some(game) = session.game_state_mut() {
                            let f = self.debug.dev_panel.tuning.debug_drop_garbage_flavor;
                            game.send_garbage(h, crackattack_core::constants::PLAY_WIDTH as i32, f);
                        }
                    }
                    self.debug.dev_panel.tuning.debug_drop_garbage_height = 0;
                }
            }
        }


        // Tick transition cooldown
        if self.runtime.session.transition_cooldown > 0 {
            self.runtime.session.transition_cooldown -= 1;
        }

        // Track suspension start time (wall clock) for UI timeout.
        {
            let interruption = self.runtime.session.game.as_ref()
                .map(|s| s.snapshot().interruption)
                .unwrap_or(crackattack_core::frame_data::SessionInterruption::None);
            self.runtime.session.suspension_timer.sync(&interruption);
        }

        // Sync post-game effects and spawn particles. Must happen in
        // tick, not render, so session mutations are in the logical
        // update path (render is read-only on session state).
        if tick_count > 0 {
            let step_events = std::mem::take(&mut self.runtime.step_events);
            if let Some(ref game) = self.runtime.session.game {
                // `frame_data()` carries no events (edges): put back those of this frame's steps,
                // for the chain rings.
                let mut frame = game.frame_data();
                frame.gameplay_events = step_events;
                // Post-game sync: session mutations in SessionRuntime,
                // renderer side-effects via returned AppCommand.
                let cmds = self.runtime.sync_post_game(&frame);
                for cmd in cmds { self.apply_app_command(cmd); }
                self.renderer.spawn_nucleons_and_chain_rings(&frame);
                self.renderer.shake_logo_on_garbage_landing(&frame);
            }
        }

        // Build cached frame snapshot (consumed by render)
        if tick_count > 0 {
            let mut frame = match self.runtime.session.game.as_ref() {
                Some(active) => active.frame_data(),
                None => {
                    // No active session (menu screen) — build a minimal frame.
                    let ticks = self.runtime.session.menu_state.ticks;
                    FrameData {
                        time_step: ticks,
                        ui: crackattack_core::frame_data::UIState {
                            clock_time: ticks,
                            ..Default::default()
                        },
                        ..Default::default()
                    }
                }
            };
            // Derive presentation once per tick for cached decisions.
            if let Some(ref session) = self.runtime.session.game {
                let snap = session.snapshot();
                let pres = crate::app::presentation::derive_presentation(
                    &snap, self.runtime.session.pause_menu_open,
                );
                self.cached_has_modal = pres.modal.is_some();
                self.cached_grid_cleared = pres.grid == crate::app::presentation::GridVisibility::Cleared;
            } else {
                self.cached_has_modal = false;
                self.cached_grid_cleared = false;
            }

            // Apply grid visibility to the cached frame.
            if self.cached_grid_cleared {
                frame.blocks.clear();
                frame.garbage.clear();
            }
            self.runtime.session.cached_frame = Some(frame);
        }

        // Grace period at full rate around curtain transitions: while a curtain is active
        // the grace is reloaded; once it ends, it counts down over about 0.3 s. This
        // smooths the return to 15 Hz (otherwise the rate would drop from 50 to 15 Hz
        // exactly when the animation ends, which feels abrupt). It only advances with the
        // logical ticks (50 Hz), so it does not depend on the render rate.
        const TRANSITION_GRACE_TICKS: u32 = 15; // about 0.3 s
        let curtain_active = self.renderer.render_groups.curtain_menu_game.active
            || self.renderer.render_groups.curtain_game_postgame.active;
        if curtain_active {
            self.scheduling.transition_grace = TRANSITION_GRACE_TICKS;
        } else {
            self.scheduling.transition_grace =
                self.scheduling.transition_grace.saturating_sub(tick_count as u32);
        }

        tick_count > 0
    }

    /// Wake-up interval (and so render rate) for the current state.
    /// Only the truly static state (solo or local pause) is slowed down. 50 Hz is
    /// kept whenever the network is involved (communication latency, ready/waiting
    /// states) or a game is active. The logic always runs at 50 Hz (see `tick()`).
    fn wake_interval(&self) -> Duration {
        let s = &self.runtime.session;
        // Network: never throttled (communication and ready/waiting states are latency sensitive).
        if s.selected_mode.is_network() {
            return TICK_DURATION;
        }
        // Curtain transition (active or in its grace period): full rate, even if the flow
        // is still `Menu`. Otherwise the curtain (driven by `render_dt`) would render at
        // 15 Hz and look choppy, and the rate would drop abruptly when the animation ends.
        if self.scheduling.transition_grace > 0 {
            return TICK_DURATION;
        }
        // The menu keeps the full rate: its backdrop blocks drift continuously.
        if s.pause_menu_open {
            IDLE_TICK_DURATION // solo pause: about 15 Hz
        } else {
            TICK_DURATION // menu (backdrop), active game (Playing / PostGame / ready / countdown): 50 Hz
        }
    }

    /// Pre-render update: advance curtain animations and apply side-effects.
    /// Called before render() so that render() is read-only on session state.
    fn update(&mut self, render_dt: f32) {
        use crate::app::render_groups::CurtainEvent;

        // Menu↔Game curtain
        let event = self.renderer.render_groups.curtain_menu_game.tick(render_dt);
        match event {
            CurtainEvent::Midpoint => {
                // Curtain fully black — apply side-effects based on target state.
                if let Some(target) = self.renderer.render_groups.curtain_menu_game.take_target() {
                    if target.is_menu() {
                        self.apply_app_command(AppCommand::ResetForMenu);
                        self.apply_app_command(AppCommand::ClearActiveGame);
                        self.apply_app_command(AppCommand::PlayMusic(AppMusic::Prelude));
                    } else {
                        // The game area opens: menu music fades out and the countdown is silent; the game
                        // music starts at "Fight!".
                        self.apply_app_command(AppCommand::FadeOutMusic);
                    }
                    self.apply_app_command(AppCommand::SetFlow(target));
                }
            }
            CurtainEvent::Complete => {
                // Curtain finished opening — nothing to do (auto-deactivates).
            }
            _ => {}
        }

        // Game curtain (resolving / inter-round)
        let game_event = self.renderer.render_groups.curtain_game_postgame.tick(render_dt);
        match game_event {
            CurtainEvent::Closed => {
                self.renderer.render_groups.game.active = false;
            }
            CurtainEvent::Opened => {
                // Curtain finished opening — nothing to do.
                // ReadyPrompt is already in the snapshot before the curtain opens.
            }
            _ => {}
        }
    }

    /// Render a frame. Returns deferred AppCommand from egui modals
    /// for the caller to apply after the render path completes.
    fn render(&mut self, frame: &FrameData, modal_opt: &mut Option<NameInputState>, net_modal_opt: &mut Option<crate::app::meta_state::LanScreenState>) -> Result<Vec<AppCommand>, wgpu::SurfaceError> {
        let flow_state = &self.runtime.session.flow_state;
        let game_exists = self.runtime.session.game.is_some();
        let result = self.renderer.render_frame(
            &self.gpu,
            &self.runtime.session,
            &mut self.scheduling,
            &mut self.debug,
            self.platform.scale_factor(),
            frame,
            flow_state,
            game_exists,
            modal_opt,
            net_modal_opt,
        );

        // Apply the frame's egui outputs through the platform layer (context cursor,
        // clipboard copy).
        if let Some(out) = self.renderer.ui.pending_egui_outputs.take() {
            self.platform.set_cursor_icon(out.cursor_icon);
            if let Some(text) = out.copy_text {
                self.platform.set_clipboard_text(&text);
            }
        }

        result
    }

    /// Apply an application command. Session/audio commands are handled
    /// by SessionRuntime. Residual commands (renderer, UI, debug, window)
    /// are handled here.
    fn apply_app_command(&mut self, cmd: AppCommand) {
        // See `handle_redraw`: during the modals' take/restore window, effects on
        // `options_modal` / `lan_screen` would be overwritten. Commands from rendering go
        // through `render_cmds` (deferred). Panic in debug instead of failing silently.
        #[cfg(debug_assertions)]
        debug_assert!(
            !self.debug.modals_taken,
            "apply_app_command({cmd:?}) during the modals take/restore window — \
             defer via render_cmds (see bug WP-3, 2026-07-14)"
        );
        if let Some(residual) = self.runtime.apply_app_command(cmd) {
            use AppCommand::*;
            match residual {
                LanPasteRequest => {
                    // Deferred via `render_cmds`, so it is applied AFTER the modals are restored (the
                    // `modals_taken` guard checks this). The text is pasted directly into the session
                    // state of the manual screen.
                    let text = self.platform.clipboard_text();
                    if let Some(lan) = self.runtime.session.menu_state.lan_screen.as_mut() {
                        if lan.manual_open {
                            if let Some(text) = text {
                                lan.manual.paste(&text);
                            }
                        }
                    }
                }
                CopyToClipboard(ref text) => {
                    // Router path of "Copy my address" (the mouse goes through egui).
                    self.platform.set_clipboard_text(text);
                }
                StartTransition(target, duration) => {
                    self.renderer.render_groups.curtain_menu_game.close(target, duration);
                }
                ResetForMenu => {
                    self.renderer.scene.reset_for_menu();
                    self.renderer.particles.compute_sparkles.clear_all();
                    // Reset game curtain (may be closed from a VS match)
                    self.renderer.render_groups.curtain_game_postgame = crate::app::render_groups::CurtainState::new();
                }
                ClearRoundEffects => {
                    self.renderer.scene.reset_for_menu();
                    self.renderer.particles.compute_sparkles.clear_all();
                }
                AdvanceToReadyPrompt => {
                    // Summary closed: clear round effects (level lights etc.) behind the curtain, then
                    // open the curtain to reveal the new grid. The "Ready?" modal asks the question on
                    // top.
                    self.renderer.scene.reset_for_menu();
                    self.renderer.particles.compute_sparkles.clear_all();
                    self.renderer.render_groups.curtain_game_postgame.open(0.5);
                    // Next round: the current music (victory/defeat, or chained menu music) fades out
                    // when the game area opens, as at game start.
                    self.apply_app_command(AppCommand::FadeOutMusic);
                }
                CloseOptionsModal => {
                    // Live model: leaving the options applies and saves. Request the apply; rendering
                    // then produces Confirmed, then ApplyOptionsConfirmed (applies everything, saves,
                    // closes, regenerates the nameplate).
                    if let Some(modal) = self.runtime.session.menu_state.options_modal.as_mut() {
                        modal.confirm_requested = true;
                    }
                }
                RequestExit => {
                    self.debug.should_exit = true;
                }
                RefreshNameplates => {
                    self.renderer.refresh_nameplates(&self.gpu, &self.runtime.session);
                    // Recompile key bindings (options may have changed them)
                    self.key_bindings = keycodes::CompiledBindings::compile(&self.runtime.session.user_config.key_bindings);
                }
                PreviewNameplate { ref name } => {
                    self.renderer.preview_nameplate(&self.gpu, name);
                }
                CloseGameCurtain(duration) => {
                    self.renderer.render_groups.curtain_game_postgame.close_simple(duration);
                }
                EnsureGameCurtainOpen => {
                    // Invariant guard: a starting round must be visible. Does nothing in the normal flow
                    // (the curtain is already open from AdvanceToReadyPrompt). If it fires, some path
                    // missed the opening: repair minimally (visibility) and log.
                    let curtain = &mut self.renderer.render_groups.curtain_game_postgame;
                    if curtain.is_closing_or_closed() {
                        log::error!("Curtain invariant violated: closed at round start — forcing it open");
                        curtain.open(0.5);
                    }
                }
                SetFullscreen(enabled) => {
                    {
                        self.platform.set_fullscreen(enabled);
                    }
                    // Options confirmation also needs nameplate + keybindings refresh.
                    self.renderer.refresh_nameplates(&self.gpu, &self.runtime.session);
                    self.key_bindings = keycodes::CompiledBindings::compile(&self.runtime.session.user_config.key_bindings);
                }
                RequestRedraw => {
                    self.platform.request_redraw();
                }
                other => {
                    log::warn!("Unhandled residual command: {:?}", other);
                }
            }
        }
    }

}

use crate::renderer::init::{create_ui_projection, compute_useful_zone};
use platform_shell::PlatformShell as _;





mod renderer_runtime;
mod gamepad;
mod keycodes;
mod ui_conventions;
mod platform_shell;
mod sdl3_platform;
mod session_runtime;
// Network negotiation (explicit phase and zone services).
mod negotiation;
mod app_bootstrap;
mod script;
#[cfg(feature = "dev")]
mod dev_ui;
use crate::platform::cli;

use session_runtime::{SessionRuntime, SessionState};


/// The first system language the game supports, using `sys-locale` (works on all platforms):
/// `fr-CA` gives French, `de-AT` German, and so on. `None` when no system language is
/// supported (English is then used). The persisted `i18n.language` option takes priority.
fn detect_system_language() -> Option<crate::app::locale::Language> {
    sys_locale::get_locales().find_map(|l| language_of_locale(&l))
}

/// Supported language of one system locale tag. Chinese is only Simplified (`zh`, `zh-CN`,
/// `zh-SG`, `zh-Hans`...): a Traditional Chinese system (`zh-TW`, `zh-HK`, `zh-MO`, `zh-Hant`)
/// gets no match rather than the wrong script.
fn language_of_locale(tag: &str) -> Option<crate::app::locale::Language> {
    let tag = tag.to_ascii_lowercase();
    let mut parts = tag.split(['-', '_']);
    let primary = parts.next().unwrap_or("");
    if primary == "zh" && parts.any(|p| matches!(p, "hant" | "tw" | "hk" | "mo")) {
        return None;
    }
    crate::app::locale::Language::from_code(primary)
}

/// Version line: `crackattack <version> (<origin>)`. The origin is the build stamp
/// (`CRACKATTACK_DISTRIBUTION`), or `unknown` if absent.
fn version_line() -> String {
    let dist = crate::platform::distribution::Distribution::current();
    format!("crackattack {} ({})", env!("CARGO_PKG_VERSION"), dist.stamp())
}

fn main() {
    let cli = cli::parse_args();
    if cli.help {
        cli::print_help();
        return;
    }
    if cli.version {
        println!("{}", version_line());
        return;
    }

    // Fail fast: an invalid `--set` / `--open` entry fails before any init. An option is
    // never ignored silently, which would skew a QA scenario unnoticed.
    let cli_errors = cli::validate(&cli);
    if !cli_errors.is_empty() {
        for e in &cli_errors {
            eprintln!("Error: {e}");
        }
        std::process::exit(1);
    }

    crate::platform::diagnostics::init_logging(&version_line());

    // Guard: validate the assets directory BEFORE any window/GPU init, so that a launch
    // from the wrong directory fails early and cleanly (no flashing window).
    // `find_assets_dir` prints a clear message and calls `exit(1)` if nothing valid is
    // found.
    let _ = crate::renderer::init::find_assets_dir();

    // UI language. Priority: CLI override `config:i18n.language` > persisted choice >
    // system auto-detection.
    {
        use crate::app::locale::{Language, set_language};
        use crate::platform::persist::UserConfig;
        // Apply the `--set config:` overrides BEFORE reading the language, so that
        // `--set config:i18n.language=fr` takes effect at boot.
        let mut cfg = UserConfig::load();
        cfg.apply_cli_set_overrides(&cli.set_overrides);
        // A present but INVALID code ("de", "") falls back to auto-detection instead of
        // forcing English.
        match cfg.language.as_deref().and_then(Language::from_code) {
            Some(lang) => {
                set_language(lang);
                log::info!("Locale: {} (user setting/CLI)", lang.display_name());
            }
            None => match detect_system_language() {
                Some(lang) => {
                    set_language(lang);
                    log::info!("Locale: {} (auto-detected)", lang.display_name());
                }
                None => log::info!("Locale: English (default)"),
            },
        }
        // The first `load()` ran before the language was known, so its default name is the
        // English one: give the player the default name of their language.
        if crate::platform::persist::is_first_launch() {
            cfg.player_name = crate::platform::persist::default_player_name().to_string();
            cfg.save();
        }
    }

    log::info!("Crack Attack! WebGPU Renderer");
    log::info!("Controls: Arrows = Move, Space = Swap, P = Pause, ESC = Quit");

    let boot = cli::apply(&cli);

    // Miniscript: validate the file EARLY (before window/GPU), so that an unreadable or
    // invalid script fails cleanly. `--script-dry-run` (always accompanied by `--script`,
    // enforced by `validate()`) prints the timeline and exits; otherwise the bootstrap
    // parses the file again for real.
    if let Some(path) = boot.script.clone() {
        match std::fs::read_to_string(&path)
            .map_err(|e| format!("unreadable: {e}"))
            .and_then(|src| script::ScriptRunner::parse(&src).map_err(|e| format!("invalid: {e}")))
        {
            Ok(runner) => {
                // The dry run comes BEFORE the seed check: inspecting a timeline captures nothing,
                // so no seed is needed.
                if boot.script_dry_run {
                    println!("Timeline \"{path}\" ({} steps):", runner.describe().len());
                    for l in runner.describe() { println!("  {l}"); }
                    return;
                }
                // Seed check: the default seed is the Unix time, so a script that CAPTURES without
                // an explicit seed would produce non-reproducible PNGs with no signal. Refuse before
                // any window opens. The seed counts as set when `boot.seed` is present (`--seed 0`
                // is valid).
                let seed_set = boot.seed.is_some();
                if !runner.capture_names().is_empty() && !seed_set {
                    eprintln!("Error: miniscript \"{path}\" contains captures but no explicit \
seed (--seed N or --set session:game.seed=N) — captures would not be reproducible.");
                    std::process::exit(1);
                }
                if !seed_set {
                    log::warn!("script without explicit seed: run not reproducible (seed = Unix time)");
                }
            }
            Err(e) => { eprintln!("Error: miniscript \"{path}\" {e}"); std::process::exit(1); }
        }
    }

    let mut audio_output = AudioOutput::new();

    if boot.sound_preview {
        crate::platform::sound::preview_sfx(
            &mut audio_output,
            &crate::renderer::init::find_assets_dir(),
            boot.sounds_dir.as_deref().map(std::path::Path::new),
        );
    }

    // SDL3 is the only platform (pull loop, single owner of the run loop). The gamepad is
    // served inside the loop (`PadPoller`); there is no service thread.
    run_sdl_backend(Some(audio_output), boot);
}

/// SDL3 driver: a **pull** loop. Each iteration ticks the game logic, then
/// `handle_redraw` runs the rest of the cycle (drain `poll_events`, split between
/// egui and router, update, render). Pacing uses `wake_interval()` (50 Hz in game and
/// network, slower on static screens), with an early wake-up when `request_redraw`
/// was signaled.
fn run_sdl_backend(audio_output: Option<AudioOutput>, boot: crate::platform::boot_config::BootConfig) {
    log::info!("SDL3 backend — gamepad integrated into the loop (PadPoller, no thread)");
    let position = boot.window_position;
    let platform = match sdl3_platform::Sdl3Platform::new(crate::GAME_NAME, WINDOW_WIDTH, WINDOW_HEIGHT, position) {
        Ok(p) => p,
        Err(e) => crate::platform::diagnostics::fatal(&format!("The game could not start (SDL3: {e}).")),
    };
    // Key name display (Options -> Controls): the labels of the CURRENT keyboard layout
    // are set first (SDL is initialized), then the display function. Persistence keeps
    // stable identifiers.
    keycodes::set_layout_labels(sdl3_platform::layout_key_labels());
    crate::renderer::ui::options_modal::set_key_display_fn(keycodes::key_display_name);

    let mut state = AppState::new(platform, audio_output, boot);

    // Fullscreen: persisted preference, otherwise the platform default.
    let want_fullscreen = if !crate::platform::persist::is_first_launch() {
        state.runtime.session.user_config.fullscreen
    } else {
        crate::platform::profile::fullscreen_default()
    };
    if want_fullscreen {
        log::info!("Fullscreen at start ({})",
            if crate::platform::persist::is_first_launch() { "platform default" } else { "saved choice" });
        state.platform.set_fullscreen(true);
    }

    // Menu music (Prelude).
    if matches!(state.runtime.session.flow_state, FlowState::Menu) {
        if let Some(ref mut snd) = state.runtime.sound_manager {
            snd.play_music(MusicTrack::Prelude);
        }
    }
    log::info!("Window created - Use arrows to move, Space to swap, ESC to quit");

    loop {
        let frame_start = Instant::now();
        // The TICK runs here: the 50 Hz logic (fixed-step accumulator in `tick()`) advances
        // whatever the render rate.
        let _ticked = state.tick();
        if state.occluded {
            // Window hidden: pump the events (including Restored) and tick, but skip rendering
            // (GPU submit is about 80% of the cost). Relaxed rate.
            state.drain_inputs();
            if state.debug.should_exit { break; }
            std::thread::sleep(Duration::from_millis(100));
            continue;
        }
        if let Err(fatal) = state.handle_redraw() {
            if fatal { break; }
        }
        if state.debug.should_exit { break; }
        // Pacing: in active game or network (budget = TICK_DURATION), the vsync present in
        // `handle_redraw` paces the loop by itself (maximum smoothness). On static screens
        // (larger budget) the wait is interruptible: `wait_for_events` wakes as soon as an
        // OS event arrives (minimal input latency) or `request_redraw` is signaled. The
        // event comes out first in the next drain.
        let budget = state.wake_interval();
        if budget > TICK_DURATION {
            let elapsed = frame_start.elapsed();
            if elapsed < budget {
                state.platform.wait_for_events(budget - elapsed);
            }
        }
    }
    // Drain pending captures: a capture is asynchronous (GPU readback, then a writer
    // thread), so exiting without draining would silently lose the last PNG. `is_idle()`
    // only becomes true once the PNG is written (the Saving state is tracked).
    if let Some(crate::platform::screenshot::ScreenshotRequest::Named(f)) =
        state.debug.screenshot_request.take()
    {
        state.renderer.screenshot
            .record_failure(f, "exited before the capture frame was rendered".to_string());
    }
    let drain_start = Instant::now();
    while !state.renderer.screenshot.is_idle() {
        if drain_start.elapsed() > Duration::from_secs(10) {
            log::error!("captures: drain timed out (10 s, GPU callback lost?) — exit code 1");
            std::process::exit(1);
        }
        state.gpu.device.poll(wgpu::Maintain::Poll);
        state.renderer.screenshot.poll();
        std::thread::sleep(Duration::from_millis(2));
    }

    // Recording: write the captured `.mini` on exit, sealed by a `quit` at the end frame
    // (without it, replaying the re-export would never finish).
    if let Some(rec) = &state.recorder {
        let end_frame = state.scheduling.frame_counter;
        if rec.dropped_inputs() > 0 {
            log::warn!(
                "Recording: {} non-keyboard input(s) (mouse/gamepad/text) NOT captured — incomplete replay",
                rec.dropped_inputs());
        }
        match rec.write(end_frame) {
            Ok(()) => log::info!("Recording written: {} ({} events)", rec.path(), rec.len()),
            Err(e) => log::error!("Recording \"{}\": write failed ({e})", rec.path()),
        }
    }
    // Machine-readable miniscript summary: a single line a shell script can check,
    // instead of scattered messages.
    if state.debug.script_path.is_some() {
        let outcomes = state.renderer.screenshot.outcomes();
        // Captures PLANNED by the timeline, not just requested: a campaign cut short before
        // its first capture (max-frames, expired gate) must fail.
        let planned = state.script.as_ref().map(|r| r.capture_names().len()).unwrap_or(0);
        let written: Vec<String> = outcomes.iter()
            .filter_map(|(_, r)| r.as_ref().ok().map(|p| p.display().to_string()))
            .collect();
        let errors: Vec<String> = outcomes.iter()
            .filter_map(|(n, r)| r.as_ref().err().map(|e| format!("{n}: {e}")))
            .collect();
        let (sw, sh) = state.platform.pixel_size();
        log::info!(
            "script: summary — captures planned={} requested={} written={} seed={} surface={}x{} files=[{}] errors=[{}]",
            planned, state.debug.script_captures_demanded, written.len(),
            state.debug.explicit_seed.unwrap_or(0),
            sw, sh, written.join(", "), errors.join(" ; "),
        );
        if let Err(e) = crate::platform::screenshot::campaign_verdict(
            planned, state.debug.script_captures_demanded, outcomes,
        ) {
            log::error!("script: {e} — exit code 1");
            std::process::exit(1);
        }
    }
    if state.debug.script_assert_failed {
        log::error!("script: assertion(s) failed — exit code 1");
        std::process::exit(1);
    }
    log::info!("SDL3 backend: clean exit");
}

#[cfg(test)]
mod lib_projection_tests {
    #[test]
    fn ui_projection_is_valid() {
        let p = crate::renderer::init::create_ui_projection(700, 700);
        assert!(p.projection[0][0] != 0.0);
        assert!(p.projection[1][1] != 0.0);
    }

    #[test]
    fn viewport_letterbox_square() {
        let (x, y, w, h) = crate::renderer::init::compute_effective_viewport(800, 600);
        assert_eq!(w, 600); // contain: width = height for 1:1 aspect
        assert_eq!(h, 600);
        assert_eq!(x, 100); // centered horizontally
        assert_eq!(y, 0);
    }
}

#[cfg(test)]
mod locale_detection_tests {
    use super::language_of_locale;
    use crate::app::locale::Language;

    /// Region and script variants map to the supported language; Traditional Chinese does not
    /// fall back to Simplified.
    #[test]
    fn system_locale_tags() {
        assert_eq!(language_of_locale("fr-CA"), Some(Language::French));
        assert_eq!(language_of_locale("de_AT"), Some(Language::German));
        assert_eq!(language_of_locale("zh-CN"), Some(Language::Chinese));
        assert_eq!(language_of_locale("zh-Hans-SG"), Some(Language::Chinese));
        assert_eq!(language_of_locale("zh"), Some(Language::Chinese));
        assert_eq!(language_of_locale("zh-TW"), None);
        assert_eq!(language_of_locale("zh-Hant-HK"), None);
        assert_eq!(language_of_locale("ja-JP"), Some(Language::Japanese));
        assert_eq!(language_of_locale("ko_KR"), Some(Language::Korean));
        assert_eq!(language_of_locale("it-IT"), Some(Language::Italian));
        assert_eq!(language_of_locale("pl_PL"), Some(Language::Polish));
        assert_eq!(language_of_locale("tr-TR"), Some(Language::Turkish));
        assert_eq!(language_of_locale("nl-NL"), None);
    }
}
