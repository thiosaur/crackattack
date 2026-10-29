//! Match log of the local ladder, stored in `ladder.json` in the config directory.
//!
//! The log is the only stored artifact of the ranking system. Ratings are never stored: they are
//! recomputed from the log (`crackattack_rating::recompute`). The file is not protected against
//! editing: it belongs to the player, and nothing on the client side can be secret in an open
//! source game.
//!
//! Versions: `format_version` is checked on read. A file of a newer format is used but never
//! rewritten. An unreadable or invalid file is set aside (`.corrupt-<ts>`) and the backup
//! `ladder.json.bak` is used instead.
//!
//! Concurrency: `append` does reload, push, write under the inter-process lock, and the write is
//! atomic (`persist::write_atomic`), the same pattern as `state.json`. Two instances do not lose
//! each other's entries. Without the lock nothing is written: the match is reported as not saved.
//! The store knows nothing about time: timestamps come in the entries.
//!
//! The `players` map also holds contact metadata for reconnecting to a peer: last listen
//! address and port, last self-announced rating, and a timestamp. `serde(default)` keeps older
//! logs loadable.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crackattack_rating::{MatchResult, PlayerId};
use serde::{Deserialize, Serialize};

/// Format written by this version. Increment it when an older version could not read the file
/// faithfully; an older version then uses the file without rewriting it.
const FORMAT_VERSION: u32 = 1;

/// Serialized form of `ladder.json`. Files written before 2.0.0-beta.1 also hold a `checksum`
/// field, ignored on read.
#[derive(Serialize, Deserialize)]
struct LadderFile {
    format_version: u32,
    /// Hex uuid to display name and contact metadata.
    #[serde(default)]
    players: HashMap<String, PlayerMeta>,
    #[serde(default)]
    entries: Vec<MatchResult>,
}

/// Metadata of a player in the log: display name and reconnection contact.
#[derive(Serialize, Deserialize, Clone, Default)]
pub struct PlayerMeta {
    pub name: String,
    /// Last IP address seen. A reconnection fallback that can go stale (DHCP).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_addr: Option<String>,
    /// Listen port announced in the handshake (not the ephemeral source port). 0 is never stored.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_port: Option<u16>,
    /// Time of the last match against this peer (Unix seconds).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_seen_ts: Option<u64>,
    /// Last self-announced rating (mDNS TXT `rt`), if it was visible. The UI labels it as
    /// such: it is not a ranking, only the last rating the peer claimed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_announced_rt: Option<i32>,
}

/// Contact info captured during a match. Merged into `PlayerMeta` on append.
#[derive(Clone, Debug, Default)]
pub struct ContactInfo {
    pub addr: Option<String>,
    /// Announced listen port (0 = not listening, not stored).
    pub listen_port: u16,
    pub seen_ts: u64,
    pub announced_rt: Option<i32>,
}

/// Default path: `ladder.json` in the config directory.
fn default_path() -> Option<PathBuf> {
    super::persist::config_dir().map(|d| d.join("ladder.json"))
}

/// The log store: entries and names in memory, synchronized with the file.
pub struct LadderStore {
    /// `None` means no persistence is possible (no home directory): memory-only store.
    path: Option<PathBuf>,
    players: HashMap<String, PlayerMeta>,
    entries: Vec<MatchResult>,
    /// The file has a newer format than this version knows: it is used, never rewritten.
    read_only: bool,
}

impl LadderStore {
    /// Loads the log from the standard location.
    pub fn load() -> Self {
        Self::load_from(default_path())
    }

    /// Loads from an explicit path (tests).
    ///
    /// A missing file gives an empty store, and nothing is created before the first `append`.
    /// An invalid file is set aside and the backup is used, or else the store starts empty. A
    /// file of a newer format is used read-only. It never panics.
    pub fn load_from(path: Option<PathBuf>) -> Self {
        let mut store = Self { path, players: HashMap::new(), entries: Vec::new(), read_only: false };
        let Some(ref p) = store.path else { return store };
        let file = match read_file(p) {
            ReadOutcome::Missing => None,
            ReadOutcome::Valid(f) => Some(f),
            ReadOutcome::Newer(f) => {
                log::warn!("ladder: newer format in {}, used read-only", p.display());
                store.read_only = true;
                f
            }
            ReadOutcome::Corrupt(reason) => {
                set_aside_corrupt(p, &reason);
                match read_file(&backup_path(p)) {
                    ReadOutcome::Valid(f) => {
                        log::warn!("ladder: restored from backup");
                        Some(f)
                    }
                    _ => None,
                }
            }
        };
        if let Some(f) = file {
            store.players = f.players;
            store.entries = f.entries;
        }
        store
    }

    /// Appends an entry (and the names of both players), then rewrites the file. Returns
    /// `false` if the match could not be saved (the reason is logged); the store is then
    /// unchanged.
    ///
    /// Reload, push, write, under the inter-process lock: starts from the disk state if it is
    /// valid (so entries from another instance are kept). `contact` is the peer's `(id, info)`,
    /// merged into that peer's `PlayerMeta`.
    pub fn append(
        &mut self,
        entry: MatchResult,
        name_a: &str,
        name_b: &str,
        contact: Option<(PlayerId, ContactInfo)>,
    ) -> bool {
        if self.read_only {
            log::warn!("ladder: match not saved, the log belongs to a newer version of the game");
            return false;
        }
        let Some(dir) = self.path.as_ref().and_then(|p| p.parent().map(|d| d.to_path_buf())) else {
            // No persistence possible: memory-only store.
            self.push(entry, name_a, name_b, &contact);
            return false;
        };
        if let Err(e) = std::fs::create_dir_all(&dir) {
            log::warn!("ladder: cannot create dir {}: {e}", dir.display());
        }
        match super::persist::with_config_lock(&dir, || self.append_locked(entry, name_a, name_b, &contact)) {
            Some(saved) => saved,
            None => {
                log::warn!("ladder: match not saved (config lock unavailable)");
                false
            }
        }
    }

    /// Body of the transaction, under the lock.
    fn append_locked(
        &mut self,
        entry: MatchResult,
        name_a: &str,
        name_b: &str,
        contact: &Option<(PlayerId, ContactInfo)>,
    ) -> bool {
        // Start from the disk state if valid (it may hold entries from another instance).
        if let Some(ref p) = self.path {
            match read_file(p) {
                ReadOutcome::Valid(f) => {
                    self.entries = f.entries;
                    for (k, v) in f.players {
                        self.players.entry(k).or_insert(v);
                    }
                }
                ReadOutcome::Missing => {}
                ReadOutcome::Newer(_) => {
                    log::warn!("ladder: match not saved, a newer version of the game wrote the log");
                    return false;
                }
                ReadOutcome::Corrupt(reason) => set_aside_corrupt(p, &reason),
            }
        }
        self.push(entry, name_a, name_b, contact);
        if self.write() {
            true
        } else {
            self.entries.pop();
            false
        }
    }

    /// Adds the entry and the names and contact of both players, in memory.
    fn push(&mut self, entry: MatchResult, name_a: &str, name_b: &str, contact: &Option<(PlayerId, ContactInfo)>) {
        self.entries.push(entry);
        self.players.entry(entry.a.to_hex()).or_default().name = name_a.to_string();
        self.players.entry(entry.b.to_hex()).or_default().name = name_b.to_string();
        if let Some((id, c)) = contact.as_ref() {
            let meta = self.players.entry(id.to_hex()).or_default();
            if let Some(addr) = c.addr.clone() {
                meta.last_addr = Some(addr);
            }
            if c.listen_port != 0 {
                meta.last_port = Some(c.listen_port);
            }
            meta.last_seen_ts = Some(c.seen_ts);
            if let Some(rt) = c.announced_rt {
                meta.last_announced_rt = Some(rt);
            }
        }
    }

    /// The log entries, in append order (the input of `recompute`).
    pub fn entries(&self) -> &[MatchResult] {
        &self.entries
    }

    /// Last name seen for a player.
    pub fn display_name(&self, id: &PlayerId) -> Option<&str> {
        self.players.get(&id.to_hex()).map(|m| m.name.as_str())
    }

    /// Full metadata of a player: name and reconnection contact.
    pub fn player_meta(&self, id: &PlayerId) -> Option<&PlayerMeta> {
        self.players.get(&id.to_hex())
    }

    /// Writes the file atomically (`persist::write_atomic`). The previous file becomes
    /// `ladder.json.bak` if it was valid. Returns `false` on failure (logged).
    fn write(&self) -> bool {
        let Some(ref path) = self.path else { return false };
        if let ReadOutcome::Valid(_) = read_file(path) {
            if let Err(e) = std::fs::copy(path, backup_path(path)) {
                log::warn!("ladder: backup failed: {e}");
            }
        }
        let file = LadderFile {
            format_version: FORMAT_VERSION,
            players: self.players.clone(),
            entries: self.entries.clone(),
        };
        let json = match serde_json::to_string_pretty(&file) {
            Ok(j) => j,
            Err(e) => {
                log::warn!("ladder: serialize failed: {e}");
                return false;
            }
        };
        match super::persist::write_atomic(path, json.as_bytes()) {
            Ok(()) => true,
            Err(e) => {
                log::warn!("ladder: write failed: {e}");
                false
            }
        }
    }
}

fn backup_path(path: &Path) -> PathBuf {
    path.with_extension("json.bak")
}

enum ReadOutcome {
    Missing,
    Valid(LadderFile),
    /// Newer format: read on a best-effort basis (`None` if this version cannot parse it).
    Newer(Option<LadderFile>),
    Corrupt(String),
}

/// Reads the file and checks its format version.
fn read_file(path: &Path) -> ReadOutcome {
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return ReadOutcome::Missing,
        Err(e) => return ReadOutcome::Corrupt(format!("unreadable: {e}")),
    };
    let version = serde_json::from_str::<serde_json::Value>(&text).ok()
        .and_then(|v| v.get("format_version").and_then(|f| f.as_u64()));
    if version.is_some_and(|v| v > FORMAT_VERSION as u64) {
        return ReadOutcome::Newer(serde_json::from_str(&text).ok());
    }
    match serde_json::from_str(&text) {
        Ok(f) => ReadOutcome::Valid(f),
        Err(e) => ReadOutcome::Corrupt(format!("invalid JSON: {e}")),
    }
}

/// Sets the corrupt file aside (`.corrupt-<ts>`). The ladder restarts empty and the original
/// is kept for inspection.
fn set_aside_corrupt(path: &Path, reason: &str) {
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let aside = path.with_extension(format!("json.corrupt-{ts}"));
    match std::fs::rename(path, &aside) {
        Ok(()) => log::warn!(
            "ladder: corrupt log ({reason}), set aside as {}, ladder reset",
            aside.display()
        ),
        Err(e) => log::warn!("ladder: corrupt log ({reason}) and rename failed: {e}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crackattack_rating::Outcome;
    /// Unique ladder path; the enclosing directory is removed on drop (also on failure).
    struct TestPath {
        path: PathBuf,
        _dir: crate::platform::test_support::TestDir,
    }

    impl std::ops::Deref for TestPath {
        type Target = PathBuf;
        fn deref(&self) -> &PathBuf {
            &self.path
        }
    }

    impl AsRef<Path> for TestPath {
        fn as_ref(&self) -> &Path {
            &self.path
        }
    }

    fn test_path() -> TestPath {
        let dir = crate::platform::test_support::TestDir::new("ladder");
        TestPath { path: dir.path().join("ladder.json"), _dir: dir }
    }

    fn pid(n: u8) -> PlayerId {
        PlayerId([n; 16])
    }

    fn entry(a: u8, b: u8, outcome: Outcome, ts: u64) -> MatchResult {
        MatchResult { a: pid(a), b: pid(b), outcome, ts }
    }

    /// Round trip: 3 appends, then a reload gives the same entries and names.
    #[test]
    fn round_trip() {
        let p = test_path();
        let mut s = LadderStore::load_from(Some(p.clone()));
        s.append(entry(1, 2, Outcome::WinA, 100), "Alice", "Bob", None);
        s.append(entry(1, 2, Outcome::WinB, 200), "Alice", "Bob", None);
        s.append(entry(1, 3, Outcome::WinA, 300), "Alice", "Carol", None);

        let r = LadderStore::load_from(Some(p.clone()));
        assert_eq!(r.entries(), s.entries());
        assert_eq!(r.entries().len(), 3);
        assert_eq!(r.display_name(&pid(1)), Some("Alice"));
        assert_eq!(r.display_name(&pid(3)), Some("Carol"));
    }

    /// The file is the player's: an edit by hand is accepted as is.
    #[test]
    fn manual_edit_is_accepted() {
        let p = test_path();
        let mut s = LadderStore::load_from(Some(p.clone()));
        assert!(s.append(entry(1, 2, Outcome::WinA, 100), "Alice", "Bob", None));
        let text = std::fs::read_to_string(&p).unwrap();
        std::fs::write(&p, text.replace("win_a", "win_b")).unwrap();
        let r = LadderStore::load_from(Some(p.clone()));
        assert_eq!(r.entries()[0].outcome, Outcome::WinB);
    }

    /// A log of a newer format is used read-only: an append is refused and the file is left
    /// untouched.
    #[test]
    fn newer_format_is_read_only() {
        let p = test_path();
        let mut s = LadderStore::load_from(Some(p.clone()));
        assert!(s.append(entry(1, 2, Outcome::WinA, 100), "Alice", "Bob", None));
        let text = std::fs::read_to_string(&p).unwrap().replace("\"format_version\": 1", "\"format_version\": 999");
        std::fs::write(&p, &text).unwrap();
        let mut r = LadderStore::load_from(Some(p.clone()));
        assert_eq!(r.entries().len(), 1, "entries readable by this version are used");
        assert!(!r.append(entry(1, 2, Outcome::WinB, 200), "Alice", "Bob", None));
        assert_eq!(std::fs::read_to_string(&p).unwrap(), text);
    }

    /// A truncated log is set aside and the backup is used.
    #[test]
    fn truncated_log_restored_from_backup() {
        let p = test_path();
        let mut s = LadderStore::load_from(Some(p.clone()));
        assert!(s.append(entry(1, 2, Outcome::WinA, 100), "Alice", "Bob", None));
        assert!(s.append(entry(1, 2, Outcome::WinB, 200), "Alice", "Bob", None));
        let full = std::fs::read_to_string(&p).unwrap();
        std::fs::write(&p, &full[..full.len() / 2]).unwrap();
        let r = LadderStore::load_from(Some(p.clone()));
        assert_eq!(r.entries().len(), 1, "the backup holds the log before the last append");
    }

    /// Without the lock nothing is written, and the match is reported as not saved.
    #[test]
    fn never_writes_without_the_lock() {
        let p = test_path();
        let dir = p.parent().unwrap().to_path_buf();
        std::fs::create_dir_all(&dir).unwrap();
        let mut s = LadderStore::load_from(Some(p.clone()));
        let saved = crate::platform::persist::with_config_lock(&dir, || {
            s.append(entry(1, 2, Outcome::WinA, 100), "Alice", "Bob", None)
        });
        assert_eq!(saved, Some(false));
        assert!(!p.exists());
        assert!(s.entries().is_empty());
    }

    /// Invalid JSON is rejected and set aside, without a panic.
    #[test]
    fn invalid_json_rejected() {
        let p = test_path();
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, "{ not json").unwrap();
        let r = LadderStore::load_from(Some(p.clone()));
        assert!(r.entries().is_empty());
        assert!(!p.exists());
    }

    /// First run: a missing file gives an empty store, and nothing is created before the first append.
    #[test]
    fn first_run_creates_nothing() {
        let p = test_path();
        let s = LadderStore::load_from(Some(p.clone()));
        assert!(s.entries().is_empty());
        assert!(!p.exists());
        assert!(!p.parent().unwrap().exists());
    }

    /// Renaming: the last name seen wins.
    #[test]
    fn last_seen_name_wins() {
        let p = test_path();
        let mut s = LadderStore::load_from(Some(p.clone()));
        s.append(entry(1, 2, Outcome::WinA, 100), "Alice", "Bob", None);
        s.append(entry(1, 2, Outcome::WinA, 200), "Alicia", "Bob", None);
        assert_eq!(s.display_name(&pid(1)), Some("Alicia"));
        let r = LadderStore::load_from(Some(p.clone()));
        assert_eq!(r.display_name(&pid(1)), Some("Alicia"));
    }

    /// Two stores on the same file (reload before append): no entry is lost.
    #[test]
    fn two_stores_no_lost_entry() {
        let p = test_path();
        let mut s1 = LadderStore::load_from(Some(p.clone()));
        let mut s2 = LadderStore::load_from(Some(p.clone())); // loaded empty, before s1 appends
        s1.append(entry(1, 2, Outcome::WinA, 100), "Alice", "Bob", None);
        s2.append(entry(3, 4, Outcome::WinB, 200), "Carol", "Dave", None);

        let r = LadderStore::load_from(Some(p.clone()));
        assert_eq!(r.entries().len(), 2, "s2's append must not overwrite s1's");
    }

    /// Contact metadata is merged on append and persists.
    #[test]
    fn contact_meta_merged_and_persisted() {
        let p = test_path();
        let mut s = LadderStore::load_from(Some(p.clone()));
        s.append(entry(1, 2, Outcome::WinA, 100), "Alice", "Bob", Some((pid(2), ContactInfo {
            addr: Some("192.168.1.42".into()),
            listen_port: 45678,
            seen_ts: 100,
            announced_rt: Some(1512),
        })));
        // Second match: the address changes and the rating is no longer visible, so the old one is kept.
        s.append(entry(1, 2, Outcome::WinB, 200), "Alice", "Bob", Some((pid(2), ContactInfo {
            addr: Some("192.168.1.77".into()),
            listen_port: 0, // no listen port announced, so last_port is kept
            seen_ts: 200,
            announced_rt: None,
        })));

        let r = LadderStore::load_from(Some(p.clone()));
        let m = r.player_meta(&pid(2)).expect("Bob's meta");
        assert_eq!(m.name, "Bob");
        assert_eq!(m.last_addr.as_deref(), Some("192.168.1.77"));
        assert_eq!(m.last_port, Some(45678));
        assert_eq!(m.last_seen_ts, Some(200));
        assert_eq!(m.last_announced_rt, Some(1512));
        // Alice (side a) has no contact.
        assert!(r.player_meta(&pid(1)).unwrap().last_addr.is_none());
    }

    /// A log whose `players` entries hold only `name` loads without error.
    #[test]
    fn legacy_players_map_loads() {
        let p = test_path();
        let mut s = LadderStore::load_from(Some(p.clone()));
        s.append(entry(1, 2, Outcome::WinA, 100), "Alice", "Bob", None);
        let text = std::fs::read_to_string(&p).unwrap();
        assert!(!text.contains("last_addr"), "without contact info, no meta must be written");
        let r = LadderStore::load_from(Some(p.clone()));
        assert_eq!(r.display_name(&pid(1)), Some("Alice"));
    }

    /// A rename in the JSON is accepted.
    #[test]
    fn player_names_editable() {
        let p = test_path();
        let mut s = LadderStore::load_from(Some(p.clone()));
        s.append(entry(1, 2, Outcome::WinA, 100), "Alice", "Bob", None);

        let text = std::fs::read_to_string(&p).unwrap();
        std::fs::write(&p, text.replace("Alice", "Reine Alice")).unwrap();

        let r = LadderStore::load_from(Some(p.clone()));
        assert_eq!(r.entries().len(), 1, "a cosmetic rename is still accepted");
        assert_eq!(r.display_name(&pid(1)), Some("Reine Alice"));
    }

    /// Reference logs of released versions (`tests/fixtures/<version>/ladder.json`), including
    /// the pre-beta format with its checksum field: all entries load, and an append keeps them.
    #[test]
    fn reference_logs_load_and_accept_an_append() {
        for version in ["pre-beta", "2.0.0-beta.1", "2.0.0-beta.2", "2.0.0-beta.3", "2.0.0-beta.4"] {
            let src = format!("{}/tests/fixtures/{version}/ladder.json", env!("CARGO_MANIFEST_DIR"));
            let p = test_path();
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::copy(&src, &p).unwrap();
            let mut s = LadderStore::load_from(Some(p.clone()));
            assert_eq!(s.entries().len(), 3, "{version}");
            assert_eq!(s.entries()[2].outcome, Outcome::Draw, "{version}");
            assert_eq!(s.player_meta(&s.entries()[0].b).unwrap().last_port, Some(45678), "{version}");
            assert!(s.append(entry(1, 2, Outcome::WinA, 100), "Alice", "Bob", None), "{version}");
            assert_eq!(LadderStore::load_from(Some(p.clone())).entries().len(), 4, "{version}");
        }
    }
}
