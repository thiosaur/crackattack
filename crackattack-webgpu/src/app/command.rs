//! Application commands — pure intentions that main.rs executes.
//!
//! `app/` decides what to do and returns `AppCommand` values.
//! `main.rs` applies them via `apply_app_command`, which is the only
//! place that touches GPU, audio, persistence, and windowing.

use crate::app::meta_state::{FlowState, GameMode};

/// Application-level music intent, independent of the audio backend.
/// Converted to `MusicTrack` in `apply_app_command`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppMusic {
    Prelude,
}

/// A side-effect command produced by app logic, executed by main.rs.
#[derive(Debug)]
pub enum AppCommand {
    SetFlow(FlowState),
    StartTransition(FlowState, f32),
    AdvancePostGameScreen,
    SetTransitionCooldown(i32),

    PlayMusic(AppMusic),
    /// Fade out the current music (menu to game area).
    FadeOutMusic,
    PlayUiConfirm,
    PlayUiNavigate,

    EnterPostGame,

    SetGameplayKey { action: u8, pressed: bool },
    /// Held scroll direction of the post-game content: -1 = up, 1 = down, 0 = released.
    /// `nav_view` applies one delta per frame while it is non-zero.
    SetPostGameScrollDir(i32),
    /// Focus of a post-game nav button (Left/Right): Menu, Next or none.
    SetPostGameNavFocus(crate::app::post_game::PostGameNavFocus),
    /// Between-round forfeit confirmation modal. `None` closes it. `Some(yes)` opens it (or
    /// updates the selection); `yes` means the "forfeit" button is focused.
    SetAbandonConfirm(Option<bool>),

    ResetForMenu,
    /// Clear round-scope transient effects (sparkles, level lights).
    /// Triggered by `SessionEvent::RoundStarted`.
    ClearRoundEffects,
    ClearActiveGame,

    ApplyOptionsConfirmed {
        player_name: String,
        sfx_enabled: bool,
        music_enabled: bool,
        sfx_volume: f32,
        music_volume: f32,
        /// `None` = automatic (system language), `Some("en")`, `Some("fr")`.
        language: Option<String>,
        fullscreen: bool,
        key_bindings: crate::platform::persist::KeyBindings,
    },
    RefreshNameplates,
    PreviewNameplate { name: String },
    PreviewMusicVolume(f32),

    MenuNavigate(Direction),
    MenuBack,
    ConfirmMenuSelection,
    /// Select a menu item by absolute index (mouse/touch hover or click).
    MenuSelectIndex(usize),
    /// Focus the Quit item (last item of the root menu).
    MenuFocusQuit,
    /// Open the options modal. `tab: Some(..)` is a deep link (CLI `--open menu:options/<section>`).
    /// `None` means opened from the menu; the section comes from the selected leaf.
    OpenOptionsModal { tab: Option<crate::app::meta_state::OptionsTab> },
    /// "Paste" button of the manual LAN screen. `main.rs` reads the clipboard through the
    /// platform shell and splits it into `LanScreenState.manual`.
    LanPasteRequest,
    CloseOptionsModal,
    /// Open a section of the Info screen (read-only menu overlay).
    OpenInfoScreen(crate::app::menu_tree::InfoSection),
    CloseInfoScreen,
    /// Info/Updates page, "Check for updates" checkbox: toggles and persists the daily
    /// automatic check.
    ToggleUpdateChecks,
    /// Info/Updates page, "Check now": an attempt outside the 24 h interval, even when the
    /// checkbox is off. Only one runs at a time, and the user always gets an explicit answer.
    CheckUpdatesNow,
    /// Credits page: move the action selection (Up/Down, clamped list, 0 = none).
    InfoActionSelectMove(i32),
    /// Credits page: select action `i` (1-based), on mouse hover.
    InfoActionSelectSet(usize),
    /// Open a URL in the system browser (Credits links). URLs come from the distribution
    /// profile, never from the network.
    OpenUrl(String),
    /// Scroll the Info screen by a delta in pixels. One-shot: set by keyboard/gamepad nav,
    /// then consumed and reset by the renderer. Positive = down.
    ScrollInfo(f32),
    /// Focus of the Info page: `true` = Back button, `false` = content (Left/Right).
    SetInfoFocusBack(bool),
    /// Move the options-modal row/column focus by (dx, dy) (keyboard/gamepad nav).
    OptionsNavMove(i32, i32),
    /// Focus a specific options widget (absolute focus, set by mouse hover).
    OptionsNavFocus(crate::app::options_nav::OptWidget),
    /// Activate the focused options-modal widget (Confirm/A).
    OptionsNavActivate,
    /// Move the on-screen keyboard focus by (dx, dy).
    OptionsKbMove(i32, i32),
    /// Activate the focused on-screen keyboard key (Confirm/A).
    OptionsKbActivate,
    /// Close the on-screen keyboard without typing (Cancel/B).
    OptionsKbClose,
    /// Open the network screen. With `manual`, it opens directly on the manual connection
    /// view and does **not** run mDNS discovery (discovery only runs in the auto-search view).
    OpenLanScreen { manual: bool },
    CloseLanScreen,
    ConnectToPeer { addr: std::net::IpAddr, port: u16 },
    /// Network rank page: move the selection among opponent rows (keyboard/gamepad).
    RankSelectMove(i32),
    /// Network rank page: set the selection (mouse hover).
    RankSelectSet(usize),
    /// Opponents page: quick reconnect to the selected opponent. Opens the LAN screen in the
    /// reconnect phase (repeated attempts, a fresh mDNS address takes priority).
    ReconnectOpponent(usize),
    /// Switch from the reconnect screen to manual connection, with the stored IP and port
    /// prefilled (the user can, for example, fix the port).
    ReconnectToManual,
    AcceptInvite,
    DeclineInvite,
    /// Cancel a sent invite (`Inviting` phase): drops the pending connection, goes back to
    /// `Available` and returns to the list.
    CancelInvite,
    /// Move the LAN peer-list selection by a delta (keyboard/gamepad nav).
    LanSelectMove(i32),
    /// Manual LAN screen: move the focus in the vertical ring (Copy / IPv4 / IPv6 / Port /
    /// Paste / Connect / Back). Keyboard and gamepad.
    ManualNavMove(i32),
    /// Activate the focused item of the manual screen (Confirm/A). A field opens the
    /// on-screen keyboard, Paste emits `LanPasteRequest`, Connect emits `ConnectToPeer`, etc.
    ManualNavActivate,
    /// Manual screen on-screen keyboard: move the grid focus by (dx, dy).
    ManualKbMove(i32, i32),
    /// Activate the focused key of the manual screen keyboard (Confirm/A).
    ManualKbActivate,
    /// Close the manual screen keyboard (Cancel/B). The field is still finalized.
    ManualKbClose,
    /// Write to the system clipboard (handled by `main.rs`, which owns the platform shell).
    /// Router path of the "Copy my address" button; the mouse uses egui `copy_text` instead.
    CopyToClipboard(String),
    /// Open the pause menu. If game is running, request a local pause.
    OpenPauseMenu,
    /// Explicit resume request to the session (solo/vs-computer). Closes the menu.
    ResumeGame,
    /// Signal ready to resume (network). The session marks the local side as ready.
    /// The menu stays open until both sides are ready and sync completes.
    SignalResumeReady,
    /// Quit the game. Calls session.abandon(), transitions to menu.
    QuitToMenu,
    SelectMode(GameMode),

    SignalLocalReady,

    /// "Next" on the post-round summary. Commits nothing: it closes the summary and opens
    /// the curtain on the new grid, where the "Ready for a new round?" modal asks the
    /// question. Consent goes through `SignalLocalReady`, as in round 1.
    AdvanceToReadyPrompt,
    /// Request to leave the game. The runtime resolves it: if a round is committed, ask for
    /// confirmation ("Forfeit?"); otherwise leave directly (ready prompt or post-round,
    /// where nothing is committed).
    RequestQuitGame,
    /// Close the game curtain (fade to black over `duration` seconds).
    CloseGameCurtain(f32),
    /// Invariant safety net, emitted on every `RoundStarted`. If the game/post-game curtain
    /// is neither open nor opening, log an error and open it. Does nothing in the normal
    /// flow (the curtain is already open).
    EnsureGameCurtainOpen,

    CreateGame { mode: GameMode, seed: u32, ranking_entry_score: i32 },
    /// Apply fullscreen mode. Handled by main.rs (window owner).
    SetFullscreen(bool),

    RequestExit,

    RequestRedraw,
}

/// Direction for menu navigation.
#[derive(Debug, Clone, Copy)]
pub enum Direction {
    Up,
    Down,
}

/// Platform-independent key abstraction.
/// The binary converts physical input to `AppKey`, which keeps `app/` independent of
/// the windowing library.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppKey {
    Up,
    Down,
    Left,
    Right,
    Raise,     // gameplay: raise the stack (INPUT_ADVANCE)
    Swap,      // Space
    Cancel,    // Escape
    Pause,     // P or Escape
}
