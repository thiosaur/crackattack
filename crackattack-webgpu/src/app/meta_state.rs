//! Application meta-state: flow control, menu navigation, options modal.


/// Unified application flow state.
///
/// Each variant encodes the full logical state. Transitions are managed
/// externally by the CurtainState in render_groups.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FlowState {
    Menu,
    Playing,
    PostGame,
}

impl FlowState {
    pub fn is_menu(&self) -> bool { matches!(self, Self::Menu) }
    pub fn is_post_game(&self) -> bool { matches!(self, Self::PostGame) }
    pub fn is_in_game(&self) -> bool { matches!(self, Self::Playing | Self::PostGame) }
}

/// Game mode selected from the menu.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GameMode {
    Solo,
    VsComputer(crackattack_core::ai_difficulty::AIDifficulty),
    VsNetwork,
}

impl GameMode {
    /// Score is tracked, persisted, and displayed (Solo only).
    pub fn uses_score(&self) -> bool { matches!(self, Self::Solo) }
    /// Multi-round combat (best-of-3). VS modes.
    pub fn is_combat(&self) -> bool { !matches!(self, Self::Solo) }
    /// Network communication with another human player.
    pub fn is_network(&self) -> bool { matches!(self, Self::VsNetwork) }
}

/// Menu state for navigation.
#[derive(Debug, Clone)]
pub struct MenuState {
    /// Navigation path in the menu tree (`menu_tree`): child indices taken from the root.
    /// Empty means the main menu. `back()` pops it and restores the parent's selection.
    path: Vec<usize>,
    /// Selected index in current menu (0-based)
    pub selected: usize,
    /// Tick counter for animations
    pub ticks: i32,
    /// Options modal open (name editing)
    pub options_modal: Option<NameInputState>,
    /// Network connection modal
    pub lan_screen: Option<LanScreenState>,
    /// Info screen: `Some(section)` means the overlay is open on that section.
    pub info_section: Option<crate::app::menu_tree::InfoSection>,
    /// Info page focus: the Back button (`true`) or the content (`false`, the default).
    /// Left/Right switch; Up/Down scroll while the content has focus.
    pub info_focus_back: bool,
    /// Network ranking page: index of the selected opponent row.
    pub rank_selected: usize,
    /// Set when the selection moved this frame, so the renderer scrolls it into view.
    /// A `Cell` because the renderer consumes (clears) it.
    pub rank_scroll_pending: std::cell::Cell<bool>,
    /// Credits page: selected action, 1-based (`0` = none, Up/Down scroll the text).
    /// See `InfoExtras::actions`. The list stops at its ends (no wrap).
    pub info_action_selected: usize,
    /// Set when the credits selection moved (keyboard or gamepad), so the renderer
    /// scrolls it into view and clears the flag.
    pub info_action_scroll_pending: std::cell::Cell<bool>,
    /// Pending Info scroll (delta in px, one-shot). Set by navigation, consumed by the renderer.
    pub info_scroll_delta: f32,
}

impl Default for MenuState {
    fn default() -> Self {
        Self::new()
    }
}

impl MenuState {
    pub fn new() -> Self {
        Self { path: Vec::new(), selected: 0, ticks: 0, options_modal: None, lan_screen: None, info_section: None, info_focus_back: false, rank_selected: 0, rank_scroll_pending: std::cell::Cell::new(false), info_action_selected: 0, info_action_scroll_pending: std::cell::Cell::new(false), info_scroll_delta: 0.0 }
    }

    /// Navigation depth (0 = main menu).
    pub fn depth(&self) -> usize {
        self.path.len()
    }

    /// The currently selected node in the menu tree.
    fn selected_node(&self) -> Option<&'static crate::app::menu_tree::MenuNode> {
        crate::app::menu_tree::children_at(&self.path).get(self.selected)
    }

    /// Advance animation by one tick.
    pub fn tick(&mut self) {
        self.ticks += 1;
    }

    /// Number of items in the current menu level.
    pub fn item_count(&self) -> usize {
        self.labels().len()
    }

    /// Move selection up (wraps around), skipping disabled items.
    /// Returns `true` if the selection moved (the caller plays the navigation sound only then).
    pub fn move_up(&mut self) -> bool {
        self.move_selection(false)
    }

    /// Move selection down (wraps around), skipping disabled items. Cf. [`Self::move_up`].
    pub fn move_down(&mut self) -> bool {
        self.move_selection(true)
    }

    /// One press moves the selection once, in either direction. Debouncing happens at the
    /// source: the router filters keyboard repeat and the gamepad stick latches with hysteresis.
    fn move_selection(&mut self, forward: bool) -> bool {
        let labels = self.labels();
        let n = labels.len();
        let mut next = self.selected;
        for _ in 0..n {
            // One cyclic step; disabled items are skipped by the loop.
            next = crate::app::focus::ring_move(next, n, if forward { 1 } else { -1 });
            if labels[next].1 {
                let moved = next != self.selected;
                self.selected = next;
                return moved;
            }
        }
        false
    }

    /// Confirm the selection: enters a branch (pushing the path) and does nothing on a leaf.
    /// Leaf actions are routed through `peek_confirm_action` (see `menu_tree::MenuAction`).
    pub fn confirm(&mut self) {
        use crate::app::menu_tree::MenuNode;
        if let Some(MenuNode::Branch { default, .. }) = self.selected_node() {
            // Enter the branch and remember the index in the path.
            let default = *default;
            self.path.push(self.selected);
            self.selected = default;
        }
    }

    /// Go back (ESC in a sub-menu, or close the modal). Pops the path and restores the
    /// parent's selection (the index taken is stored in `path`).
    pub fn back(&mut self) -> bool {
        if self.options_modal.is_some() {
            // Close the options screen; the path is unchanged, so we return to the sub-menu.
            self.options_modal = None;
            true
        } else if let Some(parent_index) = self.path.pop() {
            self.selected = parent_index;
            true
        } else {
            false
        }
    }

    /// Preview what Enter/Space would do without mutating.
    /// Used by input_handler to decide which AppCommands to produce.
    pub fn peek_confirm_action(&self) -> crate::app::input_handler::MenuConfirmAction {
        self.action_at(self.selected)
    }

    /// Confirm action for item `i` of the current level.
    /// Lets mouse and touch confirm the clicked row rather than the current selection.
    pub fn action_at(&self, i: usize) -> crate::app::input_handler::MenuConfirmAction {
        use crate::app::input_handler::MenuConfirmAction;
        use crate::app::menu_tree::{MenuAction, MenuNode};
        match crate::app::menu_tree::children_at(&self.path).get(i) {
            Some(MenuNode::Branch { .. }) => MenuConfirmAction::MenuConfirm,
            Some(MenuNode::Leaf { action, .. }) => match action {
                MenuAction::StartSolo => MenuConfirmAction::SelectMode(GameMode::Solo),
                MenuAction::StartVsComputer(d) => {
                    MenuConfirmAction::SelectMode(GameMode::VsComputer(*d))
                }
                MenuAction::OpenNetwork => MenuConfirmAction::OpenNetwork,
                MenuAction::OpenNetworkManual => MenuConfirmAction::OpenNetworkManual,
                MenuAction::OpenOptions(_) => MenuConfirmAction::OpenOptions,
                MenuAction::OpenInfo(section) => MenuConfirmAction::OpenInfo(*section),
                MenuAction::Quit => MenuConfirmAction::Quit,
                MenuAction::Back => MenuConfirmAction::Back,
            },
            None => MenuConfirmAction::MenuConfirm,
        }
    }

    /// Select item `i`, clamped to the current level. Used for absolute selection by
    /// mouse and touch (hover/click), unlike the relative `move_up`/`move_down`.
    pub fn select_index(&mut self, i: usize) {
        let n = self.item_count();
        if n > 0 {
            self.selected = i.min(n - 1);
        }
    }

    /// Check if OPTIONS is selected and Enter pressed — open modal.
    /// Returns true if modal was opened.
    /// `tab: Some(..)` is a deep link (CLI): it opens that section directly, whatever the
    /// menu selection. `None` takes the section from the selected Options leaf.
    pub fn try_open_options(
        &mut self, tab: Option<OptionsTab>, player_name: &str,
        sfx_enabled: bool, music_enabled: bool,
        sfx_volume: f32, music_volume: f32,
        language: Option<&str>, fullscreen: bool,
        key_bindings: crate::platform::persist::KeyBindings,
    ) -> bool {
        // The leaf action carries the section.
        use crate::app::menu_tree::{MenuAction, MenuNode};
        let section = match tab {
            Some(t) => t,
            None => match self.selected_node() {
                Some(MenuNode::Leaf { action: MenuAction::OpenOptions(tab), .. }) => *tab,
                _ => return false,
            },
        };
        let mut st = NameInputState::new(
            player_name, sfx_enabled, music_enabled,
            sfx_volume, music_volume, language, fullscreen, key_bindings,
        );
        st.tab = section;
        self.options_modal = Some(st);
        true
    }

    /// Whether we are in the VS Network zone: the network sub-menu, the ranking and
    /// opponents pages, or the LAN screen. Network services (TCP listener and mDNS) run
    /// there, so the player is reachable and announced anywhere in the zone.
    pub fn in_network_zone(&self) -> bool {
        use crate::app::menu_tree::{InfoSection, LabelKey};
        self.lan_screen.is_some()
            || matches!(self.info_section,
                Some(InfoSection::NetworkRank) | Some(InfoSection::NetworkOpponents))
            || crate::app::menu_tree::path_contains_label(&self.path, LabelKey::VsNetwork)
    }

    /// Items of the current level as `(label key, enabled)`. The renderer resolves the
    /// text. `enabled` is always `true` for now; it is the hook for per-item disabling.
    pub fn labels(&self) -> Vec<(crate::app::menu_tree::LabelKey, bool)> {
        crate::app::menu_tree::children_at(&self.path)
            .iter()
            .map(|node| (node.label(), true))
            .collect()
    }
}

/// Options modal tab
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum OptionsTab {
    #[default]
    Player,
    Sound,
    /// Keyboard configuration (per-key bindings).
    Keyboard,
    /// Gamepad configuration (direction mode, swap and raise buttons).
    Gamepad,
}

/// Incoming invite. This is session-level state, not LAN-screen state: the modal is drawn
/// over whichever screen is open (sub-menu, ranking, recent opponents...) and never
/// navigates away from it.
#[derive(Debug)]
pub struct IncomingInvite {
    pub from_name: String,
    /// Rating the inviter announces over mDNS, if visible.
    pub from_rating: Option<i32>,
    /// The invite was accepted; a "connecting" message shows during the handshake.
    pub connecting: bool,
    /// Button focus (0 = accept, 1 = decline), shared by keyboard, gamepad and mouse.
    pub selection: std::cell::Cell<usize>,
}

/// A peer visible in the LAN discovery list.
#[derive(Debug, Clone)]
pub struct LanPeer {
    pub name: String,
    pub addr: std::net::IpAddr,
    pub port: u16,
    pub session_id: String,
    pub status: crackattack_net::discovery::PeerStatus,
    /// Rating the peer announces (mDNS TXT record `rt`). `None` means unplaced.
    pub announced_rating: Option<i32>,
    /// Persistent peer identity (TXT record `uid`). A reliable reconnection target.
    pub uuid_hex: Option<String>,
    /// `false` when the peer runs another protocol version: listed for information, not
    /// playable (the player must update).
    pub compatible: bool,
}

/// After a version mismatch, which of the two games is the older one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OlderGame {
    /// Ours: the local player must update.
    Ours,
    /// The peer's: the other player must update.
    Theirs,
}

/// Error shown on the LAN screen. Variants are structured; the technical detail
/// (I/O or protocol error) is an untranslated `String`.
#[derive(Debug, Clone)]
pub enum LanError {
    PeerBusy,
    /// The peer runs another game version (incompatible protocol). The message says whose game
    /// is the older one, so the right player knows to update.
    VersionMismatch(OlderGame),
    Handshake(String),
    Connection(String),
    Discovery(String),
    PortBind(String),
    OwnAddress,
}

impl LanError {
    /// Localized message. Called every frame at render time; no translated string is cached.
    pub fn message(&self, l: &crate::app::locale::Locale) -> String {
        match self {
            Self::PeerBusy => l.error_peer_busy.to_string(),
            Self::VersionMismatch(OlderGame::Ours) => l.error_version_mismatch_ours.to_string(),
            Self::VersionMismatch(OlderGame::Theirs) => l.error_version_mismatch_theirs.to_string(),
            Self::Handshake(d) => l.error_handshake_fmt.replace("{}", d),
            Self::Connection(d) => l.error_connection_fmt.replace("{}", d),
            Self::Discovery(d) => l.error_discovery_fmt.replace("{}", d),
            Self::PortBind(d) => l.error_port_fmt.replace("{}", d),
            Self::OwnAddress => l.own_address_error.to_string(),
        }
    }
}

/// Guidance shown under a LAN screen error. After a version mismatch it is the profile's
/// update destination if there is one, otherwise a note that updates come from the
/// player's store or distribution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LanErrorHint {
    /// "See download": a navigable item that opens this URL (from the profile).
    Download(String),
    /// Note only; not navigable.
    ManagedBy(crate::platform::distribution::ManagedBy),
}

impl LanScreenPhase {
    /// Error phase without guidance (local errors: discovery, port, own address).
    pub fn error(error: LanError) -> Self {
        LanScreenPhase::Error { error, hint: None }
    }
}

/// Phase of the LAN discovery screen.
#[derive(Debug, Clone)]
pub enum LanScreenPhase {
    /// Scanning for peers, showing the list.
    Scanning,
    /// We clicked a peer, waiting for their accept/decline.
    Inviting { peer_name: String },
    /// Connection error; the player can retry or cancel. The error is structured and
    /// translated at display time through `LanError::message`, so it follows a language change.
    ///
    /// `hint` is guidance shown under the error. It is only set for a version mismatch, and
    /// the session resolves it when it creates the error.
    Error { error: LanError, hint: Option<LanErrorHint> },
    /// Quick reconnect to a saved opponent: a dedicated screen that tells the other
    /// player what to do and retries repeatedly while it stays open.
    Reconnecting { peer_name: String },
}

/// A pending quick reconnect (from the ranking page). Discovery is scanned for a short
/// grace period. If a peer with the same name shows up over mDNS, its fresh address wins.
/// Otherwise, when the period ends, the invite goes to the address saved in the log.
#[derive(Debug, Clone)]
pub struct PendingReconnect {
    pub name: String,
    /// Identity of the target peer (32 hex chars). The preferred target is the mDNS
    /// announcement carrying this `uid` (reliable, survives renames). The stored IP and
    /// port are the fallback.
    pub uuid_hex: String,
    pub addr: std::net::IpAddr,
    pub port: u16,
    /// Time of the next attempt. Attempts repeat while the reconnect screen is open, and
    /// each one prefers the peer's fresh mDNS address if it is visible.
    pub next_attempt: std::time::Instant,
}

/// State for the LAN discovery screen.
#[derive(Debug, Clone)]
pub struct LanScreenState {
    pub phase: LanScreenPhase,
    pub peers: Vec<LanPeer>,
    /// Quick reconnect in progress. `poll_pending_reconnect` drives it while the phase is
    /// `Reconnecting`.
    pub pending_reconnect: Option<PendingReconnect>,
    /// Error shown inside the manual screen (for example an invite to ourselves). A phase
    /// error would be hidden behind the dialog. Cleared on the next attempt.
    pub manual_error: Option<String>,
    /// The TCP port we're actually listening on.
    pub local_port: u16,
    /// Index of the selected peer in the list. Session-owned so that keyboard, gamepad
    /// and touch share a single selection.
    pub selected: usize,
    /// Manual IP/port dialog open. Session-owned so `main.rs` knows when not to intercept
    /// gamepad and keyboard navigation, which the text fields need.
    pub manual_open: bool,
    /// Manual screen state: IPv4/IPv6/port input plus focus (focus ring and on-screen keyboard).
    /// Owned by the session.
    pub manual: crate::app::manual_connect::ManualConnectState,
    /// Local network detection for the manual screen, done once each time the screen
    /// opens. `None` means not detected yet for this opening; `Some(None)` means detection
    /// ran and found no network. Re-detecting on each opening avoids showing a stale address.
    pub manual_net_info: Option<Option<crackattack_net::discovery::LocalNetInfo>>,
}

impl LanScreenState {
    pub fn new(local_port: u16) -> Self {
        Self {
            phase: LanScreenPhase::Scanning,
            peers: Vec::new(),
            pending_reconnect: None,
            manual_error: None,
            local_port,
            selected: 0,
            manual_open: false,
            manual: Default::default(),
            manual_net_info: None,
        }
    }

    pub fn peers(&self) -> &[LanPeer] {
        &self.peers
    }

    /// Number of navigable items for the current phase. It bounds keyboard and gamepad selection.
    /// - Scanning: the peers plus 1 (the footer Back button).
    /// - Error: the optional "see download" action plus Back. The peer list is not
    ///   shown, so it is not part of the focus ring.
    /// - Reconnecting: 2 (manual connection, Back).
    /// - Inviting: 0 (only Back/Esc acts).
    pub fn nav_count(&self) -> usize {
        match self.phase {
            LanScreenPhase::Scanning => self.peers.len() + 1,
            LanScreenPhase::Error { .. } => self.error_action_count() + 1,
            // Reconnecting: [Manual connection] / [Back].
            LanScreenPhase::Reconnecting { .. } => 2,
            LanScreenPhase::Inviting { .. } => 0,
        }
    }

    /// Index of the footer Back button: after the peers when scanning, after the optional
    /// guidance action when in the error phase.
    pub fn retour_index(&self) -> usize {
        match self.phase {
            LanScreenPhase::Error { .. } => self.error_action_count(),
            _ => self.peers.len(),
        }
    }

    /// Error phase: number of navigable actions before Back (0 or 1).
    fn error_action_count(&self) -> usize {
        match &self.phase {
            LanScreenPhase::Error { hint: Some(LanErrorHint::Download(_)), .. } => 1,
            _ => 0,
        }
    }

    /// Move the selection (keyboard or gamepad). It wraps top to bottom, like every
    /// vertical navigation in the game (main menu, option rows, on-screen keyboard).
    /// This is for the automatic screen (peer list); the manual connection screen has its
    /// own focus ring (`manual.nav_focus` in `app/manual_connect.rs`).
    pub fn move_selection(&mut self, delta: i32) {
        let n = self.nav_count();
        if n == 0 {
            self.selected = 0;
            return;
        }
        // Clamp first (peers may have left), then take a shared cyclic step. Peers that cannot
        // be played (another network version) are not in the ring: the step skips them.
        let mut next = self.selected.min(n - 1);
        for _ in 0..n {
            next = crate::app::focus::ring_move(next, n, delta);
            if self.is_selectable(next) {
                self.selected = next;
                return;
            }
        }
        self.selected = self.selected.min(n - 1);
    }

    /// Whether the focus ring can rest on item `i`. Only a peer on another network version is
    /// left out: it is listed for information.
    pub fn is_selectable(&self, i: usize) -> bool {
        !matches!(self.phase, LanScreenPhase::Scanning)
            || self.peers.get(i).map_or(true, |p| p.compatible)
    }

    /// Replaces the peer list (a discovery update) and keeps the selection on a selectable
    /// item: if the new list leaves it on a peer that cannot be played, it moves on to the
    /// next selectable item, ending on Back at worst.
    pub fn set_peers(&mut self, peers: Vec<LanPeer>) {
        self.peers = peers;
        let n = self.nav_count();
        if n == 0 {
            return;
        }
        let mut i = self.selected.min(n - 1);
        for _ in 0..n {
            if self.is_selectable(i) {
                break;
            }
            i = crate::app::focus::ring_move(i, n, 1);
        }
        self.selected = i;
    }
}

/// State for the options modal (name + audio + language).
#[derive(Debug, Clone)]
pub struct NameInputState {
    pub text: String,
    /// Format of the field being edited (max length, allowed characters, default...).
    pub format: crate::app::onscreen_keyboard::InputFormat,
    pub tab: OptionsTab,
    pub sfx_enabled: bool,
    pub music_enabled: bool,
    pub sfx_volume: f32,
    pub music_volume: f32,
    /// Language choice: `None` is Auto (system language), `Some(lang)` is explicit.
    pub language_choice: Option<crate::app::locale::Language>,
    pub fullscreen: bool,
    /// Whether music tracks are present. When `false`, the Music block is hidden and
    /// skipped by navigation, since an option with no effect would be misleading.
    pub music_available: bool,
    /// Editable key bindings (copied from UserConfig on open, applied on OK).
    pub key_bindings: crate::platform::persist::KeyBindings,
    /// Active key capture: which action + slot is waiting for a key press.
    /// Set by the Controls tab UI, consumed by main.rs keyboard handler.
    pub capture_binding: Option<(crate::platform::persist::BindingAction, usize)>,
    /// 2D focus `(row, col)` for row-based navigation (see `options_nav`).
    pub nav_focus: (usize, usize),
    /// Request to leave the options (changes apply live: apply, save, close). Set by
    /// Back/Esc/B and consumed by the renderer.
    pub confirm_requested: bool,
    /// On-screen keyboard open to edit the name (for players without a physical keyboard).
    pub keyboard_open: bool,
    /// Focus `(row, col)` in the on-screen keyboard grid.
    pub kb_focus: (usize, usize),
    /// Remembered target abscissa ("goal-X", 0-1) for vertical keyboard navigation. Only
    /// horizontal moves and clicks update it; vertical moves reproject onto it, so focus
    /// does not drift when crossing the narrow bottom row.
    pub kb_goal_x: f32,
}


impl NameInputState {
    pub fn new(
        initial: &str,
        sfx_enabled: bool, music_enabled: bool,
        sfx_volume: f32, music_volume: f32,
        language: Option<&str>,
        fullscreen: bool,
        key_bindings: crate::platform::persist::KeyBindings,
    ) -> Self {
        // A saved code gives an explicit language; a missing or unknown code gives Auto.
        let language_choice = language.and_then(crate::app::locale::Language::from_code);
        Self {
            text: initial.to_string(),
            format: crate::app::onscreen_keyboard::InputFormat::player_name(),
            tab: OptionsTab::Player,
            sfx_enabled,
            music_enabled,
            sfx_volume,
            music_volume,
            language_choice,
            fullscreen,
            music_available: true,
            key_bindings,
            capture_binding: None,
            nav_focus: (0, 0),
            confirm_requested: false,
            keyboard_open: false,
            kb_focus: (1, 0),
            kb_goal_x: 0.05,
        }
    }

    // --- Row-based navigation (see `options_nav`) ---

    /// Current navigable layout (see `options_nav::nav_layout`). The volume row of a
    /// disabled sound toggle is left out.
    fn nav_rows(&self) -> Vec<Vec<crate::app::options_nav::OptWidget>> {
        crate::app::options_nav::nav_layout(self.tab, self.sfx_enabled, self.music_enabled)
            .into_iter()
            .filter(|row| self.music_available || !matches!(row.as_slice(),
                [crate::app::options_nav::OptWidget::MusicEnabled]
                | [crate::app::options_nav::OptWidget::MusicVolume]))
            .collect()
    }

    /// Focus a given widget (mouse hover moves focus). Does nothing if the widget is not
    /// in the current tab's layout.
    pub fn focus_widget(&mut self, target: crate::app::options_nav::OptWidget) {
        let rows = self.nav_rows();
        for (r, row) in rows.iter().enumerate() {
            if let Some(c) = row.iter().position(|w| *w == target) {
                self.nav_focus = (r, c);
                return;
            }
        }
    }

    /// The widget focused according to `nav_focus` and the tab's layout.
    pub fn focused_widget(&self) -> Option<crate::app::options_nav::OptWidget> {
        let rows = self.nav_rows();
        rows.get(self.nav_focus.0)
            .and_then(|r| r.get(self.nav_focus.1))
            .copied()
    }

    /// Move the focus. Up/Down (`dy`) change row, wrapping. Left/Right (`dx`) adjust a
    /// value widget, or otherwise move within the row.
    pub fn opt_move(&mut self, dx: i32, dy: i32) {
        let rows = self.nav_rows();
        if rows.is_empty() {
            return;
        }
        if dy != 0 {
            let row = crate::app::focus::ring_move(self.nav_focus.0, rows.len(), dy);
            let col = self.nav_focus.1.min(rows[row].len().saturating_sub(1));
            self.nav_focus = (row, col);
        } else if dx != 0 {
            let (row, col) = self.nav_focus;
            match rows.get(row).and_then(|r| r.get(col)).copied() {
                Some(w) if w.is_value() => self.adjust_widget(w, dx),
                _ => {
                    let ncols = rows.get(row).map(|r| r.len()).unwrap_or(1).max(1);
                    self.nav_focus = (row, crate::app::focus::clamp_move(col, ncols, dx));
                }
            }
        }
    }

    /// Cycle the language choice: Auto, then each of `Language::ALL`, then Auto again.
    /// `dir` is +1 or -1. It indexes `Language::ALL`, so it scales with the number of languages.
    fn cycle_language(&mut self, dir: i32) {
        use crate::app::locale::Language;
        let n = Language::ALL.len() + 1; // +1 for "Auto"
        let cur = match self.language_choice {
            None => 0,
            Some(l) => 1 + Language::ALL.iter().position(|&x| x == l).unwrap_or(0),
        };
        let next = crate::app::focus::ring_move(cur, n, dir);
        self.language_choice = (next != 0).then(|| Language::ALL[next - 1]);
    }

    /// Adjust a value widget (language cycle, toggles, sliders in 5% steps).
    fn adjust_widget(&mut self, w: crate::app::options_nav::OptWidget, dir: i32) {
        use crate::app::options_nav::OptWidget;
        match w {
            OptWidget::Language => self.cycle_language(dir),
            OptWidget::Fullscreen => self.fullscreen = !self.fullscreen,
            OptWidget::SfxEnabled => self.sfx_enabled = !self.sfx_enabled,
            OptWidget::MusicEnabled => self.music_enabled = !self.music_enabled,
            OptWidget::SfxVolume => {
                self.sfx_volume = (self.sfx_volume + dir as f32 * 0.05).clamp(0.0, 1.0);
            }
            OptWidget::MusicVolume => {
                self.music_volume = (self.music_volume + dir as f32 * 0.05).clamp(0.0, 1.0);
            }
            OptWidget::DirMode => {
                use crate::platform::persist::DirMode;
                let order = [DirMode::DPad, DirMode::Stick, DirMode::Both];
                let i = order.iter().position(|&m| m == self.key_bindings.dir_mode).unwrap_or(0);
                self.key_bindings.dir_mode = order[crate::app::focus::ring_move(i, order.len(), dir)];
            }
            _ => {}
        }
    }

    /// Activate the focused widget (`A`). Back sets the exit request (consumed by the
    /// renderer), Name opens the on-screen keyboard, and value widgets toggle or cycle.
    pub fn opt_activate(&mut self) {
        use crate::app::options_nav::OptWidget;
        match self.focused_widget() {
            Some(OptWidget::EditName) => self.kb_open(),
            Some(OptWidget::Language) => self.cycle_language(1),
            Some(OptWidget::Fullscreen) => self.fullscreen = !self.fullscreen,
            Some(OptWidget::SfxEnabled) => self.sfx_enabled = !self.sfx_enabled,
            Some(OptWidget::MusicEnabled) => self.music_enabled = !self.music_enabled,
            Some(OptWidget::DirMode) => self.adjust_widget(OptWidget::DirMode, 1),
            Some(OptWidget::BindingSlot(action, slot)) => {
                // Arm capture: the next key or button is assigned to this slot.
                self.capture_binding = Some((action, slot));
            }
            Some(OptWidget::ResetBindings) => {
                self.key_bindings = crate::platform::persist::KeyBindings::default();
                self.capture_binding = None;
            }
            // Back leaves the options (changes apply live: apply, save, close).
            Some(OptWidget::Back) => self.confirm_requested = true,
            _ => {}
        }
    }

    // --- On-screen keyboard: edits `self.text` directly ---

    /// Open the on-screen keyboard, with focus on the first letter.
    pub fn kb_open(&mut self) {
        self.keyboard_open = true;
        self.kb_focus_to(1, 0);
    }

    /// Close the on-screen keyboard.
    pub fn kb_close(&mut self) {
        self.keyboard_open = false;
    }

    /// Focus the keyboard on `(row, col)` and remember the target abscissa. Used on click
    /// and after a horizontal move. Delegates to the model.
    pub fn kb_focus_to(&mut self, row: usize, col: usize) {
        crate::app::onscreen_keyboard::osk_focus_to(
            &mut self.kb_focus, &mut self.kb_goal_x, self.format.charset, row, col);
    }

    /// Move the keyboard focus. Vertical moves reproject onto the target abscissa;
    /// horizontal moves wrap. Delegates to the model, whose logic the network editor shares.
    pub fn kb_move(&mut self, dx: i32, dy: i32) {
        crate::app::onscreen_keyboard::osk_move(
            &mut self.kb_focus, &mut self.kb_goal_x, self.format.charset, dx, dy);
    }

    /// Activate the focused key (insert, backspace, clear or OK). Closes the keyboard on `Done`.
    pub fn kb_activate(&mut self) {
        use crate::app::onscreen_keyboard::{osk_activate, KbOutcome};
        if osk_activate(&mut self.text, self.kb_focus, self.format) == KbOutcome::Close {
            self.keyboard_open = false;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flow_state_helpers() {
        assert!(FlowState::Menu.is_menu());
        assert!(!FlowState::Menu.is_post_game());
        assert!(!FlowState::Menu.is_in_game());

        assert!(FlowState::Playing.is_in_game());
        assert!(!FlowState::Playing.is_menu());

        assert!(FlowState::PostGame.is_post_game());
        assert!(FlowState::PostGame.is_in_game());
        assert!(!FlowState::PostGame.is_menu());
    }

    /// The main menu selection wraps, and consecutive presses each move it.
    /// Skipping disabled items (`labels[i].1`) cannot be tested here: `labels()` returns
    /// `true` for every item, since no item in the tree can be disabled.
    #[test]
    fn menu_selection_wraps() {
        let mut m = MenuState::new();
        let n = m.item_count();
        assert!(n >= 2, "the root tree must have several items");

        // Presses without a tick in between each move, in both directions.
        assert!(m.move_down());
        assert!(m.move_down());
        assert_eq!(m.selected, 2);
        assert!(m.move_up());
        assert!(m.move_up());
        assert_eq!(m.selected, 0);

        // Vertical wrap: Down from the last goes to the first; Up from the first goes to the last.
        m.selected = n - 1;
        m.move_down();
        assert_eq!(m.selected, 0);
        m.move_up();
        assert_eq!(m.selected, n - 1);
    }

    fn peer(name: &str) -> LanPeer {
        LanPeer {
            name: name.to_string(),
            addr: std::net::IpAddr::from([127, 0, 0, 1]),
            port: 45678,
            session_id: String::new(),
            status: crackattack_net::discovery::PeerStatus::Available,
            announced_rating: None,
            uuid_hex: None,
            compatible: true,
        }
    }

    /// In the error phase the peer list is hidden, so it is not in the focus ring: only the
    /// optional "see download" action and Back are reachable. Confirm must not target a hidden peer.
    #[test]
    fn lan_error_ring_excludes_hidden_peers() {
        let mut s = LanScreenState::new(45678);
        s.peers = vec![peer("A"), peer("B")];

        s.phase = LanScreenPhase::error(LanError::PeerBusy);
        assert_eq!((s.nav_count(), s.retour_index()), (1, 0));
        s.selected = 0;
        s.move_selection(1);
        assert_eq!(s.selected, 0, "Back alone: nothing else to reach");

        s.phase = LanScreenPhase::Error {
            error: LanError::VersionMismatch(OlderGame::Ours),
            hint: Some(LanErrorHint::Download("https://example.test/releases".into())),
        };
        assert_eq!((s.nav_count(), s.retour_index()), (2, 1));
        s.move_selection(1);
        assert_eq!(s.selected, 1);
        s.move_selection(1);
        assert_eq!(s.selected, 0, "wraps over [Download, Back]");

        // The "managed by" note is not navigable.
        s.phase = LanScreenPhase::Error {
            error: LanError::VersionMismatch(OlderGame::Ours),
            hint: Some(LanErrorHint::ManagedBy(crate::platform::distribution::ManagedBy::Store)),
        };
        assert_eq!((s.nav_count(), s.retour_index()), (1, 0));

        // Scanning: the peers are back in the ring.
        s.phase = LanScreenPhase::Scanning;
        assert_eq!((s.nav_count(), s.retour_index()), (3, 2));
    }

    /// A peer on another network version is listed but outside the focus ring: the keyboard
    /// and gamepad step over it, and a list update never leaves the selection on it.
    #[test]
    fn lan_selection_skips_other_version_peers() {
        let mut s = LanScreenState::new(45678);
        let mut other = peer("Old");
        other.compatible = false;
        // [A, Old, Back]: compatible peers come first, then the other-version ones.
        s.set_peers(vec![peer("A"), other.clone()]);
        assert_eq!((s.nav_count(), s.retour_index()), (3, 2));
        assert!(s.is_selectable(0) && !s.is_selectable(1) && s.is_selectable(2));
        s.selected = 0;
        s.move_selection(1);
        assert_eq!(s.selected, 2, "Old is stepped over: A -> Back");
        s.move_selection(-1);
        assert_eq!(s.selected, 0, "Back -> A, not Old");

        // Only an other-version peer: the selection rests on Back.
        s.selected = 0;
        s.set_peers(vec![other.clone()]);
        assert_eq!(s.selected, s.retour_index());
        s.move_selection(1);
        assert_eq!(s.selected, s.retour_index(), "nothing else to reach");

        // The selected compatible peer disappears while an other-version one takes its index.
        s.set_peers(vec![peer("A"), other.clone()]);
        s.selected = 0;
        s.set_peers(vec![other]);
        assert_eq!(s.selected, s.retour_index());
    }

    /// LAN list navigation stays bounded (keyboard and gamepad share this code). While
    /// scanning, the last navigable item is the footer Back button (index `peers.len()`),
    /// not the last peer.
    #[test]
    fn lan_selection_wraps() {
        let mut s = LanScreenState::new(45678);
        // Empty list: a single item, so wrapping stays on 0.
        s.move_selection(1);
        s.move_selection(-1);
        assert_eq!(s.selected, 0);

        // nav = [A, B, C, Back] -> indices 0..=3. Selection wraps (see `move_selection`).
        s.peers = vec![peer("A"), peer("B"), peer("C")];
        s.move_selection(1);
        assert_eq!(s.selected, 1);
        s.move_selection(1);
        s.move_selection(1); // last = Back (index 3 = peers.len())
        assert_eq!(s.selected, 3);
        s.move_selection(1); // wraps to the top
        assert_eq!(s.selected, 0);
        s.move_selection(-1); // wraps to the bottom
        assert_eq!(s.selected, 3);

        // A selection left out of range (peers left) is clamped on the next move.
        s.selected = 99;
        s.move_selection(0);
        assert_eq!(s.selected, 3);
    }

    fn options() -> NameInputState {
        NameInputState::new(
            "Joueur", true, true, 0.5, 0.5, None, false,
            crate::platform::persist::KeyBindings::default(),
        )
    }

    /// Without music tracks, the Music block is removed from navigation (Sound page).
    #[test]
    fn options_music_rows_hidden_without_tracks() {
        use crate::app::options_nav::OptWidget;
        let mut m = options();
        m.tab = OptionsTab::Sound;
        m.music_enabled = true;
        let has = |m: &NameInputState, w| m.nav_rows().iter().flatten().any(|x| *x == w);
        assert!(has(&m, OptWidget::MusicEnabled) && has(&m, OptWidget::MusicVolume));
        m.music_available = false;
        assert!(!has(&m, OptWidget::MusicEnabled) && !has(&m, OptWidget::MusicVolume));
        assert!(has(&m, OptWidget::SfxEnabled) && has(&m, OptWidget::Back));
    }

    /// Row navigation on the Player page (2D focus; adjust versus move).
    #[test]
    fn options_row_nav_and_activation() {
        use crate::app::options_nav::OptWidget;
        // Player layout: [EditName] [Language] [Fullscreen] [Ok, Cancel]
        let mut m = options();
        assert_eq!(m.focused_widget(), Some(OptWidget::EditName));

        // Up/Down change row, wrapping.
        m.opt_move(0, 1);
        assert_eq!(m.focused_widget(), Some(OptWidget::Language));
        m.opt_move(0, -1);
        assert_eq!(m.focused_widget(), Some(OptWidget::EditName));
        m.opt_move(0, -1); // wraps to the last row
        assert_eq!(m.focused_widget(), Some(OptWidget::Back));

        // The last row is [Back], the single exit item.
        m.nav_focus = (3, 0);
        assert_eq!(m.focused_widget(), Some(OptWidget::Back));
        m.opt_move(1, 0); // clamped (single item)
        assert_eq!(m.focused_widget(), Some(OptWidget::Back));

        // Value widget: Left/Right adjust (language cycles Auto, languages, Auto).
        use crate::app::locale::Language;
        m.nav_focus = (1, 0); // Language
        assert_eq!(m.language_choice, None); // Auto
        m.opt_move(1, 0);
        assert_eq!(m.language_choice, Some(Language::English)); // ALL[0]
        m.opt_move(-1, 0);
        assert_eq!(m.language_choice, None); // back to Auto
        m.opt_move(-1, 0); // wraps from Auto to the last language
        assert_eq!(m.language_choice, Language::ALL.last().copied()); // ALL[last]

        // Fullscreen: Left/Right toggle.
        m.nav_focus = (2, 0);
        assert!(!m.fullscreen);
        m.opt_move(1, 0);
        assert!(m.fullscreen);

        // A on Name opens the on-screen keyboard.
        m.nav_focus = (0, 0);
        m.opt_activate();
        assert!(m.keyboard_open);

        // A on Back sets the exit request.
        let mut m2 = options();
        m2.nav_focus = (3, 0); // Back
        m2.opt_activate();
        assert!(m2.confirm_requested);
    }

    /// Row navigation on the Sound page (toggles and adjustable sliders).
    #[test]
    fn options_sound_row_nav() {
        use crate::app::options_nav::OptWidget;
        // Sound layout (all enabled): [SfxEnabled] [SfxVolume] [MusicEnabled] [MusicVolume] [Back]
        let mut m = NameInputState::new(
            "X", true, true, 0.5, 0.5, None, false,
            crate::platform::persist::KeyBindings::default(),
        );
        m.tab = OptionsTab::Sound;
        m.nav_focus = (0, 0);
        assert_eq!(m.focused_widget(), Some(OptWidget::SfxEnabled));

        // SFX toggle: Left/Right switch it.
        m.opt_move(1, 0);
        assert!(!m.sfx_enabled);
        // With SFX off, the SFX volume leaves the path: Down from SfxEnabled goes straight
        // to MusicEnabled (the SfxVolume row is skipped).
        m.opt_move(0, 1);
        assert_eq!(m.focused_widget(), Some(OptWidget::MusicEnabled));

        // Re-enable SFX to test the volume.
        m.nav_focus = (0, 0);
        m.opt_move(1, 0);
        assert!(m.sfx_enabled);

        // SFX volume slider: Left/Right adjust by 5%, clamped.
        m.nav_focus = (1, 0);
        assert_eq!(m.focused_widget(), Some(OptWidget::SfxVolume));
        m.opt_move(1, 0);
        assert!((m.sfx_volume - 0.55).abs() < 1e-6);
        m.opt_move(-1, 0);
        assert!((m.sfx_volume - 0.50).abs() < 1e-6);

        // Same for the music volume (music enabled).
        m.nav_focus = (3, 0);
        assert_eq!(m.focused_widget(), Some(OptWidget::MusicVolume));
        m.opt_move(-1, 0);
        assert!((m.music_volume - 0.45).abs() < 1e-6);

        // The last row is [Back], the single exit item.
        m.nav_focus = (4, 0);
        assert_eq!(m.focused_widget(), Some(OptWidget::Back));
    }

    /// Row navigation on the controls pages: binding slots (A arms capture), the `dir_mode`
    /// selector (Left/Right cycle) and the Reset button.
    #[test]
    fn options_controls_row_nav() {
        use crate::app::options_nav::OptWidget;
        use crate::platform::persist::{BindingAction, DirMode};
        let mut m = NameInputState::new(
            "X", true, true, 0.5, 0.5, None, false,
            crate::platform::persist::KeyBindings::default(),
        );
        // === Keyboard page ===
        m.tab = OptionsTab::Keyboard;
        // Directions have a single keyboard slot (the arrow key); there is no WASD default.
        m.nav_focus = (0, 0);
        assert_eq!(m.focused_widget(), Some(OptWidget::BindingSlot(BindingAction::Up, 0)));

        // A on a slot arms capture for the right (action, slot).
        m.opt_activate();
        assert_eq!(m.capture_binding, Some((BindingAction::Up, 0)));
        m.capture_binding = None;

        // Footer = [Back, ResetBindings]: Reset (col 1) restores the default bindings.
        let footer = crate::app::options_nav::layout(OptionsTab::Keyboard)
            .iter().position(|r| r == &vec![OptWidget::Back, OptWidget::ResetBindings]).unwrap();
        m.key_bindings.up = vec!["KeyZ".into()];
        m.nav_focus = (footer, 1);
        assert_eq!(m.focused_widget(), Some(OptWidget::ResetBindings));
        m.opt_activate();
        assert_eq!(m.key_bindings.up, crate::platform::persist::KeyBindings::default().up);

        // === Gamepad page ===
        m.tab = OptionsTab::Gamepad;
        let dirmode_row = crate::app::options_nav::layout(OptionsTab::Gamepad)
            .iter().position(|r| r == &vec![OptWidget::DirMode]).unwrap();
        m.nav_focus = (dirmode_row, 0);
        assert_eq!(m.focused_widget(), Some(OptWidget::DirMode));
        assert_eq!(m.key_bindings.dir_mode, DirMode::Both);
        m.opt_move(1, 0); // Both -> DPad (cycle)
        assert_eq!(m.key_bindings.dir_mode, DirMode::DPad);
        m.opt_move(-1, 0); // DPad -> Both
        assert_eq!(m.key_bindings.dir_mode, DirMode::Both);
    }

    /// On-screen keyboard wiring on `NameInputState` (open, navigate, type, OK closes),
    /// through the pure model in `app::onscreen_keyboard`.
    #[test]
    fn onscreen_keyboard_wiring() {
        let mut m = options();
        m.text.clear();
        m.kb_open();
        assert!(m.keyboard_open);
        assert_eq!(m.kb_focus, (1, 0)); // 'A'

        // Type 'A' (the key focused at the start).
        m.kb_activate();
        assert_eq!(m.text, "A");

        // Move down one row (K...) and type.
        m.kb_move(0, 1);
        m.kb_activate();
        assert_eq!(m.text, "AK");

        // The test does not depend on exact key positions: it checks that Done closes.
        m.text = "BOB".to_string();
        // Focus 'Done': last row, last column.
        let rows = crate::app::onscreen_keyboard::layout(m.format.charset);
        m.kb_focus = (rows.len() - 1, rows.last().unwrap().len() - 1);
        m.kb_activate(); // Done closes
        assert!(!m.keyboard_open);
        assert_eq!(m.text, "BOB"); // text kept
    }

    /// Vertical keyboard navigation keeps the abscissa and does not drift when crossing the
    /// narrow bottom row (the goal-X is remembered).
    #[test]
    fn keyboard_vertical_nav_keeps_abscissa() {
        let mut m = options();
        m.kb_open();
        // Column 7 of the letter row (goal-X ~0.75).
        for _ in 0..7 {
            m.kb_move(1, 0);
        }
        assert_eq!(m.kb_focus, (1, 7));
        // Go down to the bottom row (4 keys); focus reprojects onto the key containing 0.75.
        m.kb_move(0, 1);
        m.kb_move(0, 1);
        m.kb_move(0, 1);
        assert_eq!(m.kb_focus, (4, 3));
        // Going back up: the bottom letter row "UVWXYZ.-_" has 9 keys. Goal-X ~0.75
        // reprojects onto column 6 ("."), with no lateral drift kept between moves.
        m.kb_move(0, -1);
        assert_eq!(m.kb_focus, (3, 6));
    }

    /// Horizontal keyboard navigation wraps from one end to the other.
    #[test]
    fn keyboard_horizontal_nav_wraps() {
        let mut m = options();
        m.kb_open();
        // Row 1 = "ABCDEFGHIJ" (10 keys). Left from col 0 wraps to col 9.
        m.kb_focus = (1, 0);
        m.kb_move(-1, 0);
        assert_eq!(m.kb_focus, (1, 9));
        // Right from col 9 wraps to col 0.
        m.kb_move(1, 0);
        assert_eq!(m.kb_focus, (1, 0));
    }

    #[test]
    fn menu_state_navigation() {
        let mut menu = MenuState::new();
        assert_eq!(menu.selected, 0);
        assert_eq!(menu.depth(), 0);

        menu.move_down();
        assert_eq!(menu.selected, 1); // VS COMPUTER

        menu.move_down();
        assert_eq!(menu.selected, 2); // VS NETWORK
    }

    #[test]
    fn menu_state_solo_confirm() {
        let menu = MenuState::new();
        assert_eq!(menu.selected, 0); // SOLO
        // Leaf actions are routed through `peek_confirm_action`.
        use crate::app::input_handler::MenuConfirmAction;
        assert!(matches!(
            menu.peek_confirm_action(),
            MenuConfirmAction::SelectMode(GameMode::Solo)
        ));
    }

    #[test]
    fn menu_state_vs_submenu() {
        let mut menu = MenuState::new();
        menu.move_down(); // VS COMPUTER
        menu.confirm(); // enters sub-menu
        assert_eq!(menu.depth(), 1);
        assert_eq!(menu.selected, 1); // Medium default

        use crate::app::input_handler::MenuConfirmAction;
        assert!(matches!(
            menu.peek_confirm_action(),
            MenuConfirmAction::SelectMode(GameMode::VsComputer(_))
        ));
    }

    #[test]
    fn menu_state_back() {
        let mut menu = MenuState::new();
        menu.move_down();
        menu.confirm(); // enter sub-menu
        assert_eq!(menu.depth(), 1);

        let went_back = menu.back();
        assert!(went_back);
        assert_eq!(menu.depth(), 0);
        // `back()` restores the parent's selection (VS Computer = index 1).
        assert_eq!(menu.selected, 1);
    }

    /// The Options branch leads to `OpenOptions(section)` leaves, which
    /// `peek_confirm_action` resolves.
    #[test]
    fn menu_state_options_submenu_tree() {
        use crate::app::input_handler::MenuConfirmAction;
        let mut menu = MenuState::new();
        // Go to OPTIONS (index 3) and enter.
        menu.selected = 3;
        menu.confirm();
        assert_eq!(menu.depth(), 1);
        assert_eq!(menu.selected, 0); // Player is the default

        // Every sub-menu entry is OpenOptions.
        assert!(matches!(menu.peek_confirm_action(), MenuConfirmAction::OpenOptions));
        // `back()` returns to OPTIONS (index 3) in the main menu.
        assert!(menu.back());
        assert_eq!(menu.depth(), 0);
        assert_eq!(menu.selected, 3);
    }
}
