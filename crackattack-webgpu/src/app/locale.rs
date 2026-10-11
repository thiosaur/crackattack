//! Localization strings for all user-facing text.
//!
//! **Reference language: English (EN)**
//!
//! To add a language:
//! 1. Add a new `const XX: Locale` below with all fields translated
//! 2. Add the variant to the `Language` enum
//! 3. Wire it into `current()` and `set_language()`
//!
//! ## Translation guidelines
//! - Each field has a doc comment describing **context** (where it appears),
//!   **constraints** (max length, rendering), and **notes** for translators.
//! - Fields ending in `_fmt` contain `{}` placeholders — keep them in place.
//! - **Rendering**: essentially ALL UI text (menu included) is drawn with **egui**
//!   (full Unicode, so accents are fine and there is no ASCII or length limit). The bitmap
//!   font (`text_texture.rs`) is used ONLY for player **nameplates** (ASCII, accents
//!   stripped, about 13-14 visible characters, no apostrophe). Nameplates show player
//!   names, not translatable strings, so no `Locale` field is bitmap-rendered.

use std::sync::atomic::{AtomicU8, Ordering};

/// All translatable strings for the game UI.
pub struct Locale {
    // Main menu — [egui] Unicode. ALL CAPS is a visual convention, not a technical
    // constraint: accents and length are free.

    /// Main menu item: solo game mode.
    pub menu_solo: &'static str,
    /// Main menu item: play against AI.
    pub menu_vs_computer: &'static str,
    /// Main menu item: play over LAN.
    pub menu_vs_network: &'static str,
    /// Main menu item: open settings.
    pub menu_options: &'static str,
    /// Main menu: accessibility label of the Info icon (not shown as text).
    pub menu_info: &'static str,
    /// Info submenu: "Rules" entry.
    pub menu_rules: &'static str,
    /// Info submenu: "Credits" entry.
    pub menu_credits: &'static str,
    pub menu_updates: &'static str,
    /// Main menu: "Quit" (closes the game).
    pub menu_quit: &'static str,
    /// Leaves a SUB-MENU for the parent menu. Distinct from `back` (generic dialog
    /// button), `cancel` (cancels an action) and `menu_label` (the "Menu" title).
    pub menu_back: &'static str,
    /// Difficulty submenu: easy AI.
    pub menu_easy: &'static str,
    /// Difficulty submenu: medium AI.
    pub menu_medium: &'static str,
    /// Difficulty submenu: hard AI.
    pub menu_hard: &'static str,

    // Pause modal — [egui] centered modal over frozen game

    /// Pause modal title. Context: displayed at top of pause overlay.
    pub pause_title: &'static str,
    /// Pause modal: resume button (solo/vs-computer).
    pub resume: &'static str,
    /// Pause modal: quit button (returns to menu).
    pub quit: &'static str,

    // Pause context messages — [egui] status text inside pause modal

    /// Pause modal: opponent has disconnected (network, terminal).
    pub opponent_disconnected: &'static str,

    // Symmetric pause (network) — [egui] ready-to-resume workflow

    /// Pause modal: button text before confirming resume (network).
    pub ready_to_resume: &'static str,
    /// Pause modal: button text after confirming resume. Contains ✓ symbol.
    pub ready_check: &'static str,
    /// Pause modal: status when local player is ready but opponent isn't.
    pub waiting_opponent_resume: &'static str,
    /// Pause modal: status when opponent is ready but local player isn't.
    pub opponent_ready: &'static str,

    // Ready modal — [egui] "Ready?" prompt between VS rounds

    /// Ready modal: main button text.
    pub ready: &'static str,
    /// Post-round summary screen: advance to the ready prompt (safe, engages nothing).
    pub next: &'static str,
    /// Ready modal (rounds ≥ 2): the fused consent question — being ready IS
    /// consenting to another round.
    pub ready_new_round: &'static str,
    /// Post-round screen, reduced offer (opponent gone): the only exit.
    pub menu_label: &'static str,

    // Waiting banner — [egui] centered banner during network waits

    /// Banner: waiting for remote player (pre-game or between rounds).
    pub waiting_for_opponent: &'static str,
    /// Banner: connection lost (during gameplay).
    pub connection_lost_banner: &'static str,

    // Network modal — [egui] LAN discovery and manual connection screen

    /// Network modal: window title.
    pub vs_network: &'static str,
    /// Network modal: cancel button.
    pub cancel: &'static str,
    /// Manual connect: indicator that the game always listens for incoming connections.
    pub listening_incoming: &'static str,
    /// Network submenu: entry opening the automatic (mDNS) discovery screen.
    pub network_auto: &'static str,
    /// Manual connect: verb prefix for the "Set IPv4/IPv6/Port" buttons.
    pub define_field: &'static str,
    /// Manual connect: warning when an entered IPv4/IPv6 address is malformed.
    pub invalid_address: &'static str,
    /// Network modal: manual connection tab title (and toggle to the manual screen).
    pub manual_connection: &'static str,
    /// Manual connect: label for our own address to share with the opponent.
    pub your_address: &'static str,
    /// Manual connect: label for the field where you paste the opponent's address.
    pub opponent_address: &'static str,
    /// Manual connect: copy-our-address button.
    pub copy_address: &'static str,
    /// Manual connect: paste-opponent-address button (reads clipboard, splits ip/port).
    pub paste_address: &'static str,
    /// Manual connect: invalid port warning (non-empty, not a valid 0-65535).
    pub invalid_port: &'static str,
    /// Manual connect: "Port" field label (opponent).
    pub port_field: &'static str,
    /// Network modal: suffix for a peer busy negotiating an invitation (not in a game yet).
    pub busy_suffix: &'static str,
    /// Network modal: back button.
    pub back: &'static str,
    /// Network modal: section title for discovered players.
    pub players_on_lan: &'static str,
    /// Network modal: status while scanning for LAN players.
    pub looking_for_players: &'static str,
    /// Network modal: scanning indicator (short).
    pub scanning: &'static str,
    /// Network modal: no network available message.
    pub no_network_detected: &'static str,
    /// Network modal: connect button.
    pub connect: &'static str,
    /// Network modal: waiting for specific player. `{}` = player name.
    pub waiting_for_fmt: &'static str,
    /// Network modal: incoming invitation. `{}` = player name.
    pub wants_to_play_fmt: &'static str,
    /// Plain "Accept" label (for navigable buttons, no key shown).
    pub accept: &'static str,
    /// Plain "Decline" label.
    pub decline: &'static str,
    /// Network modal: connecting status. `{}` = IP or player name.
    pub connecting_to_fmt: &'static str,
    /// Network modal: suffix shown next to a player who is in a game.
    pub in_game_suffix: &'static str,

    // Options modal — [egui] settings tabs (Player, Sound, Controls)

    /// Options modal: Player tab label.
    pub player_tab: &'static str,
    /// Options modal: Sound tab label.
    pub sound_tab: &'static str,
    /// Options modal: Controls tab label.
    pub keyboard_tab: &'static str,
    /// Options modal: Gamepad tab label.
    pub gamepad_tab: &'static str,
    // Controls tab labels.
    /// Controls tab: section titles.
    pub ctrl_movement: &'static str,
    pub ctrl_actions: &'static str,
    pub ctrl_directions: &'static str,
    /// Controls tab: rebindable actions (one per row).
    pub act_move_up: &'static str,
    pub act_move_down: &'static str,
    pub act_move_left: &'static str,
    pub act_move_right: &'static str,
    pub act_swap: &'static str,
    pub act_raise: &'static str,
    pub act_pause: &'static str,
    /// Gamepad controls: direction mode.
    pub dir_dpad: &'static str,
    pub dir_stick: &'static str,
    pub dir_both: &'static str,
    /// Key names written as full words. Short codes such as LB/RB/D-Up are not translated.
    pub key_space: &'static str,
    pub key_enter: &'static str,
    pub key_esc: &'static str,
    pub key_backspace: &'static str,
    /// On-screen keyboard (OSK): ABBREVIATED labels for narrow keys, distinct from the
    /// full words above. The space key reuses `key_space`.
    pub osk_backspace: &'static str,
    pub osk_clear: &'static str,
    /// Default player name, used at first launch and when the name field is cleared.
    /// Strict ASCII (it is drawn on the bitmap nameplate) and at most 18 characters.
    /// It is saved on the first save and does NOT change with the language afterwards.
    pub default_player_name: &'static str,
    /// Language selector: "Auto" entry (system language detection).
    pub language_auto: &'static str,
    // LAN screen, Error phase: structured messages translated at display time
    // (`LanError::message`). The `{}` detail is technical (io/protocol) and not translated.
    /// The invited peer is busy with another negotiation.
    pub error_peer_busy: &'static str,
    /// The peer runs another game version (incompatible protocol). Player-facing
    /// explanation, no technical detail.
    pub error_version_mismatch: &'static str,
    pub error_handshake_fmt: &'static str,
    pub error_connection_fmt: &'static str,
    pub error_discovery_fmt: &'static str,
    pub error_port_fmt: &'static str,
    /// Options modal: player name input label.
    pub player_name: &'static str,
    /// Options modal: "Edit" button (opens the name editor / on-screen keyboard).
    pub edit_name: &'static str,
    /// Options modal: language selector label.
    pub language_label: &'static str,
    /// Options modal: fullscreen checkbox label.
    pub fullscreen: &'static str,
    /// Options modal: sound effects toggle/slider label.
    pub sound_effects: &'static str,
    /// Options modal: music toggle/slider label.
    pub music: &'static str,
    /// Options modal: Controls tab — capture prompt.
    pub press_key_or_button: &'static str,
    /// Options modal: Controls tab — cancel capture hint. `{}` = key/button label.
    pub cancel_capture_hint_fmt: &'static str,
    /// Options modal: Controls tab — reset button.
    pub reset_defaults: &'static str,

    // End screen — [egui] post-game panels (high scores, stats)

    /// High scores panel: title.
    pub high_scores: &'static str,
    /// High scores panel: empty state message.
    pub no_scores_yet: &'static str,
    /// Stats panel: title (solo and VS).
    pub game_stats: &'static str,

    // Stat labels — [egui] left column in stats grid
    // Constraint: keep concise (~20 chars max), end with colon.

    /// Stat label: player score (solo mode).
    pub stat_score: &'static str,
    /// Stat label: round/game duration.
    pub stat_duration: &'static str,
    /// Stat label: total blocks eliminated.
    pub stat_blocks_eliminated: &'static str,
    /// Stat label: number of chain reactions.
    pub stat_chains: &'static str,
    /// Stat label: highest chain multiplier reached.
    pub stat_best_chain: &'static str,
    /// Stat label: garbage blocks shattered.
    pub stat_garbage_shattered: &'static str,
    /// Stat label: number of swaps performed.
    pub stat_swaps: &'static str,
    /// Stat label: blocks eliminated per minute.
    pub stat_blocks_per_min: &'static str,
    /// Stat label: garbage lines sent to opponent (VS).
    pub stat_garbage_sent: &'static str,
    /// Stat label: garbage lines received from opponent (VS).
    pub stat_garbage_received: &'static str,

    // Stat column headers — [egui] top of stats grid
    // Constraint: very short (5-10 chars).

    /// Stats column header: local player.
    pub stat_col_you: &'static str,
    /// Stats column header: opponent.
    pub stat_col_opponent: &'static str,
    /// Stats column header: personal record.
    pub stat_col_record: &'static str,

    // Post-game nav footer — [egui] buttons outside dialogs (Menu/Next), the same on
    // every post-game screen; icon + text labels.

    /// Post-game footer: "forfeit" button between rounds (bottom left). It leaves the
    /// current match, hence the confirmation.
    pub forfeit: &'static str,
    /// Forfeit confirmation modal: title/question.
    pub forfeit_confirm: &'static str,

    // Menu footer — [egui-small] bottom of main menu, info ticker

    /// Menu footer: call-to-action to star on GitHub.
    pub github_cta: &'static str,
    /// Ticker: "best solo score" label (followed by the value).
    pub ticker_best_solo: &'static str,
    /// Ticker: "best combo" label (followed by "x{n}").
    pub ticker_best_combo: &'static str,
    /// Ticker: "longest solo game" label (followed by m:ss).
    pub ticker_longest_solo: &'static str,
    /// Ticker: "best blocks/minute" label (followed by the value).
    pub ticker_blocks_per_min: &'static str,
    /// Ticker: "update" label (clickable item, followed by the version).
    /// Only read by the ticker under `update-check`. The field stays in every build
    /// so `Locale` is complete, hence the `allow` when the feature is off.
    #[cfg_attr(not(feature = "update-check"), allow(dead_code))]
    pub ticker_update: &'static str,
    /// Newer version available. `{}` = version number (menu ticker).
    #[cfg_attr(not(feature = "update-check"), allow(dead_code))]
    pub update_available_fmt: &'static str,
    /// Info page: "ignore this version" button (persistent).
    pub ignore_this_version: &'static str,
    /// Info page: "managed by your store" note (store builds).
    pub update_managed_by_store: &'static str,
    /// Info page: "managed by your distribution" note (distro packages).
    pub update_managed_by_distro: &'static str,
    /// Info page: the available-version line IS the button (opens the download page).
    pub update_view_download_fmt: &'static str,
    /// Info page: check requested by the player; the answer is always explicit.
    pub update_check_now: &'static str,
    /// Status line: check in progress.
    #[cfg_attr(not(feature = "update-check"), allow(dead_code))]
    pub update_status_checking: &'static str,
    /// Status line: check succeeded, nothing newer. `{}` = date.
    #[cfg_attr(not(feature = "update-check"), allow(dead_code))]
    pub update_status_up_to_date_fmt: &'static str,
    /// Status line: newer version available.
    #[cfg_attr(not(feature = "update-check"), allow(dead_code))]
    pub update_status_available_fmt: &'static str,
    /// Status line: newer version, but ignored. Never shown as "up to date".
    #[cfg_attr(not(feature = "update-check"), allow(dead_code))]
    pub update_status_ignored_fmt: &'static str,
    /// Status line: check failed, no earlier success.
    #[cfg_attr(not(feature = "update-check"), allow(dead_code))]
    pub update_status_failed: &'static str,
    /// Status line: check failed. `{}` = date of the last success.
    #[cfg_attr(not(feature = "update-check"), allow(dead_code))]
    pub update_status_failed_since_fmt: &'static str,
    /// Status line: no check made yet.
    #[cfg_attr(not(feature = "update-check"), allow(dead_code))]
    pub update_status_never: &'static str,
    /// Options modal: checkbox, shown only when an update check is possible.
    pub update_checks_option: &'static str,
    /// Info page: link to the repository (support, not a notice).
    pub contribute: &'static str,
    /// Network screen, after a version mismatch: opens the download page.
    pub view_download: &'static str,
    /// Updates page: "Installed version: {}".
    pub update_installed_fmt: &'static str,
    /// Updates page, for a binary without update checks (built from source).
    pub update_unavailable: &'static str,

    // Network ranking ("Network rank" page, deltas, ticker)

    /// VS Network submenu: entry and title of the "Network rank" page.
    pub menu_network_rank: &'static str,
    /// Ticker: "network rank" label (followed by "League · value").
    pub ticker_network_rank: &'static str,
    /// Network rank page: "Latest games" section.
    pub rank_last_parties: &'static str,
    /// Network rank page: empty state.
    pub rank_no_matches: &'static str,
    /// League names. Rookie means the rating is still calibrating.
    pub league_rookie: &'static str,
    pub league_copper: &'static str,
    pub league_bronze: &'static str,
    pub league_silver: &'static str,
    pub league_gold: &'static str,
    pub league_platinum: &'static str,
    pub league_diamond: &'static str,
    pub league_master: &'static str,
    pub league_grandmaster: &'static str,

    // Network rank page: live opponents.
    /// Quick reconnect action from the page.
    pub reconnect: &'static str,
    /// Prefix for the player's own rank in the section header (for example "Me: Gold · 1512").
    /// It CARRIES its own colon, so each language sets its own spacing.
    pub rank_me_prefix: &'static str,
    /// Menu entry and title of the "Opponents" page.
    pub menu_network_opponents: &'static str,

    // Reconnect screen. `{}` = peer name.
    /// Title: "Reconnecting to {}".
    pub reconnecting_to_fmt: &'static str,
    /// Instruction: what the other player must do to be reachable.
    pub reconnect_instructions: &'static str,
    /// Under the spinner: attempts continue while the screen is open.
    pub reconnect_trying: &'static str,
    /// Error: the invite target is our OWN listening address.
    pub own_address_error: &'static str,
    /// Opponents page: the peer is currently announced over mDNS.
    pub online_label: &'static str,
    /// Round number: title of the post-round panel and subtitle of the "Ready?" modal.
    pub round_fmt: &'static str,
    /// "X is ready", inside the "Ready?" modal. `{}` is a player name: in languages whose
    /// grammar depends on the name (particles, gender), keep the name outside the sentence.
    pub player_ready_fmt: &'static str,
    /// Opponent who has not answered the "Ready?" modal yet (same rule for `{}`).
    pub player_not_ready_fmt: &'static str,
    /// Relative date, less than an hour ago.
    pub ago_just_now: &'static str,
    /// Relative date in hours. Use an abbreviation that does not change with the number.
    pub ago_hours_fmt: &'static str,
    /// Relative date in days (same rule).
    pub ago_days_fmt: &'static str,
}

impl Locale {
    /// Round label, "Round N" (open-ended series, no total). Title of the post-round
    /// panel and subtitle of the "Ready?" modal.
    pub fn round_label(&self, number: i32) -> String {
        self.round_fmt.replace("{}", &number.to_string())
    }

    /// "X is ready": the only message of the consent protocol. It is shown INSIDE the
    /// "Ready?" modal, when the player answers, and never on the post-round summary.
    pub fn player_ready(&self, name: &str) -> String {
        self.player_ready_fmt.replace("{}", name)
    }

    /// Opponent status in the "Ready?" modal: has not answered yet.
    pub fn player_not_ready(&self, name: &str) -> String {
        self.player_not_ready_fmt.replace("{}", name)
    }

    /// Localized name of a league.
    pub fn league_name(&self, l: crackattack_rating::League) -> &'static str {
        use crackattack_rating::League as L;
        match l {
            L::Rookie => self.league_rookie,
            L::Copper => self.league_copper,
            L::Bronze => self.league_bronze,
            L::Silver => self.league_silver,
            L::Gold => self.league_gold,
            L::Platinum => self.league_platinum,
            L::Diamond => self.league_diamond,
            L::Master => self.league_master,
            L::Grandmaster => self.league_grandmaster,
        }
    }

    /// Compact relative date from a Unix timestamp in seconds: "just now", then hours,
    /// then days ("3h ago", "2d ago").
    pub fn ago(&self, ts: u64, now: u64) -> String {
        let dt = now.saturating_sub(ts);
        if dt < 3_600 {
            self.ago_just_now.to_string()
        } else if dt < 86_400 {
            self.ago_hours_fmt.replace("{}", &(dt / 3_600).to_string())
        } else {
            self.ago_days_fmt.replace("{}", &(dt / 86_400).to_string())
        }
    }
}

// English (reference language)

pub const EN: Locale = Locale {
    // Main menu
    menu_solo: "SOLO",
    menu_vs_computer: "VS COMPUTER",
    menu_vs_network: "VS NETWORK",
    menu_options: "OPTIONS",
    menu_info: "About",
    menu_rules: "How to play",
    menu_credits: "Credits",
    menu_updates: "Updates",
    menu_quit: "QUIT",
    menu_back: "BACK",
    menu_easy: "Easy",
    menu_medium: "Medium",
    menu_hard: "Hard",
    // Pause modal
    pause_title: "Paused",
    resume: "Resume",
    quit: "Quit",
    opponent_disconnected: "Opponent disconnected",
    ready_to_resume: "Ready to resume",
    ready_check: "Ready \u{2713}",
    waiting_opponent_resume: "Waiting for opponent...",
    opponent_ready: "Opponent ready",
    // Ready modal
    ready: "Ready",
    next: "Next",
    ready_new_round: "Ready for another round?",
    menu_label: "Menu",
    // Waiting banner
    waiting_for_opponent: "Waiting for opponent...",
    connection_lost_banner: "Connection lost...",
    // Network modal
    vs_network: "VS Network",
    cancel: "Cancel",
    listening_incoming: "Waiting for incoming connections...",
    network_auto: "Auto discovery",
    define_field: "Set",
    invalid_address: "Invalid address",
    manual_connection: "Manual connection",
    your_address: "Your address (share it):",
    opponent_address: "Opponent's address:",
    copy_address: "Copy my address",
    paste_address: "Paste opponent's address",
    invalid_port: "Invalid port",
    port_field: "Port",
    busy_suffix: "(busy)",
    back: "Back",
    players_on_lan: "Players on the local network",
    looking_for_players: "Looking for players...",
    scanning: "Scanning...",
    no_network_detected: "No local network detected",
    connect: "Connect",
    waiting_for_fmt: "Waiting for {}...",
    wants_to_play_fmt: "{} wants to play with you",
    accept: "Accept",
    decline: "Decline",
    connecting_to_fmt: "Connecting to {}...",
    in_game_suffix: "(in game)",
    player_tab: "General",
    sound_tab: "Sound",
    keyboard_tab: "Keyboard",
    gamepad_tab: "Gamepad",
    ctrl_movement: "Movement",
    ctrl_actions: "Actions",
    ctrl_directions: "Directions",
    act_move_up: "Move up",
    act_move_down: "Move down",
    act_move_left: "Move left",
    act_move_right: "Move right",
    act_swap: "Swap",
    act_raise: "Raise",
    act_pause: "Pause",
    dir_dpad: "D-Pad",
    dir_stick: "Stick",
    dir_both: "Both",
    key_space: "Space",
    key_enter: "Enter",
    key_esc: "Esc",
    key_backspace: "Backspace",
    osk_backspace: "Bksp",
    osk_clear: "Clear",
    default_player_name: "Player 1",
    language_auto: "Auto",
    error_peer_busy: "That player is busy",
    error_version_mismatch: "Your game version doesn't match the other player's. Update so you both have the same version.",
    error_handshake_fmt: "Could not connect to the other game: {}",
    error_connection_fmt: "Connection failed: {}",
    error_discovery_fmt: "Could not search for players: {}",
    error_port_fmt: "Cannot open network port: {}",
    player_name: "Player:",
    edit_name: "Edit",
    language_label: "Language:",
    fullscreen: "Fullscreen",
    sound_effects: "Sound effects",
    music: "Music",
    press_key_or_button: "Press a key or button...",
    cancel_capture_hint_fmt: "{} to cancel",
    reset_defaults: "Reset to defaults",
    // End screen
    high_scores: "High scores",
    no_scores_yet: "No scores yet.",
    game_stats: "Game stats",
    // Stat labels
    stat_score: "Score:",
    stat_duration: "Duration:",
    stat_blocks_eliminated: "Blocks cleared:",
    stat_chains: "Chains:",
    stat_best_chain: "Best chain:",
    stat_garbage_shattered: "Garbage shattered:",
    stat_swaps: "Swaps:",
    stat_blocks_per_min: "Blocks/min:",
    stat_garbage_sent: "Garbage sent:",
    stat_garbage_received: "Garbage received:",
    // Stat column headers
    stat_col_you: "You",
    stat_col_opponent: "Opponent",
    stat_col_record: "Record",
    // Footer and ticker
    forfeit: "Forfeit",
    forfeit_confirm: "Forfeit the match?",
    github_cta: "★ Star us on GitHub!",
    ticker_update: "Update",
    update_available_fmt: "v{} available!",
    ignore_this_version: "Ignore this version",
    update_managed_by_store: "Updates are handled by your app store.",
    update_managed_by_distro: "Updates are handled by your distribution.",
    update_view_download_fmt: "Version {} available — Download",
    update_check_now: "Check now",
    update_status_checking: "Checking...",
    update_status_up_to_date_fmt: "You have the latest version (checked {})",
    update_status_available_fmt: "Version {} available",
    update_status_ignored_fmt: "Version {} available (ignored)",
    update_status_failed: "Unable to check right now",
    update_status_failed_since_fmt: "Unable to check right now (last successful check {})",
    update_status_never: "Never checked",
    update_checks_option: "Check for updates",
    contribute: "Contribute",
    view_download: "Download page",
    update_installed_fmt: "Installed version: {}",
    update_unavailable: "This build does not check for updates.",
    ticker_best_solo: "Best solo",
    ticker_best_combo: "Best combo",
    ticker_longest_solo: "Longest solo",
    ticker_blocks_per_min: "Best blocks/min",
    menu_network_rank: "Network rank",
    ticker_network_rank: "Network rank",
    rank_last_parties: "Recent matches",
    rank_no_matches: "No ranked network matches yet.",
    league_rookie: "Rookie",
    league_copper: "Copper",
    league_bronze: "Bronze",
    league_silver: "Silver",
    league_gold: "Gold",
    league_platinum: "Platinum",
    league_diamond: "Diamond",
    league_master: "Master",
    league_grandmaster: "Grandmaster",
    reconnect: "Reconnect",
    rank_me_prefix: "You:",
    menu_network_opponents: "Recent opponents",
    reconnecting_to_fmt: "Reconnecting to {}",
    reconnect_instructions: "Ask {} to open the VS Network menu.",
    reconnect_trying: "Trying to connect... (retries until you leave this screen)",
    own_address_error: "This is your own address",
    online_label: "online",
    round_fmt: "Round {}",
    player_ready_fmt: "{} is ready",
    player_not_ready_fmt: "{} is not ready yet",
    ago_just_now: "just now",
    ago_hours_fmt: "{}h ago",
    ago_days_fmt: "{}d ago",
};

// French

pub const FR: Locale = Locale {
    // Main menu
    menu_solo: "SOLO",
    menu_vs_computer: "VS ORDINATEUR",
    menu_vs_network: "VS RÉSEAU",
    menu_options: "OPTIONS",
    menu_info: "À propos",
    menu_rules: "Règles",
    menu_credits: "Crédits",
    menu_updates: "Mises à jour",
    menu_quit: "QUITTER",
    menu_back: "RETOUR",
    menu_easy: "Facile",
    menu_medium: "Moyen",
    menu_hard: "Difficile",
    // Pause
    pause_title: "Pause",
    resume: "Reprendre",
    quit: "Quitter",
    opponent_disconnected: "Adversaire déconnecté",
    ready_to_resume: "Prêt à reprendre",
    ready_check: "Prêt \u{2713}",
    waiting_opponent_resume: "En attente de l'adversaire...",
    opponent_ready: "Adversaire prêt",
    // Ready
    ready: "Prêt",
    next: "Suivant",
    ready_new_round: "Prêt pour une nouvelle manche ?",
    menu_label: "Menu",
    // Waiting banner
    waiting_for_opponent: "En attente de l'adversaire...",
    connection_lost_banner: "Connexion perdue...",
    // Network modal
    vs_network: "VS Réseau",
    cancel: "Annuler",
    listening_incoming: "En attente de connexions...",
    network_auto: "Recherche auto",
    define_field: "Définir",
    invalid_address: "Adresse invalide",
    manual_connection: "Connexion manuelle",
    your_address: "Votre adresse (à partager) :",
    opponent_address: "Adresse de l'adversaire :",
    copy_address: "Copier mon adresse",
    paste_address: "Coller l'adresse de l'adversaire",
    invalid_port: "Port invalide",
    port_field: "Port",
    busy_suffix: "(occupé)",
    back: "Retour",
    players_on_lan: "Joueurs sur le réseau local",
    looking_for_players: "Recherche de joueurs...",
    scanning: "Recherche...",
    no_network_detected: "Aucun réseau local détecté",
    connect: "Se connecter",
    waiting_for_fmt: "En attente de {}...",
    wants_to_play_fmt: "{} veut jouer avec vous",
    accept: "Accepter",
    decline: "Refuser",
    connecting_to_fmt: "Connexion à {}...",
    in_game_suffix: "(en jeu)",
    player_tab: "Général",
    sound_tab: "Son",
    keyboard_tab: "Clavier",
    gamepad_tab: "Manette",
    ctrl_movement: "Déplacement",
    ctrl_actions: "Actions",
    ctrl_directions: "Directions",
    act_move_up: "Haut",
    act_move_down: "Bas",
    act_move_left: "Gauche",
    act_move_right: "Droite",
    act_swap: "Échanger",
    act_raise: "Monter",
    act_pause: "Pause",
    dir_dpad: "Croix",
    dir_stick: "Stick",
    dir_both: "Les deux",
    key_space: "Espace",
    key_enter: "Entrée",
    key_esc: "Échap",
    key_backspace: "Retour arrière",
    osk_backspace: "Effac.",
    osk_clear: "Vider",
    default_player_name: "Joueur 1",
    language_auto: "Auto",
    error_peer_busy: "Le joueur est occupé",
    error_version_mismatch: "Votre jeu et celui de l'autre joueur ne sont pas compatibles (versions différentes). Mettez à jour pour jouer ensemble.",
    error_handshake_fmt: "Impossible de joindre l'autre jeu : {}",
    error_connection_fmt: "Échec de la connexion : {}",
    error_discovery_fmt: "Impossible de rechercher des joueurs : {}",
    error_port_fmt: "Impossible d'ouvrir le port réseau : {}",
    player_name: "Joueur :",
    edit_name: "Modifier",
    language_label: "Langue :",
    fullscreen: "Plein écran",
    sound_effects: "Effets sonores",
    music: "Musique",
    press_key_or_button: "Appuyez sur une touche...",
    cancel_capture_hint_fmt: "{} pour annuler",
    reset_defaults: "Valeurs par défaut",
    // End screen
    high_scores: "Meilleurs scores",
    no_scores_yet: "Aucun score pour l'instant.",
    game_stats: "Statistiques",
    // Stats
    stat_score: "Score :",
    stat_duration: "Durée :",
    stat_blocks_eliminated: "Blocs éliminés :",
    stat_chains: "Chaînes :",
    stat_best_chain: "Meilleure chaîne :",
    stat_garbage_shattered: "Déchets brisés :",
    stat_swaps: "Échanges :",
    stat_blocks_per_min: "Blocs/min :",
    stat_garbage_sent: "Déchets envoyés :",
    stat_garbage_received: "Déchets reçus :",
    // Stat column headers
    stat_col_you: "Vous",
    stat_col_opponent: "Adversaire",
    stat_col_record: "Record",
    // Footer and ticker
    forfeit: "Abandonner",
    forfeit_confirm: "Abandonner la partie ?",
    github_cta: "\u{2605} Donnez-nous une étoile sur GitHub !",
    ticker_update: "Mise à jour",
    update_available_fmt: "v{} disponible !",
    ignore_this_version: "Ignorer cette version",
    update_managed_by_store: "Mises à jour gérées par votre magasin d'applications.",
    update_managed_by_distro: "Mises à jour gérées par votre distribution.",
    update_view_download_fmt: "Version {} disponible — Télécharger",
    update_check_now: "Vérifier maintenant",
    update_status_checking: "Vérification...",
    update_status_up_to_date_fmt: "Vous avez la dernière version (vérifié le {})",
    update_status_available_fmt: "Version {} disponible",
    update_status_ignored_fmt: "Version {} disponible (ignorée)",
    update_status_failed: "Impossible de vérifier pour le moment",
    update_status_failed_since_fmt: "Impossible de vérifier pour le moment (dernière vérification réussie le {})",
    update_status_never: "Jamais vérifié",
    update_checks_option: "Vérifier les mises à jour",
    contribute: "Contribuer",
    view_download: "Page de téléchargement",
    update_installed_fmt: "Version installée : {}",
    update_unavailable: "Cette version ne vérifie pas les mises à jour.",
    ticker_best_solo: "Meilleur solo",
    ticker_best_combo: "Meilleur combo",
    ticker_longest_solo: "Solo le plus long",
    ticker_blocks_per_min: "Record blocs/min",
    menu_network_rank: "Rang réseau",
    ticker_network_rank: "Rang réseau",
    rank_last_parties: "Dernières parties",
    rank_no_matches: "Aucune partie réseau classée pour l'instant.",
    league_rookie: "Recrue",
    league_copper: "Cuivre",
    league_bronze: "Bronze",
    league_silver: "Argent",
    league_gold: "Or",
    league_platinum: "Platine",
    league_diamond: "Diamant",
    league_master: "Maître",
    league_grandmaster: "Grand maître",
    reconnect: "Reconnecter",
    rank_me_prefix: "Vous :",
    menu_network_opponents: "Adversaires récents",
    reconnecting_to_fmt: "Reconnexion à {}",
    reconnect_instructions: "Demandez à {} d'ouvrir le menu VS Réseau.",
    reconnect_trying: "Connexion en cours... (nouvel essai tant que cet écran est ouvert)",
    own_address_error: "Cette adresse est la vôtre",
    online_label: "en ligne",
    round_fmt: "Manche {}",
    player_ready_fmt: "{} est prêt",
    player_not_ready_fmt: "{} n'est pas encore prêt",
    ago_just_now: "à l'instant",
    ago_hours_fmt: "il y a {} h",
    ago_days_fmt: "il y a {} j",
};

// German

pub const DE: Locale = Locale {
    // Main menu
    menu_solo: "SOLO",
    menu_vs_computer: "VS COMPUTER",
    menu_vs_network: "VS NETZWERK",
    menu_options: "OPTIONEN",
    menu_info: "Über",
    menu_rules: "Spielanleitung",
    menu_credits: "Mitwirkende",
    menu_updates: "Updates",
    menu_quit: "BEENDEN",
    menu_back: "ZURÜCK",
    menu_easy: "Leicht",
    menu_medium: "Mittel",
    menu_hard: "Schwer",
    // Pause modal
    pause_title: "Pausiert",
    resume: "Fortsetzen",
    quit: "Beenden",
    opponent_disconnected: "Gegner getrennt",
    ready_to_resume: "Bereit zum Fortsetzen",
    ready_check: "Bereit \u{2713}",
    waiting_opponent_resume: "Warte auf den Gegner...",
    opponent_ready: "Gegner bereit",
    // Ready modal
    ready: "Bereit",
    next: "Weiter",
    ready_new_round: "Bereit für eine weitere Runde?",
    menu_label: "Menü",
    // Waiting banner
    waiting_for_opponent: "Warte auf den Gegner...",
    connection_lost_banner: "Verbindung verloren...",
    // Network modal
    vs_network: "VS Netzwerk",
    cancel: "Abbrechen",
    listening_incoming: "Warte auf eingehende Verbindungen...",
    network_auto: "Automatische Suche",
    define_field: "Festlegen",
    invalid_address: "Ungültige Adresse",
    manual_connection: "Manuelle Verbindung",
    your_address: "Deine Adresse (zum Teilen):",
    opponent_address: "Adresse des Gegners:",
    copy_address: "Meine Adresse kopieren",
    paste_address: "Adresse des Gegners einfügen",
    invalid_port: "Ungültiger Port",
    port_field: "Port",
    busy_suffix: "(beschäftigt)",
    back: "Zurück",
    players_on_lan: "Spieler im lokalen Netzwerk",
    looking_for_players: "Suche nach Spielern...",
    scanning: "Suche läuft...",
    no_network_detected: "Kein lokales Netzwerk gefunden",
    connect: "Verbinden",
    waiting_for_fmt: "Warte auf {}...",
    wants_to_play_fmt: "{} möchte mit dir spielen",
    accept: "Annehmen",
    decline: "Ablehnen",
    connecting_to_fmt: "Verbinde mit {}...",
    in_game_suffix: "(im Spiel)",
    player_tab: "Allgemein",
    sound_tab: "Ton",
    keyboard_tab: "Tastatur",
    gamepad_tab: "Gamepad",
    ctrl_movement: "Bewegung",
    ctrl_actions: "Aktionen",
    ctrl_directions: "Richtungen",
    act_move_up: "Nach oben",
    act_move_down: "Nach unten",
    act_move_left: "Nach links",
    act_move_right: "Nach rechts",
    act_swap: "Tauschen",
    act_raise: "Anheben",
    act_pause: "Pause",
    dir_dpad: "Steuerkreuz",
    dir_stick: "Stick",
    dir_both: "Beides",
    key_space: "Leertaste",
    key_enter: "Eingabe",
    key_esc: "Esc",
    key_backspace: "Rücktaste",
    osk_backspace: "Rück",
    osk_clear: "Leeren",
    default_player_name: "Spieler 1",
    language_auto: "Auto",
    error_peer_busy: "Dieser Spieler ist beschäftigt",
    error_version_mismatch: "Deine Spielversion passt nicht zu der des anderen Spielers. Aktualisiert, damit ihr beide dieselbe Version habt.",
    error_handshake_fmt: "Verbindung zum anderen Spiel fehlgeschlagen: {}",
    error_connection_fmt: "Verbindung fehlgeschlagen: {}",
    error_discovery_fmt: "Spielersuche nicht möglich: {}",
    error_port_fmt: "Netzwerk-Port kann nicht geöffnet werden: {}",
    player_name: "Spieler:",
    edit_name: "Ändern",
    language_label: "Sprache:",
    fullscreen: "Vollbild",
    sound_effects: "Soundeffekte",
    music: "Musik",
    press_key_or_button: "Taste oder Knopf drücken...",
    cancel_capture_hint_fmt: "{} zum Abbrechen",
    reset_defaults: "Standardwerte",
    // End screen
    high_scores: "Bestenliste",
    no_scores_yet: "Noch keine Punkte.",
    game_stats: "Spielstatistik",
    // Stat labels
    stat_score: "Punkte:",
    stat_duration: "Dauer:",
    stat_blocks_eliminated: "Aufgelöste Blöcke:",
    stat_chains: "Ketten:",
    stat_best_chain: "Beste Kette:",
    stat_garbage_shattered: "Zerstörter Schrott:",
    stat_swaps: "Tauschzüge:",
    stat_blocks_per_min: "Blöcke/Min.:",
    stat_garbage_sent: "Gesendeter Schrott:",
    stat_garbage_received: "Erhaltener Schrott:",
    // Stat column headers
    stat_col_you: "Du",
    stat_col_opponent: "Gegner",
    stat_col_record: "Rekord",
    // Footer and ticker
    forfeit: "Aufgeben",
    forfeit_confirm: "Das Match aufgeben?",
    github_cta: "\u{2605} Gib uns einen Stern auf GitHub!",
    ticker_update: "Update",
    update_available_fmt: "v{} verfügbar!",
    ignore_this_version: "Diese Version ignorieren",
    update_managed_by_store: "Updates werden von deinem App-Store verwaltet.",
    update_managed_by_distro: "Updates werden von deiner Distribution verwaltet.",
    update_view_download_fmt: "Version {} verfügbar — Herunterladen",
    update_check_now: "Jetzt prüfen",
    update_status_checking: "Wird geprüft...",
    update_status_up_to_date_fmt: "Du hast die neueste Version (geprüft {})",
    update_status_available_fmt: "Version {} verfügbar",
    update_status_ignored_fmt: "Version {} verfügbar (ignoriert)",
    update_status_failed: "Prüfung gerade nicht möglich",
    update_status_failed_since_fmt: "Prüfung gerade nicht möglich (letzte erfolgreiche Prüfung {})",
    update_status_never: "Noch nie geprüft",
    update_checks_option: "Nach Updates suchen",
    contribute: "Mitmachen",
    view_download: "Download-Seite",
    update_installed_fmt: "Installierte Version: {}",
    update_unavailable: "Diese Version sucht nicht nach Updates.",
    ticker_best_solo: "Bestes Solo",
    ticker_best_combo: "Beste Kombo",
    ticker_longest_solo: "Längstes Solo",
    ticker_blocks_per_min: "Rekord Blöcke/Min.",
    menu_network_rank: "Netzwerk-Rang",
    ticker_network_rank: "Netzwerk-Rang",
    rank_last_parties: "Letzte Matches",
    rank_no_matches: "Noch keine gewerteten Netzwerk-Matches.",
    league_rookie: "Neuling",
    league_copper: "Kupfer",
    league_bronze: "Bronze",
    league_silver: "Silber",
    league_gold: "Gold",
    league_platinum: "Platin",
    league_diamond: "Diamant",
    league_master: "Meister",
    league_grandmaster: "Großmeister",
    reconnect: "Neu verbinden",
    rank_me_prefix: "Du:",
    menu_network_opponents: "Letzte Gegner",
    reconnecting_to_fmt: "Verbinde erneut mit {}",
    reconnect_instructions: "Bitte {}, das Menü VS Netzwerk zu öffnen.",
    reconnect_trying: "Verbindungsversuch... (wird wiederholt, bis du diesen Bildschirm verlässt)",
    own_address_error: "Das ist deine eigene Adresse",
    online_label: "online",
    round_fmt: "Runde {}",
    player_ready_fmt: "{} ist bereit",
    player_not_ready_fmt: "{} ist noch nicht bereit",
    ago_just_now: "gerade eben",
    ago_hours_fmt: "vor {} Std.",
    ago_days_fmt: "vor {} T.",
};

// Spanish

pub const ES: Locale = Locale {
    // Main menu
    menu_solo: "SOLO",
    menu_vs_computer: "VS ORDENADOR",
    menu_vs_network: "VS RED",
    menu_options: "OPCIONES",
    menu_info: "Acerca de",
    menu_rules: "Cómo jugar",
    menu_credits: "Créditos",
    menu_updates: "Actualizaciones",
    menu_quit: "SALIR",
    menu_back: "VOLVER",
    menu_easy: "Fácil",
    menu_medium: "Media",
    menu_hard: "Difícil",
    // Pause modal
    pause_title: "En pausa",
    resume: "Continuar",
    quit: "Salir",
    opponent_disconnected: "Rival desconectado",
    ready_to_resume: "Listo para continuar",
    ready_check: "Listo \u{2713}",
    waiting_opponent_resume: "Esperando al rival...",
    opponent_ready: "Rival listo",
    // Ready modal
    ready: "Listo",
    next: "Siguiente",
    ready_new_round: "¿Listo para otra ronda?",
    menu_label: "Menú",
    // Waiting banner
    waiting_for_opponent: "Esperando al rival...",
    connection_lost_banner: "Conexión perdida...",
    // Network modal
    vs_network: "VS Red",
    cancel: "Cancelar",
    listening_incoming: "Esperando conexiones...",
    network_auto: "Búsqueda automática",
    define_field: "Definir",
    invalid_address: "Dirección no válida",
    manual_connection: "Conexión manual",
    your_address: "Tu dirección (compártela):",
    opponent_address: "Dirección del rival:",
    copy_address: "Copiar mi dirección",
    paste_address: "Pegar la dirección del rival",
    invalid_port: "Puerto no válido",
    port_field: "Puerto",
    busy_suffix: "(ocupado)",
    back: "Volver",
    players_on_lan: "Jugadores en la red local",
    looking_for_players: "Buscando jugadores...",
    scanning: "Buscando...",
    no_network_detected: "No se detecta ninguna red local",
    connect: "Conectar",
    waiting_for_fmt: "Esperando a {}...",
    wants_to_play_fmt: "{} quiere jugar contigo",
    accept: "Aceptar",
    decline: "Rechazar",
    connecting_to_fmt: "Conectando con {}...",
    in_game_suffix: "(en partida)",
    player_tab: "General",
    sound_tab: "Sonido",
    keyboard_tab: "Teclado",
    gamepad_tab: "Mando",
    ctrl_movement: "Movimiento",
    ctrl_actions: "Acciones",
    ctrl_directions: "Direcciones",
    act_move_up: "Arriba",
    act_move_down: "Abajo",
    act_move_left: "Izquierda",
    act_move_right: "Derecha",
    act_swap: "Intercambiar",
    act_raise: "Subir",
    act_pause: "Pausa",
    dir_dpad: "Cruceta",
    dir_stick: "Stick",
    dir_both: "Ambos",
    key_space: "Espacio",
    key_enter: "Intro",
    key_esc: "Esc",
    key_backspace: "Retroceso",
    osk_backspace: "Borrar",
    osk_clear: "Vaciar",
    default_player_name: "Jugador 1",
    language_auto: "Auto",
    error_peer_busy: "Ese jugador está ocupado",
    error_version_mismatch: "Tu versión del juego no coincide con la del otro jugador. Actualizad para tener los dos la misma versión.",
    error_handshake_fmt: "No se pudo conectar con el otro juego: {}",
    error_connection_fmt: "Error de conexión: {}",
    error_discovery_fmt: "No se pudo buscar jugadores: {}",
    error_port_fmt: "No se puede abrir el puerto de red: {}",
    player_name: "Jugador:",
    edit_name: "Editar",
    language_label: "Idioma:",
    fullscreen: "Pantalla completa",
    sound_effects: "Efectos de sonido",
    music: "Música",
    press_key_or_button: "Pulsa una tecla o botón...",
    cancel_capture_hint_fmt: "{} para cancelar",
    reset_defaults: "Restablecer",
    // End screen
    high_scores: "Mejores puntuaciones",
    no_scores_yet: "Aún no hay puntuaciones.",
    game_stats: "Estadísticas",
    // Stat labels
    stat_score: "Puntos:",
    stat_duration: "Duración:",
    stat_blocks_eliminated: "Bloques eliminados:",
    stat_chains: "Cadenas:",
    stat_best_chain: "Mejor cadena:",
    stat_garbage_shattered: "Basura rota:",
    stat_swaps: "Intercambios:",
    stat_blocks_per_min: "Bloques/min:",
    stat_garbage_sent: "Basura enviada:",
    stat_garbage_received: "Basura recibida:",
    // Stat column headers
    stat_col_you: "Tú",
    stat_col_opponent: "Rival",
    stat_col_record: "Récord",
    // Footer and ticker
    forfeit: "Rendirse",
    forfeit_confirm: "¿Rendirse en la partida?",
    github_cta: "\u{2605} ¡Danos una estrella en GitHub!",
    ticker_update: "Actualización",
    update_available_fmt: "¡v{} disponible!",
    ignore_this_version: "Ignorar esta versión",
    update_managed_by_store: "Las actualizaciones las gestiona tu tienda de aplicaciones.",
    update_managed_by_distro: "Las actualizaciones las gestiona tu distribución.",
    update_view_download_fmt: "Versión {} disponible — Descargar",
    update_check_now: "Comprobar ahora",
    update_status_checking: "Comprobando...",
    update_status_up_to_date_fmt: "Tienes la última versión (comprobado {})",
    update_status_available_fmt: "Versión {} disponible",
    update_status_ignored_fmt: "Versión {} disponible (ignorada)",
    update_status_failed: "No se puede comprobar ahora",
    update_status_failed_since_fmt: "No se puede comprobar ahora (última comprobación correcta {})",
    update_status_never: "Nunca comprobado",
    update_checks_option: "Buscar actualizaciones",
    contribute: "Colaborar",
    view_download: "Página de descarga",
    update_installed_fmt: "Versión instalada: {}",
    update_unavailable: "Esta versión no busca actualizaciones.",
    ticker_best_solo: "Mejor solo",
    ticker_best_combo: "Mejor combo",
    ticker_longest_solo: "Solo más largo",
    ticker_blocks_per_min: "Récord bloques/min",
    menu_network_rank: "Rango en red",
    ticker_network_rank: "Rango en red",
    rank_last_parties: "Partidas recientes",
    rank_no_matches: "Aún no hay partidas en red clasificadas.",
    league_rookie: "Novato",
    league_copper: "Cobre",
    league_bronze: "Bronce",
    league_silver: "Plata",
    league_gold: "Oro",
    league_platinum: "Platino",
    league_diamond: "Diamante",
    league_master: "Maestro",
    league_grandmaster: "Gran maestro",
    reconnect: "Reconectar",
    rank_me_prefix: "Tú:",
    menu_network_opponents: "Rivales recientes",
    reconnecting_to_fmt: "Reconectando con {}",
    reconnect_instructions: "Pide a {} que abra el menú VS Red.",
    reconnect_trying: "Intentando conectar... (se reintenta hasta que salgas de esta pantalla)",
    own_address_error: "Esta es tu propia dirección",
    online_label: "en línea",
    round_fmt: "Ronda {}",
    player_ready_fmt: "{} está listo",
    player_not_ready_fmt: "{} aún no está listo",
    ago_just_now: "ahora mismo",
    ago_hours_fmt: "hace {} h",
    ago_days_fmt: "hace {} d",
};

// Russian. The default player name stays in Latin letters: the bitmap nameplate font has no
// Cyrillic glyphs (see `CharSet::PlayerName`).

pub const RU: Locale = Locale {
    // Main menu
    menu_solo: "СОЛО",
    menu_vs_computer: "ПРОТИВ КОМПЬЮТЕРА",
    menu_vs_network: "ПО СЕТИ",
    menu_options: "НАСТРОЙКИ",
    menu_info: "О программе",
    menu_rules: "Как играть",
    menu_credits: "Авторы",
    menu_updates: "Обновления",
    menu_quit: "ВЫХОД",
    menu_back: "НАЗАД",
    menu_easy: "Легко",
    menu_medium: "Средне",
    menu_hard: "Сложно",
    // Pause modal
    pause_title: "Пауза",
    resume: "Продолжить",
    quit: "Выйти",
    opponent_disconnected: "Соперник отключился",
    ready_to_resume: "Готов продолжить",
    ready_check: "Готов \u{2713}",
    waiting_opponent_resume: "Ожидание соперника...",
    opponent_ready: "Соперник готов",
    // Ready modal
    ready: "Готов",
    next: "Далее",
    ready_new_round: "Готовы к следующему раунду?",
    menu_label: "Меню",
    // Waiting banner
    waiting_for_opponent: "Ожидание соперника...",
    connection_lost_banner: "Соединение потеряно...",
    // Network modal
    vs_network: "Игра по сети",
    cancel: "Отмена",
    listening_incoming: "Ожидание входящих подключений...",
    network_auto: "Автопоиск",
    define_field: "Задать",
    invalid_address: "Неверный адрес",
    manual_connection: "Подключение вручную",
    your_address: "Ваш адрес (поделитесь им):",
    opponent_address: "Адрес соперника:",
    copy_address: "Скопировать мой адрес",
    paste_address: "Вставить адрес соперника",
    invalid_port: "Неверный порт",
    port_field: "Порт",
    busy_suffix: "(занят)",
    back: "Назад",
    players_on_lan: "Игроки в локальной сети",
    looking_for_players: "Поиск игроков...",
    scanning: "Поиск...",
    no_network_detected: "Локальная сеть не найдена",
    connect: "Подключиться",
    waiting_for_fmt: "Ожидание {}...",
    wants_to_play_fmt: "{} хочет сыграть с вами",
    accept: "Принять",
    decline: "Отклонить",
    connecting_to_fmt: "Подключение к {}...",
    in_game_suffix: "(в игре)",
    player_tab: "Общие",
    sound_tab: "Звук",
    keyboard_tab: "Клавиатура",
    gamepad_tab: "Геймпад",
    ctrl_movement: "Движение",
    ctrl_actions: "Действия",
    ctrl_directions: "Направления",
    act_move_up: "Вверх",
    act_move_down: "Вниз",
    act_move_left: "Влево",
    act_move_right: "Вправо",
    act_swap: "Поменять",
    act_raise: "Поднять",
    act_pause: "Пауза",
    dir_dpad: "Крестовина",
    dir_stick: "Стик",
    dir_both: "Оба",
    key_space: "Пробел",
    key_enter: "Enter",
    key_esc: "Esc",
    key_backspace: "Backspace",
    osk_backspace: "Стер.",
    osk_clear: "Очист.",
    default_player_name: "Igrok 1",
    language_auto: "Авто",
    error_peer_busy: "Этот игрок занят",
    error_version_mismatch: "Ваша версия игры не совпадает с версией другого игрока. Обновитесь, чтобы у вас была одна версия.",
    error_handshake_fmt: "Не удалось подключиться к другой игре: {}",
    error_connection_fmt: "Ошибка подключения: {}",
    error_discovery_fmt: "Не удалось найти игроков: {}",
    error_port_fmt: "Не удалось открыть сетевой порт: {}",
    player_name: "Игрок:",
    edit_name: "Изменить",
    language_label: "Язык:",
    fullscreen: "Полный экран",
    sound_effects: "Звуковые эффекты",
    music: "Музыка",
    press_key_or_button: "Нажмите клавишу или кнопку...",
    cancel_capture_hint_fmt: "{} — отмена",
    reset_defaults: "По умолчанию",
    // End screen
    high_scores: "Лучшие результаты",
    no_scores_yet: "Пока нет результатов.",
    game_stats: "Статистика",
    // Stat labels
    stat_score: "Очки:",
    stat_duration: "Время:",
    stat_blocks_eliminated: "Убрано блоков:",
    stat_chains: "Цепочки:",
    stat_best_chain: "Лучшая цепочка:",
    stat_garbage_shattered: "Разбито мусора:",
    stat_swaps: "Перестановки:",
    stat_blocks_per_min: "Блоков/мин:",
    stat_garbage_sent: "Отправлено мусора:",
    stat_garbage_received: "Получено мусора:",
    // Stat column headers
    stat_col_you: "Вы",
    stat_col_opponent: "Соперник",
    stat_col_record: "Рекорд",
    // Footer and ticker
    forfeit: "Сдаться",
    forfeit_confirm: "Сдаться в этом матче?",
    github_cta: "\u{2605} Поставьте нам звезду на GitHub!",
    ticker_update: "Обновление",
    update_available_fmt: "Доступна v{}!",
    ignore_this_version: "Пропустить эту версию",
    update_managed_by_store: "Обновлениями управляет ваш магазин приложений.",
    update_managed_by_distro: "Обновлениями управляет ваш дистрибутив.",
    update_view_download_fmt: "Доступна версия {} — Скачать",
    update_check_now: "Проверить сейчас",
    update_status_checking: "Проверка...",
    update_status_up_to_date_fmt: "У вас последняя версия (проверено {})",
    update_status_available_fmt: "Доступна версия {}",
    update_status_ignored_fmt: "Доступна версия {} (пропущена)",
    update_status_failed: "Сейчас проверить не удаётся",
    update_status_failed_since_fmt: "Сейчас проверить не удаётся (последняя успешная проверка {})",
    update_status_never: "Ещё не проверялось",
    update_checks_option: "Проверять обновления",
    contribute: "Участвовать",
    view_download: "Страница загрузки",
    update_installed_fmt: "Установленная версия: {}",
    update_unavailable: "Эта сборка не проверяет обновления.",
    ticker_best_solo: "Лучшее соло",
    ticker_best_combo: "Лучшее комбо",
    ticker_longest_solo: "Самое долгое соло",
    ticker_blocks_per_min: "Рекорд блоков/мин",
    menu_network_rank: "Сетевой рейтинг",
    ticker_network_rank: "Сетевой рейтинг",
    rank_last_parties: "Последние матчи",
    rank_no_matches: "Пока нет рейтинговых сетевых матчей.",
    league_rookie: "Новичок",
    league_copper: "Медь",
    league_bronze: "Бронза",
    league_silver: "Серебро",
    league_gold: "Золото",
    league_platinum: "Платина",
    league_diamond: "Алмаз",
    league_master: "Мастер",
    league_grandmaster: "Гроссмейстер",
    reconnect: "Переподключиться",
    rank_me_prefix: "Вы:",
    menu_network_opponents: "Недавние соперники",
    reconnecting_to_fmt: "Переподключение к {}",
    reconnect_instructions: "Попросите {} открыть меню «Игра по сети».",
    reconnect_trying: "Попытка подключения... (повторяется, пока открыт этот экран)",
    own_address_error: "Это ваш собственный адрес",
    online_label: "в сети",
    round_fmt: "Раунд {}",
    player_ready_fmt: "{}: готов",
    player_not_ready_fmt: "{}: ещё не готов",
    ago_just_now: "только что",
    ago_hours_fmt: "{} ч назад",
    ago_days_fmt: "{} дн. назад",
};

// Simplified Chinese. Its glyphs come from the reduced Noto Sans SC font (see
// `renderer::ui::icons` and `tools/fonts/subset_cjk.py`); the default player name stays in
// Latin letters for the bitmap nameplate.

pub const ZH: Locale = Locale {
    // Main menu
    menu_solo: "单人",
    menu_vs_computer: "对战电脑",
    menu_vs_network: "网络对战",
    menu_options: "选项",
    menu_info: "关于",
    menu_rules: "玩法",
    menu_credits: "制作人员",
    menu_updates: "更新",
    menu_quit: "退出",
    menu_back: "返回",
    menu_easy: "简单",
    menu_medium: "中等",
    menu_hard: "困难",
    // Pause modal
    pause_title: "已暂停",
    resume: "继续",
    quit: "退出",
    opponent_disconnected: "对手已断开连接",
    ready_to_resume: "准备继续",
    ready_check: "准备 \u{2713}",
    waiting_opponent_resume: "正在等待对手...",
    opponent_ready: "对手已准备",
    // Ready modal
    ready: "准备",
    next: "下一步",
    ready_new_round: "准备好下一局了吗？",
    menu_label: "菜单",
    // Waiting banner
    waiting_for_opponent: "正在等待对手...",
    connection_lost_banner: "连接已断开...",
    // Network modal
    vs_network: "网络对战",
    cancel: "取消",
    listening_incoming: "正在等待连接...",
    network_auto: "自动搜索",
    define_field: "设置",
    invalid_address: "地址无效",
    manual_connection: "手动连接",
    your_address: "你的地址（分享给对手）：",
    opponent_address: "对手的地址：",
    copy_address: "复制我的地址",
    paste_address: "粘贴对手的地址",
    invalid_port: "端口无效",
    port_field: "端口",
    busy_suffix: "（忙碌）",
    back: "返回",
    players_on_lan: "局域网中的玩家",
    looking_for_players: "正在寻找玩家...",
    scanning: "正在搜索...",
    no_network_detected: "未检测到局域网",
    connect: "连接",
    waiting_for_fmt: "正在等待 {}...",
    wants_to_play_fmt: "{} 想和你对战",
    accept: "接受",
    decline: "拒绝",
    connecting_to_fmt: "正在连接 {}...",
    in_game_suffix: "（游戏中）",
    player_tab: "常规",
    sound_tab: "声音",
    keyboard_tab: "键盘",
    gamepad_tab: "手柄",
    ctrl_movement: "移动",
    ctrl_actions: "操作",
    ctrl_directions: "方向",
    act_move_up: "上移",
    act_move_down: "下移",
    act_move_left: "左移",
    act_move_right: "右移",
    act_swap: "交换",
    act_raise: "上升",
    act_pause: "暂停",
    dir_dpad: "十字键",
    dir_stick: "摇杆",
    dir_both: "两者",
    key_space: "空格",
    key_enter: "回车",
    key_esc: "Esc",
    key_backspace: "退格",
    osk_backspace: "退格",
    osk_clear: "清空",
    default_player_name: "Player 1",
    language_auto: "自动",
    error_peer_busy: "该玩家正忙",
    error_version_mismatch: "你的游戏版本与对方不一致。请更新，使双方版本相同。",
    error_handshake_fmt: "无法连接到对方游戏：{}",
    error_connection_fmt: "连接失败：{}",
    error_discovery_fmt: "无法搜索玩家：{}",
    error_port_fmt: "无法打开网络端口：{}",
    player_name: "玩家：",
    edit_name: "修改",
    language_label: "语言：",
    fullscreen: "全屏",
    sound_effects: "音效",
    music: "音乐",
    press_key_or_button: "请按下按键...",
    cancel_capture_hint_fmt: "按 {} 取消",
    reset_defaults: "恢复默认",
    // End screen
    high_scores: "最高分",
    no_scores_yet: "暂无分数。",
    game_stats: "游戏统计",
    // Stat labels
    stat_score: "分数：",
    stat_duration: "时长：",
    stat_blocks_eliminated: "消除方块：",
    stat_chains: "连锁：",
    stat_best_chain: "最长连锁：",
    stat_garbage_shattered: "击碎垃圾：",
    stat_swaps: "交换次数：",
    stat_blocks_per_min: "方块/分钟：",
    stat_garbage_sent: "发送垃圾：",
    stat_garbage_received: "收到垃圾：",
    // Stat column headers
    stat_col_you: "你",
    stat_col_opponent: "对手",
    stat_col_record: "纪录",
    // Footer and ticker
    forfeit: "认输",
    forfeit_confirm: "确定认输吗？",
    github_cta: "\u{2605} 在 GitHub 上给我们点星！",
    ticker_update: "更新",
    update_available_fmt: "v{} 已发布！",
    ignore_this_version: "忽略此版本",
    update_managed_by_store: "更新由你的应用商店管理。",
    update_managed_by_distro: "更新由你的发行版管理。",
    update_view_download_fmt: "版本 {} 已发布 — 下载",
    update_check_now: "立即检查",
    update_status_checking: "正在检查...",
    update_status_up_to_date_fmt: "已是最新版本（检查于 {}）",
    update_status_available_fmt: "版本 {} 已发布",
    update_status_ignored_fmt: "版本 {} 已发布（已忽略）",
    update_status_failed: "暂时无法检查",
    update_status_failed_since_fmt: "暂时无法检查（上次成功检查于 {}）",
    update_status_never: "从未检查",
    update_checks_option: "检查更新",
    contribute: "参与贡献",
    view_download: "下载页面",
    update_installed_fmt: "已安装版本：{}",
    update_unavailable: "此版本不检查更新。",
    ticker_best_solo: "单人最佳",
    ticker_best_combo: "最佳组合",
    ticker_longest_solo: "单人最长",
    ticker_blocks_per_min: "方块/分钟纪录",
    menu_network_rank: "网络排名",
    ticker_network_rank: "网络排名",
    rank_last_parties: "最近比赛",
    rank_no_matches: "暂无网络排名赛。",
    league_rookie: "新手",
    league_copper: "铜",
    league_bronze: "青铜",
    league_silver: "白银",
    league_gold: "黄金",
    league_platinum: "铂金",
    league_diamond: "钻石",
    league_master: "大师",
    league_grandmaster: "宗师",
    reconnect: "重新连接",
    rank_me_prefix: "你：",
    menu_network_opponents: "最近的对手",
    reconnecting_to_fmt: "正在重新连接 {}",
    reconnect_instructions: "请让 {} 打开“网络对战”菜单。",
    reconnect_trying: "正在尝试连接...（离开此界面前会一直重试）",
    own_address_error: "这是你自己的地址",
    online_label: "在线",
    round_fmt: "第 {} 局",
    player_ready_fmt: "{} 已准备",
    player_not_ready_fmt: "{} 尚未准备",
    ago_just_now: "刚刚",
    ago_hours_fmt: "{}小时前",
    ago_days_fmt: "{}天前",
};

// Japanese. Its glyphs come from the reduced Noto Sans JP font, placed before the Chinese font
// while Japanese is active (shared ideographs differ in shape between the two).

pub const JA: Locale = Locale {
    // Main menu
    menu_solo: "ソロ",
    menu_vs_computer: "VS コンピュータ",
    menu_vs_network: "VS ネットワーク",
    menu_options: "オプション",
    menu_info: "情報",
    menu_rules: "遊び方",
    menu_credits: "クレジット",
    menu_updates: "アップデート",
    menu_quit: "終了",
    menu_back: "戻る",
    menu_easy: "かんたん",
    menu_medium: "ふつう",
    menu_hard: "むずかしい",
    // Pause modal
    pause_title: "一時停止中",
    resume: "再開",
    quit: "終了",
    opponent_disconnected: "対戦相手の接続が切れました",
    ready_to_resume: "再開の準備OK",
    ready_check: "準備OK \u{2713}",
    waiting_opponent_resume: "対戦相手を待っています...",
    opponent_ready: "対戦相手の準備OK",
    // Ready modal
    ready: "準備OK",
    next: "次へ",
    ready_new_round: "次のラウンドの準備はいいですか？",
    menu_label: "メニュー",
    // Waiting banner
    waiting_for_opponent: "対戦相手を待っています...",
    connection_lost_banner: "接続が切れました...",
    // Network modal
    vs_network: "VS ネットワーク",
    cancel: "キャンセル",
    listening_incoming: "接続を待っています...",
    network_auto: "自動検索",
    define_field: "設定",
    invalid_address: "無効なアドレス",
    manual_connection: "手動接続",
    your_address: "あなたのアドレス（相手に伝えてください）：",
    opponent_address: "対戦相手のアドレス：",
    copy_address: "自分のアドレスをコピー",
    paste_address: "相手のアドレスを貼り付け",
    invalid_port: "無効なポート",
    port_field: "ポート",
    busy_suffix: "（取り込み中）",
    back: "戻る",
    players_on_lan: "ローカルネットワークのプレイヤー",
    looking_for_players: "プレイヤーを探しています...",
    scanning: "検索中...",
    no_network_detected: "ローカルネットワークが見つかりません",
    connect: "接続",
    waiting_for_fmt: "{} を待っています...",
    wants_to_play_fmt: "{} さんが対戦を申し込んでいます",
    accept: "受ける",
    decline: "断る",
    connecting_to_fmt: "{} に接続中...",
    in_game_suffix: "（対戦中）",
    player_tab: "一般",
    sound_tab: "サウンド",
    keyboard_tab: "キーボード",
    gamepad_tab: "ゲームパッド",
    ctrl_movement: "移動",
    ctrl_actions: "アクション",
    ctrl_directions: "方向",
    act_move_up: "上へ",
    act_move_down: "下へ",
    act_move_left: "左へ",
    act_move_right: "右へ",
    act_swap: "入れ替え",
    act_raise: "せり上げ",
    act_pause: "一時停止",
    dir_dpad: "十字キー",
    dir_stick: "スティック",
    dir_both: "両方",
    key_space: "スペース",
    key_enter: "Enter",
    key_esc: "Esc",
    key_backspace: "BackSpace",
    osk_backspace: "削除",
    osk_clear: "クリア",
    default_player_name: "Player 1",
    language_auto: "自動",
    error_peer_busy: "そのプレイヤーは取り込み中です",
    error_version_mismatch: "ゲームのバージョンが相手と異なります。同じバージョンにアップデートしてください。",
    error_handshake_fmt: "相手のゲームに接続できませんでした：{}",
    error_connection_fmt: "接続に失敗しました：{}",
    error_discovery_fmt: "プレイヤーを検索できません：{}",
    error_port_fmt: "ネットワークポートを開けません：{}",
    player_name: "プレイヤー：",
    edit_name: "変更",
    language_label: "言語：",
    fullscreen: "フルスクリーン",
    sound_effects: "効果音",
    music: "音楽",
    press_key_or_button: "キーかボタンを押してください...",
    cancel_capture_hint_fmt: "{} でキャンセル",
    reset_defaults: "初期設定に戻す",
    // End screen
    high_scores: "ハイスコア",
    no_scores_yet: "スコアはまだありません。",
    game_stats: "ゲーム統計",
    // Stat labels
    stat_score: "スコア：",
    stat_duration: "時間：",
    stat_blocks_eliminated: "消したブロック：",
    stat_chains: "連鎖：",
    stat_best_chain: "最大連鎖：",
    stat_garbage_shattered: "壊したおじゃま：",
    stat_swaps: "入れ替え回数：",
    stat_blocks_per_min: "ブロック/分：",
    stat_garbage_sent: "送ったおじゃま：",
    stat_garbage_received: "受けたおじゃま：",
    // Stat column headers
    stat_col_you: "あなた",
    stat_col_opponent: "相手",
    stat_col_record: "記録",
    // Footer and ticker
    forfeit: "降参",
    forfeit_confirm: "降参しますか？",
    github_cta: "\u{2605} GitHub でスターをお願いします！",
    ticker_update: "アップデート",
    update_available_fmt: "v{} が公開されました！",
    ignore_this_version: "このバージョンを無視",
    update_managed_by_store: "アップデートはアプリストアで管理されています。",
    update_managed_by_distro: "アップデートはディストリビューションで管理されています。",
    update_view_download_fmt: "バージョン {} が公開されました — ダウンロード",
    update_check_now: "今すぐ確認",
    update_status_checking: "確認中...",
    update_status_up_to_date_fmt: "最新バージョンです（{} に確認）",
    update_status_available_fmt: "バージョン {} が公開されています",
    update_status_ignored_fmt: "バージョン {} が公開されています（無視中）",
    update_status_failed: "現在確認できません",
    update_status_failed_since_fmt: "現在確認できません（最後に確認できたのは {}）",
    update_status_never: "未確認",
    update_checks_option: "アップデートを確認する",
    contribute: "開発に参加",
    view_download: "ダウンロードページ",
    update_installed_fmt: "インストール済みバージョン：{}",
    update_unavailable: "このビルドはアップデートを確認しません。",
    ticker_best_solo: "ソロ最高",
    ticker_best_combo: "最高コンボ",
    ticker_longest_solo: "ソロ最長",
    ticker_blocks_per_min: "ブロック/分 記録",
    menu_network_rank: "ネットワークランク",
    ticker_network_rank: "ネットワークランク",
    rank_last_parties: "最近の対戦",
    rank_no_matches: "ランク付きのネットワーク対戦はまだありません。",
    league_rookie: "ルーキー",
    league_copper: "カッパー",
    league_bronze: "ブロンズ",
    league_silver: "シルバー",
    league_gold: "ゴールド",
    league_platinum: "プラチナ",
    league_diamond: "ダイヤモンド",
    league_master: "マスター",
    league_grandmaster: "グランドマスター",
    reconnect: "再接続",
    rank_me_prefix: "あなた：",
    menu_network_opponents: "最近の対戦相手",
    reconnecting_to_fmt: "{} に再接続中",
    reconnect_instructions: "{} さんに「VS ネットワーク」メニューを開いてもらってください。",
    reconnect_trying: "接続を試みています...（この画面を閉じるまで再試行します）",
    own_address_error: "これはあなた自身のアドレスです",
    online_label: "オンライン",
    round_fmt: "ラウンド {}",
    player_ready_fmt: "{}：準備OK",
    player_not_ready_fmt: "{}：まだ準備中",
    ago_just_now: "たった今",
    ago_hours_fmt: "{}時間前",
    ago_days_fmt: "{}日前",
};

// Korean. Hangul comes from the reduced Noto Sans KR font. A player name is followed by the
// suffix 님, so the particle after it never depends on the name.

pub const KO: Locale = Locale {
    // Main menu
    menu_solo: "솔로",
    menu_vs_computer: "VS 컴퓨터",
    menu_vs_network: "VS 네트워크",
    menu_options: "옵션",
    menu_info: "정보",
    menu_rules: "게임 방법",
    menu_credits: "크레딧",
    menu_updates: "업데이트",
    menu_quit: "종료",
    menu_back: "뒤로",
    menu_easy: "쉬움",
    menu_medium: "보통",
    menu_hard: "어려움",
    // Pause modal
    pause_title: "일시 정지",
    resume: "계속하기",
    quit: "나가기",
    opponent_disconnected: "상대의 연결이 끊어졌습니다",
    ready_to_resume: "계속할 준비 완료",
    ready_check: "준비 완료 \u{2713}",
    waiting_opponent_resume: "상대를 기다리는 중...",
    opponent_ready: "상대 준비 완료",
    // Ready modal
    ready: "준비",
    next: "다음",
    ready_new_round: "다음 라운드를 시작할까요?",
    menu_label: "메뉴",
    // Waiting banner
    waiting_for_opponent: "상대를 기다리는 중...",
    connection_lost_banner: "연결이 끊어졌습니다...",
    // Network modal
    vs_network: "VS 네트워크",
    cancel: "취소",
    listening_incoming: "연결을 기다리는 중...",
    network_auto: "자동 검색",
    define_field: "설정",
    invalid_address: "잘못된 주소",
    manual_connection: "수동 연결",
    your_address: "내 주소 (상대에게 알려 주세요):",
    opponent_address: "상대 주소:",
    copy_address: "내 주소 복사",
    paste_address: "상대 주소 붙여넣기",
    invalid_port: "잘못된 포트",
    port_field: "포트",
    busy_suffix: "(바쁨)",
    back: "뒤로",
    players_on_lan: "로컬 네트워크의 플레이어",
    looking_for_players: "플레이어를 찾는 중...",
    scanning: "검색 중...",
    no_network_detected: "로컬 네트워크를 찾을 수 없습니다",
    connect: "연결",
    waiting_for_fmt: "{} 대기 중...",
    wants_to_play_fmt: "{}님이 함께 플레이하고 싶어 합니다",
    accept: "수락",
    decline: "거절",
    connecting_to_fmt: "{}에 연결 중...",
    in_game_suffix: "(게임 중)",
    player_tab: "일반",
    sound_tab: "사운드",
    keyboard_tab: "키보드",
    gamepad_tab: "게임패드",
    ctrl_movement: "이동",
    ctrl_actions: "동작",
    ctrl_directions: "방향",
    act_move_up: "위로",
    act_move_down: "아래로",
    act_move_left: "왼쪽으로",
    act_move_right: "오른쪽으로",
    act_swap: "바꾸기",
    act_raise: "올리기",
    act_pause: "일시 정지",
    dir_dpad: "방향 패드",
    dir_stick: "스틱",
    dir_both: "둘 다",
    key_space: "스페이스",
    key_enter: "Enter",
    key_esc: "Esc",
    key_backspace: "Backspace",
    osk_backspace: "지우기",
    osk_clear: "모두 지우기",
    default_player_name: "Player 1",
    language_auto: "자동",
    error_peer_busy: "그 플레이어는 바쁩니다",
    error_version_mismatch: "게임 버전이 상대와 다릅니다. 같은 버전으로 업데이트하세요.",
    error_handshake_fmt: "상대 게임에 연결할 수 없습니다: {}",
    error_connection_fmt: "연결 실패: {}",
    error_discovery_fmt: "플레이어를 검색할 수 없습니다: {}",
    error_port_fmt: "네트워크 포트를 열 수 없습니다: {}",
    player_name: "플레이어:",
    edit_name: "변경",
    language_label: "언어:",
    fullscreen: "전체 화면",
    sound_effects: "효과음",
    music: "음악",
    press_key_or_button: "키나 버튼을 누르세요...",
    cancel_capture_hint_fmt: "{} 키로 취소",
    reset_defaults: "기본값으로",
    // End screen
    high_scores: "최고 점수",
    no_scores_yet: "아직 점수가 없습니다.",
    game_stats: "게임 통계",
    // Stat labels
    stat_score: "점수:",
    stat_duration: "시간:",
    stat_blocks_eliminated: "지운 블록:",
    stat_chains: "연쇄:",
    stat_best_chain: "최대 연쇄:",
    stat_garbage_shattered: "부순 방해 블록:",
    stat_swaps: "바꾼 횟수:",
    stat_blocks_per_min: "블록/분:",
    stat_garbage_sent: "보낸 방해 블록:",
    stat_garbage_received: "받은 방해 블록:",
    // Stat column headers
    stat_col_you: "나",
    stat_col_opponent: "상대",
    stat_col_record: "기록",
    // Footer and ticker
    forfeit: "기권",
    forfeit_confirm: "기권할까요?",
    github_cta: "\u{2605} GitHub에서 별을 눌러 주세요!",
    ticker_update: "업데이트",
    update_available_fmt: "v{} 출시!",
    ignore_this_version: "이 버전 무시",
    update_managed_by_store: "업데이트는 앱 스토어에서 관리합니다.",
    update_managed_by_distro: "업데이트는 배포판에서 관리합니다.",
    update_view_download_fmt: "버전 {} 출시 — 다운로드",
    update_check_now: "지금 확인",
    update_status_checking: "확인 중...",
    update_status_up_to_date_fmt: "최신 버전입니다 ({} 확인)",
    update_status_available_fmt: "버전 {} 출시",
    update_status_ignored_fmt: "버전 {} 출시 (무시함)",
    update_status_failed: "지금은 확인할 수 없습니다",
    update_status_failed_since_fmt: "지금은 확인할 수 없습니다 (마지막 확인 {})",
    update_status_never: "확인한 적 없음",
    update_checks_option: "업데이트 확인",
    contribute: "참여하기",
    view_download: "다운로드 페이지",
    update_installed_fmt: "설치된 버전: {}",
    update_unavailable: "이 빌드는 업데이트를 확인하지 않습니다.",
    ticker_best_solo: "솔로 최고",
    ticker_best_combo: "최고 콤보",
    ticker_longest_solo: "솔로 최장",
    ticker_blocks_per_min: "블록/분 기록",
    menu_network_rank: "네트워크 랭크",
    ticker_network_rank: "네트워크 랭크",
    rank_last_parties: "최근 경기",
    rank_no_matches: "아직 랭크 네트워크 경기가 없습니다.",
    league_rookie: "루키",
    league_copper: "카퍼",
    league_bronze: "브론즈",
    league_silver: "실버",
    league_gold: "골드",
    league_platinum: "플래티넘",
    league_diamond: "다이아몬드",
    league_master: "마스터",
    league_grandmaster: "그랜드마스터",
    reconnect: "다시 연결",
    rank_me_prefix: "나:",
    menu_network_opponents: "최근 상대",
    reconnecting_to_fmt: "{}에 다시 연결 중",
    reconnect_instructions: "{}님에게 VS 네트워크 메뉴를 열어 달라고 요청하세요.",
    reconnect_trying: "연결을 시도하는 중... (이 화면을 떠날 때까지 다시 시도합니다)",
    own_address_error: "내 주소입니다",
    online_label: "온라인",
    round_fmt: "라운드 {}",
    player_ready_fmt: "{}님 준비 완료",
    player_not_ready_fmt: "{}님 아직 준비 중",
    ago_just_now: "방금",
    ago_hours_fmt: "{}시간 전",
    ago_days_fmt: "{}일 전",
};

// Polish

pub const PL: Locale = Locale {
    menu_solo: "SOLO",
    menu_vs_computer: "VS KOMPUTER",
    menu_vs_network: "VS SIEĆ",
    menu_options: "OPCJE",
    menu_info: "Informacje",
    menu_rules: "Jak grać",
    menu_credits: "Twórcy",
    menu_updates: "Aktualizacje",
    menu_quit: "WYJDŹ",
    menu_back: "WSTECZ",
    menu_easy: "Łatwy",
    menu_medium: "Średni",
    menu_hard: "Trudny",
    pause_title: "Pauza",
    resume: "Wznów",
    quit: "Wyjdź",
    opponent_disconnected: "Przeciwnik się rozłączył",
    ready_to_resume: "Gotowy do wznowienia",
    ready_check: "Gotowy \u{2713}",
    waiting_opponent_resume: "Oczekiwanie na przeciwnika...",
    opponent_ready: "Przeciwnik gotowy",
    ready: "Gotowy",
    next: "Dalej",
    ready_new_round: "Gotowy na kolejną rundę?",
    menu_label: "Menu",
    waiting_for_opponent: "Oczekiwanie na przeciwnika...",
    connection_lost_banner: "Utracono połączenie...",
    vs_network: "VS Sieć",
    cancel: "Anuluj",
    listening_incoming: "Oczekiwanie na połączenia...",
    network_auto: "Automatyczne wyszukiwanie",
    define_field: "Ustaw",
    invalid_address: "Nieprawidłowy adres",
    manual_connection: "Połączenie ręczne",
    your_address: "Twój adres (udostępnij go):",
    opponent_address: "Adres przeciwnika:",
    copy_address: "Kopiuj mój adres",
    paste_address: "Wklej adres przeciwnika",
    invalid_port: "Nieprawidłowy port",
    port_field: "Port",
    busy_suffix: "(zajęty)",
    back: "Wstecz",
    players_on_lan: "Gracze w sieci lokalnej",
    looking_for_players: "Szukanie graczy...",
    scanning: "Szukanie...",
    no_network_detected: "Nie wykryto sieci lokalnej",
    connect: "Połącz",
    waiting_for_fmt: "Oczekiwanie na gracza {}...",
    wants_to_play_fmt: "{} chce z tobą zagrać",
    accept: "Akceptuj",
    decline: "Odrzuć",
    connecting_to_fmt: "Łączenie z graczem {}...",
    in_game_suffix: "(w grze)",
    player_tab: "Ogólne",
    sound_tab: "Dźwięk",
    keyboard_tab: "Klawiatura",
    gamepad_tab: "Pad",
    ctrl_movement: "Ruch",
    ctrl_actions: "Akcje",
    ctrl_directions: "Kierunki",
    act_move_up: "W górę",
    act_move_down: "W dół",
    act_move_left: "W lewo",
    act_move_right: "W prawo",
    act_swap: "Zamień",
    act_raise: "Podnieś",
    act_pause: "Pauza",
    dir_dpad: "Krzyżak",
    dir_stick: "Gałka",
    dir_both: "Oba",
    key_space: "Spacja",
    key_enter: "Enter",
    key_esc: "Esc",
    key_backspace: "Backspace",
    osk_backspace: "Usuń",
    osk_clear: "Wyczyść",
    default_player_name: "Gracz 1",
    language_auto: "Auto",
    error_peer_busy: "Ten gracz jest zajęty",
    error_version_mismatch: "Twoja wersja gry różni się od wersji drugiego gracza. Zaktualizujcie grę, aby mieć tę samą wersję.",
    error_handshake_fmt: "Nie udało się połączyć z drugą grą: {}",
    error_connection_fmt: "Połączenie nieudane: {}",
    error_discovery_fmt: "Nie udało się wyszukać graczy: {}",
    error_port_fmt: "Nie można otworzyć portu sieciowego: {}",
    player_name: "Gracz:",
    edit_name: "Zmień",
    language_label: "Język:",
    fullscreen: "Pełny ekran",
    sound_effects: "Efekty dźwiękowe",
    music: "Muzyka",
    press_key_or_button: "Naciśnij klawisz lub przycisk...",
    cancel_capture_hint_fmt: "{}, aby anulować",
    reset_defaults: "Przywróć domyślne",
    high_scores: "Najlepsze wyniki",
    no_scores_yet: "Brak wyników.",
    game_stats: "Statystyki",
    stat_score: "Wynik:",
    stat_duration: "Czas:",
    stat_blocks_eliminated: "Usunięte bloki:",
    stat_chains: "Łańcuchy:",
    stat_best_chain: "Najlepszy łańcuch:",
    stat_garbage_shattered: "Rozbite śmieci:",
    stat_swaps: "Zamiany:",
    stat_blocks_per_min: "Bloki/min:",
    stat_garbage_sent: "Wysłane śmieci:",
    stat_garbage_received: "Otrzymane śmieci:",
    stat_col_you: "Ty",
    stat_col_opponent: "Przeciwnik",
    stat_col_record: "Rekord",
    forfeit: "Poddaj się",
    forfeit_confirm: "Poddać mecz?",
    github_cta: "\u{2605} Daj nam gwiazdkę na GitHubie!",
    ticker_update: "Aktualizacja",
    update_available_fmt: "Dostępna v{}!",
    ignore_this_version: "Ignoruj tę wersję",
    update_managed_by_store: "Aktualizacjami zarządza sklep z aplikacjami.",
    update_managed_by_distro: "Aktualizacjami zarządza twoja dystrybucja.",
    update_view_download_fmt: "Dostępna wersja {} — Pobierz",
    update_check_now: "Sprawdź teraz",
    update_status_checking: "Sprawdzanie...",
    update_status_up_to_date_fmt: "Masz najnowszą wersję (sprawdzono {})",
    update_status_available_fmt: "Dostępna wersja {}",
    update_status_ignored_fmt: "Dostępna wersja {} (zignorowana)",
    update_status_failed: "Nie można teraz sprawdzić",
    update_status_failed_since_fmt: "Nie można teraz sprawdzić (ostatnie udane sprawdzenie: {})",
    update_status_never: "Nigdy nie sprawdzano",
    update_checks_option: "Sprawdzaj aktualizacje",
    contribute: "Współtwórz",
    view_download: "Strona pobierania",
    update_installed_fmt: "Zainstalowana wersja: {}",
    update_unavailable: "Ta wersja nie sprawdza aktualizacji.",
    ticker_best_solo: "Najlepsze solo",
    ticker_best_combo: "Najlepsze combo",
    ticker_longest_solo: "Najdłuższe solo",
    ticker_blocks_per_min: "Rekord bloków/min",
    menu_network_rank: "Ranking sieciowy",
    ticker_network_rank: "Ranking sieciowy",
    rank_last_parties: "Ostatnie mecze",
    rank_no_matches: "Brak rankingowych meczów sieciowych.",
    league_rookie: "Nowicjusz",
    league_copper: "Miedź",
    league_bronze: "Brąz",
    league_silver: "Srebro",
    league_gold: "Złoto",
    league_platinum: "Platyna",
    league_diamond: "Diament",
    league_master: "Mistrz",
    league_grandmaster: "Arcymistrz",
    reconnect: "Połącz ponownie",
    rank_me_prefix: "Ty:",
    menu_network_opponents: "Ostatni przeciwnicy",
    reconnecting_to_fmt: "Ponowne łączenie z graczem {}",
    reconnect_instructions: "Poproś gracza {} o otwarcie menu VS Sieć.",
    reconnect_trying: "Łączenie... (ponawiane, dopóki nie opuścisz tego ekranu)",
    own_address_error: "To twój własny adres",
    online_label: "online",
    round_fmt: "Runda {}",
    player_ready_fmt: "{} jest gotowy",
    player_not_ready_fmt: "{} nie jest jeszcze gotowy",
    ago_just_now: "przed chwilą",
    ago_hours_fmt: "{} godz. temu",
    ago_days_fmt: "{} d. temu",
};

// Turkish. The ticker labels go through `uppercase`, which applies the Turkish dotted
// capital İ.

pub const TR: Locale = Locale {
    menu_solo: "SOLO",
    menu_vs_computer: "VS BİLGİSAYAR",
    menu_vs_network: "VS AĞ",
    menu_options: "SEÇENEKLER",
    menu_info: "Hakkında",
    menu_rules: "Nasıl oynanır",
    menu_credits: "Emeği geçenler",
    menu_updates: "Güncellemeler",
    menu_quit: "ÇIKIŞ",
    menu_back: "GERİ",
    menu_easy: "Kolay",
    menu_medium: "Orta",
    menu_hard: "Zor",
    pause_title: "Duraklatıldı",
    resume: "Devam et",
    quit: "Çık",
    opponent_disconnected: "Rakibin bağlantısı koptu",
    ready_to_resume: "Devam etmeye hazır",
    ready_check: "Hazır \u{2713}",
    waiting_opponent_resume: "Rakip bekleniyor...",
    opponent_ready: "Rakip hazır",
    ready: "Hazır",
    next: "İleri",
    ready_new_round: "Yeni bir tura hazır mısın?",
    menu_label: "Menü",
    waiting_for_opponent: "Rakip bekleniyor...",
    connection_lost_banner: "Bağlantı koptu...",
    vs_network: "VS Ağ",
    cancel: "İptal",
    listening_incoming: "Gelen bağlantılar bekleniyor...",
    network_auto: "Otomatik arama",
    define_field: "Ayarla",
    invalid_address: "Geçersiz adres",
    manual_connection: "Elle bağlantı",
    your_address: "Adresin (paylaş):",
    opponent_address: "Rakibin adresi:",
    copy_address: "Adresimi kopyala",
    paste_address: "Rakibin adresini yapıştır",
    invalid_port: "Geçersiz port",
    port_field: "Port",
    busy_suffix: "(meşgul)",
    back: "Geri",
    players_on_lan: "Yerel ağdaki oyuncular",
    looking_for_players: "Oyuncu aranıyor...",
    scanning: "Aranıyor...",
    no_network_detected: "Yerel ağ algılanmadı",
    connect: "Bağlan",
    waiting_for_fmt: "{} bekleniyor...",
    wants_to_play_fmt: "{} seninle oynamak istiyor",
    accept: "Kabul et",
    decline: "Reddet",
    connecting_to_fmt: "{} ile bağlanılıyor...",
    in_game_suffix: "(oyunda)",
    player_tab: "Genel",
    sound_tab: "Ses",
    keyboard_tab: "Klavye",
    gamepad_tab: "Oyun kolu",
    ctrl_movement: "Hareket",
    ctrl_actions: "Eylemler",
    ctrl_directions: "Yönler",
    act_move_up: "Yukarı",
    act_move_down: "Aşağı",
    act_move_left: "Sola",
    act_move_right: "Sağa",
    act_swap: "Değiştir",
    act_raise: "Yükselt",
    act_pause: "Duraklat",
    dir_dpad: "Yön tuşları",
    dir_stick: "Analog çubuk",
    dir_both: "İkisi de",
    key_space: "Boşluk",
    key_enter: "Enter",
    key_esc: "Esc",
    key_backspace: "Geri sil",
    osk_backspace: "Sil",
    osk_clear: "Temizle",
    default_player_name: "Oyuncu 1",
    language_auto: "Otomatik",
    error_peer_busy: "Bu oyuncu meşgul",
    error_version_mismatch: "Oyun sürümün diğer oyuncununkiyle aynı değil. Aynı sürümü kullanmak için güncelleyin.",
    error_handshake_fmt: "Diğer oyuna bağlanılamadı: {}",
    error_connection_fmt: "Bağlantı başarısız: {}",
    error_discovery_fmt: "Oyuncu aranamadı: {}",
    error_port_fmt: "Ağ portu açılamıyor: {}",
    player_name: "Oyuncu:",
    edit_name: "Düzenle",
    language_label: "Dil:",
    fullscreen: "Tam ekran",
    sound_effects: "Ses efektleri",
    music: "Müzik",
    press_key_or_button: "Bir tuşa veya düğmeye bas...",
    cancel_capture_hint_fmt: "İptal için {}",
    reset_defaults: "Varsayılanlara dön",
    high_scores: "En yüksek skorlar",
    no_scores_yet: "Henüz skor yok.",
    game_stats: "Oyun istatistikleri",
    stat_score: "Skor:",
    stat_duration: "Süre:",
    stat_blocks_eliminated: "Temizlenen bloklar:",
    stat_chains: "Zincirler:",
    stat_best_chain: "En iyi zincir:",
    stat_garbage_shattered: "Kırılan çöp:",
    stat_swaps: "Değişimler:",
    stat_blocks_per_min: "Blok/dk:",
    stat_garbage_sent: "Gönderilen çöp:",
    stat_garbage_received: "Alınan çöp:",
    stat_col_you: "Sen",
    stat_col_opponent: "Rakip",
    stat_col_record: "Rekor",
    forfeit: "Pes et",
    forfeit_confirm: "Maçtan çekilmek istiyor musun?",
    github_cta: "\u{2605} GitHub'da bize yıldız ver!",
    ticker_update: "Güncelleme",
    update_available_fmt: "v{} çıktı!",
    ignore_this_version: "Bu sürümü yok say",
    update_managed_by_store: "Güncellemeleri uygulama mağazan yönetir.",
    update_managed_by_distro: "Güncellemeleri dağıtımın yönetir.",
    update_view_download_fmt: "Sürüm {} çıktı — İndir",
    update_check_now: "Şimdi denetle",
    update_status_checking: "Denetleniyor...",
    update_status_up_to_date_fmt: "En son sürüme sahipsin (denetlendi: {})",
    update_status_available_fmt: "Sürüm {} çıktı",
    update_status_ignored_fmt: "Sürüm {} çıktı (yok sayıldı)",
    update_status_failed: "Şu anda denetlenemiyor",
    update_status_failed_since_fmt: "Şu anda denetlenemiyor (son başarılı denetim: {})",
    update_status_never: "Hiç denetlenmedi",
    update_checks_option: "Güncellemeleri denetle",
    contribute: "Katkıda bulun",
    view_download: "İndirme sayfası",
    update_installed_fmt: "Yüklü sürüm: {}",
    update_unavailable: "Bu sürüm güncellemeleri denetlemez.",
    ticker_best_solo: "En iyi solo",
    ticker_best_combo: "En iyi kombo",
    ticker_longest_solo: "En uzun solo",
    ticker_blocks_per_min: "Rekor blok/dk",
    menu_network_rank: "Ağ sıralaması",
    ticker_network_rank: "Ağ sıralaması",
    rank_last_parties: "Son maçlar",
    rank_no_matches: "Henüz sıralamalı ağ maçı yok.",
    league_rookie: "Çaylak",
    league_copper: "Bakır",
    league_bronze: "Bronz",
    league_silver: "Gümüş",
    league_gold: "Altın",
    league_platinum: "Platin",
    league_diamond: "Elmas",
    league_master: "Usta",
    league_grandmaster: "Büyük usta",
    reconnect: "Yeniden bağlan",
    rank_me_prefix: "Sen:",
    menu_network_opponents: "Son rakipler",
    reconnecting_to_fmt: "{} ile yeniden bağlanılıyor",
    reconnect_instructions: "{} adlı oyuncudan VS Ağ menüsünü açmasını iste.",
    reconnect_trying: "Bağlanılıyor... (bu ekrandan çıkana kadar yeniden denenir)",
    own_address_error: "Bu senin kendi adresin",
    online_label: "çevrimiçi",
    round_fmt: "Tur {}",
    player_ready_fmt: "{} hazır",
    player_not_ready_fmt: "{} henüz hazır değil",
    ago_just_now: "az önce",
    ago_hours_fmt: "{} sa önce",
    ago_days_fmt: "{} gün önce",
};

// Italian

pub const IT: Locale = Locale {
    menu_solo: "SOLO",
    menu_vs_computer: "VS COMPUTER",
    menu_vs_network: "VS RETE",
    menu_options: "OPZIONI",
    menu_info: "Informazioni",
    menu_rules: "Come giocare",
    menu_credits: "Riconoscimenti",
    menu_updates: "Aggiornamenti",
    menu_quit: "ESCI",
    menu_back: "INDIETRO",
    menu_easy: "Facile",
    menu_medium: "Medio",
    menu_hard: "Difficile",
    pause_title: "In pausa",
    resume: "Riprendi",
    quit: "Esci",
    opponent_disconnected: "Avversario disconnesso",
    ready_to_resume: "Pronto a riprendere",
    ready_check: "Pronto \u{2713}",
    waiting_opponent_resume: "In attesa dell'avversario...",
    opponent_ready: "Avversario pronto",
    ready: "Pronto",
    next: "Avanti",
    ready_new_round: "Pronto per un altro round?",
    menu_label: "Menu",
    waiting_for_opponent: "In attesa dell'avversario...",
    connection_lost_banner: "Connessione persa...",
    vs_network: "VS Rete",
    cancel: "Annulla",
    listening_incoming: "In attesa di connessioni...",
    network_auto: "Ricerca automatica",
    define_field: "Imposta",
    invalid_address: "Indirizzo non valido",
    manual_connection: "Connessione manuale",
    your_address: "Il tuo indirizzo (condividilo):",
    opponent_address: "Indirizzo dell'avversario:",
    copy_address: "Copia il mio indirizzo",
    paste_address: "Incolla l'indirizzo dell'avversario",
    invalid_port: "Porta non valida",
    port_field: "Porta",
    busy_suffix: "(occupato)",
    back: "Indietro",
    players_on_lan: "Giocatori sulla rete locale",
    looking_for_players: "Ricerca di giocatori...",
    scanning: "Ricerca...",
    no_network_detected: "Nessuna rete locale rilevata",
    connect: "Connetti",
    waiting_for_fmt: "In attesa di {}...",
    wants_to_play_fmt: "{} vuole giocare con te",
    accept: "Accetta",
    decline: "Rifiuta",
    connecting_to_fmt: "Connessione a {}...",
    in_game_suffix: "(in partita)",
    player_tab: "Generale",
    sound_tab: "Audio",
    keyboard_tab: "Tastiera",
    gamepad_tab: "Controller",
    ctrl_movement: "Movimento",
    ctrl_actions: "Azioni",
    ctrl_directions: "Direzioni",
    act_move_up: "Su",
    act_move_down: "Giù",
    act_move_left: "Sinistra",
    act_move_right: "Destra",
    act_swap: "Scambia",
    act_raise: "Alza",
    act_pause: "Pausa",
    dir_dpad: "Croce",
    dir_stick: "Levetta",
    dir_both: "Entrambi",
    key_space: "Spazio",
    key_enter: "Invio",
    key_esc: "Esc",
    key_backspace: "Backspace",
    osk_backspace: "Canc.",
    osk_clear: "Svuota",
    default_player_name: "Giocatore 1",
    language_auto: "Auto",
    error_peer_busy: "Quel giocatore è occupato",
    error_version_mismatch: "La tua versione del gioco non corrisponde a quella dell'altro giocatore. Aggiornate per avere entrambi la stessa versione.",
    error_handshake_fmt: "Impossibile connettersi all'altro gioco: {}",
    error_connection_fmt: "Connessione non riuscita: {}",
    error_discovery_fmt: "Impossibile cercare giocatori: {}",
    error_port_fmt: "Impossibile aprire la porta di rete: {}",
    player_name: "Giocatore:",
    edit_name: "Modifica",
    language_label: "Lingua:",
    fullscreen: "Schermo intero",
    sound_effects: "Effetti sonori",
    music: "Musica",
    press_key_or_button: "Premi un tasto o un pulsante...",
    cancel_capture_hint_fmt: "{} per annullare",
    reset_defaults: "Ripristina predefiniti",
    high_scores: "Punteggi migliori",
    no_scores_yet: "Ancora nessun punteggio.",
    game_stats: "Statistiche",
    stat_score: "Punteggio:",
    stat_duration: "Durata:",
    stat_blocks_eliminated: "Blocchi eliminati:",
    stat_chains: "Catene:",
    stat_best_chain: "Catena migliore:",
    stat_garbage_shattered: "Spazzatura distrutta:",
    stat_swaps: "Scambi:",
    stat_blocks_per_min: "Blocchi/min:",
    stat_garbage_sent: "Spazzatura inviata:",
    stat_garbage_received: "Spazzatura ricevuta:",
    stat_col_you: "Tu",
    stat_col_opponent: "Avversario",
    stat_col_record: "Record",
    forfeit: "Abbandona",
    forfeit_confirm: "Abbandonare la partita?",
    github_cta: "\u{2605} Lasciaci una stella su GitHub!",
    ticker_update: "Aggiornamento",
    update_available_fmt: "v{} disponibile!",
    ignore_this_version: "Ignora questa versione",
    update_managed_by_store: "Gli aggiornamenti sono gestiti dal tuo negozio di applicazioni.",
    update_managed_by_distro: "Gli aggiornamenti sono gestiti dalla tua distribuzione.",
    update_view_download_fmt: "Versione {} disponibile — Scarica",
    update_check_now: "Controlla ora",
    update_status_checking: "Controllo in corso...",
    update_status_up_to_date_fmt: "Hai l'ultima versione (controllato il {})",
    update_status_available_fmt: "Versione {} disponibile",
    update_status_ignored_fmt: "Versione {} disponibile (ignorata)",
    update_status_failed: "Impossibile controllare al momento",
    update_status_failed_since_fmt: "Impossibile controllare al momento (ultimo controllo riuscito il {})",
    update_status_never: "Mai controllato",
    update_checks_option: "Controlla aggiornamenti",
    contribute: "Contribuisci",
    view_download: "Pagina di download",
    update_installed_fmt: "Versione installata: {}",
    update_unavailable: "Questa versione non controlla gli aggiornamenti.",
    ticker_best_solo: "Miglior solo",
    ticker_best_combo: "Miglior combo",
    ticker_longest_solo: "Solo più lungo",
    ticker_blocks_per_min: "Record blocchi/min",
    menu_network_rank: "Classifica in rete",
    ticker_network_rank: "Classifica in rete",
    rank_last_parties: "Partite recenti",
    rank_no_matches: "Ancora nessuna partita in rete classificata.",
    league_rookie: "Recluta",
    league_copper: "Rame",
    league_bronze: "Bronzo",
    league_silver: "Argento",
    league_gold: "Oro",
    league_platinum: "Platino",
    league_diamond: "Diamante",
    league_master: "Maestro",
    league_grandmaster: "Gran maestro",
    reconnect: "Riconnetti",
    rank_me_prefix: "Tu:",
    menu_network_opponents: "Avversari recenti",
    reconnecting_to_fmt: "Riconnessione a {}",
    reconnect_instructions: "Chiedi a {} di aprire il menu VS Rete.",
    reconnect_trying: "Connessione in corso... (riprova finché non lasci questa schermata)",
    own_address_error: "Questo è il tuo indirizzo",
    online_label: "online",
    round_fmt: "Round {}",
    player_ready_fmt: "{} è pronto",
    player_not_ready_fmt: "{} non è ancora pronto",
    ago_just_now: "proprio ora",
    ago_hours_fmt: "{} h fa",
    ago_days_fmt: "{} g fa",
};

// Language management

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Language {
    English,
    French,
    German,
    Spanish,
    Russian,
    /// Simplified Chinese.
    Chinese,
    Japanese,
    Korean,
    Polish,
    Turkish,
    Italian,
}

impl Language {
    /// Convert from persistence string ("en", "fr").
    pub fn from_code(code: &str) -> Option<Self> {
        match code {
            "en" => Some(Language::English),
            "fr" => Some(Language::French),
            "de" => Some(Language::German),
            "es" => Some(Language::Spanish),
            "ru" => Some(Language::Russian),
            "zh" => Some(Language::Chinese),
            "ja" => Some(Language::Japanese),
            "ko" => Some(Language::Korean),
            "pl" => Some(Language::Polish),
            "tr" => Some(Language::Turkish),
            "it" => Some(Language::Italian),
            _ => None,
        }
    }

    /// Convert to persistence string.
    pub fn code(self) -> &'static str {
        match self {
            Language::English => "en",
            Language::French => "fr",
            Language::German => "de",
            Language::Spanish => "es",
            Language::Russian => "ru",
            Language::Chinese => "zh",
            Language::Japanese => "ja",
            Language::Korean => "ko",
            Language::Polish => "pl",
            Language::Turkish => "tr",
            Language::Italian => "it",
        }
    }

    /// Display name in the language itself (also the native name shown in the language selector).
    pub fn display_name(self) -> &'static str {
        match self {
            Language::English => "English",
            Language::French => "Français",
            Language::German => "Deutsch",
            Language::Spanish => "Español",
            Language::Russian => "Русский",
            Language::Chinese => "简体中文",
            Language::Japanese => "日本語",
            Language::Korean => "한국어",
            Language::Polish => "Polski",
            Language::Turkish => "Türkçe",
            Language::Italian => "Italiano",
        }
    }

    /// Single source of truth for the list of languages (UI, cycling, persistence).
    /// To add a language: add a variant, an entry here and its strings in `Locale`.
    pub const ALL: &'static [Language] = &[
        Language::English, Language::French, Language::German, Language::Spanish,
        Language::Italian, Language::Polish, Language::Russian, Language::Turkish,
        Language::Chinese, Language::Japanese, Language::Korean,
    ];
}

// Stores the index into `Language::ALL`, not a magic code, so adding a language
// needs no extra mapping table.
static CURRENT_LANGUAGE: AtomicU8 = AtomicU8::new(0); // 0 = Language::ALL[0]

pub fn current_language() -> Language {
    let idx = CURRENT_LANGUAGE.load(Ordering::Relaxed) as usize;
    Language::ALL.get(idx).copied().unwrap_or(Language::English)
}

pub fn set_language(lang: Language) {
    let idx = Language::ALL.iter().position(|&l| l == lang).unwrap_or(0);
    CURRENT_LANGUAGE.store(idx as u8, Ordering::Relaxed);
}

/// Get the current locale strings.

impl Locale {
    /// All text fields, for the coverage tests.
    /// The list is maintained by hand. `Locale` only holds `&'static str`, so the test
    /// `all_text_fields_is_complete` (struct size == n × size of a `&str`) catches any
    /// field missing here.
    #[cfg(test)]
    fn all_text_fields(&self) -> Vec<&'static str> {
        vec![
            self.menu_solo,
            self.menu_vs_computer,
            self.menu_vs_network,
            self.menu_options,
            self.menu_info,
            self.menu_rules,
            self.menu_credits,
            self.menu_updates,
            self.menu_quit,
            self.menu_back,
            self.menu_easy,
            self.menu_medium,
            self.menu_hard,
            self.pause_title,
            self.resume,
            self.quit,
            self.opponent_disconnected,
            self.ready_to_resume,
            self.ready_check,
            self.waiting_opponent_resume,
            self.opponent_ready,
            self.ready,
            self.next,
            self.ready_new_round,
            self.menu_label,
            self.waiting_for_opponent,
            self.connection_lost_banner,
            self.vs_network,
            self.cancel,
            self.listening_incoming,
            self.network_auto,
            self.define_field,
            self.invalid_address,
            self.manual_connection,
            self.your_address,
            self.opponent_address,
            self.copy_address,
            self.paste_address,
            self.invalid_port,
            self.port_field,
            self.busy_suffix,
            self.back,
            self.players_on_lan,
            self.looking_for_players,
            self.scanning,
            self.no_network_detected,
            self.connect,
            self.waiting_for_fmt,
            self.wants_to_play_fmt,
            self.accept,
            self.decline,
            self.connecting_to_fmt,
            self.in_game_suffix,
            self.player_tab,
            self.sound_tab,
            self.keyboard_tab,
            self.gamepad_tab,
            self.ctrl_movement,
            self.ctrl_actions,
            self.ctrl_directions,
            self.act_move_up,
            self.act_move_down,
            self.act_move_left,
            self.act_move_right,
            self.act_swap,
            self.act_raise,
            self.act_pause,
            self.dir_dpad,
            self.dir_stick,
            self.dir_both,
            self.key_space,
            self.key_enter,
            self.key_esc,
            self.key_backspace,
            self.osk_backspace,
            self.osk_clear,
            self.default_player_name,
            self.language_auto,
            self.error_peer_busy,
            self.error_version_mismatch,
            self.error_handshake_fmt,
            self.error_connection_fmt,
            self.error_discovery_fmt,
            self.error_port_fmt,
            self.player_name,
            self.edit_name,
            self.language_label,
            self.fullscreen,
            self.sound_effects,
            self.music,
            self.press_key_or_button,
            self.cancel_capture_hint_fmt,
            self.reset_defaults,
            self.high_scores,
            self.no_scores_yet,
            self.game_stats,
            self.stat_score,
            self.stat_duration,
            self.stat_blocks_eliminated,
            self.stat_chains,
            self.stat_best_chain,
            self.stat_garbage_shattered,
            self.stat_swaps,
            self.stat_blocks_per_min,
            self.stat_garbage_sent,
            self.stat_garbage_received,
            self.stat_col_you,
            self.stat_col_opponent,
            self.stat_col_record,
            self.forfeit,
            self.forfeit_confirm,
            self.github_cta,
            self.ticker_update,
            self.update_available_fmt,
            self.ignore_this_version,
            self.update_managed_by_store,
            self.update_managed_by_distro,
            self.update_view_download_fmt,
            self.update_check_now,
            self.update_status_checking,
            self.update_status_up_to_date_fmt,
            self.update_status_available_fmt,
            self.update_status_ignored_fmt,
            self.update_status_failed,
            self.update_status_failed_since_fmt,
            self.update_status_never,
            self.update_checks_option,
            self.contribute,
            self.view_download,
            self.update_installed_fmt,
            self.update_unavailable,
            self.ticker_best_solo,
            self.ticker_best_combo,
            self.ticker_longest_solo,
            self.ticker_blocks_per_min,
            self.menu_network_rank,
            self.ticker_network_rank,
            self.rank_last_parties,
            self.rank_no_matches,
            self.league_rookie,
            self.league_copper,
            self.league_bronze,
            self.league_silver,
            self.league_gold,
            self.league_platinum,
            self.league_diamond,
            self.league_master,
            self.league_grandmaster,
            self.reconnect,
            self.rank_me_prefix,
            self.menu_network_opponents,
            self.reconnecting_to_fmt,
            self.reconnect_instructions,
            self.reconnect_trying,
            self.own_address_error,
            self.online_label,
            self.round_fmt,
            self.player_ready_fmt,
            self.player_not_ready_fmt,
            self.ago_just_now,
            self.ago_hours_fmt,
            self.ago_days_fmt,
        ]
    }
}

/// Upper case in the current language. `str::to_uppercase` ignores the language, and Turkish
/// writes the capital of `i` as `İ` (the dotless `ı` becomes `I` either way).
pub fn uppercase(text: &str) -> String {
    if current_language() == Language::Turkish {
        text.replace('i', "İ").to_uppercase()
    } else {
        text.to_uppercase()
    }
}

/// Localized label of a rebindable action.
pub fn binding_action_label(l: &Locale, action: crate::platform::persist::BindingAction) -> &'static str {
    use crate::platform::persist::BindingAction as A;
    match action {
        A::Up => l.act_move_up,
        A::Down => l.act_move_down,
        A::Left => l.act_move_left,
        A::Right => l.act_move_right,
        A::Swap => l.act_swap,
        A::Raise => l.act_raise,
        A::Pause => l.act_pause,
    }
}

pub fn t() -> &'static Locale {
    match current_language() {
        Language::English => &EN,
        Language::French => &FR,
        Language::German => &DE,
        Language::Spanish => &ES,
        Language::Russian => &RU,
        Language::Chinese => &ZH,
        Language::Japanese => &JA,
        Language::Korean => &KO,
        Language::Polish => &PL,
        Language::Turkish => &TR,
        Language::Italian => &IT,
    }
}

/// Serializes the tests that mutate the global `CURRENT_LANGUAGE` (the one below and
/// the `prompts` test); otherwise they interleave when run in parallel.
/// `pub(crate)` because it is shared between modules.
#[cfg(test)]
pub(crate) static LANG_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Holds `LANG_TEST_LOCK` and restores the language that was active on entry, even if the
/// test panics (a test that triggers `set_language` indirectly must use it).
#[cfg(test)]
pub(crate) struct LangGuard {
    _lock: std::sync::MutexGuard<'static, ()>,
    previous: Language,
}

#[cfg(test)]
impl LangGuard {
    pub(crate) fn new() -> Self {
        let lock = LANG_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        Self { _lock: lock, previous: current_language() }
    }
}

#[cfg(test)]
impl Drop for LangGuard {
    fn drop(&mut self) {
        set_language(self.previous);
    }
}

#[cfg(test)]
mod i18n_tests {
    use super::*;

    // No text field is empty in any locale.
    #[test]
    fn no_empty_field() {
        for (name, loc) in [("EN", &EN), ("FR", &FR), ("DE", &DE), ("ES", &ES), ("RU", &RU), ("ZH", &ZH), ("JA", &JA), ("KO", &KO), ("PL", &PL), ("TR", &TR), ("IT", &IT)] {
            for (i, f) in loc.all_text_fields().iter().enumerate() {
                assert!(!f.is_empty(), "{name} : champ #{i} vide");
            }
        }
    }

    // `{}` placeholders match between EN and FR (a missing or doubled `{}` would
    // silently drop or duplicate a name).
    #[test]
    fn placeholder_parity() {
        let en = EN.all_text_fields();
        for (name, loc) in [("FR", &FR), ("DE", &DE), ("ES", &ES), ("RU", &RU), ("ZH", &ZH), ("JA", &JA), ("KO", &KO), ("PL", &PL), ("TR", &TR), ("IT", &IT)] {
            let other = loc.all_text_fields();
            assert_eq!(en.len(), other.len());
            for i in 0..en.len() {
                let ce = en[i].matches("{}").count();
                let co = other[i].matches("{}").count();
                assert_eq!(ce, co, "field #{i}: {ce} `{{}}` in EN vs {co} in {name}");
            }
        }
    }

    // One ellipsis style: ASCII `...` everywhere, never U+2026.
    #[test]
    fn ellipsis_style_uniform() {
        for (name, loc) in [("EN", &EN), ("FR", &FR), ("DE", &DE), ("ES", &ES), ("RU", &RU), ("ZH", &ZH), ("JA", &JA), ("KO", &KO), ("PL", &PL), ("TR", &TR), ("IT", &IT)] {
            for f in loc.all_text_fields() {
                assert!(!f.contains('\u{2026}'), "{name} : « {f} » utilise U+2026 au lieu de « ... »");
            }
        }
    }

    // `from_code(code())` round-trips for every language, and `ALL` is exhaustive
    // (the one link the compiler does not protect).
    #[test]
    fn code_roundtrip_and_all_exhaustive() {
        for &lang in Language::ALL {
            assert_eq!(Language::from_code(lang.code()), Some(lang), "round trip broken for {lang:?}");
        }
        // Exhaustiveness guard: every variant MUST be in `ALL`. Adding a language
        // without updating this match breaks the build here.
        for lang in [Language::English, Language::French, Language::German, Language::Spanish, Language::Russian, Language::Chinese,
                     Language::Japanese, Language::Korean, Language::Polish, Language::Turkish, Language::Italian] {
            let present = match lang {
                Language::English | Language::French | Language::German | Language::Spanish | Language::Russian
                | Language::Chinese | Language::Japanese | Language::Korean | Language::Polish
                | Language::Turkish | Language::Italian => {
                    Language::ALL.contains(&lang)
                }
            };
            assert!(present, "{lang:?} missing from Language::ALL");
        }
    }

    // Turkish upper case keeps the dot of `i`; other languages use the plain rule.
    #[test]
    fn uppercase_follows_the_language() {
        let _guard = LangGuard::new();
        set_language(Language::Turkish);
        assert_eq!(uppercase("En iyi solo, ılık"), "EN İYİ SOLO, ILIK");
        set_language(Language::Italian);
        assert_eq!(uppercase("Miglior solo"), "MIGLIOR SOLO");
    }

    // `code()` and `display_name()` are unique across `ALL`.
    #[test]
    fn codes_and_names_unique() {
        let mut codes: Vec<&str> = Language::ALL.iter().map(|l| l.code()).collect();
        codes.sort_unstable();
        let n = codes.len();
        codes.dedup();
        assert_eq!(codes.len(), n, "codes de langue dupliqués");
        let mut names: Vec<&str> = Language::ALL.iter().map(|l| l.display_name()).collect();
        names.sort_unstable();
        let n = names.len();
        names.dedup();
        assert_eq!(names.len(), n, "noms de langue dupliqués");
    }

    // `all_text_fields` is complete. `Locale` only has `&'static str` fields, so the
    // struct size counts them; a field missing from the list fails here.
    #[test]
    fn all_text_fields_is_complete() {
        let expected = std::mem::size_of::<Locale>() / std::mem::size_of::<&'static str>();
        assert_eq!(
            EN.all_text_fields().len(),
            expected,
            "all_text_fields ne liste pas tous les champs du struct Locale"
        );
    }

    // `set_language` is wired to `t()`: the selector changes the rendered texts.
    #[test]
    fn set_language_wires_to_t() {
        let _g = super::LANG_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        // `EN` and `FR` are `const`s (they may be duplicated in memory), so `ptr::eq`
        // does not work. Compare `current_language()` and a distinctive field instead.
        set_language(Language::French);
        assert_eq!(current_language(), Language::French);
        assert_eq!(t().player_name, FR.player_name);
        set_language(Language::English);
        assert_eq!(current_language(), Language::English);
        assert_eq!(t().player_name, EN.player_name);
    }
}
