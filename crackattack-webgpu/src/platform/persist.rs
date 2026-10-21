//! Persistence — save/load player state and high scores.
//!
//! Storage location: `state.json` in the config directory (see `config_dir`).

use std::path::PathBuf;
use serde::{Serialize, Deserialize};

/// Maximum score entries in the table.
const MAX_SCORES: usize = 30;

/// Identifies an action for key binding configuration.
/// Separate from `AppKey` (runtime abstraction) — this is a config concept.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BindingAction {
    Up, Down, Left, Right,
    // `Raise` is the gameplay action that raises the stack. UI confirm and cancel are fixed
    // conventions (see `ui_conventions`), not rebindable. `Pause` is rebindable on the
    // keyboard; the gamepad pause (Start) and Escape are fixed conventions.
    Swap, Raise, Pause,
}

impl BindingAction {
    /// All actions in display order.
    pub const ALL: [BindingAction; 7] = [
        Self::Up, Self::Down, Self::Left, Self::Right,
        Self::Swap, Self::Raise, Self::Pause,
    ];

    /// Maximum number of input slots for this action.
    /// Directions: 4 (2 keyboard + 2 gamepad: d-pad + stick).
    /// Other actions: 2 (1 keyboard + 1 gamepad).
    pub fn max_keys(self) -> usize {
        match self {
            Self::Up | Self::Down | Self::Left | Self::Right => 4,
            _ => 2,
        }
    }
}

/// Gamepad direction mode: which directional inputs count as movement.
///
/// On a gamepad the player picks a mode instead of rebinding each direction. On the keyboard,
/// directions are rebound key by key.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum DirMode {
    /// D-pad only.
    DPad,
    /// Left stick only.
    Stick,
    /// Both (the default).
    #[default]
    Both,
}

/// Persistent input bindings. Each action maps to a list of input names.
/// Names are stable strings, independent of any input library:
/// keyboard ("ArrowUp", "KeyW"), gamepad ("PadSouth", "PadDPadUp", "PadLeftStickUp").
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct KeyBindings {
    pub up: Vec<String>,
    pub down: Vec<String>,
    pub left: Vec<String>,
    pub right: Vec<String>,
    pub swap: Vec<String>,
    /// Gameplay action that raises the stack. The alias loads older `state.json` files where
    /// this binding was called `confirm`.
    #[serde(alias = "confirm")]
    pub raise: Vec<String>,
    /// Rebindable keyboard pause (keyboard slot only). The gamepad pause (Start) and Escape
    /// are fixed conventions (see `ui_conventions`) that add to this binding. Any gamepad
    /// input placed here is ignored when bindings are compiled (see `CompiledBindings::compile`).
    pub pause: Vec<String>,
    /// Gamepad direction mode (D-pad / Stick / Both). Defaults to `Both`, so existing
    /// configs are unchanged.
    #[serde(default)]
    pub dir_mode: DirMode,
}

impl Default for KeyBindings {
    fn default() -> Self {
        Self {
            // One keyboard key per direction by default (the arrow), no WASD. Players who
            // want WASD rebind. D-pad and left stick cover the gamepad.
            up: vec!["ArrowUp".into(), "PadDPadUp".into(), "PadLeftStickUp".into()],
            down: vec!["ArrowDown".into(), "PadDPadDown".into(), "PadLeftStickDown".into()],
            left: vec!["ArrowLeft".into(), "PadDPadLeft".into(), "PadLeftStickLeft".into()],
            right: vec!["ArrowRight".into(), "PadDPadRight".into(), "PadLeftStickRight".into()],
            swap: vec!["Space".into(), "PadSouth".into()],
            raise: vec!["Enter".into(), "PadWest".into()],
            // Keyboard pause is P. The gamepad pause is Start, a fixed convention, not a binding.
            pause: vec!["KeyP".into()],
            dir_mode: DirMode::Both,
        }
    }
}

impl KeyBindings {
    /// Get the key list for an action.
    pub fn get(&self, action: BindingAction) -> &Vec<String> {
        match action {
            BindingAction::Up => &self.up,
            BindingAction::Down => &self.down,
            BindingAction::Left => &self.left,
            BindingAction::Right => &self.right,
            BindingAction::Swap => &self.swap,
            BindingAction::Raise => &self.raise,
            BindingAction::Pause => &self.pause,
        }
    }

    /// Get a mutable reference to the key list for an action.
    pub fn get_mut(&mut self, action: BindingAction) -> &mut Vec<String> {
        match action {
            BindingAction::Up => &mut self.up,
            BindingAction::Down => &mut self.down,
            BindingAction::Left => &mut self.left,
            BindingAction::Right => &mut self.right,
            BindingAction::Swap => &mut self.swap,
            BindingAction::Raise => &mut self.raise,
            BindingAction::Pause => &mut self.pause,
        }
    }

    /// Assign a key to an action at a given slot. Enforces max slot count.
    /// Returns the (action, slot) that was displaced, if any (conflict resolution).
    pub fn assign(&mut self, action: BindingAction, slot: usize, key_name: String) -> Option<(BindingAction, usize)> {
        // Remove this key from any other action (last assignment wins)
        let mut displaced = None;
        for other in BindingAction::ALL {
            let keys = self.get_mut(other);
            if let Some(pos) = keys.iter().position(|k| k == &key_name) {
                if other != action || pos != slot {
                    displaced = Some((other, pos));
                    keys.remove(pos);
                }
            }
        }
        // Assign to the target action
        let keys = self.get_mut(action);
        let max = action.max_keys();
        if slot < keys.len() {
            keys[slot] = key_name;
        } else if keys.len() < max {
            keys.push(key_name);
        }
        displaced
    }

    /// Normalize: remove duplicates, enforce max slots per action.
    pub fn normalize(&mut self) {
        for action in BindingAction::ALL {
            let max = action.max_keys();
            let keys = self.get_mut(action);
            keys.dedup();
            keys.truncate(max);
        }
    }

    /// Disk boundary: an action none of whose inputs is known (a key renamed by a later
    /// version, a hand edit) takes its default inputs back, so it is never left unplayable.
    /// Unknown names next to a known one are kept as they are.
    fn repair_unknown_inputs(&mut self) {
        let defaults = KeyBindings::default();
        for action in BindingAction::ALL {
            let keys = self.get(action);
            if keys.iter().all(|k| crate::keycodes::name_to_physical_input(k).is_none()) {
                log::warn!("Controls: {action:?} had no known input ({keys:?}), default restored");
                *self.get_mut(action) = defaults.get(action).clone();
            }
        }
    }

    /// Validate: every action must have at least one key.
    /// Returns the first action with no keys, if any.
    pub fn validate(&self) -> Option<BindingAction> {
        for action in BindingAction::ALL {
            if self.get(action).is_empty() {
                return Some(action);
            }
        }
        None
    }
}

/// Former all-caps default names. They are replaced on load by the current default
/// ("Player 1" / "Joueur 1"). A name chosen by the player is never touched.
const LEGACY_DEFAULT_NAMES: [&str; 2] = ["PLAYER 1", "JOUEUR 1"];

/// Default player name, localized (one default per language).
///
/// Resolved at use time: the language is set before the first `UserConfig` is created at boot.
pub fn default_player_name() -> &'static str {
    crate::app::locale::t().default_player_name
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ScoreEntry {
    pub name: String,
    pub score: i32,
}

/// Key identifying a solo record metric.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum RecordKey {
    BestScore,
    LongestGame,
    MostBlocks,
    BestChain,
    BestBlocksPerMin,
    MostSwaps,
}

/// A single record entry: best-ever value for one metric.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RecordEntry {
    pub key: RecordKey,
    pub value: i32,
    pub name: String,
    pub date: String,
}

/// Format of `state.json` written by this version. Increment it when an older version could not
/// read the file faithfully (a new record kind, a field whose meaning changes). An older version
/// that finds a newer format uses the file but never rewrites it.
const STATE_FORMAT_VERSION: u32 = 1;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct UserConfig {
    /// Format of the file (see [`STATE_FORMAT_VERSION`]). Files written before this field
    /// existed have no value and read as 1.
    pub format_version: u32,
    pub player_name: String,
    pub scores: Vec<ScoreEntry>,
    pub records: Vec<RecordEntry>,
    pub sfx_enabled: bool,
    pub music_enabled: bool,
    pub sfx_volume: f32,
    pub music_volume: f32,
    /// Language override. None = auto-detect from system.
    /// Values: "en", "fr".
    pub language: Option<String>,
    /// Fullscreen mode (borderless).
    pub fullscreen: bool,
    /// Key bindings for gameplay actions.
    pub key_bindings: KeyBindings,
    /// Persistent player identity for the network ladder: 16 random bytes (OS CSPRNG) as hex
    /// (32 chars), generated on first load (see `load`). The name (`player_name`) is only an
    /// editable display label. This id is what follows the player in the network handshake and
    /// the match log. It is never shown in the UI.
    pub player_uuid: String,

    // ── Update check. These fields do nothing while `PUBLIC_PROJECT` is `None` and/or the
    // `update-check` feature is off. ──
    /// Update check toggle (on by default: a daily check on the direct channel). No checkbox
    /// is shown while the profile or identity does not allow a fetch. Editable in `state.json`.
    pub update_checks_enabled: bool,
    /// Time (Unix seconds) of the last attempt, successful or not, so an offline start does
    /// not retry on every launch. `0` means never tried.
    pub last_update_check: u64,
    /// Update version the player chose to ignore. Empty means none.
    pub ignored_update_version: String,
    /// Time (Unix seconds) of the last successful read. With `last_update_check`, it gives the
    /// status line of the Info page. `0` means never.
    pub last_update_success: u64,
    /// Latest remote version seen that is newer than the installed one. Empty means none.
    /// A notification found yesterday survives a restart without a connection. It is cleared
    /// once it no longer exceeds the installed version.
    pub available_update_version: String,

    // ── Temporary overrides (from `--set config:...`) ──
    // Runtime-only, never serialized (`serde(skip)`). An override is applied directly to the
    // typed fields above, so every reader sees the effective value. `base_snapshot` keeps the
    // persisted value so that `save()` drops the overrides. Overrides are never persisted.
    /// Snapshot of the fields before any override (taken at the first one). `None` means no override.
    #[serde(skip)]
    base_snapshot: Option<Box<UserConfig>>,
    /// Keys overridden this session (to restore the base on `save()`).
    #[serde(skip)]
    overridden: Vec<ConfigKey>,
    /// The file on disk has a newer format than this version knows: it is used, never rewritten.
    #[serde(skip)]
    read_only: bool,
}

/// A publicly overridable option (the `config:` namespace of `--set`).
///
/// The public id is decoupled from the internal field. Only player-facing options are listed:
/// `player_uuid` (persisted without a UI) is not.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigKey {
    PlayerName,
    SfxEnabled,
    MusicEnabled,
    SfxVolume,
    MusicVolume,
    Language,
    Fullscreen,
}

impl ConfigKey {
    /// Public ids accepted by `parse`, for error messages and help.
    pub const PUBLIC_IDS: &'static [&'static str] = &[
        "player.name", "audio.sfx_enabled", "audio.music_enabled", "audio.sfx_volume",
        "audio.music_volume", "i18n.language", "video.fullscreen",
    ];

    /// Parses a public `config:` id (without the `config:` prefix), e.g. `player.name`.
    pub fn parse(id: &str) -> Option<ConfigKey> {
        Some(match id {
            "player.name" => ConfigKey::PlayerName,
            "audio.sfx_enabled" => ConfigKey::SfxEnabled,
            "audio.music_enabled" => ConfigKey::MusicEnabled,
            "audio.sfx_volume" => ConfigKey::SfxVolume,
            "audio.music_volume" => ConfigKey::MusicVolume,
            "i18n.language" => ConfigKey::Language,
            "video.fullscreen" => ConfigKey::Fullscreen,
            _ => return None,
        })
    }

    /// Validates a string value and applies it to the typed field. Returns `Err` with a message if invalid.
    fn set(&self, cfg: &mut UserConfig, v: &str) -> Result<(), String> {
        // One boolean grammar, shared with the session params.
        let parse_bool = crate::platform::session_param::parse_bool;
        let parse_vol = |v: &str| -> Result<f32, String> {
            let f: f32 = v.parse().map_err(|_| format!("expected a number (got \"{v}\")"))?;
            Ok(f.clamp(0.0, 1.0))
        };
        match self {
            ConfigKey::PlayerName => {
                // CLI boundary: same normalization as UI input.
                let name = crate::app::onscreen_keyboard::CharSet::sanitize_player_name(v);
                if name.is_empty() { return Err("empty name".into()); }
                cfg.player_name = name;
            }
            ConfigKey::SfxEnabled => cfg.sfx_enabled = parse_bool(v)?,
            ConfigKey::MusicEnabled => cfg.music_enabled = parse_bool(v)?,
            ConfigKey::SfxVolume => cfg.sfx_volume = parse_vol(v)?,
            ConfigKey::MusicVolume => cfg.music_volume = parse_vol(v)?,
            ConfigKey::Language => {
                cfg.language = match v {
                    "auto" => None,
                    _ if crate::app::locale::Language::from_code(v).is_some() => Some(v.to_string()),
                    _ => {
                        let codes: Vec<&str> = crate::app::locale::Language::ALL.iter().map(|l| l.code()).collect();
                        return Err(format!("unknown language \"{v}\" (auto|{})", codes.join("|")));
                    }
                };
            }
            ConfigKey::Fullscreen => cfg.fullscreen = parse_bool(v)?,
        }
        Ok(())
    }

    /// Copies this field from `src` to `dst` (restores the base on `save()`).
    fn copy_field(&self, dst: &mut UserConfig, src: &UserConfig) {
        match self {
            ConfigKey::PlayerName => dst.player_name = src.player_name.clone(),
            ConfigKey::SfxEnabled => dst.sfx_enabled = src.sfx_enabled,
            ConfigKey::MusicEnabled => dst.music_enabled = src.music_enabled,
            ConfigKey::SfxVolume => dst.sfx_volume = src.sfx_volume,
            ConfigKey::MusicVolume => dst.music_volume = src.music_volume,
            ConfigKey::Language => dst.language = src.language.clone(),
            ConfigKey::Fullscreen => dst.fullscreen = src.fullscreen,
        }
    }
}

/// Generates a player identity: 16 bytes from the OS CSPRNG, hex-encoded (32 chars).
fn generate_player_uuid() -> String {
    let mut bytes = [0u8; 16];
    if let Err(e) = getrandom::getrandom(&mut bytes) {
        // OS randomness is unavailable (should not happen on desktop). Fall back to the clock:
        // a weaker identity, but it works and is persisted once.
        log::warn!("getrandom failed ({e}), falling back to clock-derived uuid");
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        bytes[..16].copy_from_slice(&nanos.to_le_bytes());
    }
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Parses a hex `player_uuid` (32 chars) into 16 bytes. Returns `None` if malformed.
pub fn parse_uuid_hex(s: &str) -> Option<[u8; 16]> {
    if s.len() != 32 || !s.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let mut out = [0u8; 16];
    for (i, chunk) in s.as_bytes().chunks(2).enumerate() {
        let hi = (chunk[0] as char).to_digit(16)?;
        let lo = (chunk[1] as char).to_digit(16)?;
        out[i] = ((hi << 4) | lo) as u8;
    }
    Some(out)
}

/// Whether a score entry is a default one (from the original table, never set by the player).
///
/// Detected by identical (name, score). The shipped table is fixed. A collision (a player named
/// "Cup" scoring exactly 480) is negligible and only affects display.
pub fn is_default_score(entry: &ScoreEntry) -> bool {
    default_scores()
        .iter()
        .any(|d| d.name == entry.name && d.score == entry.score)
}

/// Default high score table (30 entries, scores 600 down to 20).
fn default_scores() -> Vec<ScoreEntry> {
    const NAMES: [&str; 30] = [
        "AluminumAngel", "***********", "Fish Person", "SidekickSeven",
        "5Moon Mellon", "Mr. Twinkles", "Cup", "Shadow",
        "enoyaw", "[", "Book", "Plum",
        "Stankpants", "Food", "Mrs. Hidges", "Banana",
        "1337 h/|x0R", "Ladle Head", "Apple", "ScrollLock",
        "]", "Tater-Tot", "Orange", "Sen-TR++",
        "Happy Puppy", "Mr. Muddle", "Tony the Tortoise", "Rap Master B",
        "ABC", "Tod Tortellini",
    ];
    const TOP_SCORE: i32 = 600;
    const LEN: i32 = 30;
    NAMES.iter().enumerate().map(|(i, &name)| {
        ScoreEntry {
            name: name.to_string(),
            score: ((LEN - i as i32) * TOP_SCORE) / LEN,
        }
    }).collect()
}

impl Default for UserConfig {
    fn default() -> Self {
        Self {
            format_version: STATE_FORMAT_VERSION,
            player_name: default_player_name().to_string(),
            scores: default_scores(),
            records: Vec::new(),
            sfx_enabled: true,
            music_enabled: true,
            sfx_volume: 0.5,
            music_volume: 0.3,
            language: None,
            fullscreen: false,
            key_bindings: KeyBindings::default(),
            // Empty by default. The uuid is generated in `load`, not here: a random default
            // would give a different uuid on every load until something is saved (serde calls
            // `default()` for any missing field).
            player_uuid: String::new(),
            update_checks_enabled: true,
            last_update_check: 0,
            ignored_update_version: String::new(),
            last_update_success: 0,
            available_update_version: String::new(),
            base_snapshot: None,
            overridden: Vec::new(),
            read_only: false,
        }
    }
}

/// Result of reading one state file.
enum StateRead {
    /// No file: first launch.
    Missing,
    /// Usable content (possibly salvaged field by field).
    Read(Box<UserConfig>),
    /// Unreadable file (I/O error other than "not found") or not a JSON object.
    Unusable(String),
}

/// Config directory, following each OS convention: macOS
/// `~/Library/Application Support/crack-attack`, Windows `%APPDATA%\crack-attack`,
/// Linux `$XDG_CONFIG_HOME/crack-attack` (or `~/.config/...`). Shared with the ladder log
/// (`platform::ladder`).
///
/// Config and data are not split into separate directories, because `state.json` mixes
/// options and scores.
pub(crate) fn config_dir() -> Option<PathBuf> {
    // Tests never touch the player's real files: code under test that saves through the normal
    // path (session flows) gets "no config directory" and stays in memory. Tests of the files
    // themselves use explicit temp paths.
    if cfg!(test) {
        return None;
    }
    dirs::config_dir()
        .map(|d| d.join("crack-attack"))
        // Unlikely fallback when no config dir is known: `~/.crack-attack`.
        .or_else(|| dirs::home_dir().map(|h| h.join(".crack-attack")))
}

/// Inter-process lock on the config directory.
///
/// It serializes read-modify-write transactions (`state.json`, `ladder.json`) between
/// instances. Running several instances is supported (local network tests), and without a
/// lock writes would be lost silently.
///
/// It is an advisory OS lock (`fd-lock`: flock/LockFileEx), released automatically when the
/// process dies, so no stale lock is left behind. The wait is bounded (about 500 ms). On
/// failure the write is skipped with a warning. The game is never blocked.
pub(crate) fn with_config_lock<T>(dir: &std::path::Path, f: impl FnOnce() -> T) -> Option<T> {
    let path = dir.join(".lock");
    let file = match std::fs::OpenOptions::new().create(true).write(true).open(&path) {
        Ok(f) => f,
        Err(e) => {
            log::warn!("Lock {} unavailable: {e}, write abandoned", path.display());
            return None;
        }
    };
    let mut lock = fd_lock::RwLock::new(file);
    for _ in 0..20 {
        if let Ok(_guard) = lock.try_write() {
            return Some(f());
        }
        std::thread::sleep(std::time::Duration::from_millis(25));
    }
    log::warn!("Lock {} still busy after 500 ms, write abandoned", path.display());
    None
}

/// Writes `contents` to `path` atomically and durably: a temp file next to it (named with the
/// PID, so two instances never share it), flushed to disk, then renamed over `path`. A crash
/// leaves either the old file or the new one, never a truncated mix. Shared by `state.json`
/// and `ladder.json`.
pub(crate) fn write_atomic(path: &std::path::Path, contents: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let tmp = path.with_file_name(format!("{name}.tmp.{}", std::process::id()));
    let result = (|| {
        let mut file = std::fs::File::create(&tmp)?;
        file.write_all(contents)?;
        file.sync_all()?;
        std::fs::rename(&tmp, path)?;
        // Make the rename itself durable (not possible on Windows, where it is not needed).
        #[cfg(unix)]
        if let Some(dir) = path.parent() {
            std::fs::File::open(dir)?.sync_all()?;
        }
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

fn state_path() -> Option<PathBuf> {
    config_dir().map(|d| d.join("state.json"))
}

/// Whether `text` declares a newer format than this version writes.
fn is_newer_format(text: &str) -> bool {
    serde_json::from_str::<serde_json::Value>(text).ok()
        .and_then(|v| v.get("format_version").and_then(|f| f.as_u64()))
        .is_some_and(|f| f > STATE_FORMAT_VERSION as u64)
}

/// Keeps a timestamped copy of a faulty state file (`state.json.corrupt-<ts>`) for manual recovery.
fn set_aside(path: &std::path::Path) {
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs()).unwrap_or(0);
    let aside = path.with_extension(format!("json.corrupt-{ts}"));
    match std::fs::copy(path, &aside) {
        Ok(_) => log::warn!("Original state file kept at {}", aside.display()),
        Err(e) => log::warn!("Failed to set aside the state file: {}", e),
    }
}

/// Set by the first `load()` that finds no state file.
static FIRST_LAUNCH: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Whether this session started without a state file (first launch). Distinguishes a platform
/// default (fullscreen on Steam Deck, default name in the player's language) from a saved
/// choice. Valid once `UserConfig::load()` has run; the file it creates does not change it.
pub fn is_first_launch() -> bool {
    FIRST_LAUNCH.load(std::sync::atomic::Ordering::Relaxed)
}

impl UserConfig {
    /// Load state from `state.json` in the config directory.
    ///
    /// An unusable file (unreadable, or not JSON) is set aside and the backup `state.json.bak`
    /// is used instead; defaults come last. A file of a newer format is used but never
    /// rewritten. Never fails.
    pub fn load() -> Self {
        match state_path() {
            Some(path) => Self::load_from(&path),
            None => Self::default(),
        }
    }

    /// [`load`](Self::load) from an explicit path (tests).
    fn load_from(path: &std::path::Path) -> Self {
        let path = path.to_path_buf();
        let mut cfg = match Self::read_file(&path) {
            StateRead::Missing => {
                log::info!("No state file at {}, using defaults", path.display());
                FIRST_LAUNCH.store(true, std::sync::atomic::Ordering::Relaxed);
                Self::default()
            }
            StateRead::Read(cfg) => {
                log::info!("Loaded state from {}", path.display());
                *cfg
            }
            StateRead::Unusable(reason) => {
                log::warn!("State file {} unusable ({reason})", path.display());
                set_aside(&path);
                let bak = path.with_extension("json.bak");
                match Self::read_file(&bak) {
                    StateRead::Read(cfg) => {
                        log::warn!("State restored from {}", bak.display());
                        *cfg
                    }
                    _ => {
                        log::warn!("No usable state backup, using defaults");
                        Self::default()
                    }
                }
            }
        };
        if cfg.read_only {
            log::warn!(
                "state.json has format {} (this version knows {STATE_FORMAT_VERSION}): it is used but will not be modified",
                cfg.format_version
            );
        }
        // Player identity: generated on the first load (missing or malformed) and saved
        // immediately. It is never regenerated afterwards, because it follows the player. If
        // another instance saved an identity first, the one on disk wins.
        if parse_uuid_hex(&cfg.player_uuid).is_none() {
            cfg.player_uuid = generate_player_uuid();
            log::info!("Generated player uuid");
            cfg.save_to(&path);
            if let StateRead::Read(disk) = Self::read_file(&path) {
                if parse_uuid_hex(&disk.player_uuid).is_some() {
                    cfg.player_uuid = disk.player_uuid;
                }
            }
        }
        cfg
    }

    /// Reads one state file. A partially invalid file is salvaged field by field and a copy is
    /// set aside; a file of a newer format is marked read-only.
    fn read_file(path: &std::path::Path) -> StateRead {
        let text = match std::fs::read_to_string(path) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return StateRead::Missing,
            Err(e) => return StateRead::Unusable(e.to_string()),
        };
        if !matches!(serde_json::from_str(&text), Ok(serde_json::Value::Object(_))) {
            return StateRead::Unusable("not a JSON object".into());
        }
        let newer = is_newer_format(&text);
        let (mut cfg, salvaged) = Self::from_json_report(&text);
        if salvaged && !newer {
            // Keep a copy of the faulty file: the salvage replaced its invalid fields.
            set_aside(path);
        }
        cfg.read_only = newer;
        StateRead::Read(Box::new(cfg))
    }

    /// The player identity as bytes (for the network handshake). All zeros if malformed,
    /// which cannot happen after `load()` (it guarantees a valid uuid).
    pub fn player_uuid_bytes(&self) -> [u8; 16] {
        parse_uuid_hex(&self.player_uuid).unwrap_or([0u8; 16])
    }

    /// Save state to `state.json` in the config directory. Returns `false` if nothing was
    /// written (the reason is logged).
    ///
    /// Reload-merge-write under the inter-process lock: reloads the current file and merges
    /// domain-specific fields, so two instances do not overwrite each other. The previous file
    /// becomes `state.json.bak`, but only if it was valid: a corrupt file never replaces a good
    /// backup. A file of a newer format is never rewritten.
    pub fn save(&self) -> bool {
        match state_path() {
            Some(path) => self.save_to(&path),
            None => {
                log::warn!("Cannot determine home directory, state not saved");
                false
            }
        }
    }

    /// [`save`](Self::save) to an explicit path (tests). The lock lives in the same directory.
    fn save_to(&self, path: &std::path::Path) -> bool {
        if self.read_only {
            log::warn!("State not saved: state.json belongs to a newer version of the game");
            return false;
        }
        let Some(dir) = path.parent() else { return false; };
        if let Err(e) = std::fs::create_dir_all(dir) {
            log::warn!("Failed to create state dir {}: {}", dir.display(), e);
            return false;
        }

        let done = with_config_lock(dir, || {
            // Overridden fields go back to their base (pre-override) value, so a CLI override
            // never leaks into the saved file. See `persistable_snapshot`.
            let mut merged = self.persistable_snapshot();
            merged.format_version = STATE_FORMAT_VERSION;

            if let Ok(text) = std::fs::read_to_string(path) {
                if is_newer_format(&text) {
                    log::warn!("State not saved: a newer version of the game wrote state.json");
                    return false;
                }
                merged.merge_from_disk(&Self::from_json(&text));
                if serde_json::from_str::<UserConfig>(&text).is_ok() {
                    if let Err(e) = write_atomic(&path.with_extension("json.bak"), text.as_bytes()) {
                        log::warn!("Failed to write state backup: {}", e);
                    }
                }
            }

            let json = match serde_json::to_string_pretty(&merged) {
                Ok(json) => json,
                Err(e) => {
                    log::warn!("Failed to serialize state: {}", e);
                    return false;
                }
            };
            match write_atomic(path, json.as_bytes()) {
                Ok(()) => {
                    log::info!("Saved state to {}", path.display());
                    true
                }
                Err(e) => {
                    log::warn!("Failed to write state file: {}", e);
                    false
                }
            }
        });
        done.unwrap_or_else(|| {
            log::warn!("State not saved (config lock unavailable)");
            false
        })
    }

    /// A persistable clone: overridden fields (`--set config:`) go back to their base
    /// (pre-override) value, and the override bookkeeping is cleared. This is what gets written,
    /// never the overrides. Testable without a disk.
    pub fn persistable_snapshot(&self) -> UserConfig {
        let mut out = self.clone();
        if let Some(ref base) = self.base_snapshot {
            for key in &self.overridden {
                key.copy_field(&mut out, base);
            }
        }
        out.base_snapshot = None;
        out.overridden.clear();
        out
    }

    /// Applies a temporary `config:` override. The value is written to the typed field (so
    /// every reader sees it) and the pre-override base is remembered so that `save()` drops it.
    /// Never persisted. Returns `Err` if the value is invalid.
    pub fn apply_override(&mut self, key: ConfigKey, value: &str) -> Result<(), String> {
        // The base snapshot is taken once, before the first override, and then frozen.
        if self.base_snapshot.is_none() {
            let mut base = self.clone();
            base.base_snapshot = None; // no recursion in the snapshot
            self.base_snapshot = Some(Box::new(base));
        }
        key.set(self, value)?;
        if !self.overridden.contains(&key) {
            self.overridden.push(key);
        }
        Ok(())
    }

    /// Applies the `--set config:...` overrides directly to the typed fields, before any reader
    /// (nameplate, log, session) sees them, so the effective value is seen everywhere.
    /// `save()` and `persistable_snapshot()` drop them. The `session:` and `profile:`
    /// namespaces (boot params) are consumed elsewhere (`cli::apply` via `session_param`) and
    /// silently skipped here. Only a truly unknown namespace is reported.
    pub fn apply_cli_set_overrides(&mut self, specs: &[String]) {
        for spec in specs {
            let Some((id, value)) = spec.split_once('=') else {
                log::warn!("CLI: --set \"{spec}\" ignored (expected <id>=<value>)");
                continue;
            };
            match id.split_once(':') {
                Some(("config", key_id)) => match ConfigKey::parse(key_id) {
                    Some(key) => match self.apply_override(key, value) {
                        // Debug level: this runs on every `UserConfig::load()`. The single
                        // INFO message is emitted by `cli::apply()`.
                        Ok(()) => log::debug!("CLI: override config:{key_id}={value}"),
                        Err(e) => log::warn!("CLI: --set config:{key_id} ignored ({e})"),
                    },
                    None => log::warn!("CLI: --set config:{key_id} unknown"),
                },
                Some(("session", _)) | Some(("profile", _)) => {} // boot params, handled by cli::apply
                _ => log::warn!("CLI: --set \"{id}\": unknown namespace (config:/session:/profile:)"),
            }
        }
    }

    /// Merge domain-specific fields from the disk version to avoid
    /// stale overwrites when multiple instances run concurrently.
    ///
    /// - records: keep the best value per RecordKey (max wins)
    /// - scores: union, re-sort, truncate to MAX_SCORES
    /// - options (name, audio, language, etc.): last-writer-wins (self wins)
    fn merge_from_disk(&mut self, disk: &Self) {
        // Records: for each key on disk, keep it if better than ours
        for disk_rec in &disk.records {
            let dominated = self.records.iter().any(|r|
                r.key == disk_rec.key && r.value >= disk_rec.value
            );
            if !dominated {
                // Disk has a better record — adopt it
                self.records.retain(|r| r.key != disk_rec.key);
                self.records.push(disk_rec.clone());
            }
        }

        // Scores: merge, sort descending, truncate
        for disk_score in &disk.scores {
            let already = self.scores.iter().any(|s|
                s.name == disk_score.name && s.score == disk_score.score
            );
            if !already {
                self.scores.push(disk_score.clone());
            }
        }
        self.scores.sort_by(|a, b| b.score.cmp(&a.score));
        self.scores.dedup_by(|a, b| a.name == b.name && a.score == b.score);
        self.scores.truncate(MAX_SCORES);

        // Player identity: a valid identity already on disk always wins. It never changes once
        // established, so this only matters when two instances generated one at the same time.
        if parse_uuid_hex(&disk.player_uuid).is_some() {
            self.player_uuid = disk.player_uuid.clone();
        }
    }

    /// Parse from JSON string. Falls back to defaults for missing/invalid fields.
    fn from_json(text: &str) -> Self {
        Self::from_json_report(text).0
    }

    /// Disk boundary for the name, applied by every read path (including `load()`, which goes
    /// through [`from_json_report`]).
    fn normalize_player_name(&mut self) {
        // A name saved before the charset was narrowed ('_', apostrophe...) is normalized on
        // read. Otherwise the bitmap nameplate would silently drop characters.
        self.player_name =
            crate::app::onscreen_keyboard::CharSet::sanitize_player_name(&self.player_name);
        // Empty, or a former all-caps default: use the current default.
        if self.player_name.is_empty() || LEGACY_DEFAULT_NAMES.contains(&self.player_name.as_str()) {
            self.player_name = default_player_name().to_string();
        }
    }

    /// Like [`from_json`], and reports whether a salvage took place (partially invalid file).
    /// `load()` uses it to set the faulty file aside.
    ///
    /// serde fails as a whole on a single badly typed field. Without salvage, one corrupt field
    /// (manual edit, truncation) would reset everything to defaults: scores, records, bindings
    /// and above all `player_uuid`, the ladder identity, which would be regenerated silently.
    /// Here each valid field is kept one by one, and only invalid fields fall back to their
    /// default.
    fn from_json_report(text: &str) -> (Self, bool) {
        let (mut config, salvaged) = match serde_json::from_str::<UserConfig>(text) {
            Ok(mut config) => {
                // Fill empty scores with defaults
                if config.scores.is_empty() {
                    config.scores = default_scores();
                }
                (config, false)
            }
            Err(e) => {
                log::warn!("Failed to parse state ({}), salvaging valid fields", e);
                (Self::salvage_fields(text), true)
            }
        };
        config.normalize_player_name();
        config.key_bindings.repair_unknown_inputs();
        (config, salvaged)
    }

    /// Field-by-field salvage. Starting from the serialized defaults, each field found on disk
    /// is adopted only if the whole document still deserializes with it. It is generic (no field
    /// list to maintain). The `serde(skip)` override fields do not go through the JSON and stay
    /// empty.
    fn salvage_fields(text: &str) -> Self {
        let mut cfg = Self::default();
        if cfg.scores.is_empty() {
            cfg.scores = default_scores();
        }
        let Ok(serde_json::Value::Object(disk)) = serde_json::from_str(text) else {
            return cfg; // not even a JSON object: full defaults
        };
        let Ok(mut good) = serde_json::to_value(&cfg) else { return cfg; };
        let mut salvaged: Vec<&str> = Vec::new();
        for (key, value) in &disk {
            let mut candidate = good.clone();
            candidate[key.as_str()] = value.clone();
            if let Ok(parsed) = serde_json::from_value::<UserConfig>(candidate.clone()) {
                good = candidate;
                cfg = parsed;
                salvaged.push(key);
            } else {
                log::warn!("State field '{}' is invalid, using its default", key);
            }
        }
        log::info!("Salvaged {}/{} state fields", salvaged.len(), disk.len());
        if cfg.scores.is_empty() {
            cfg.scores = default_scores();
        }
        cfg
    }

    /// Insert a score into the table. Returns the rank (0-based) if it made it.
    pub fn submit_score(&mut self, name: &str, score: i32) -> Option<usize> {
        let rank = self.scores.iter().position(|e| score > e.score)
            .unwrap_or(self.scores.len());

        if rank >= MAX_SCORES {
            return None;
        }

        self.scores.insert(rank, ScoreEntry {
            name: name.to_string(),
            score,
        });
        self.scores.truncate(MAX_SCORES);
        Some(rank)
    }

    /// The highest score currently present in the high-score table.
    pub fn highest_score(&self) -> i32 {
        self.scores.iter().map(|e| e.score).max().unwrap_or(0)
    }

    /// The minimum score needed to enter the high-score table.
    /// This is the current cutoff score, not the best score.
    pub fn ranking_entry_score(&self) -> i32 {
        self.scores.iter().map(|e| e.score).min().unwrap_or(0)
    }

    /// Submit a record. Returns true if the record was beaten (strictly superior).
    /// On equality, the existing record is kept.
    pub fn submit_record(&mut self, key: RecordKey, value: i32, name: &str) -> bool {
        if let Some(existing) = self.records.iter().find(|r| r.key == key) {
            if value <= existing.value {
                return false;
            }
        }
        self.records.retain(|r| r.key != key);
        self.records.push(RecordEntry {
            key,
            value,
            name: name.to_string(),
            date: today_string(),
        });
        true
    }

}

/// Today's date as "YYYY-MM-DD".
fn today_string() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    date_string(secs)
}

/// Unix seconds to "YYYY-MM-DD" (UTC).
pub(crate) fn date_string(secs: u64) -> String {
    // Days since epoch → year/month/day (simplified civil calendar)
    let days = (secs / 86400) as i64;
    let (y, m, d) = civil_from_days(days);
    format!("{:04}-{:02}-{:02}", y, m, d)
}

/// Convert days since Unix epoch to (year, month, day).
/// Algorithm from Howard Hinnant (public domain).
fn civil_from_days(days: i64) -> (i32, u32, u32) {
    let z = days + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = (z - era * 146097) as u32;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (y as i32, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;

    // Two real processes: threads would prove nothing, because the lock is inter-process. The
    // parent spawns 2 children (this same test binary, filtered on the child test and armed by
    // an environment variable). Each does 200 read-modify-write cycles on a counter under the
    // lock, and no update may be lost.
    #[test]
    fn config_lock_serializes_two_processes() {
        let guard = crate::platform::test_support::TestDir::new("lock");
        let base = guard.path().to_path_buf();
        std::fs::create_dir_all(&base).unwrap();
        std::fs::write(base.join("counter.txt"), "0").unwrap();
        let exe = std::env::current_exe().unwrap();
        // Children are killed and reaped on drop, so a failed assertion cannot leave one
        // writing into the directory while it is being removed.
        struct Reap(std::process::Child);
        impl Drop for Reap {
            fn drop(&mut self) {
                let _ = self.0.kill();
                let _ = self.0.wait();
            }
        }
        let spawn = || {
            Reap(
                std::process::Command::new(&exe)
                    .args(["platform::persist::tests::lock_child_increments", "--exact", "--nocapture"])
                    .env("CA_LOCK_TEST_DIR", &base)
                    .spawn()
                    .unwrap(),
            )
        };
        let mut a = spawn();
        let mut b = spawn();
        assert!(a.0.wait().unwrap().success());
        assert!(b.0.wait().unwrap().success());
        let v: u64 = std::fs::read_to_string(base.join("counter.txt"))
            .unwrap().trim().parse().unwrap();
        assert_eq!(v, 400, "lost writes: the lock does not serialize");
    }

    /// Child of the test above. It does nothing without the environment variable, so it
    /// passes as an empty test in a normal run of the suite.
    #[test]
    fn lock_child_increments() {
        let Some(dir) = std::env::var_os("CA_LOCK_TEST_DIR") else { return };
        let dir = std::path::PathBuf::from(dir);
        for _ in 0..200 {
            // `with_config_lock` gives up after 500 ms by design; on a loaded CI runner the
            // other process can hold the lock longer than that, so the test retries (a few
            // seconds in all) instead of failing on scheduling noise.
            let ok = (0..20).find_map(|_| {
                with_config_lock(&dir, || {
                    let p = dir.join("counter.txt");
                    let v: u64 = std::fs::read_to_string(&p).unwrap().trim().parse().unwrap();
                    std::fs::write(&p, format!("{}", v + 1)).unwrap();
                })
            });
            assert!(ok.is_some(), "lock never acquired (timeout)");
        }
    }


    /// The generated uuid is valid (32 hex chars, 16 bytes) and two draws differ.
    #[test]
    fn player_uuid_generate_and_parse() {
        let a = generate_player_uuid();
        let b = generate_player_uuid();
        assert_eq!(a.len(), 32);
        assert!(parse_uuid_hex(&a).is_some());
        assert_ne!(a, b, "two generations must differ");
        // round trip: bytes to hex to bytes
        let bytes = parse_uuid_hex(&a).unwrap();
        let rehex: String = bytes.iter().map(|x| format!("{x:02x}")).collect();
        assert_eq!(rehex, a);
    }

    /// A malformed uuid (empty, too short, non-hex) gives `None`. A `state.json` without the
    /// field deserializes to an empty string (`serde(default)`), which means "to generate".
    #[test]
    fn player_uuid_malformed_rejected() {
        assert!(parse_uuid_hex("").is_none());
        assert!(parse_uuid_hex("abcd").is_none());
        assert!(parse_uuid_hex(&"g".repeat(32)).is_none());
        let legacy: UserConfig = serde_json::from_str(r#"{ "player_name": "X" }"#).unwrap();
        assert!(legacy.player_uuid.is_empty());
        assert_eq!(legacy.player_uuid_bytes(), [0u8; 16]);
    }

    /// `merge_from_disk` never loses a valid uuid present on disk.
    #[test]
    fn player_uuid_survives_merge() {
        let mut mem = UserConfig::default(); // empty uuid
        let mut disk = UserConfig::default();
        disk.player_uuid = generate_player_uuid();
        mem.merge_from_disk(&disk);
        assert_eq!(mem.player_uuid, disk.player_uuid);
        // Two instances generated an identity at the same time: the one already on disk wins.
        let mut mem2 = UserConfig::default();
        mem2.player_uuid = generate_player_uuid();
        mem2.merge_from_disk(&disk);
        assert_eq!(mem2.player_uuid, disk.player_uuid);
    }

    /// A `state.json` where the gameplay binding is called `confirm` deserializes into
    /// `raise` (via `#[serde(alias = "confirm")]`).
    #[test]
    fn keybindings_migrates_confirm_to_raise() {
        let legacy = r#"{ "confirm": ["KeyR", "PadNorth"] }"#;
        let kb: KeyBindings = serde_json::from_str(legacy).unwrap();
        assert_eq!(kb.raise, vec!["KeyR".to_string(), "PadNorth".to_string()]);
        // The other fields take their defaults.
        assert_eq!(kb.swap, KeyBindings::default().swap);
    }

    /// A `state.json` containing a `cancel` binding (not a config field, since cancel is a
    /// fixed convention) deserializes without error. serde ignores the unknown field.
    #[test]
    fn keybindings_drops_legacy_cancel_field() {
        let legacy = r#"{ "cancel": ["Escape", "PadEast"], "pause": ["KeyP"] }"#;
        let kb: KeyBindings = serde_json::from_str(legacy).unwrap();
        assert_eq!(kb.pause, vec!["KeyP".to_string()]);
        // The other fields take their defaults.
        assert_eq!(kb.swap, KeyBindings::default().swap);
    }

    fn empty_config() -> UserConfig {
        UserConfig {
            format_version: STATE_FORMAT_VERSION,
            player_name: default_player_name().to_string(),
            scores: Vec::new(),
            records: Vec::new(),
            sfx_enabled: true,
            music_enabled: false,
            sfx_volume: 0.7,
            music_volume: 0.5,
            language: None,
            fullscreen: false,
            key_bindings: KeyBindings::default(),
            player_uuid: String::new(),
            update_checks_enabled: true,
            last_update_check: 0,
            ignored_update_version: String::new(),
            last_update_success: 0,
            available_update_version: String::new(),
            base_snapshot: None,
            overridden: Vec::new(),
            read_only: false,
        }
    }

    #[test]
    fn round_trip_default() {
        let config = UserConfig::default();
        let json = serde_json::to_string_pretty(&config).unwrap();
        let parsed: UserConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.player_name, default_player_name());
        assert_eq!(parsed.scores.len(), 30);
        assert_eq!(parsed.sfx_enabled, true);
        assert_eq!(parsed.music_enabled, true);
    }

    #[test]
    fn round_trip_with_scores() {
        let config = UserConfig {
            format_version: STATE_FORMAT_VERSION,
            player_name: "PLAYER".to_string(),
            scores: vec![
                ScoreEntry { name: "PLAYER".to_string(), score: 15000 },
                ScoreEntry { name: "SIGMA".to_string(), score: 12000 },
            ],
            records: Vec::new(),
            sfx_enabled: true,
            music_enabled: false,
            sfx_volume: 0.7,
            music_volume: 0.5,
            language: None,
            fullscreen: false,
            key_bindings: KeyBindings::default(),
            player_uuid: String::new(),
            update_checks_enabled: true,
            last_update_check: 0,
            ignored_update_version: String::new(),
            last_update_success: 0,
            available_update_version: String::new(),
            base_snapshot: None,
            overridden: Vec::new(),
            read_only: false,
        };
        let json = serde_json::to_string_pretty(&config).unwrap();
        let parsed: UserConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.player_name, "PLAYER");
        assert_eq!(parsed.scores.len(), 2);
        assert_eq!(parsed.scores[0].name, "PLAYER");
        assert_eq!(parsed.scores[0].score, 15000);
        assert_eq!(parsed.scores[1].score, 12000);
    }

    #[test]
    fn round_trip_special_chars() {
        // The ] name that broke the artisanal parser now works
        let config = UserConfig {
            format_version: STATE_FORMAT_VERSION,
            player_name: "A\"B".to_string(),
            scores: vec![
                ScoreEntry { name: "]".to_string(), score: 100 },
                ScoreEntry { name: "[".to_string(), score: 50 },
            ],
            records: Vec::new(),
            sfx_enabled: true,
            music_enabled: false,
            sfx_volume: 0.7,
            music_volume: 0.5,
            language: None,
            fullscreen: false,
            key_bindings: KeyBindings::default(),
            player_uuid: String::new(),
            update_checks_enabled: true,
            last_update_check: 0,
            ignored_update_version: String::new(),
            last_update_success: 0,
            available_update_version: String::new(),
            base_snapshot: None,
            overridden: Vec::new(),
            read_only: false,
        };
        let json = serde_json::to_string_pretty(&config).unwrap();
        let parsed: UserConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.player_name, "A\"B");
        assert_eq!(parsed.scores.len(), 2);
        assert_eq!(parsed.scores[0].name, "]");
        assert_eq!(parsed.scores[1].name, "[");
    }

    #[test]
    fn missing_fields_use_defaults() {
        let json = r#"{ "player_name": "TEST" }"#;
        let parsed = UserConfig::from_json(json);
        assert_eq!(parsed.player_name, "TEST");
        assert_eq!(parsed.sfx_enabled, true); // default
        assert_eq!(parsed.music_enabled, true); // default
        assert_eq!(parsed.scores.len(), 30); // default_scores filled
    }

    /// A former all-caps default becomes the current default. A chosen name stays intact,
    /// even if it looks similar ("Player 1" typed by hand, "PLAYER 12").
    #[test]
    fn legacy_uppercase_default_name_migrated() {
        for old in ["PLAYER 1", "JOUEUR 1"] {
            let parsed = UserConfig::from_json(&format!(r#"{{ "player_name": "{old}" }}"#));
            assert_eq!(parsed.player_name, default_player_name(), "{old}");
        }
        for kept in ["PLAYER 12", "Bob"] {
            let parsed = UserConfig::from_json(&format!(r#"{{ "player_name": "{kept}" }}"#));
            assert_eq!(parsed.player_name, kept);
        }
        assert_ne!(default_player_name(), default_player_name().to_uppercase());
        // The real load (`load()`) goes through `from_json_report`: same normalization.
        let (parsed, _) = UserConfig::from_json_report(r#"{ "player_name": "PLAYER 1" }"#);
        assert_eq!(parsed.player_name, default_player_name());
    }

    #[test]
    fn invalid_json_returns_default() {
        let parsed = UserConfig::from_json("not json at all {{{");
        assert_eq!(parsed.player_name, default_player_name());
        assert_eq!(parsed.scores.len(), 30);
    }

    /// One badly typed field must not cost the whole config, and above all not `player_uuid`,
    /// the ladder identity.
    #[test]
    fn salvage_preserves_identity_on_bad_field() {
        let uuid = "0123456789abcdef0123456789abcdef";
        let json = format!(
            r#"{{ "player_name": "Player", "player_uuid": "{}", "music_enabled": true,
                 "sfx_volume": "fort" }}"#, // sfx_volume has the wrong type (string instead of number)
            uuid
        );
        let (parsed, salvaged) = UserConfig::from_json_report(&json);
        assert!(salvaged, "an invalid field must trigger salvage");
        assert_eq!(parsed.player_uuid, uuid, "the identity must survive");
        assert_eq!(parsed.player_name, "Player");
        assert_eq!(parsed.music_enabled, true);
        assert_eq!(parsed.sfx_volume, UserConfig::default().sfx_volume,
            "the faulty field falls back to its default");
        assert_eq!(parsed.scores.len(), 30);
    }

    #[test]
    fn salvage_flag_false_on_valid_json() {
        let (_, salvaged) = UserConfig::from_json_report(r#"{ "player_name": "OK" }"#);
        assert!(!salvaged);
        let (_, salvaged) = UserConfig::from_json_report("total garbage");
        assert!(salvaged, "an unreadable file is reported (full defaults)");
    }

    #[test]
    fn submit_score_ranking() {
        let mut config = empty_config();
        assert_eq!(config.submit_score("A", 100), Some(0));
        assert_eq!(config.submit_score("B", 200), Some(0));
        assert_eq!(config.submit_score("C", 50), Some(2));
        assert_eq!(config.scores[0].name, "B");
        assert_eq!(config.scores[1].name, "A");
        assert_eq!(config.scores[2].name, "C");
    }

    #[test]
    fn submit_score_max_limit() {
        let mut config = empty_config();
        for i in 0..MAX_SCORES {
            config.submit_score("X", (i as i32 + 1) * 100);
        }
        assert_eq!(config.scores.len(), MAX_SCORES);
        assert_eq!(config.submit_score("Y", 50), None);
        assert_eq!(config.scores.len(), MAX_SCORES);
        let top = MAX_SCORES as i32 * 100 + 500;
        assert_eq!(config.submit_score("Z", top), Some(0));
        assert_eq!(config.scores.len(), MAX_SCORES);
        assert_eq!(config.scores[0].name, "Z");
    }

    #[test]
    fn highest_score_empty() {
        let config = empty_config();
        assert_eq!(config.highest_score(), 0);
    }

    #[test]
    fn highest_score_default() {
        let config = UserConfig::default();
        assert_eq!(config.highest_score(), 600);
    }

    // ── Temporary overrides ──────────────────────────────────────────────────────────────

    #[test]
    fn override_applies_to_typed_field() {
        let mut c = UserConfig::default();
        c.player_name = "Base".into();
        c.music_volume = 0.8;
        c.apply_override(ConfigKey::PlayerName, "Thib").unwrap();
        c.apply_override(ConfigKey::MusicVolume, "0.2").unwrap();
        // Every reader sees the effective value.
        assert_eq!(c.player_name, "Thib");
        assert!((c.music_volume - 0.2).abs() < 1e-6);
    }

    #[test]
    fn override_never_persists() {
        let mut c = UserConfig::default();
        c.player_name = "Base".into();
        c.music_volume = 0.8;
        c.apply_override(ConfigKey::PlayerName, "Thib").unwrap();
        // The persistable snapshot drops the override (base restored)...
        let snap = c.persistable_snapshot();
        assert_eq!(snap.player_name, "Base");
        // ...but keeps the non-overridden fields as they are.
        assert!((snap.music_volume - 0.8).abs() < 1e-6);
        // The runtime object keeps the effective value.
        assert_eq!(c.player_name, "Thib");
    }

    #[test]
    fn override_does_not_leak_when_other_option_changes() {
        // The player changes another option in game, then saves: the override does not leak.
        let mut c = UserConfig::default();
        c.player_name = "Base".into();
        c.fullscreen = false;
        c.apply_override(ConfigKey::PlayerName, "Thib").unwrap();
        c.fullscreen = true; // legitimate change during the session (not overridden)
        let snap = c.persistable_snapshot();
        assert_eq!(snap.player_name, "Base", "the override must not be persisted");
        assert!(snap.fullscreen, "the legitimate change must be persisted");
    }

    #[test]
    fn override_validation_and_parse() {
        let mut c = UserConfig::default();
        assert!(c.apply_override(ConfigKey::MusicVolume, "not-a-number").is_err());
        assert!(c.apply_override(ConfigKey::Language, "zz").is_err());
        assert!(c.apply_override(ConfigKey::PlayerName, "  ").is_err());
        // Volume clamping.
        c.apply_override(ConfigKey::MusicVolume, "5").unwrap();
        assert!((c.music_volume - 1.0).abs() < 1e-6);
        // Parsing of public ids.
        assert_eq!(ConfigKey::parse("player.name"), Some(ConfigKey::PlayerName));
        assert_eq!(ConfigKey::parse("audio.music_volume"), Some(ConfigKey::MusicVolume));
        assert_eq!(ConfigKey::parse("i18n.language"), Some(ConfigKey::Language));
        assert!(ConfigKey::parse("rating.foo").is_none()); // not part of the public surface
        // Every advertised id must parse (an id added to the list but not to the `match`
        // would otherwise go unnoticed), and distinct ids give distinct keys.
        let keys: Vec<_> = ConfigKey::PUBLIC_IDS
            .iter()
            .map(|id| ConfigKey::parse(id).unwrap_or_else(|| panic!("public id {id} does not parse")))
            .collect();
        for (i, a) in keys.iter().enumerate() {
            for b in &keys[i + 1..] {
                assert_ne!(a, b);
            }
        }
    }

    /// Temp directory for one test, removed on drop (also when the test fails).
    struct TempDir(PathBuf);
    impl TempDir {
        fn new(tag: &str) -> Self {
            static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
            let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let dir = std::env::temp_dir().join(format!("ca_state_{tag}_{}_{n}", std::process::id()));
            std::fs::create_dir_all(&dir).unwrap();
            TempDir(dir)
        }
        fn state(&self) -> PathBuf {
            self.0.join("state.json")
        }
    }
    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// A file of a newer format is used, and never rewritten (byte for byte).
    #[test]
    fn newer_format_is_used_but_never_rewritten() {
        let dir = TempDir::new("newer");
        let text = r#"{ "format_version": 999, "player_name": "ZOE", "future_option": true,
            "player_uuid": "00112233445566778899aabbccddeeff" }"#;
        std::fs::write(dir.state(), text).unwrap();
        let cfg = UserConfig::load_from(&dir.state());
        assert_eq!(cfg.player_name, "ZOE");
        assert!(cfg.read_only);
        assert!(!cfg.save_to(&dir.state()), "a newer file must not be rewritten");
        assert_eq!(std::fs::read_to_string(dir.state()).unwrap(), text);
    }

    /// A file written before `format_version` existed reads as format 1, without salvage, and
    /// the next save writes the current format.
    #[test]
    fn unversioned_file_reads_as_format_1() {
        let dir = TempDir::new("legacy");
        std::fs::write(dir.state(), r#"{ "player_name": "ZOE", "player_uuid": "00112233445566778899aabbccddeeff" }"#).unwrap();
        let cfg = UserConfig::load_from(&dir.state());
        assert!(!cfg.read_only);
        assert_eq!(cfg.player_name, "ZOE");
        assert!(cfg.save_to(&dir.state()));
        let saved: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(dir.state()).unwrap()).unwrap();
        assert_eq!(saved["format_version"], STATE_FORMAT_VERSION);
    }

    /// A truncated `state.json` is set aside and the backup restores identity and scores; the
    /// next save does not replace the good backup with the corrupt file.
    #[test]
    fn truncated_file_restored_from_backup() {
        let dir = TempDir::new("backup");
        let mut good = UserConfig::default();
        good.player_uuid = "00112233445566778899aabbccddeeff".into();
        good.submit_score("ZOE", 9999);
        assert!(good.save_to(&dir.state()));
        assert!(good.save_to(&dir.state()), "second save creates the backup");
        let full = std::fs::read_to_string(dir.state()).unwrap();
        std::fs::write(dir.state(), &full[..full.len() / 2]).unwrap(); // crash mid-write

        let cfg = UserConfig::load_from(&dir.state());
        assert_eq!(cfg.player_uuid, good.player_uuid);
        assert_eq!(cfg.highest_score(), 9999);
        let aside = std::fs::read_dir(&dir.0).unwrap().filter_map(Result::ok)
            .any(|e| e.file_name().to_string_lossy().contains("corrupt"));
        assert!(aside, "the truncated file must be kept aside");

        assert!(cfg.save_to(&dir.state()));
        let bak = std::fs::read_to_string(dir.state().with_extension("json.bak")).unwrap();
        assert!(serde_json::from_str::<UserConfig>(&bak).is_ok(), "the backup must stay valid");
    }

    /// Ladder parameters are no longer part of `state.json`: an old file that holds
    /// inconsistent ones (which made the rating computation panic) loads without them.
    #[test]
    fn legacy_rating_block_is_ignored() {
        let dir = TempDir::new("rating");
        std::fs::write(dir.state(), r#"{ "rating": { "rd_min": 500.0, "default_rd": 350.0 },
            "player_uuid": "00112233445566778899aabbccddeeff" }"#).unwrap();
        let cfg = UserConfig::load_from(&dir.state());
        assert!(cfg.save_to(&dir.state()));
        assert!(!std::fs::read_to_string(dir.state()).unwrap().contains("rd_min"));
    }

    /// Reference files of released versions (`tests/fixtures/<version>/state.json`): each one
    /// loads without salvage, keeps the player's data, and survives a save and reload. Add the
    /// folder of every released version.
    #[test]
    fn reference_state_files_load_and_round_trip() {
        for version in ["pre-beta", "2.0.0-beta.1"] {
            let src = format!("{}/tests/fixtures/{version}/state.json", env!("CARGO_MANIFEST_DIR"));
            let dir = TempDir::new("fixture");
            std::fs::copy(&src, dir.state()).unwrap();
            let check = |cfg: &UserConfig| {
                assert_eq!(cfg.player_name, "ZOE", "{version}");
                assert_eq!(cfg.player_uuid, "00112233445566778899aabbccddeeff", "{version}");
                assert_eq!(cfg.highest_score(), 9999, "{version}");
                assert!(cfg.records.iter().any(|r| r.key == RecordKey::BestChain && r.value == 7), "{version}");
                assert_eq!(cfg.key_bindings.swap[0], "KeyX", "{version}");
                assert_eq!(cfg.language.as_deref(), Some("fr"), "{version}");
                assert!(!cfg.read_only, "{version}");
            };
            let cfg = UserConfig::load_from(&dir.state());
            check(&cfg);
            let salvaged = std::fs::read_dir(&dir.0).unwrap().filter_map(Result::ok)
                .any(|e| e.file_name().to_string_lossy().contains("corrupt"));
            assert!(!salvaged, "{version}: a reference file must load without salvage");
            assert!(cfg.save_to(&dir.state()), "{version}");
            check(&UserConfig::load_from(&dir.state()));
        }
    }

    /// A binding whose inputs are all unknown (renamed key) takes its default back; a binding
    /// that still has a known input is left as it is.
    #[test]
    fn unknown_inputs_restore_the_default_binding() {
        let (cfg, _) = UserConfig::from_json_report(
            r#"{ "key_bindings": { "swap": ["FutureKey"], "raise": ["FutureKey", "KeyR"] } }"#);
        assert_eq!(cfg.key_bindings.swap, KeyBindings::default().swap);
        assert_eq!(cfg.key_bindings.raise, vec!["FutureKey".to_string(), "KeyR".to_string()]);
    }
}
