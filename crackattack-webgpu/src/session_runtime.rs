//! Session runtime: session state, audio context, and session-level command handling.

use crackattack_core::{GameState, ScoringMode, FrameData};
use crackattack_core::input::Input;
use crackattack_core::vs::{
    GameSession, SoloSession, VsComputerGameSession, GridOpponent,
};

use crate::app::command::{AppCommand, AppMusic};
use crate::app::meta_state::{MenuState, GameMode, FlowState};
use crate::app::post_game::{PostGameEffectsState, PostGameInput};
use crate::platform::persist::UserConfig;
use crate::platform::sound::{AudioOutput, SoundManager, MusicTrack};

/// Tracks when a suspension overlay (ReadyPrompt, WaitingRemote,
/// ConnectionLost, ResumeSync) started, for UI timeout display.
pub(crate) struct SuspensionTimer {
    kind: u8,
    started_at: Option<std::time::Instant>,
}

impl SuspensionTimer {
    fn new() -> Self {
        Self { kind: 0, started_at: None }
    }

    /// Update from the current interruption. Returns true if changed.
    pub(crate) fn sync(&mut self, interruption: &crackattack_core::frame_data::SessionInterruption) {
        use crackattack_core::frame_data::SessionInterruption;
        let kind: u8 = match interruption {
            SessionInterruption::ReadyPrompt { .. } => 1,
            SessionInterruption::WaitingRemoteReady { .. } => 2,
            SessionInterruption::ConnectionLost { .. } => 3,
            SessionInterruption::ResumeSync { .. } => 4,
            _ => 0,
        };
        if kind != self.kind {
            self.started_at = if kind == 0 { None } else { Some(std::time::Instant::now()) };
            self.kind = kind;
        }
    }

    /// Seconds since the current suspension started, or 0 if none.
    pub(crate) fn elapsed_secs(&self) -> f32 {
        self.started_at.map_or(0.0, |t| t.elapsed().as_secs_f32())
    }
}

/// Bootstrap configuration for the session, computed from CLI args.
/// Decouples SessionRuntime from the CLI statics.
pub(crate) struct SessionConfig {
    pub seed: u32,
    pub autoplay: bool,
    /// Grid AI level driving the autoplay (`None` = the classic autoplay).
    pub autoplay_ai: Option<crackattack_core::ai_difficulty::AIDifficulty>,
    /// E2E bound: autoplay quits at the prompt after N rounds played (`None` = unlimited).
    pub autoplay_max_rounds: Option<u32>,
    pub vs_mode: bool,
    pub ai_difficulty: u8,
    pub network: Option<crate::platform::cli::NetworkMode>,
    /// Targets from `--open <id>`, resolved into `BootTarget`s at boot.
    pub open_targets: Vec<String>,
    /// Overrides from `--set <id>=<value>` (`config:` namespace). Applied to `user_config`
    /// at boot and never persisted. `--name` is an alias for `config:player.name`.
    pub set_overrides: Vec<String>,
    /// Sound set under test (`--sounds DIR`): sound effects are read from this folder, file
    /// by file, falling back to the defaults.
    pub sounds_dir: Option<std::path::PathBuf>,
    /// Music under test (`--music DIR`): tracks are read from this folder, track by track,
    /// falling back to the defaults.
    pub music_dir: Option<std::path::PathBuf>,
    /// Adaptive music rung set under test (`--rungs DIR`); defaults to `music/adaptive/`.
    pub rungs_dir: Option<std::path::PathBuf>,
    /// Adaptive music debug log (`--music-debug`).
    pub music_debug: bool,
}

use crate::negotiation::{ConnectFailure, NegotiationState, OpponentNetInfo};

/// Logical game session state (meta-flow, input, config, AI).
pub(crate) struct SessionState {
    /// Active game (None during Menu, Some during Playing/PostGame).
    pub(crate) game: Option<Box<dyn GameSession>>,
    /// Network negotiation: explicit phase (outgoing/incoming invite) plus area services
    /// (TCP listener, mDNS discovery, peers seen). See `negotiation`.
    pub(crate) negotiation: NegotiationState,
    pub(crate) input: Input,
    pub(crate) ai_controller: crackattack_core::AIController,
    /// Grid AI replacing `ai_controller` under `--autoplay-ai easy|medium|hard`; built on first use.
    pub(crate) autoplay_grid_level: Option<crackattack_core::ai_difficulty::AIDifficulty>,
    pub(crate) autoplay_grid: Option<crackattack_core::vs::GridAutoplay>,
    pub(crate) flow_state: FlowState,
    pub(crate) game_seed: u32,
    pub(crate) menu_state: MenuState,
    pub(crate) selected_mode: GameMode,
    pub(crate) player_name: String,
    pub(crate) user_config: UserConfig,
    /// Local ladder match log (loaded at boot; appended to at each round verdict).
    pub(crate) ladder: crate::platform::ladder::LadderStore,
    /// Ranking recomputed from the log. Never persisted (disposable cache); refreshed on
    /// load and after each append.
    pub(crate) ladder_records: std::collections::HashMap<
        crackattack_rating::PlayerId,
        crackattack_rating::PlayerRecord,
    >,
    /// Rating delta of the last rated round (shown on the between-round stats).
    pub(crate) ladder_round_delta: Option<f64>,
    /// Cumulative rating delta of the current match (shown on the final screen). Reset when a
    /// network game starts.
    pub(crate) ladder_match_delta: Option<f64>,
    /// Cooldown (in ticks) after screen transition before inputs are accepted
    pub(crate) transition_cooldown: i32,
    /// `QuitToMenu` was applied and the curtain is closing on the game. A repeated request
    /// is ignored: it would restart the curtain, which then never finishes closing.
    pub(crate) leaving_game: bool,
    /// Cached stats from the finished game
    pub(crate) game_over_stats: Option<crackattack_core::stats::GameStats>,
    /// Final score for the high scores screen
    pub(crate) game_over_score: i32,
    /// Rank returned by submit_score (0-based), or None if score didn't make the table
    pub(crate) game_over_rank: Option<usize>,
    /// Records beaten in this game (for post-game highlighting).
    pub(crate) game_over_beaten_records: Vec<crate::platform::persist::RecordKey>,
    /// Stats from the last VS round (displayed during ReadyPrompt between rounds).
    pub(crate) round_end_stats: Option<crate::app::post_game::RoundEndStats>,
    pub(crate) cached_frame: Option<FrameData>,
    pub(crate) post_game_fx: PostGameEffectsState,
    pub(crate) post_game_input: PostGameInput,
    pub(crate) post_game_screen: crate::app::post_game::PostGameScreen,
    pub(crate) autoplay_enabled: bool,
    /// E2E bound (`--autoplay-rounds`): quit at the prompt after N rounds played.
    pub(crate) autoplay_max_rounds: Option<u32>,
    /// Network contact of the current opponent: set on connection, recorded in the log at
    /// each rated round, cleared on `ClearActiveGame`.
    pub(crate) opponent_net_info: Option<OpponentNetInfo>,
    /// A reconnection started from the recent-opponents page returns there on failure or
    /// abandon, instead of falling back to the LAN list or the menu.
    pub(crate) reconnect_return_to_opponents: bool,
    /// Rating self-announced by the current opponent (mDNS `rt`), frozen when the invite is
    /// sent. `None` if unrated, or for a manual or CLI connection.
    pub(crate) opponent_announced_rt: Option<i32>,
    /// Pause menu is open (Resume / Quit).
    pub(crate) pause_menu_open: bool,
    /// When the pause menu was opened (for Quit delay protection).
    pub(crate) pause_menu_opened_at: Option<std::time::Instant>,
    /// True if the local player initiated the current pause (pressed P/Escape).
    /// False if the pause was triggered by the remote opponent.
    /// Used to skip the anti-reflex delay for the initiator.
    pub(crate) pause_initiated_locally: bool,
    /// Currently selected item in the pause menu (0=Resume, 1=Quit).
    /// Reset to 0 when the menu opens. Stored here instead of in egui
    /// temp state to avoid fragile dependency on egui persistence.
    /// Uses Cell so the render pass can update it without &mut self.
    pub(crate) pause_menu_selection: std::cell::Cell<usize>,
    /// Selection of the two buttons of the "Ready?" modal (0 = Ready, 1 = Menu). Driven by the
    /// router (keyboard/gamepad) and mouse hover; read by the renderer. A `Cell`, like the pause.
    pub(crate) ready_selection: std::cell::Cell<usize>,
    /// Tracks when the current suspension overlay started (UI timeout).
    pub(crate) suspension_timer: SuspensionTimer,
    /// Startup commands (from the CLI shortcuts), applied once on the first frame through
    /// the same path as the menu actions (`SelectMode`/`CreateGame` or
    /// `OpenLanScreen`/`ConnectToPeer`). No special boot branch, hence no side effects
    /// (listener always armed, connection at boot).
    pub(crate) pending_startup: Vec<AppCommand>,
    /// Requested listen port (CLI `--vs network-create <port>`). The area listener and the LAN
    /// screen bind exactly this port, for a deterministic E2E host. `None` = dynamic range.
    pub(crate) requested_listen_port: Option<u16>,
    /// Update check: receives the result of the check thread and holds the notification to
    /// display. Inert while `PUBLIC_PROJECT` is `None` or the channel profile has no fetch.
    #[cfg(feature = "update-check")]
    pub(crate) update_check: UpdateCheckState,
}

/// Runtime state of the update checker. `started` guarantees at most one attempt per
/// SESSION (the delay between sessions lives in `user_config.last_update_check`).
#[cfg(feature = "update-check")]
#[derive(Default)]
pub(crate) struct UpdateCheckState {
    started: bool,
    rx: Option<std::sync::mpsc::Receiver<crate::update::checker::CheckOutcome>>,
    /// `Some` = this binary can check for updates (set at session creation, before any
    /// `--open` deep link). Enables the Options checkbox and the "Check now" button.
    pub(crate) source: Option<crate::update::UpdateSource>,
    /// `Some` = notification to display (ticker and Info page).
    pub(crate) available: Option<crate::update::checker::UpdateAvailable>,
}

#[cfg(feature = "update-check")]
impl UpdateCheckState {
    /// An attempt is in progress ("Checking..." status line).
    pub(crate) fn checking(&self) -> bool {
        self.rx.is_some()
    }
}

/// Current time in Unix seconds (`0` if the clock is before 1970).
#[cfg(feature = "update-check")]
fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// One Up/Down step on an Info page with actions. `selected` is 1-based (0 = the text, no
/// action focused). Credits is text plus an action, so the step stops at its ends. The
/// Updates page is only actions: its focus loops, like every other list screen (from the
/// last action back to the first, and from the first to the last).
fn info_action_step(selected: usize, count: usize, delta: i32, loops: bool) -> usize {
    if loops && count > 0 {
        return match (selected, delta > 0) {
            (0, true) => 1,
            (0, false) => count,
            _ => crate::app::focus::ring_move(selected.min(count) - 1, count, delta) + 1,
        };
    }
    // [text, actions...]: 0 = no action.
    crate::app::focus::clamp_move(selected, count + 1, delta)
}

/// State of the Updates page, derived from the persisted state and the attempt in progress.
/// A version the player once ignored still counts as available here: the page is opened on
/// purpose, so it always offers the download.
#[cfg(feature = "update-check")]
fn update_view(session: &SessionState, source: &crate::update::UpdateSource)
    -> crate::renderer::ui::info_modal::UpdateView
{
    use crate::platform::persist::date_string;
    use crate::renderer::ui::info_modal::UpdateView;
    use crate::update::{update_status, UpdateStatus};
    let cfg = &session.user_config;
    match update_status(
        session.update_check.checking(),
        cfg.last_update_check,
        cfg.last_update_success,
        &cfg.available_update_version,
        &cfg.ignored_update_version,
    ) {
        UpdateStatus::Checking => UpdateView::Checking,
        UpdateStatus::Never => UpdateView::NeverChecked,
        UpdateStatus::Failed { last_success } => UpdateView::Failed {
            last_success: last_success.map(date_string),
        },
        UpdateStatus::Available(version) | UpdateStatus::Ignored(version) => UpdateView::Available {
            version,
            url: source.open_url.clone(),
        },
        UpdateStatus::UpToDate { checked_at } => UpdateView::UpToDate { checked: date_string(checked_at) },
    }
}

impl SessionState {
    /// Hint after a network version mismatch: the same destination as the update if this
    /// binary has one, otherwise the "managed by" notice.
    pub(crate) fn version_mismatch_hint(&self) -> Option<crate::app::meta_state::LanErrorHint> {
        use crate::app::meta_state::LanErrorHint;
        use crate::platform::distribution::{Distribution, PUBLIC_PROJECT};
        #[cfg(feature = "update-check")]
        if let Some(source) = &self.update_check.source {
            return Some(LanErrorHint::Download(source.open_url.clone()));
        }
        let profile = Distribution::current().profile();
        if let Some(url) = profile.update_url(PUBLIC_PROJECT.as_ref()) {
            return Some(LanErrorHint::Download(url));
        }
        profile.managed_info().map(LanErrorHint::ManagedBy)
    }

    /// Updates page state: from the check when this binary has an update source, otherwise the
    /// store / distribution note if there is one, otherwise "unavailable".
    fn update_view(&self, managed: Option<&'static str>) -> crate::renderer::ui::info_modal::UpdateView {
        use crate::renderer::ui::info_modal::UpdateView;
        #[cfg(feature = "update-check")]
        if let Some(source) = &self.update_check.source {
            return update_view(self, source);
        }
        match managed {
            Some(note) => UpdateView::Managed(note),
            None => UpdateView::Unavailable,
        }
    }

    /// Surfaces of the Credits page, resolved from the profile, the public identity and the
    /// update check state. Shared by the renderer and the router, so every action reachable
    /// with the mouse is also reachable with keyboard and gamepad.
    pub(crate) fn info_extras(&self) -> crate::renderer::ui::info_modal::InfoExtras {
        use crate::platform::distribution::{Distribution, ManagedBy, PUBLIC_PROJECT};
        let profile = Distribution::current().profile();
        let t = crate::app::locale::t();
        crate::renderer::ui::info_modal::InfoExtras {
            contribute_url: profile.contribute_url(PUBLIC_PROJECT.as_ref()),
            update_view: self.update_view(profile.managed_info().map(|m| match m {
                ManagedBy::Store => t.update_managed_by_store,
                ManagedBy::Distro => t.update_managed_by_distro,
            })),
            #[cfg(feature = "update-check")]
            update_checks: self.update_check.source.as_ref()
                .map(|_| self.user_config.update_checks_enabled),
            #[cfg(not(feature = "update-check"))]
            update_checks: None,
        }
    }

    /// Actions of the open Info page (Credits, Updates), in display order.
    pub(crate) fn info_actions(&self) -> Vec<crate::renderer::ui::info_modal::InfoAction> {
        self.menu_state.info_section
            .map(|s| self.info_extras().actions(s))
            .unwrap_or_default()
    }

    pub(crate) fn set_flow(&mut self, state: FlowState) {
        self.flow_state = state;
    }
}

/// Session runtime: owns session state + audio. All session mutations
/// go through this struct. Commands that need renderer/UI/debug are
/// returned as residuals for the caller to handle.
pub(crate) struct SessionRuntime {
    pub(crate) session: SessionState,
    pub(crate) sound_manager: Option<SoundManager>,
    /// Game events of the steps run since the last render (edges: only the tick delivers
    /// them, `frame_data()` does not serve them again). Taken by the renderer for
    /// event-driven effects (chain rings).
    pub(crate) step_events: Vec<crackattack_core::gameplay_event::GameplayEvent>,
}

impl SessionRuntime {
    /// Build session runtime from a bootstrap config (decoupled from CLI).
    pub(crate) fn new(config: SessionConfig, user_config: UserConfig, audio_output: Option<AudioOutput>, data_dir: &std::path::Path) -> Self {
        let mut user_config = user_config;
        // Apply the `--set config:` overrides again, for entry points that build a
        // `SessionRuntime` without the earlier overlay (tests). Idempotent.
        user_config.apply_cli_set_overrides(&config.set_overrides);
        let game_seed = config.seed;
        let is_vs = config.vs_mode;

        // Launch shortcuts do not build state at boot. They name a semantic target
        // (`BootTarget`) that resolves to the same `AppCommand` sequence as the menu and is
        // queued in `pending_startup` (applied on the first frame). The game always starts at
        // the menu: no special boot path, no side effects.
        use crate::platform::boot_target::{BootContext, BootTarget};
        let ai_diff = match config.ai_difficulty {
            0 => crackattack_core::ai_difficulty::AIDifficulty::Easy,
            2 => crackattack_core::ai_difficulty::AIDifficulty::Hard,
            _ => crackattack_core::ai_difficulty::AIDifficulty::Medium,
        };
        let entry = user_config.ranking_entry_score();
        let ranking_entry = if entry > 0 { entry } else { 1 };

        // Build the boot targets. An explicit `--open` wins; otherwise derive them from the
        // flags (network, vs computer, solo autoplay). Autoplay is an independent AI driver
        // (`autoplay_enabled`): it plays whatever game was launched.
        let mut boot_targets: Vec<BootTarget> = Vec::new();
        for id in &config.open_targets {
            match BootTarget::parse(id) {
                Ok(t) => boot_targets.push(t),
                // Defensive: unreachable from the binary (`cli::validate()` fails first); kept
                // for constructions outside main (tests).
                Err(e) => log::warn!("CLI: --open {id} ignored ({e})"),
            }
        }
        if boot_targets.is_empty() {
        if let Some(ref net) = config.network {
            match net {
                crate::platform::cli::NetworkMode::Create { port } => {
                    boot_targets.push(BootTarget::NetworkHost { port: Some(*port) });
                }
                crate::platform::cli::NetworkMode::Join { host, port } => {
                    match host.parse::<std::net::IpAddr>() {
                        Ok(addr) => boot_targets.push(BootTarget::NetworkJoin { addr, port: *port }),
                        // Host is not an IP: open the network screen only (the join is ignored).
                        Err(_) => {
                            log::warn!("CLI: --vs network-join expects an IP (got {host}) — opening the Network VS screen alone");
                            boot_targets.push(BootTarget::ScreenLan);
                        }
                    }
                }
            }
        } else if config.autoplay || is_vs {
            // Autoplay drives the AI on the launched game: vs computer is an AI game, else solo.
            boot_targets.push(if is_vs {
                BootTarget::PlayVsComputer(ai_diff)
            } else {
                BootTarget::PlaySolo
            });
        }
        } // no explicit --open

        // Resolve targets into startup commands; remember the requested listen port (fixed-port host).
        let boot_ctx = BootContext { seed: game_seed, ranking_entry_score: ranking_entry };
        let requested_listen_port: Option<u16> =
            boot_targets.iter().find_map(|t| t.requested_listen_port());
        let pending_startup: Vec<AppCommand> = boot_targets
            .iter()
            .flat_map(|t| t.resolve(&boot_ctx))
            .collect();
        if !boot_targets.is_empty() {
            log::info!("CLI: boot targets {:?}", boot_targets);
        }

        let (game, flow_state, selected_mode) = (None, FlowState::Menu, GameMode::Solo);

        // Local ladder: load the log and recompute the ranking (the rating is never stored).
        // Parameters come from the user config.
        let ladder = crate::platform::ladder::LadderStore::load();
        let ladder_records = crackattack_rating::recompute(ladder.entries(), &crackattack_rating::RatingConfig::default());
        if !ladder.entries().is_empty() {
            log::info!(
                "Ladder: {} entries, {} players",
                ladder.entries().len(),
                ladder_records.len()
            );
        }

        SessionRuntime {
            step_events: Vec::new(),
            sound_manager: audio_output.map(|ao| {
                let mut sm = SoundManager::new(ao, data_dir, config.sounds_dir.as_deref());
                sm.set_music_override(config.music_dir.clone());
                sm.set_rungs_dir(config.rungs_dir.clone());
                sm.set_music_debug(config.music_debug);
                sm.sfx_enabled = user_config.sfx_enabled;
                sm.music_enabled = user_config.music_enabled;
                sm.set_sfx_volume(user_config.sfx_volume);
                sm.set_music_volume(user_config.music_volume);
                sm
            }),
            session: SessionState {
                game,
                negotiation: NegotiationState::new(),
                input: Input::new(),
                ai_controller: crackattack_core::AIController::new(),
                autoplay_grid_level: config.autoplay_ai,
                autoplay_grid: None,
                flow_state,
                game_seed,
                menu_state: MenuState::new(),
                selected_mode,
                // Mirror of `user_config` (the `config:player.name` override is already applied),
                // so the name is the same everywhere: local nameplate, discovery, options,
                // handshake.
                player_name: user_config.player_name.clone(),
                user_config,
                ladder,
                ladder_records,
                ladder_round_delta: None,
                ladder_match_delta: None,
                transition_cooldown: 0,
                leaving_game: false,
                game_over_stats: None,
                game_over_score: 0,
                game_over_rank: None,
                game_over_beaten_records: Vec::new(),
                round_end_stats: None,
                cached_frame: None,
                post_game_fx: PostGameEffectsState::default(),
                post_game_input: PostGameInput::default(),
                post_game_screen: crate::app::post_game::PostGameScreen::default(),
                autoplay_enabled: config.autoplay,
                autoplay_max_rounds: config.autoplay_max_rounds,
                opponent_net_info: None,
                reconnect_return_to_opponents: false,
                opponent_announced_rt: None,
                pause_menu_open: false,
                pause_menu_opened_at: None,
                pause_initiated_locally: true,
                pause_menu_selection: std::cell::Cell::new(0),
                ready_selection: std::cell::Cell::new(0),
                suspension_timer: SuspensionTimer::new(),
                pending_startup,
                requested_listen_port,
                #[cfg(feature = "update-check")]
                update_check: UpdateCheckState { source: crate::update::update_source(), ..Default::default() },
            },
        }
    }

    /// Apply a command. Returns Some(cmd) for commands that need
    /// renderer, UI, debug, or window handling.
    pub(crate) fn apply_app_command(&mut self, cmd: AppCommand) -> Option<AppCommand> {
        use AppCommand::*;
        match cmd {
            // Flow & post-game
            SetFlow(state) => { self.session.set_flow(state); }
            StartTransition(..) => return Some(cmd),
            // Clipboard needs the platform layer: returned as residual (handled in main).
            LanPasteRequest | CopyToClipboard(_) => return Some(cmd),
            SetTransitionCooldown(ticks) => { self.session.transition_cooldown = ticks; }
            AdvancePostGameScreen | EnterPostGame => return self.handle_post_game(cmd),
            AdvanceToReadyPrompt => {
                // Closes the summary without committing to anything: the "Ready for a new
                // round?" question follows (modal, curtain open).
                self.session.post_game_input.post_round_advanced = true;
                // Anti-reflex: ~0.5 s before the modal accepts a confirmation, so a mashed key
                // cannot answer the question unseen.
                self.session.transition_cooldown = 25;
                // Returns as residual for curtain opening and round-effect cleanup.
                return Some(AppCommand::AdvanceToReadyPrompt);
            }
            RequestQuitGame => {
                // Confirm only if leaving costs a rated round.
                let engaged = self.session.game.as_ref()
                    .map_or(false, |s| s.snapshot().round_engaged);
                if engaged {
                    self.session.post_game_input.abandon_confirm_open = true;
                    self.session.post_game_input.abandon_confirm_yes = false;
                } else {
                    return self.apply_app_command(AppCommand::QuitToMenu);
                }
            }
            SetGameplayKey { action, pressed } => { self.session.input.set_key(action, pressed); }
            SetPostGameScrollDir(dir) => {
                self.session.post_game_input.scroll_dir = dir;
            }
            SetPostGameNavFocus(focus) => {
                self.session.post_game_input.nav_focus = focus;
            }
            SetAbandonConfirm(sel) => {
                self.session.post_game_input.abandon_confirm_open = sel.is_some();
                self.session.post_game_input.abandon_confirm_yes = sel.unwrap_or(false);
            }

            // Audio
            PlayMusic(..) | FadeOutMusic | PlayUiConfirm | PlayUiNavigate => {
                self.handle_audio(cmd);
            }

            // Menu
            MenuNavigate(..) | MenuBack | ConfirmMenuSelection | MenuSelectIndex(_)
            | MenuFocusQuit
            | OpenOptionsModal { .. } | ApplyOptionsConfirmed { .. }
            | OptionsNavMove(..) | OptionsNavFocus(_) | OptionsNavActivate
            | OptionsKbMove(..) | OptionsKbActivate | OptionsKbClose
            | PreviewMusicVolume(_) => {
                return self.handle_menu(cmd);
            }

            // Info screen (read-only menu overlay): `Some(section)` = open on that section.
            OpenInfoScreen(section) => {
                self.session.menu_state.info_section = Some(section);
                self.session.menu_state.rank_selected = 0;
                // The Updates page opens with its first action focused (the main button), so one
                // press of Confirm does the obvious thing; the other pages open with none.
                let first_action = matches!(section, crate::app::menu_tree::InfoSection::Updates)
                    && !self.session.info_actions().is_empty();
                self.session.menu_state.info_action_selected = usize::from(first_action);
                self.session.menu_state.info_focus_back = false; // content focused by default
                self.session.menu_state.info_scroll_delta = 0.0;
            }
            CloseInfoScreen => { self.session.menu_state.info_section = None; }
            ToggleUpdateChecks => {
                let cfg = &mut self.session.user_config;
                cfg.update_checks_enabled = !cfg.update_checks_enabled;
                cfg.save();
            }
            InfoActionSelectMove(delta) => {
                let count = self.session.info_actions().len();
                let loops = matches!(self.session.menu_state.info_section,
                    Some(crate::app::menu_tree::InfoSection::Updates));
                let n = info_action_step(
                    self.session.menu_state.info_action_selected, count, delta, loops);
                if n != self.session.menu_state.info_action_selected {
                    self.session.menu_state.info_action_selected = n;
                    self.session.menu_state.info_action_scroll_pending.set(true);
                    return self.apply_app_command(AppCommand::PlayUiNavigate);
                }
            }
            InfoActionSelectSet(i) => {
                self.session.menu_state.info_action_selected = i;
            }
            OpenUrl(url) => {
                #[cfg(feature = "native")]
                if let Err(e) = open::that(&url) {
                    log::warn!("Cannot open {url}: {e}");
                }
                #[cfg(not(feature = "native"))]
                let _ = url;
            }
            // "Check now" button (Info page): ignores the check interval and a disabled
            // checkbox; no effect while an attempt is in progress.
            CheckUpdatesNow => {
                #[cfg(feature = "update-check")]
                self.start_update_check();
            }
            ScrollInfo(d) => { self.session.menu_state.info_scroll_delta = d; }
            SetInfoFocusBack(b) => { self.session.menu_state.info_focus_back = b; }

            // Network rank page: live opponents
            RankSelectMove(delta) => {
                let count = self.build_rank_page_data().opponents.len();
                if count > 0 {
                    // Scrollable list: vertical movement stops at the ends.
                    let n = crate::app::focus::clamp_move(
                        self.session.menu_state.rank_selected, count, delta);
                    if n != self.session.menu_state.rank_selected {
                        self.session.menu_state.rank_selected = n;
                        self.session.menu_state.rank_scroll_pending.set(true);
                        return self.apply_app_command(AppCommand::PlayUiNavigate);
                    }
                }
            }
            RankSelectSet(i) => {
                self.session.menu_state.rank_selected = i;
            }
            ReconnectOpponent(index) => {
                // Quick reconnect: opens the LAN screen (with discovery) and arms an invite.
                // A same-name peer visible on mDNS gives a fresh address; otherwise, when the
                // grace delay expires, the address stored in the log is used (fallback).
                let data = self.build_rank_page_data();
                let Some(row) = data.opponents.get(index) else { return None };
                if !row.reachable {
                    return None;
                }
                // Fallback target: peer metadata stored in the log, else the online mDNS
                // announcement carrying its uuid (a row is actionable when only "online").
                let stored = self.session.ladder.player_meta(&row.id).and_then(|m| {
                    match (m.last_addr.clone(), m.last_port) {
                        (Some(a), Some(p)) => a.parse::<std::net::IpAddr>().ok().map(|ip| (ip, p)),
                        _ => None,
                    }
                });
                let fresh = self.session.negotiation.net_peers.iter()
                    .find(|p| p.uuid_hex.as_deref() == Some(row.id.to_hex().as_str()))
                    .map(|p| (p.addr, p.port));
                let Some((addr, port)) = stored.or(fresh) else { return None };
                let name = row.name.clone();
                self.session.reconnect_return_to_opponents = true;
                log::info!("Quick reconnect to {name} ({addr}:{port}) — dedicated screen, repeated attempts");
                // Leave the Info page and open the LAN screen (auto) in the reconnecting phase:
                // a dedicated screen (instruction to the peer) with repeated attempts.
                self.apply_app_command(AppCommand::CloseInfoScreen);
                self.apply_app_command(AppCommand::OpenLanScreen { manual: false });
                if let Some(ref mut screen) = self.session.menu_state.lan_screen {
                    screen.phase = crate::app::meta_state::LanScreenPhase::Reconnecting {
                        peer_name: name.clone(),
                    };
                    screen.selected = 0;
                    screen.pending_reconnect =
                        Some(crate::app::meta_state::PendingReconnect {
                            name,
                            uuid_hex: row.id.to_hex(),
                            addr,
                            port,
                            // Short grace delay: let mDNS report a fresh address before
                            // the first attempt.
                            next_attempt: std::time::Instant::now()
                                + std::time::Duration::from_millis(1200),
                        });
                }
                return self.apply_app_command(AppCommand::PlayUiConfirm);
            }

            // Network / LAN
            OpenLanScreen { .. } | CloseLanScreen | ConnectToPeer { .. }
            | AcceptInvite | DeclineInvite | CancelInvite | ReconnectToManual
            | LanSelectMove(_)
            | ManualNavMove(_) | ManualNavActivate
            | ManualKbMove(..) | ManualKbActivate | ManualKbClose => {
                return self.handle_network(cmd);
            }

            // Pause
            OpenPauseMenu | ResumeGame
            | SignalResumeReady | QuitToMenu => {
                return self.handle_pause(cmd);
            }

            // Game lifecycle
            ClearActiveGame | SelectMode(..) | CreateGame { .. }
            | SignalLocalReady => {
                return self.handle_game(cmd);
            }

            // Residual: renderer, UI, debug, or window
            other @ (ResetForMenu | ClearRoundEffects | CloseOptionsModal
                    | RefreshNameplates
                    | PreviewNameplate { .. }
                    | SetFullscreen(_)
                    | CloseGameCurtain(_)
                    | EnsureGameCurtainOpen
                    | RequestExit | RequestRedraw) => {
                return Some(other);
            }
        }
        None
    }

    fn handle_audio(&mut self, cmd: AppCommand) {
        /// Fade of the menu music when the game area opens: 1 s at 50 Hz.
        const MUSIC_FADE_TICKS: u32 = 50;
        use AppCommand::*;
        match cmd {
            PlayMusic(music) => {
                let track = match music {
                    AppMusic::Prelude => MusicTrack::Prelude,
                };
                if let Some(ref mut snd) = self.sound_manager {
                    // Back at the menu while the menu music is already playing (chained after
                    // a win/loss): it continues instead of restarting.
                    let already = snd.current_track() == Some(track)
                        && !snd.is_music_finished() && !snd.is_music_fading();
                    if !already {
                        snd.play_music(track);
                    }
                }
            }
            FadeOutMusic => {
                if let Some(ref mut snd) = self.sound_manager { snd.fade_out_music(MUSIC_FADE_TICKS); }
            }
            PlayUiConfirm => {
                if let Some(ref snd) = self.sound_manager { snd.play_ui_confirm(); }
            }
            PlayUiNavigate => {
                if let Some(ref snd) = self.sound_manager { snd.play_ui_navigate(); }
            }
            _ => {}
        }
    }

    fn handle_menu(&mut self, cmd: AppCommand) -> Option<AppCommand> {
        use AppCommand::*;
        match cmd {
            MenuNavigate(dir) => {
                use crate::app::command::Direction;
                let moved = match dir {
                    Direction::Up => self.session.menu_state.move_up(),
                    Direction::Down => self.session.menu_state.move_down(),
                };
                // Sound iff movement: never a click without a move.
                if moved {
                    return self.apply_app_command(AppCommand::PlayUiNavigate);
                }
            }
            MenuBack => { self.session.menu_state.back(); }
            ConfirmMenuSelection => { self.session.menu_state.confirm(); }
            MenuSelectIndex(i) => { self.session.menu_state.select_index(i); }
            MenuFocusQuit => {
                // Quit is the last item of the root menu. Sound only if the focus moves.
                let last = self.session.menu_state.item_count().saturating_sub(1);
                if self.session.menu_state.selected != last {
                    self.session.menu_state.select_index(last);
                    return self.apply_app_command(AppCommand::PlayUiNavigate);
                }
            }
            OpenOptionsModal { tab } => {
                self.session.menu_state.try_open_options(
                    tab,
                    &self.session.player_name,
                    self.session.user_config.sfx_enabled,
                    self.session.user_config.music_enabled,
                    self.session.user_config.sfx_volume,
                    self.session.user_config.music_volume,
                    self.session.user_config.language.as_deref(),
                    self.session.user_config.fullscreen,
                    self.session.user_config.key_bindings.clone(),
                );
                let music_available = self.sound_manager.as_ref().is_some_and(|s| s.music_available());
                if let Some(modal) = self.session.menu_state.options_modal.as_mut() {
                    modal.music_available = music_available;
                }
            }
            ApplyOptionsConfirmed { player_name, sfx_enabled, music_enabled, sfx_volume, music_volume, language, fullscreen, key_bindings } => {
                let fullscreen_changed = fullscreen != self.session.user_config.fullscreen;
                self.session.user_config.sfx_enabled = sfx_enabled;
                self.session.user_config.music_enabled = music_enabled;
                self.session.user_config.sfx_volume = sfx_volume;
                self.session.user_config.music_volume = music_volume;
                self.session.user_config.language = language.clone();
                self.session.user_config.fullscreen = fullscreen;
                self.session.user_config.key_bindings = key_bindings;
                // Apply language change immediately
                if let Some(ref code) = language {
                    if let Some(lang) = crate::app::locale::Language::from_code(code) {
                        crate::app::locale::set_language(lang);
                    }
                } else {
                    // Auto: re-detect from the system.
                    crate::app::locale::set_language(
                        crate::detect_system_language().unwrap_or(crate::app::locale::Language::English));
                }
                if let Some(ref mut snd) = self.sound_manager {
                    snd.sfx_enabled = sfx_enabled;
                    snd.set_sfx_volume(sfx_volume);
                    snd.music_enabled = music_enabled;
                    snd.set_music_volume(music_volume);
                    if music_enabled && snd.current_track().is_none() {
                        snd.play_music(MusicTrack::Prelude);
                    }
                    if !music_enabled { snd.stop_music(); }
                }
                // Anti-caps-lock correction is done upstream by `InputFormat::finalize`
                // (`onscreen_keyboard.rs`), the only producer of this field. Do not repeat it
                // here: a byte-indexed correction panics on a name without an ASCII letter
                // (e.g. injected by state.json or `--set`).
                self.session.player_name = player_name;
                self.session.user_config.player_name = self.session.player_name.clone();
                self.session.user_config.save();
                self.session.menu_state.options_modal = None;
                // A fullscreen change is a separate residual: `SetFullscreen` is returned as the
                // primary one; the caller also triggers `RefreshNameplates`.
                if fullscreen_changed {
                    return Some(AppCommand::SetFullscreen(fullscreen));
                }
                return Some(AppCommand::RefreshNameplates);
            }
            PreviewMusicVolume(vol) => {
                if let Some(ref mut snd) = self.sound_manager {
                    snd.set_music_volume(vol);
                    // Start prelude if music was off and slider moved above 0
                    if vol > 0.0 && snd.current_track().is_none() {
                        snd.play_music(MusicTrack::Prelude);
                    }
                    if vol == 0.0 { snd.stop_music(); }
                }
            }
            OptionsNavMove(dx, dy) => {
                let moved = self.session.menu_state.options_modal.as_mut().is_some_and(|m| {
                    let before = m.nav_focus;
                    m.opt_move(dx, dy);
                    m.nav_focus != before
                });
                self.preview_options_sound();
                // Sound iff movement: adjusting a value (Left/Right) moves nothing.
                if moved {
                    return self.apply_app_command(AppCommand::PlayUiNavigate);
                }
            }
            OptionsNavActivate => {
                if let Some(ref mut m) = self.session.menu_state.options_modal {
                    m.opt_activate();
                }
                self.preview_options_sound();
            }
            OptionsNavFocus(w) => {
                if let Some(ref mut m) = self.session.menu_state.options_modal {
                    m.focus_widget(w);
                }
            }
            OptionsKbMove(dx, dy) => {
                let moved = self.session.menu_state.options_modal.as_mut().is_some_and(|m| {
                    let before = m.kb_focus;
                    m.kb_move(dx, dy);
                    m.kb_focus != before
                });
                if moved {
                    return self.apply_app_command(AppCommand::PlayUiNavigate);
                }
            }
            OptionsKbActivate => {
                if let Some(ref mut m) = self.session.menu_state.options_modal {
                    m.kb_activate();
                }
            }
            OptionsKbClose => {
                if let Some(ref mut m) = self.session.menu_state.options_modal {
                    m.kb_close();
                }
            }
            _ => {}
        }
        None
    }

    /// Live audio preview of the volumes while the Sound tab is adjusted with gamepad or
    /// keyboard (the mouse already gets it from the sliders). Volumes only: music is never
    /// started or stopped here, to avoid a surprise playback while navigating. Enabling
    /// music/SFX applies when the options are confirmed.
    fn preview_options_sound(&mut self) {
        use crate::app::meta_state::OptionsTab;
        let vols = match self.session.menu_state.options_modal.as_ref() {
            Some(m) if m.tab == OptionsTab::Sound => Some((m.sfx_volume, m.music_volume)),
            _ => None,
        };
        if let (Some((sfx, music)), Some(snd)) = (vols, self.sound_manager.as_mut()) {
            snd.set_sfx_volume(sfx);
            snd.set_music_volume(music);
        }
    }

    /// Applies a network/LAN command. May return a residual for `main.rs` (clipboard of the
    /// manual screen: `LanPasteRequest` / `CopyToClipboard`).
    fn handle_network(&mut self, cmd: AppCommand) -> Option<AppCommand> {
        use AppCommand::*;
        match cmd {
            OpenLanScreen { manual } => {
                use crate::app::meta_state::LanScreenState;
                // Zone services (TCP listener and mDNS) are managed per zone
                // (`sync_network_zone_services`). If they already run, open the screen on them
                // without rebinding.
                if self.session.negotiation.tcp_listener.is_some() {
                    let port = self.session.negotiation.local_listen_port();
                    self.session.menu_state.lan_screen = Some(LanScreenState {
                        manual_open: manual,
                        ..LanScreenState::new(port)
                    });
                    if let Some(ref mut screen) = self.session.menu_state.lan_screen {
                        screen.set_peers(self.session.negotiation.lan_list());
                    }
                    log::info!("LAN screen opened on zone services (TCP port {port}, manual={manual})");
                    return None;
                }
                // Fallback (zone not synced yet): bind directly, with mDNS in both modes.
                let rating = self.my_announced_rating();
                match self.session.negotiation.start_zone_services(
                    &self.session.player_name,
                    rating,
                    &self.session.user_config.player_uuid,
                    self.session.requested_listen_port,
                ) {
                    Ok((port, Ok(()))) => {
                        self.session.menu_state.lan_screen = Some(LanScreenState {
                            manual_open: manual,
                            ..LanScreenState::new(port)
                        });
                        log::info!(
                            "LAN screen opened (TCP port {}, discovery started, manual={})",
                            port, manual
                        );
                    }
                    Ok((port, Err(e))) => {
                        // Discovery can fail (network/OS) and the screen stays usable: in manual
                        // mode continue without mDNS; in auto mode show the error (the list would
                        // otherwise be empty with no explanation).
                        log::error!("Failed to start discovery: {}", e);
                        if manual {
                            self.session.menu_state.lan_screen = Some(LanScreenState {
                                manual_open: true,
                                ..LanScreenState::new(port)
                            });
                        } else {
                            self.session.menu_state.lan_screen = Some(LanScreenState {
                                phase: crate::app::meta_state::LanScreenPhase::error(
                                    crate::app::meta_state::LanError::Discovery(e.to_string())),
                                ..LanScreenState::new(port)
                            });
                        }
                    }
                    Err(e) => {
                        log::error!("Failed to bind TCP: {}", e);
                        self.session.menu_state.lan_screen = Some(LanScreenState {
                            phase: crate::app::meta_state::LanScreenPhase::error(
                                crate::app::meta_state::LanError::PortBind(e.to_string())),
                            ..LanScreenState::new(crackattack_net::connection::TCP_PORT_START)
                        });
                    }
                }
            }
            CloseLanScreen => {
                // The services (listener and mDNS) are kept: they are managed per zone and stop
                // when the network corner is left (`sync_network_zone_services`).
                self.session.menu_state.lan_screen = None;
                log::info!("LAN screen closed (zone services kept)");
            }
            LanSelectMove(delta) => {
                // Moves the selection (keyboard/gamepad), clamped to the list.
                if let Some(ref mut screen) = self.session.menu_state.lan_screen {
                    let before = screen.selected;
                    screen.move_selection(delta);
                    // Sound iff movement (a one-item ring does not move).
                    if screen.selected != before {
                        return self.apply_app_command(AppCommand::PlayUiNavigate);
                    }
                }
            }
            ConnectToPeer { addr, port } => {
                // Self-invite guard: targeting our own listener (loopback, or our local IP with
                // our listen port) would be refused as "busy", a confusing message. A stale
                // stored port can fall back onto us.
                let self_target = port == self.session.negotiation.local_listen_port() && (
                    addr.is_loopback()
                        || crackattack_net::discovery::detect_local_network()
                            .is_some_and(|n| std::net::IpAddr::V4(n.ip) == addr)
                );
                if self_target {
                    use crate::app::meta_state::LanScreenPhase;
                    let reconnecting = self.session.menu_state.lan_screen.as_ref()
                        .is_some_and(|s| matches!(s.phase, LanScreenPhase::Reconnecting { .. }));
                    if reconnecting {
                        // Stale stored port fell back onto us: skip this attempt. Repeats
                        // continue (fresh mDNS will take over).
                        log::info!("Reconnect: target is our own listener ({addr}:{port}) — attempt skipped");
                    } else {
                        log::warn!("Invitation to our own address ({addr}:{port}) — refused");
                        if let Some(ref mut screen) = self.session.menu_state.lan_screen {
                            if screen.manual_open {
                                // The error must live in the manual dialog: an Error phase
                                // would be hidden behind it. (The dialog field is a String,
                                // translated at construction; local path.)
                                screen.manual_error =
                                    Some(crate::app::locale::t().own_address_error.to_string());
                            } else {
                                screen.phase = LanScreenPhase::error(crate::app::meta_state::LanError::OwnAddress);
                            }
                        }
                    }
                    return None;
                }
                // Keep the listener active while waiting: any concurrent request is
                // auto-refused `Busy` (see `poll_tcp_listener`) instead of being ignored.
                // In the reconnecting phase the dedicated screen stays displayed (repeated
                // attempts): no switch to `Inviting`.
                let reconnecting = self.session.menu_state.lan_screen.as_ref().is_some_and(|s| matches!(
                    s.phase, crate::app::meta_state::LanScreenPhase::Reconnecting { .. }));
                let reconnect_name = self.session.menu_state.lan_screen.as_ref()
                    .and_then(|s| s.pending_reconnect.as_ref())
                    .map(|p| p.name.clone());
                let peer_name = self.session.menu_state.lan_screen.as_ref()
                    .and_then(|s| s.peers().iter().find(|p| p.addr == addr && p.port == port))
                    .map(|p| p.name.clone())
                    .or(reconnect_name)
                    .unwrap_or_else(|| format!("{}:{}", addr, port));
                if let Some(ref mut screen) = self.session.menu_state.lan_screen {
                    // Close the manual connection screen if open: switch to the normal modal
                    // flow (wait, connect, game/error), like auto.
                    screen.manual_open = false;
                    screen.manual_error = None;
                    if !reconnecting {
                        screen.pending_reconnect = None;
                        screen.phase = crate::app::meta_state::LanScreenPhase::Inviting {
                            peer_name: peer_name.clone(),
                        };
                    }
                }
                // Freeze the self-announced rating of the peer we invite (recorded in the log if
                // the match happens). `None` if not found (manual connection).
                self.session.negotiation.pending_announced_rt = self.session.menu_state.lan_screen.as_ref()
                    .and_then(|s| s.peers().iter().find(|p| p.addr == addr && p.port == port))
                    .and_then(|p| p.announced_rating);
                // The "busy" status follows from the `Inviting` phase (the mDNS status is
                // derived: no manual resync here).
                let our_rating = self.my_announced_rating();
                self.session.negotiation.start_outgoing_invite(
                    addr.to_string(),
                    port,
                    self.session.player_name.clone(),
                    self.session.user_config.player_uuid_bytes(),
                    our_rating,
                    self.session.game_seed,
                );
                log::info!("Connecting to peer {} at {}:{}", peer_name, addr, port);
            }
            AcceptInvite => {
                // The `Accepting` phase also carries the cancel flag; the mDNS `InGame` status
                // follows from the phase.
                self.session.negotiation.accept_pending_invite(
                    self.session.player_name.clone(),
                    self.session.user_config.player_uuid_bytes(),
                    self.session.game_seed,
                );
            }
            DeclineInvite => {
                // End of negotiation: the derived status goes back to `Available`.
                self.session.negotiation.decline_pending_invite();
            }
            ReconnectToManual => {
                // Switch to "Manual connection" from the Reconnecting screen, with the stored
                // IP/port prefilled (the user can fix the port). Cancels the attempt in progress
                // and stops the repeats. Explicitly choosing manual disables the automatic
                // return to the opponents page.
                self.session.reconnect_return_to_opponents = false;
                self.session.negotiation.cancel_active();
                if let Some(ref mut screen) = self.session.menu_state.lan_screen {
                    // Fresh manual screen, prefilled directly from the session state.
                    screen.manual = Default::default();
                    if let Some(pr) = screen.pending_reconnect.take() {
                        screen.manual.set_ip(pr.addr);
                        screen.manual.port = pr.port.to_string();
                    }
                    screen.phase = crate::app::meta_state::LanScreenPhase::Scanning;
                    screen.manual_open = true;
                    // Reopening the manual panel re-detects the network.
                    screen.manual_net_info = None;
                }
            }
            CancelInvite => {
                // Cancels the active negotiation: signal the connection thread, which closes the
                // socket (between two read timeouts). The guest sees the close and leaves its
                // modal (symmetric exit). The derived status goes back to `Available` and we
                // return to the list.
                self.session.negotiation.cancel_active();
                use crate::app::meta_state::LanScreenPhase;
                let reconnecting = self.session.menu_state.lan_screen.as_ref()
                    .is_some_and(|s| matches!(s.phase, LanScreenPhase::Reconnecting { .. }));
                if reconnecting {
                    // Reconnect abandoned: return to the recent opponents page if it was
                    // launched from there, else close the LAN screen normally.
                    self.end_reconnect();
                } else if let Some(ref mut screen) = self.session.menu_state.lan_screen {
                    screen.phase = LanScreenPhase::Scanning;
                }
            }

            // --- Manual connection screen: focus and input live in the session state
            //     (`screen.manual`); keyboard/gamepad navigation goes through `route_lan_intent`. ---
            ManualNavMove(delta) => {
                if let Some(ref mut screen) = self.session.menu_state.lan_screen {
                    if screen.manual_open {
                        // `Copy` is in the focus ring only if a local address is detected.
                        let has_copy = matches!(screen.manual_net_info, Some(Some(_)));
                        let before = screen.manual.nav_focus;
                        screen.manual.nav_move(has_copy, delta);
                        if screen.manual.nav_focus != before {
                            return self.apply_app_command(AppCommand::PlayUiNavigate);
                        }
                    }
                }
            }
            ManualNavActivate => {
                use crate::app::manual_connect::{ManualField, ManualFocus};
                let Some(ref mut screen) = self.session.menu_state.lan_screen else { return None };
                if !screen.manual_open || screen.manual.editing.is_some() {
                    return None;
                }
                match screen.manual.nav_focus {
                    ManualFocus::Copy => {
                        // Copy "ip:port"; writing the clipboard is a platform job, so residual.
                        if let Some(Some(ref info)) = screen.manual_net_info {
                            return Some(AppCommand::CopyToClipboard(
                                format!("{}:{}", info.ip, screen.local_port)));
                        }
                    }
                    ManualFocus::Ipv4 => screen.manual.open_editor(ManualField::Ipv4),
                    ManualFocus::Ipv6 => screen.manual.open_editor(ManualField::Ipv6),
                    ManualFocus::Port => screen.manual.open_editor(ManualField::Port),
                    // The shell reads the clipboard, so this is a residual (main); the result is
                    // dispatched straight into `screen.manual` on return.
                    ManualFocus::Paste => return Some(AppCommand::LanPasteRequest),
                    ManualFocus::Connect => {
                        if let Some((addr, port)) = screen.manual.connect_target() {
                            return self.apply_app_command(AppCommand::ConnectToPeer { addr, port });
                        }
                    }
                    ManualFocus::Back => {
                        return self.apply_app_command(AppCommand::CloseLanScreen);
                    }
                }
            }
            ManualKbMove(dx, dy) => {
                let moved = self.session.menu_state.lan_screen.as_mut().is_some_and(|screen| {
                    let before = screen.manual.kb_focus;
                    screen.manual.kb_move(dx, dy);
                    screen.manual.kb_focus != before
                });
                if moved {
                    return self.apply_app_command(AppCommand::PlayUiNavigate);
                }
            }
            ManualKbActivate => {
                if let Some(ref mut screen) = self.session.menu_state.lan_screen {
                    screen.manual.kb_activate();
                }
            }
            ManualKbClose => {
                if let Some(ref mut screen) = self.session.menu_state.lan_screen {
                    screen.manual.kb_close();
                }
            }
            _ => {}
        }
        None
    }

    /// Ends the reconnection (abandon, refusal or final failure): closes the LAN screen and,
    /// if it was launched from the recent opponents page, returns there.
    fn end_reconnect(&mut self) {
        if let Some(ref mut screen) = self.session.menu_state.lan_screen {
            screen.pending_reconnect = None;
        }
        self.session.menu_state.lan_screen = None;
        if self.session.reconnect_return_to_opponents {
            self.session.reconnect_return_to_opponents = false;
            self.session.menu_state.info_section =
                Some(crate::app::menu_tree::InfoSection::NetworkOpponents);
            self.session.menu_state.info_focus_back = false;
            self.session.menu_state.rank_scroll_pending.set(true);
        }
    }

    /// Starts or stops the network services depending on presence in the VS network zone
    /// (at the menu, outside a game). The LAN screen reuses these services instead of
    /// rebinding. The mechanics live in `negotiation`; only `in_zone` is computed here.
    fn sync_network_zone_services(&mut self) {
        let in_zone = matches!(self.session.flow_state, FlowState::Menu)
            && self.session.game.is_none()
            && self.session.menu_state.in_network_zone();
        let rating = self.my_announced_rating();
        self.session.negotiation.sync_zone_services(
            in_zone,
            &self.session.player_name,
            rating,
            &self.session.user_config.player_uuid,
            self.session.requested_listen_port,
        );
    }

    /// Data of the network rank page (same as the renderer) to clamp the selection and
    /// resolve a row activation.
    fn build_rank_page_data(&self) -> crate::app::ladder_view::RankPageData {
        use crate::app::ladder_view::{build, OpponentContact};
        let me = crackattack_rating::PlayerId(self.session.user_config.player_uuid_bytes());
        let names = |id: &crackattack_rating::PlayerId| {
            self.session.ladder.display_name(id).map(str::to_string)
        };
        let contact = |id: &crackattack_rating::PlayerId| {
            let online = self.session.negotiation.net_peers.iter()
                .any(|p| p.uuid_hex.as_deref() == Some(id.to_hex().as_str()));
            let mut c = self.session.ladder.player_meta(id).map(|m| OpponentContact {
                last_rt: m.last_announced_rt,
                reachable: m.last_addr.is_some() && m.last_port.is_some(),
                online: false,
            }).unwrap_or_default();
            c.online = online;
            c.reachable = c.reachable || online;
            c
        };
        build(
            self.session.ladder.entries(),
            &self.session.ladder_records,
            &names,
            &contact,
            me,
            &crackattack_rating::RatingConfig::default(),
        )
    }

    /// My rating to self-announce in discovery (TXT `rt`): rounded, and only if placed (RD
    /// under the threshold and enough matches). Otherwise `None` (provisional or unknown).
    fn my_announced_rating(&self) -> Option<i32> {
        let me = crackattack_rating::PlayerId(self.session.user_config.player_uuid_bytes());
        let cfg = crackattack_rating::RatingConfig::default();
        self.session.ladder_records.get(&me)
            .filter(|r| {
                r.rating.rd <= cfg.display_rd_threshold
                    && (r.wins + r.losses + r.draws) >= cfg.min_matches_for_ladder
            })
            .map(|r| r.rating.value.round() as i32)
    }

    /// Records a refereed round in the log, one entry per round. Only for VS network with
    /// the peer's identity (the local AI has no uuid and is never rated). A game played by the autoplay is not rated either. A forfeit caused by
    /// the opponent disconnecting is not rated: no refereed round, so no free win on a crash.
    /// After the append the ranking is recomputed from the log (never stored).
    fn record_rated_round(&mut self, won: bool) {
        use crackattack_rating::{MatchResult, Outcome, PlayerId};
        if !self.session.selected_mode.is_network() || self.session.autoplay_enabled {
            return;
        }
        let Some(snap) = self.session.game.as_ref().map(|s| s.snapshot()) else { return };
        let Some(vs) = snap.vs_display else { return };
        let Some(opp_uuid) = vs.opponent_uuid else { return };
        let me = PlayerId(self.session.user_config.player_uuid_bytes());
        let opp = PlayerId(opp_uuid);
        if me == opp {
            // Loopback on the same machine (shared state.json): a self-match is worthless
            // for the ranking (recompute would ignore it); do not pollute the log.
            log::info!("Ladder: round not rated (same identity on both sides — loopback)");
            return;
        }
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let entry = MatchResult {
            a: me,
            b: opp,
            outcome: if won { Outcome::WinA } else { Outcome::WinB },
            ts,
        };
        let cfg = crackattack_rating::RatingConfig::default();
        // Round delta = my value after minus before (before = current recompute state; the default value for a first match).
        let before = self.session.ladder_records.get(&me)
            .map(|r| r.rating.value)
            .unwrap_or(cfg.default_value);
        // Reconnection metadata of the peer (address, listen port, announced rt), merged into
        // its `players` record in the log (outside the checksum).
        let contact = self.session.opponent_net_info.as_ref().map(|ni| {
            (opp, crate::platform::ladder::ContactInfo {
                addr: Some(ni.addr.clone()),
                listen_port: ni.listen_port,
                seen_ts: ts,
                announced_rt: self.session.opponent_announced_rt,
            })
        });
        if !self.session.ladder.append(entry, &self.session.player_name, &vs.opponent_name, contact) {
            log::error!("Ladder: this match result was not saved");
        }
        self.session.ladder_records =
            crackattack_rating::recompute(self.session.ladder.entries(), &cfg);
        let after = self.session.ladder_records.get(&me)
            .map(|r| r.rating.value)
            .unwrap_or(cfg.default_value);
        let delta = after - before;
        self.session.ladder_round_delta = Some(delta);
        self.session.ladder_match_delta = Some(self.session.ladder_match_delta.unwrap_or(0.0) + delta);
        log::info!(
            "Ladder: round rated ({}) vs {} — {:+.0} → {:.0} ({} entries)",
            if won { "win" } else { "loss" },
            vs.opponent_name,
            delta,
            after,
            self.session.ladder.entries().len(),
        );
    }

    fn handle_pause(&mut self, cmd: AppCommand) -> Option<AppCommand> {
        use AppCommand::*;
        match cmd {
            OpenPauseMenu => {
                if let Some(ref mut session) = self.session.game {
                    let intr = session.snapshot().interruption;
                    match intr {
                        crackattack_core::frame_data::SessionInterruption::None
                        | crackattack_core::frame_data::SessionInterruption::ReadyPrompt { .. } => {
                            self.session.pause_initiated_locally = true;
                            session.request_pause();
                            // Open the menu immediately (like the other paths). Otherwise the
                            // visibility sync would see it appear "without a local action" and reset
                            // `pause_initiated_locally` to false, giving a false anti-reflex delay
                            // on resume of our OWN pause in network mode.
                            self.session.pause_menu_open = true;
                            self.session.pause_menu_opened_at = Some(std::time::Instant::now());
                            self.session.pause_menu_selection.set(0);
                        }
                        crackattack_core::frame_data::SessionInterruption::ConnectionLost { .. }
                        | crackattack_core::frame_data::SessionInterruption::WaitingRemoteReady { .. } => {
                            self.session.pause_initiated_locally = true;
                            self.session.pause_menu_open = true;
                            self.session.pause_menu_opened_at = Some(std::time::Instant::now());
                            self.session.pause_menu_selection.set(0);
                        }
                        _ => {}
                    }
                }
                return Some(AppCommand::RequestRedraw);
            }
            ResumeGame => {
                if let Some(ref mut session) = self.session.game {
                    session.request_resume();
                }
                return Some(AppCommand::RequestRedraw);
            }
            SignalResumeReady => {
                if let Some(ref mut session) = self.session.game {
                    session.request_resume();
                }
                return Some(AppCommand::RequestRedraw);
            }
            QuitToMenu => {
                if self.session.leaving_game {
                    return None;
                }
                self.session.leaving_game = self.session.game.is_some();
                // Ladder: abandoning a round in progress (countdown included) counts as a rated
                // loss, otherwise quitting would dodge a defeat. Quitting at the "Ready?" prompt
                // or after a round rates nothing (no round engaged). `round_engaged` comes from
                // the session PHASE, not the lifecycle, which folds the waits into Playing.
                let mid_round = self.session.game.as_ref()
                    .map_or(false, |s| s.snapshot().round_engaged);
                if mid_round {
                    self.record_rated_round(false);
                }
                if let Some(ref mut session) = self.session.game {
                    session.abandon();
                }
                // Leaving the game: close any abandon confirmation so it does not leak into the
                // next game (the next "Ready?" would show it).
                self.session.post_game_input.abandon_confirm_open = false;
                self.session.post_game_input.abandon_confirm_yes = false;
                return self.apply_app_command(AppCommand::StartTransition(FlowState::Menu, 0.5));
            }
            _ => {}
        }
        None
    }

    fn handle_game(&mut self, cmd: AppCommand) -> Option<AppCommand> {
        use AppCommand::*;
        match cmd {
            ClearActiveGame => {
                self.session.game = None;
                self.session.leaving_game = false;
                self.session.opponent_net_info = None;
                self.session.opponent_announced_rt = None;
                self.session.game_over_stats = None;
                self.session.game_over_rank = None;
                self.session.game_over_beaten_records.clear();
                self.session.round_end_stats = None;
                self.session.post_game_screen = crate::app::post_game::PostGameScreen::default();
                self.session.post_game_fx.reset();
                self.session.menu_state = MenuState::new();
            }
            SelectMode(mode) => { self.session.selected_mode = mode; }
            SignalLocalReady => {
                if let Some(ref mut session) = self.session.game {
                    session.signal_local_ready();
                }
            }
            CreateGame { mode, seed, ranking_entry_score } => {
                let session: Box<dyn GameSession> = match mode {
                    GameMode::Solo => {
                        let mut g = GameState::with_scoring(seed, ScoringMode::Classic);
                        g.ranking_entry_score = Some(if ranking_entry_score > 0 { ranking_entry_score } else { 1 });
                        Box::new(SoloSession::new(g))
                    }
                    GameMode::VsComputer(diff) => {
                        let mut vs = VsComputerGameSession::new(seed, GridOpponent::new(diff, seed));
                        vs.set_needs_ready_prompt(true);
                        Box::new(vs)
                    }
                    GameMode::VsNetwork => {
                        log::warn!("CreateGame with VsNetwork — use LAN flow instead");
                        return None;
                    }
                };
                self.session.game = Some(session);
                // Clean slate for the new game (post-game focus, abandon confirmation, etc.).
                self.session.post_game_input = PostGameInput::default();
                self.session.ladder_round_delta = None;
                self.session.ladder_match_delta = None;
                return Some(AppCommand::RefreshNameplates);
            }
            _ => {}
        }
        None
    }

    fn handle_post_game(&mut self, cmd: AppCommand) -> Option<AppCommand> {
        use AppCommand::*;
        match cmd {
            AdvancePostGameScreen => {
                use crate::app::post_game::PostGameScreen;
                let uses_score = self.session.selected_mode.uses_score();
                self.session.post_game_screen = match self.session.post_game_screen {
                    PostGameScreen::ResultMessage => {
                        // The high scores body (`draw_high_scores_body`) recenters on the
                        // player's rank itself: no position to initialize here.
                        if uses_score { PostGameScreen::HighScores } else { PostGameScreen::Stats }
                    }
                    PostGameScreen::HighScores => PostGameScreen::Stats,
                    PostGameScreen::Stats => PostGameScreen::Stats,
                };
                // New screen: no focused button (Enter = default action).
                self.session.post_game_input.nav_focus = Default::default();
            }
            EnterPostGame => {
                let won = self.session.game.as_ref()
                    .map_or(false, |s| matches!(s.snapshot().result,
                        crackattack_core::frame_data::SessionResult::SessionWin
                        | crackattack_core::frame_data::SessionResult::RoundWin
                    ));
                self.capture_post_game(won);
                let is_solo = self.session.selected_mode.uses_score();
                self.session.post_game_screen = if is_solo {
                    // The `ScrollArea` recenters on the player's rank: nothing to initialize here.
                    crate::app::post_game::PostGameScreen::HighScores
                } else {
                    crate::app::post_game::PostGameScreen::Stats
                };
                self.session.post_game_input.nav_focus = Default::default();
                // Anti-reflex: ~0.6 s during which confirmation is ignored, so a key mashed at
                // the end of the game does not skip the stats screen.
                self.session.transition_cooldown = 30;
                self.session.set_flow(FlowState::PostGame);
            }
            _ => {}
        }
        None
    }

    /// Drains the mDNS events (`negotiation` module) and mirrors the peer list to the LAN
    /// screen if open (its list and selection depend on it).
    fn poll_lan_discovery(&mut self) {
        if self.session.negotiation.poll_discovery_events() {
            if let Some(ref mut screen) = self.session.menu_state.lan_screen {
                screen.set_peers(self.session.negotiation.lan_list());
            }
        }
    }

    /// Drains the threaded invite reads (`negotiation` module). Only the scope guard (same
    /// condition as `in_zone` in `sync_network_zone_services`) and the "last known rank"
    /// fallback from the log are computed here.
    fn poll_pending_invite_reads(&mut self) {
        let in_scope = matches!(self.session.flow_state, FlowState::Menu)
            && self.session.game.is_none()
            && self.session.menu_state.in_network_zone();
        let ladder = &self.session.ladder;
        let stored_rt = |uuid_hex: &str| {
            crackattack_rating::PlayerId::from_hex(uuid_hex)
                .and_then(|id| ladder.player_meta(&id))
                .and_then(|m| m.last_announced_rt)
        };
        self.session.negotiation.poll_invite_reads(in_scope, &stored_rt);
        // An instance driven by the autoplay has nobody to click "Accept": it takes its
        // invites as soon as they arrive (E2E runs between two autoplay instances).
        let awaiting_answer = self.session.negotiation.pending_invite_info.as_ref()
            .is_some_and(|info| !info.connecting);
        if self.session.autoplay_enabled && awaiting_answer {
            log::info!("Autoplay: accepting the invite");
            self.session.negotiation.accept_pending_invite(
                self.session.player_name.clone(),
                self.session.user_config.player_uuid_bytes(),
                self.session.game_seed,
            );
        }
    }

    /// Drives the Reconnecting screen: while that phase is open, the invite is retried at a
    /// regular interval. Each attempt prefers the peer's fresh mDNS address (same name,
    /// available) if visible, otherwise the stored address. The invite goes through the normal
    /// flow (`ConnectToPeer`); a failure reschedules it (see `poll_pending_connection`) and an
    /// explicit refusal stops it (no harassment).
    fn poll_pending_reconnect(&mut self, deferred: &mut Vec<AppCommand>) {
        use crackattack_net::discovery::PeerStatus;
        use crate::app::meta_state::LanScreenPhase;
        const RETRY_INTERVAL: std::time::Duration = std::time::Duration::from_secs(4);
        let Some(screen) = self.session.menu_state.lan_screen.as_ref() else { return };
        if !matches!(screen.phase, LanScreenPhase::Reconnecting { .. }) {
            return;
        }
        let Some(pr) = screen.pending_reconnect.clone() else { return };
        // One attempt at a time: while a negotiation is in flight (the other side decides)
        // do not send a second one, and do not invite while being invited ourselves. Single
        // predicate of the `negotiation` module.
        if self.session.negotiation.in_progress() {
            return;
        }
        if std::time::Instant::now() < pr.next_attempt {
            return;
        }
        // Priority: the mDNS announcement carrying the right uuid (identity, immune to
        // renames and namesakes); fallback: IP/port of the last connection (log).
        let fresh = self.session.negotiation.net_peers.iter()
            .find(|p| p.uuid_hex.as_deref() == Some(pr.uuid_hex.as_str())
                && p.status == PeerStatus::Available)
            .map(|p| (p.addr, p.port));
        let (addr, port) = match fresh {
            Some((a, p)) => {
                log::info!("Reconnect: uuid of {} seen via mDNS — fresh address {}:{}", pr.name, a, p);
                (a, p)
            }
            None => {
                log::info!("Reconnect: attempt to the stored address {}:{}", pr.addr, pr.port);
                (pr.addr, pr.port)
            }
        };
        if let Some(ref mut screen) = self.session.menu_state.lan_screen {
            if let Some(ref mut pr) = screen.pending_reconnect {
                pr.next_attempt = std::time::Instant::now() + RETRY_INTERVAL;
            }
        }
        deferred.push(AppCommand::ConnectToPeer { addr, port });
    }

    /// Polls the negotiation in flight (`negotiation` module). Creating the game (building
    /// the `VsNetworkGameSession`, switching the flow) stays HERE: the module returns the
    /// established connection and the handshake result, it does not create the game.
    fn poll_pending_connection(&mut self, deferred: &mut Vec<AppCommand>) {
        let Some(result) = self.session.negotiation.poll_connection() else { return };
        match result {
            Ok(est) => {
                // Guard (CLI `--vs network-*` mode, where the listener is armed from boot): a
                // connection that succeeds while a game is already active NEVER replaces it.
                // It is refused by dropping it (the socket closes; the peer sees a normal
                // departure). The normal flow (LAN invites) cannot get here mid-game: the LAN
                // screen only exists at the menu.
                if self.session.game.is_some() {
                    log::warn!(
                        "Network connection completed during an active game — refused");
                    return; // `est` is dropped, which closes the socket.
                }
                log::info!("Network connection established — starting game");
                self.session.negotiation.pending_invite_info = None;
                self.session.reconnect_return_to_opponents = false;
                // Build the network game (the negotiation module returned the connection and
                // the handshake; the session belongs to the runtime).
                use crackattack_net::connection::Role;
                use crackattack_net::opponent::NetworkOpponent;
                let is_server = est.role == Role::Server;
                let opponent = NetworkOpponent::new(
                    est.conn, est.role,
                    // Network boundary: the peer's name is sanitized like ours (a modified
                    // client could send characters with no nameplate glyph). Accents pass
                    // (folded at bitmap rendering).
                    crate::app::onscreen_keyboard::CharSet::sanitize_player_name(
                        &est.handshake.opponent_name),
                    est.handshake.opponent_uuid);
                let mut vs = crackattack_core::vs::VsSession::new(est.handshake.seed, opponent);
                vs.set_win_ties(is_server); // the server wins ties
                self.session.game =
                    Some(Box::new(crackattack_net::session::VsNetworkGameSession::new(vs)));
                // Peer reconnection contact and announced rt (frozen at invite time).
                self.session.opponent_net_info = Some(est.net_info);
                self.session.opponent_announced_rt =
                    self.session.negotiation.pending_announced_rt.take();
                // Clean slate for the new network game (same as solo/VS AI `CreateGame`).
                self.session.post_game_input = PostGameInput::default();
                // Ladder: reset the delta counters for this match.
                self.session.ladder_round_delta = None;
                self.session.ladder_match_delta = None;
                self.session.flow_state = FlowState::Playing;
                self.session.selected_mode = GameMode::VsNetwork;
                self.session.negotiation.discovery = None;
                self.session.negotiation.tcp_listener = None;
                self.session.menu_state.lan_screen = None;
                deferred.push(AppCommand::RefreshNameplates);
            }
            Err(failure) => {
                log::error!("Network connection failed: {}", failure);
                self.session.negotiation.pending_invite_info = None;
                // Reconnecting phase: failure is the NORMAL case (the peer may not be listening
                // yet). Stay on the dedicated screen; the next attempt is already scheduled.
                // Exception: an explicit refusal stops the attempts (no harassing someone who
                // said no).
                {
                    use crate::app::meta_state::LanScreenPhase;
                    let reconnecting = self.session.menu_state.lan_screen.as_ref()
                        .is_some_and(|s| matches!(s.phase, LanScreenPhase::Reconnecting { .. }));
                    if reconnecting {
                        if matches!(failure, ConnectFailure::Declined) {
                            // Explicit refusal: do not insist. Return to the recent opponents
                            // page (origin) rather than showing an error screen.
                            self.end_reconnect();
                        }
                        // Other failures (peer not listening yet): stay, the next attempt is
                        // already scheduled.
                        return;
                    }
                }
                if self.session.negotiation.tcp_listener.is_none() {
                    if let Ok((listener, _)) =
                        crate::negotiation::bind_zone_listener(self.session.requested_listen_port)
                    {
                        self.session.negotiation.tcp_listener = Some(listener);
                    }
                }
                // End of negotiation: the derived status goes back to `Available`. A voluntary
                // refusal, cancellation or disappearance returns silently to the list (a
                // disappearance is only an implicit refusal). Only `Busy` or a real
                // connection/handshake error is shown.
                log::info!("Returning to LAN screen: {}", failure);
                let error = failure.to_lan_error();
                // Only the player whose game is the older one is pointed to the download.
                let hint = matches!(error, crate::app::meta_state::LanError::VersionMismatch(
                    crate::app::meta_state::OlderGame::Ours))
                    .then(|| self.session.version_mismatch_hint())
                    .flatten();
                if let Some(ref mut screen) = self.session.menu_state.lan_screen {
                    use crate::app::meta_state::LanScreenPhase;
                    screen.phase = if failure.is_silent() {
                        LanScreenPhase::Scanning
                    } else {
                        // Error ring = [optional action, Back]: focus on the first.
                        screen.selected = 0;
                        LanScreenPhase::Error { error, hint }
                    };
                }
            }
        }
    }

    /// Update check: receives the thread's result and makes ONE launch decision per session
    /// (on arrival at the menu). Chain of authority: public identity, then channel profile,
    /// then toggle/cadence. Any missing link makes the checker inert without touching the
    /// rest.
    #[cfg(feature = "update-check")]
    fn tick_update_check(&mut self, deferred: &mut Vec<AppCommand>) {
        use crate::platform::distribution::FetchPolicy;
        use crate::update::checker;

        // Receive: the thread sends an already validated result (never a visible error).
        if let Some(rx) = &self.session.update_check.rx {
            match rx.try_recv() {
                Ok(outcome) => {
                    self.session.update_check.rx = None;
                    // The result can remove the download button: the focused action must stay
                    // the same one, not the one that takes its index.
                    let before = self.session.info_actions();
                    self.apply_update_outcome(outcome);
                    self.keep_info_selection(&before);
                    deferred.push(AppCommand::RequestRedraw);
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => {}
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    self.session.update_check.rx = None;
                }
            }
        }
        if self.session.update_check.started
            || !matches!(self.session.flow_state, FlowState::Menu)
        {
            return;
        }
        // One evaluation per session, whatever the verdict.
        self.session.update_check.started = true;
        if self.session.update_check.source.is_none() {
            return;
        }
        // Yesterday's notification survives a relaunch without a connection.
        self.refresh_update_notification();
        let cfg = &self.session.user_config;
        let last = (cfg.last_update_check != 0).then_some(cfg.last_update_check);
        // `update_source` guarantees an AutoDaily profile (or the local trial of the dev build).
        if checker::should_check_now(FetchPolicy::AutoDaily, cfg.update_checks_enabled, last, unix_now()) {
            self.start_update_check();
        }
    }

    /// Starts an attempt, automatic or "Check now". One at a time: returns `false` if an
    /// attempt is already in progress or this binary cannot check.
    #[cfg(feature = "update-check")]
    fn start_update_check(&mut self) -> bool {
        use crate::update::checker;
        if self.session.update_check.rx.is_some() {
            return false;
        }
        let Some(source) = &self.session.update_check.source else { return false };
        // The ATTEMPT counts, successful or not: persisted before the fetch.
        self.session.user_config.last_update_check = unix_now();
        self.session.user_config.save();
        self.session.update_check.rx =
            Some(checker::spawn_check(checker::HttpFetcher::new(), source.api_url.clone()));
        log::debug!("update-check: attempt started");
        true
    }

    /// Validated thread result to persisted state. A successful read counts even if the
    /// remote version is not newer; a failure changes nothing.
    #[cfg(feature = "update-check")]
    fn apply_update_outcome(&mut self, outcome: crate::update::checker::CheckOutcome) {
        use crate::update::checker::CheckOutcome;
        if let CheckOutcome::Remote(remote) = outcome {
            let cfg = &mut self.session.user_config;
            cfg.last_update_success = unix_now();
            cfg.available_update_version =
                crate::update::release::available_update(env!("CARGO_PKG_VERSION"), &remote, None)
                    .map(|v| v.to_string())
                    .unwrap_or_default();
            cfg.save();
        }
        self.refresh_update_notification();
    }

    /// Notification to display (ticker and Info page), derived from the persisted state: the
    /// recorded version is still newer than the installed one and not ignored. A version the
    /// player has caught up with (update installed) is cleared.
    #[cfg(feature = "update-check")]
    fn refresh_update_notification(&mut self) {
        use crate::update::{checker::UpdateAvailable, still_newer};
        let cfg = &mut self.session.user_config;
        if !cfg.available_update_version.is_empty()
            && !still_newer(&cfg.available_update_version, env!("CARGO_PKG_VERSION"))
        {
            cfg.available_update_version.clear();
            cfg.save();
        }
        let state = &mut self.session.update_check;
        state.available = match &state.source {
            Some(source)
                if !cfg.available_update_version.is_empty()
                    && cfg.available_update_version != cfg.ignored_update_version =>
            {
                log::info!("update-check: version {} available", cfg.available_update_version);
                Some(UpdateAvailable {
                    version: cfg.available_update_version.clone(),
                    open_url: source.open_url.clone(),
                })
            }
            _ => None,
        };
    }

    /// Keeps the focused action of the open Info page on the same action when the list of
    /// actions changes under it (an update check that removes the download button). If the
    /// action is gone, the focus goes to the last one at most.
    #[cfg(feature = "update-check")]
    fn keep_info_selection(&mut self, before: &[crate::renderer::ui::info_modal::InfoAction]) {
        let selected = self.session.menu_state.info_action_selected;
        if selected == 0 {
            return;
        }
        let after = self.session.info_actions();
        self.session.menu_state.info_action_selected = before.get(selected - 1)
            .and_then(|a| after.iter().position(|x| x == a))
            .map(|i| i + 1)
            .unwrap_or_else(|| selected.min(after.len()));
    }

    /// Runs one tick of session logic: game step, AI, sound events, post-game effects.
    /// `freeze` is the dev-only simulation freeze.
    pub(crate) fn tick_session(&mut self, freeze: bool) -> Vec<AppCommand> {
        let mut deferred = Vec::new();

        // VS network zone services (TCP listener and mDNS): started on entering the zone,
        // stopped on leaving it, so we are reachable anywhere in the network corner and not
        // only on the auto-detect screen.
        self.sync_network_zone_services();
        // Network polling (LAN discovery, TCP listener, pending connection)
        self.poll_lan_discovery();
        self.poll_pending_reconnect(&mut deferred);
        self.session.negotiation.poll_tcp_listener();
        self.poll_pending_invite_reads();
        self.session.negotiation.poll_invite_gone();
        self.poll_pending_connection(&mut deferred);
        #[cfg(feature = "update-check")]
        self.tick_update_check(&mut deferred);
        // The announced mDNS status is derived (negotiation phase plus active game) and
        // pushed from this single sync point.
        let in_game = self.session.game.is_some();
        self.session.negotiation.sync_announced_status(in_game);

        // Sync pause menu visibility from snapshot.
        // The flag is a presentation state derived from the session — it
        // never gates the flow. Manual opens (ConnectionLost/WaitingRemote)
        // are preserved: the flag stays true while the interruption matches.
        {
            let should_be_open = if let Some(ref session) = self.session.game {
                let intr = session.snapshot().interruption;
                matches!(intr, crackattack_core::frame_data::SessionInterruption::Paused { .. })
                    || (self.session.pause_menu_open && matches!(intr,
                        crackattack_core::frame_data::SessionInterruption::ConnectionLost { .. }
                        | crackattack_core::frame_data::SessionInterruption::WaitingRemoteReady { .. }))
            } else {
                false
            };
            if should_be_open != self.session.pause_menu_open {
                if should_be_open {
                    // Pause appeared without local action → remote trigger.
                    self.session.pause_initiated_locally = false;
                    self.session.pause_menu_opened_at = Some(std::time::Instant::now());
                    self.session.pause_menu_selection.set(0);
                } else {
                    self.session.pause_menu_opened_at = None;
                }
                self.session.pause_menu_open = should_be_open;
                deferred.push(AppCommand::RequestRedraw);
            }
        }

        // The music follows the pause menu state (suspended while open), as in the original
        // (`Game::buttonPause` calls `Music::pause`/`resume`). `pause_music`/`resume_music` are
        // idempotent, so they can be called every tick without extra state.
        if let Some(ref mut snd) = self.sound_manager {
            snd.tick_music();
            if self.session.pause_menu_open {
                snd.pause_music();
            } else {
                snd.resume_music();
            }
        }

        match &self.session.flow_state {
            FlowState::Menu => {
                self.session.menu_state.tick();
            }
            FlowState::Playing | FlowState::PostGame => {
                if freeze { return deferred; }
                let mut events: Vec<crackattack_core::frame_data::SessionEvent> = Vec::new();
                if let Some(ref mut session) = self.session.game {
                    // Autoplay: let AI drive the input (only during active gameplay)
                    let snap_for_ai = session.snapshot();
                    let ai_can_play = self.session.autoplay_enabled
                        && snap_for_ai.lifecycle == crackattack_core::frame_data::SessionLifecycle::Playing
                        && snap_for_ai.interruption == crackattack_core::frame_data::SessionInterruption::None;
                    if ai_can_play {
                        if let Some(game) = session.game_state_mut() {
                            use crackattack_core::input::*;
                            let ai = match self.session.autoplay_grid_level {
                                Some(level) => {
                                    let seed = self.session.game_seed;
                                    self.session
                                        .autoplay_grid
                                        .get_or_insert_with(|| crackattack_core::vs::GridAutoplay::new(level, seed))
                                        .update(game)
                                }
                                None => {
                                    let mut flags = self.session.ai_controller.update(game);
                                    if crackattack_core::AIController::should_fast_forward(game) {
                                        flags |= INPUT_ADVANCE;
                                    }
                                    flags
                                }
                            };
                            for flag in [INPUT_LEFT, INPUT_RIGHT, INPUT_UP, INPUT_DOWN, INPUT_SWAP, INPUT_ADVANCE] {
                                self.session.input.set_key(flag, ai & flag != 0);
                            }
                        }
                    }

                    // Always tick the session (the session's tick() handles
                    // freeze internally by choosing step() vs frame_data()).
                    let input = self.session.input.snapshot();
                    let frame = session.tick(&input);
                    if let Some(ref mut snd) = self.sound_manager {
                        snd.process_events(&frame.gameplay_events);
                        // Adaptive music: the local player's stack, in game time (frozen
                        // during the pause, like the music).
                        if !self.session.pause_menu_open {
                            snd.update_music_stack(&frame.ui, 1.0 / crate::TICKS_PER_SECOND as f32);
                        }
                    }
                    self.step_events.extend(frame.gameplay_events.iter().cloned());

                    events = session.drain_events();
                }
                // Process lifecycle events OUTSIDE the session borrow
                // (captures below need to read self.session.game).
                for event in events {
                    use crackattack_core::frame_data::SessionEvent;
                    match event {
                        SessionEvent::RoundPrepared { round_number } => {
                            log::info!("Event: RoundPrepared({})", round_number);
                            if round_number <= 1 {
                                deferred.push(AppCommand::ClearRoundEffects);
                            }
                            if round_number > 1 {
                                // The curtain stays closed: the round summary shows on top.
                                // `OpenGameCurtain` is the `AdvanceToReadyPrompt` residual ("Next"
                                // opens the curtain, then the modal asks the question). No focused
                                // button (Enter = Next, safe).
                                self.session.post_game_input.post_round_advanced = false;
                                self.session.post_game_input.nav_focus = Default::default();
                                self.session.post_game_input.abandon_confirm_open = false;
                                self.session.post_game_input.abandon_confirm_yes = false;
                            }
                        }
                        SessionEvent::RoundStarted { round_number } => {
                            // Invariant: a round that starts must be VISIBLE. If the post-round
                            // flow missed the opening (the black-rounds class of bug), the residual
                            // repairs it and logs an error.
                            deferred.push(AppCommand::EnsureGameCurtainOpen);
                            log::info!("Event: RoundStarted({})", round_number);
                            // Effects and curtain handled by RoundPrepared
                        }
                        SessionEvent::LocalLoss => {
                            log::info!("Event: LocalLoss — starting visual fade");
                            deferred.push(AppCommand::CloseGameCurtain(3.0));
                        }
                        SessionEvent::RoundEnded { won } => {
                            log::info!("Event: RoundEnded(won={})", won);
                            // A NEW summary exists: the advance of the PREVIOUS summary no longer
                            // counts. Without this immediate invalidation, the autoplay decision (run
                            // BEFORE the events are drained on the next tick) would read the stale
                            // flag, skip `AdvanceToReadyPrompt` and start the round under a closed
                            // curtain, making every other round black. The reset in `RoundPrepared`
                            // comes too late, after ready.
                            self.session.post_game_input.post_round_advanced = false;
                            // Ladder: each refereed round is one observation.
                            self.record_rated_round(won);
                            self.capture_round_stats(won);
                            // All-mode records (combo, blocks/min) per ROUND: VS has no post-game
                            // screen, so without this call VS games would not feed these records.
                            if let Some(stats) = self.session.round_end_stats.as_ref()
                                .map(|rs| rs.local.clone())
                            {
                                let beaten = self.submit_universal_records(&stats);
                                if !beaten.is_empty() {
                                    log::info!("Records broken this round: {:?}", beaten);
                                }
                            }
                            // In solo/VsComputer, LocalLoss isn't emitted — close curtain here.
                            // In network, the curtain was already closed by LocalLoss.
                            // CloseGameCurtain is idempotent (close_simple on already-closing is safe).
                            deferred.push(AppCommand::CloseGameCurtain(3.0));
                        }
                        SessionEvent::OpponentDisconnected => {
                            log::warn!("Event: OpponentDisconnected");
                        }
                        SessionEvent::ReturnedToMenu => {
                            log::info!("Event: ReturnedToMenu");
                        }
                        SessionEvent::CountdownDigit { digit } => {
                            log::debug!("Event: CountdownDigit({:?})", digit);
                            if let Some(ref mut snd) = self.sound_manager {
                                use crackattack_core::frame_data::CountdownDigit;
                                let gameplay_event = match digit {
                                    CountdownDigit::Go => crackattack_core::gameplay_event::GameplayEvent::CountdownGo,
                                    _ => crackattack_core::gameplay_event::GameplayEvent::CountdownTick(match digit {
                                        CountdownDigit::Three => 3,
                                        CountdownDigit::Two => 2,
                                        CountdownDigit::One => 1,
                                        CountdownDigit::Go => unreachable!(),
                                    }),
                                };
                                snd.process_events(&[gameplay_event]);
                            }
                        }
                    }
                }

                // AUTOPLAY decision, made AFTER the events are reduced. Running it before the
                // drain read a stale `post_round_advanced` from the PREVIOUS summary and
                // skipped `AdvanceToReadyPrompt`, so the curtain never opened. Autoplay emits
                // the SAME `AppCommand`s as the player ("Next" then "Ready"): one decision
                // path, the one a human uses.
                if self.session.autoplay_enabled
                    && self.session.transition_cooldown <= 0
                {
                    let snap = self.session.game.as_ref().map(|g| g.snapshot());
                    if let Some(snap) = snap {
                        if let crackattack_core::frame_data::SessionInterruption::ReadyPrompt {
                            round_number, opponent_present, ..
                        } = snap.interruption {
                            let played = snap.vs_display.as_ref()
                                .map_or(0, |vs| vs.session_wins + vs.session_losses);
                            let limit_reached = self.session.autoplay_max_rounds
                                .is_some_and(|max| played >= max as i32);
                            let bilan_open = round_number > 1
                                && self.session.round_end_stats.is_some()
                                && !self.session.post_game_input.post_round_advanced;
                            if !opponent_present || limit_reached {
                                deferred.push(AppCommand::QuitToMenu);
                                // Do not push the command again every frame.
                                self.session.transition_cooldown = 30;
                            } else if bilan_open {
                                deferred.push(AppCommand::AdvanceToReadyPrompt);
                            } else {
                                deferred.push(AppCommand::SignalLocalReady);
                                self.session.transition_cooldown = 15;
                            }
                        }
                    }
                }
            }
        }
        // Hydrate opponent stats in round_end_stats if they arrived late
        // (network stats arrive during Resolving/ShowResult, after RoundEnded capture).
        if let Some(ref mut rs) = self.session.round_end_stats {
            if rs.opponent.is_none() {
                if let Some(ref s) = self.session.game {
                    let snap = s.snapshot();
                    if let Some(opp) = snap.vs_display.as_ref().and_then(|vs| vs.opponent_stats.clone()) {
                        rs.opponent = Some(opp);
                    }
                }
            }
        }

        // Keep repainting while a scroll direction is held: `nav_view` applies one delta per
        // frame, which gives smooth scrolling (no stutter from keyboard repeat).
        if self.session.post_game_input.scroll_dir != 0 {
            deferred.push(AppCommand::RequestRedraw);
        }

        self.session.input.end_frame();
        deferred
    }

    /// Capture round stats for the post-round panel.
    /// Called at RoundEnded — the session tally hasn't recorded this round
    /// yet (it records at the end of ShowResult), so the round just played
    /// is added here for display.
    fn capture_round_stats(&mut self, won: bool) {
        if let Some(ref s) = self.session.game {
            let frame = s.frame_data();
            let snap = s.snapshot();
            let opp_stats = snap.vs_display.as_ref()
                .and_then(|vs| vs.opponent_stats.clone());
            let (round_number, mut wins, mut losses) = snap.vs_display.as_ref()
                .map(|vs| (vs.round_number, vs.session_wins, vs.session_losses))
                .unwrap_or((1, 0, 0));
            if won { wins += 1 } else { losses += 1 }
            self.session.round_end_stats = Some(
                crate::app::post_game::RoundEndStats {
                    local: frame.stats.clone(),
                    opponent: opp_stats,
                    round_number,
                    session_wins: wins,
                    session_losses: losses,
                }
            );
        }
    }

    /// Records that apply to every game type (solo, VS, network), outside the `uses_score`
    /// gate: best combo (max chain) and blocks/min. They are gameplay measures independent of
    /// the score. Called at the solo post-game and at each VS `RoundEnded`, since VS has no
    /// post-game screen.
    /// Returns the records beaten (for display) after saving if needed.
    fn submit_universal_records(
        &mut self,
        stats: &crackattack_core::stats::GameStats,
    ) -> Vec<crate::platform::persist::RecordKey> {
        use crate::platform::persist::RecordKey;
        let mut beaten = Vec::new();
        // The autoplay is a test driver, not the player: it leaves no record.
        if self.session.autoplay_enabled {
            return beaten;
        }
        let name = self.session.player_name.clone();
        if self.session.user_config.submit_record(RecordKey::BestChain, stats.max_chain_level as i32, &name) {
            beaten.push(RecordKey::BestChain);
        }
        // Blocks/min is meaningful only after 1 min of play.
        if stats.duration_seconds() as i32 >= 60
            && self.session.user_config.submit_record(RecordKey::BestBlocksPerMin, stats.blocks_per_minute() as i32, &name)
        {
            beaten.push(RecordKey::BestBlocksPerMin);
        }
        if !beaten.is_empty() {
            self.session.user_config.save();
        }
        beaten
    }

    fn capture_post_game(&mut self, won: bool) {
        use crate::app::post_game::PostGamePhase;
        if self.session.post_game_fx.phase == PostGamePhase::Entered {
            return;
        }
        let Some(frame) = self.session.game.as_ref().map(|s| s.frame_data()) else { return };
        {
            self.session.game_over_stats = Some(frame.stats.clone());
            self.session.game_over_score = frame.ui.score;
            self.session.game_over_rank = None;

            use crate::platform::persist::RecordKey;
            let mut should_save = false;

            let stats_copy = frame.stats.clone();
            let mut beaten = self.submit_universal_records(&stats_copy);

            if self.session.selected_mode.uses_score() && !self.session.autoplay_enabled {
                let rank = self.session.user_config.submit_score(
                    &self.session.player_name, frame.ui.score,
                );
                self.session.game_over_rank = rank;
                if let Some(r) = rank {
                    log::info!("New high score! Rank #{}", r + 1);
                }

                // Solo-only records (the combo is already submitted above).
                let name = &self.session.player_name;
                let stats = &frame.stats;
                let score = frame.ui.score;
                let duration_secs = stats.duration_seconds() as i32;

                if self.session.user_config.submit_record(RecordKey::BestScore, score, name) {
                    beaten.push(RecordKey::BestScore);
                }
                if self.session.user_config.submit_record(RecordKey::LongestGame, duration_secs, name) {
                    beaten.push(RecordKey::LongestGame);
                }
                if self.session.user_config.submit_record(RecordKey::MostBlocks, stats.blocks_eliminated as i32, name) {
                    beaten.push(RecordKey::MostBlocks);
                }
                if self.session.user_config.submit_record(RecordKey::MostSwaps, stats.swaps_performed as i32, name) {
                    beaten.push(RecordKey::MostSwaps);
                }
                should_save = true; // the table is updated either way
            }

            self.session.game_over_beaten_records = beaten;
            if should_save {
                self.session.user_config.save();
            }
            if let Some(ref mut snd) = self.sound_manager {
                use crate::platform::sound::MusicTrack;
                snd.play_music(if won { MusicTrack::YouWin } else { MusicTrack::GameOver });
            }
            self.session.post_game_fx.phase = PostGamePhase::Entered;
            log::info!("capture_post_game: score={} won={}", frame.ui.score, won);
        }
    }

    /// Sync post-game effects. Returns intentions for renderer side-effects
    /// (texture refresh). Session mutations happen here;
    /// renderer mutations happen via the returned commands.
    pub(crate) fn sync_post_game(&mut self, frame: &FrameData) -> Vec<AppCommand> {
        use crackattack_core::frame_data::SessionLifecycle;
        use crate::app::post_game::PostGameAction;

        let mut cmds = Vec::new();

        let snapshot = match self.session.game.as_ref() {
            Some(s) => s.snapshot(),
            None => return cmds,
        };

        // Only when a game actually finished, not during normal play.
        let is_finished = matches!(snapshot.lifecycle, SessionLifecycle::Resolving | SessionLifecycle::ShowResult | SessionLifecycle::Ended);
        if !is_finished {
            self.session.post_game_fx.reset();
            return cmds;
        }
        // VS (open series): never a post-game. The post-round screen lives inside the cycle
        // (ReadyPrompt) and the forfeit (Ended) is carried by the Disconnected banner. The
        // post-game flow is for solo only.
        if self.session.selected_mode.is_combat() {
            return cmds;
        }

        // Solo: close the game curtain when resolving first starts.
        // (VS modes emit RoundEnded events which trigger CloseGameCurtain in tick_session.)
        if snapshot.lifecycle == SessionLifecycle::Resolving
            && self.session.post_game_fx.last_finished_tick < 0
        {
            cmds.push(AppCommand::CloseGameCurtain(3.0));
        }

        let actions = self.session.post_game_fx.sync(frame.ui.ticks_since_finished, &snapshot);
        for action in actions {
            match action {
                PostGameAction::Enter => cmds.push(AppCommand::EnterPostGame),
            }
        }

        self.session.post_game_fx.last_finished_tick = frame.ui.ticks_since_finished.max(0);
        cmds
    }
}

#[cfg(test)]
mod tests {
    /// The Updates page loops its focus; the Credits page (text plus one action) stops at its
    /// ends. 0 is "no action focused" (the text).
    #[test]
    fn info_action_step_loops_on_updates_and_stops_elsewhere() {
        // 3 actions: Download, Check, Toggle.
        assert_eq!(info_action_step(1, 3, 1, true), 2);
        assert_eq!(info_action_step(3, 3, 1, true), 1, "bottom -> top");
        assert_eq!(info_action_step(1, 3, -1, true), 3, "top -> bottom");
        assert_eq!(info_action_step(0, 3, 1, true), 1);
        assert_eq!(info_action_step(0, 3, -1, true), 3);
        assert_eq!(info_action_step(1, 1, 1, true), 1, "a single action stays put");
        // Credits: [text, Contribute], stops at the ends.
        assert_eq!(info_action_step(1, 1, 1, false), 1);
        assert_eq!(info_action_step(0, 1, -1, false), 0);
        assert_eq!(info_action_step(0, 1, 1, false), 1);
    }

    use super::*;
    use crate::app::meta_state::FlowState;
    use crate::app::post_game::PostGamePhase;

    fn test_runtime() -> SessionRuntime {
        let config = SessionConfig {
            seed: 42,
            autoplay: false,
            autoplay_ai: None,
            autoplay_max_rounds: None,
            vs_mode: false,
            ai_difficulty: 1,
            network: None,
            open_targets: Vec::new(),
            set_overrides: Vec::new(),
            sounds_dir: None, music_dir: None, rungs_dir: None, music_debug: false,
        };
        let user_config = crate::platform::persist::UserConfig::default();
        SessionRuntime::new(config, user_config, None, std::path::Path::new("/nonexistent"))
    }

    #[test]
    fn new_solo_starts_at_menu() {
        let rt = test_runtime();
        assert!(matches!(rt.session.flow_state, FlowState::Menu));
        assert!(rt.session.game.is_none());
        assert!(rt.sound_manager.is_none());
    }

    /// A launch shortcut starts AT THE MENU and queues the SAME sequence as the menu action
    /// (SelectMode + CreateGame + StartTransition), applied on the first frame. No game is
    /// built at boot.
    #[test]
    fn new_vs_queues_menu_launch() {
        let config = SessionConfig {
            seed: 42, autoplay: false, autoplay_ai: None, autoplay_max_rounds: None, vs_mode: true, ai_difficulty: 1,
            network: None, open_targets: Vec::new(), set_overrides: Vec::new(), sounds_dir: None, music_dir: None, rungs_dir: None, music_debug: false,
        };
        let rt = SessionRuntime::new(
            config,
            crate::platform::persist::UserConfig::default(),
            None, std::path::Path::new("/nonexistent"),
        );
        assert!(matches!(rt.session.flow_state, FlowState::Menu));
        assert!(rt.session.game.is_none(), "no game built at boot");
        assert!(rt.session.pending_startup.iter().any(|c|
            matches!(c, AppCommand::CreateGame { mode: GameMode::VsComputer(_), .. })),
            "the queue must contain CreateGame(VsComputer)");
    }

    #[test]
    fn new_autoplay_queues_solo_launch() {
        let config = SessionConfig {
            seed: 42, autoplay: true, autoplay_ai: None, autoplay_max_rounds: None, vs_mode: false, ai_difficulty: 1,
            network: None, open_targets: Vec::new(), set_overrides: Vec::new(), sounds_dir: None, music_dir: None, rungs_dir: None, music_debug: false,
        };
        let rt = SessionRuntime::new(
            config,
            crate::platform::persist::UserConfig::default(),
            None, std::path::Path::new("/nonexistent"),
        );
        assert!(matches!(rt.session.flow_state, FlowState::Menu));
        assert!(rt.session.pending_startup.iter().any(|c|
            matches!(c, AppCommand::CreateGame { mode: GameMode::Solo, .. })),
            "autoplay solo → CreateGame(Solo) queued");
    }

    /// CLI network (host/join): the queue is OpenLanScreen (+ ConnectToPeer for join), the
    /// menu flow. No special connection at boot.
    #[test]
    fn new_network_join_queues_menu_flow() {
        let config = SessionConfig {
            seed: 42, autoplay: false, autoplay_ai: None, autoplay_max_rounds: None, vs_mode: false, ai_difficulty: 1,
            network: Some(crate::platform::cli::NetworkMode::Join {
                host: "127.0.0.1".into(), port: 45690,
            }),
            open_targets: Vec::new(),
            set_overrides: Vec::new(),
            sounds_dir: None, music_dir: None, rungs_dir: None, music_debug: false,
        };
        let rt = SessionRuntime::new(
            config,
            crate::platform::persist::UserConfig::default(),
            None, std::path::Path::new("/nonexistent"),
        );
        assert!(rt.session.game.is_none());
        assert!(!rt.session.negotiation.in_progress(), "no connection at boot");
        assert!(matches!(rt.session.pending_startup.first(),
            Some(AppCommand::OpenLanScreen { .. })));
        assert!(rt.session.pending_startup.iter().any(|c|
            matches!(c, AppCommand::ConnectToPeer { port: 45690, .. })));
    }

    #[test]
    fn new_network_create_sets_requested_port() {
        let config = SessionConfig {
            seed: 42, autoplay: false, autoplay_ai: None, autoplay_max_rounds: None, vs_mode: false, ai_difficulty: 1,
            network: Some(crate::platform::cli::NetworkMode::Create { port: 45691 }),
            open_targets: Vec::new(),
            set_overrides: Vec::new(),
            sounds_dir: None, music_dir: None, rungs_dir: None, music_debug: false,
        };
        let rt = SessionRuntime::new(
            config,
            crate::platform::persist::UserConfig::default(),
            None, std::path::Path::new("/nonexistent"),
        );
        assert_eq!(rt.session.requested_listen_port, Some(45691));
        assert!(matches!(rt.session.pending_startup.first(),
            Some(AppCommand::OpenLanScreen { manual: false })));
    }

    #[test]
    fn set_flow_changes_state() {
        let mut rt = test_runtime();
        rt.apply_app_command(AppCommand::SetFlow(FlowState::Playing));
        assert!(matches!(rt.session.flow_state, FlowState::Playing));
    }

    #[test]
    fn start_transition_returns_residual() {
        let mut rt = test_runtime();
        rt.apply_app_command(AppCommand::SetFlow(FlowState::Playing));
        let residual = rt.apply_app_command(AppCommand::StartTransition(FlowState::Menu, 0.5));
        assert!(residual.is_some());
        assert!(matches!(residual.unwrap(), AppCommand::StartTransition(..)));
        assert!(matches!(rt.session.flow_state, FlowState::Playing));
    }

    // A repeated QuitToMenu (autoplay asks again every few frames) must not restart the
    // curtain, or it never finishes closing; leaving the game resets the guard.
    #[test]
    fn quit_to_menu_is_applied_once_until_the_game_is_cleared() {
        let mut rt = test_runtime();
        let create = || AppCommand::CreateGame {
            mode: GameMode::VsComputer(crackattack_core::ai_difficulty::AIDifficulty::Easy),
            seed: 1,
            ranking_entry_score: 0,
        };
        rt.apply_app_command(create());
        assert!(matches!(rt.apply_app_command(AppCommand::QuitToMenu), Some(AppCommand::StartTransition(..))));
        assert!(rt.apply_app_command(AppCommand::QuitToMenu).is_none());
        rt.apply_app_command(AppCommand::ClearActiveGame);
        rt.apply_app_command(create());
        assert!(matches!(rt.apply_app_command(AppCommand::QuitToMenu), Some(AppCommand::StartTransition(..))));
    }

    #[test]
    fn create_game_solo() {
        let mut rt = test_runtime();
        rt.apply_app_command(AppCommand::CreateGame {
            mode: GameMode::Solo,
            seed: 123,
            ranking_entry_score: 500,
        });
        assert!(rt.session.game.is_some());
        // Solo: no VS display (no opponent to show)
        let snap = rt.session.game.as_ref().unwrap().snapshot();
        assert!(snap.vs_display.is_none(), "Solo should not have VsDisplay");
    }

    /// The autoplay is a test driver: a finished game leaves no score and no record, and
    /// nothing is written to the state file.
    #[test]
    fn autoplay_games_leave_no_score_and_no_record() {
        let mut rt = test_runtime();
        rt.session.autoplay_enabled = true;
        rt.session.user_config.scores.clear();
        rt.apply_app_command(AppCommand::CreateGame {
            mode: GameMode::Solo, seed: 5, ranking_entry_score: 1,
        });
        rt.session.selected_mode = GameMode::Solo;
        rt.capture_post_game(false);
        assert!(rt.session.user_config.scores.is_empty(), "the autoplay entered the score table");
        assert!(rt.session.user_config.records.is_empty(), "the autoplay set a record");
        assert!(rt.session.game_over_rank.is_none());
    }

    #[test]
    fn create_game_vs_computer() {
        let mut rt = test_runtime();
        let diff = crackattack_core::ai_difficulty::AIDifficulty::Hard;
        rt.apply_app_command(AppCommand::CreateGame {
            mode: GameMode::VsComputer(diff),
            seed: 456,
            ranking_entry_score: 0,
        });
        // VsComputer: has VS display with opponent name
        let snap = rt.session.game.as_ref().unwrap().snapshot();
        let vs = snap.vs_display.as_ref().expect("VsComputer should have VsDisplay");
        assert!(!vs.opponent_name.is_empty(), "Opponent name should be set");
    }

    #[test]
    fn clear_active_game_resets() {
        let mut rt = test_runtime();
        rt.apply_app_command(AppCommand::CreateGame {
            mode: GameMode::Solo, seed: 1, ranking_entry_score: 100,
        });
        assert!(rt.session.game.is_some());
        // Leftovers of a finished game: without them the `is_none()` checks below would
        // hold even if `ClearActiveGame` forgot to reset them.
        rt.session.game_over_stats = Some(crackattack_core::stats::GameStats::default());
        rt.session.game_over_rank = Some(3);

        rt.apply_app_command(AppCommand::ClearActiveGame);
        assert!(rt.session.game.is_none());
        assert!(rt.session.game_over_stats.is_none());
        assert!(rt.session.game_over_rank.is_none());
    }

    #[test]
    fn residual_commands_returned_to_caller() {
        let mut rt = test_runtime();
        let result = rt.apply_app_command(AppCommand::RequestExit);
        assert!(matches!(result, Some(AppCommand::RequestExit)));

        let result = rt.apply_app_command(AppCommand::RequestRedraw);
        assert!(matches!(result, Some(AppCommand::RequestRedraw)));

        let result = rt.apply_app_command(AppCommand::ResetForMenu);
        assert!(matches!(result, Some(AppCommand::ResetForMenu)));
    }

    #[test]
    fn session_commands_return_none() {
        let mut rt = test_runtime();
        assert!(rt.apply_app_command(AppCommand::SetFlow(FlowState::Playing)).is_none());
        assert!(rt.apply_app_command(AppCommand::PlayUiConfirm).is_none());
        assert!(rt.apply_app_command(AppCommand::SetTransitionCooldown(10)).is_none());
    }

    #[test]
    fn apply_options_returns_refresh_nameplate() {
        // `language: None` re-detects the locale: serialize with the other language tests.
        let _lang = crate::app::locale::LangGuard::new();
        let mut rt = test_runtime();
        let result = rt.apply_app_command(AppCommand::ApplyOptionsConfirmed {
            player_name: "TEST".to_string(),
            sfx_enabled: false,
            music_enabled: false,
            sfx_volume: 0.5,
            music_volume: 0.3,
            language: None,
            fullscreen: false,
            key_bindings: crate::platform::persist::KeyBindings::default(),
        });
        assert!(matches!(result, Some(AppCommand::RefreshNameplates)));
        // The name is stored AS RECEIVED: anti-caps-lock correction belongs to the producer
        // (`InputFormat::finalize`).
        assert_eq!(rt.session.player_name, "TEST");
        assert!(!rt.session.user_config.sfx_enabled);
        assert_eq!(rt.session.user_config.sfx_volume, 0.5);
    }

    /// A name with no ASCII letter (multi-byte first character) must not panic when the
    /// Options are confirmed (a byte-indexed correction would break on a char boundary).
    #[test]
    fn apply_options_with_non_ascii_name_does_not_panic() {
        // `language: None` re-detects the locale: serialize with the other language tests.
        let _lang = crate::app::locale::LangGuard::new();
        let mut rt = test_runtime();
        for name in ["É", "日本", "ÉRIC"] {
            let result = rt.apply_app_command(AppCommand::ApplyOptionsConfirmed {
                player_name: name.to_string(),
                sfx_enabled: false,
                music_enabled: false,
                sfx_volume: 0.5,
                music_volume: 0.3,
                language: None,
                fullscreen: false,
                key_bindings: crate::platform::persist::KeyBindings::default(),
                });
            assert!(matches!(result, Some(AppCommand::RefreshNameplates)));
            assert_eq!(rt.session.player_name, name);
        }
    }

    #[test]
    fn enter_post_game_requires_game() {
        let mut rt = test_runtime();
        // No game → EnterPostGame is a no-op
        rt.apply_app_command(AppCommand::EnterPostGame);
        assert_eq!(rt.session.post_game_fx.phase, PostGamePhase::Inactive);
    }
}
