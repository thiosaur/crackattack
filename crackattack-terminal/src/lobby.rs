// The lobby: who else is connected to the same server, under which pseudo, doing what.
//
// A folder shared by every game of the same container. Each game keeps one file there, named
// after its pseudo, and holds a lock on it for as long as it runs: a file whose lock is held
// is a player who is there, one whose lock is free is a game that was killed, and the first
// game that sees it removes it. A line in the file says what the player is doing.
//
// Without `--lobby DIR` the others are found over the local network instead (`lan`); the
// rest of the game does not tell the two apart (`Presence`).

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::time::{Duration, Instant};

use crate::render::versus::clean_name;
use crate::render::LobbyView;

/// The pseudos a game picks from (`pseudos.txt`, one per line).
const PSEUDOS: &str = include_str!("pseudos.txt");
/// Looked at the folder at most this often.
const SCAN_EVERY: Duration = Duration::from_secs(1);
/// An unlocked file younger than this may belong to a game that has not locked it yet.
const STALE_AFTER: Duration = Duration::from_secs(3);
/// Width of the state line: written in one piece over the previous one, never shorter.
const STATE_WIDTH: usize = 64;

/// What a player is doing.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum State {
    Menu,
    Playing,
    /// Waiting for a duel; the port it listens on (127.0.0.1).
    Waiting(u16),
    /// In a duel against this pseudo.
    Duel(String),
}

impl State {
    fn encode(&self) -> String {
        match self {
            State::Menu => "menu".to_string(),
            State::Playing => "playing".to_string(),
            State::Waiting(port) => format!("waiting {}", port),
            State::Duel(name) => format!("duel {}", name),
        }
    }

    /// An unreadable line is `Menu`: the file belongs to another process, and may be torn.
    fn parse(line: &str) -> State {
        let line = line.trim();
        match line.split_once(' ') {
            None if line == "playing" => State::Playing,
            Some(("waiting", port)) => port.trim().parse().map_or(State::Menu, State::Waiting),
            Some(("duel", name)) => State::Duel(clean_name(name)),
            _ => State::Menu,
        }
    }

    /// The word the panel shows.
    pub(crate) fn label(&self) -> &'static str {
        match self {
            State::Menu => "menu",
            State::Playing => "playing",
            State::Waiting(_) => "waiting",
            State::Duel(_) => "duel",
        }
    }
}

/// Another connected player.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Player {
    pub name: String,
    pub state: State,
    /// Where it can be reached: 127.0.0.1 for the games of a server, its address on the
    /// network otherwise.
    pub ip: IpAddr,
}

/// A change among the other players since the last look.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Notice {
    Joined(String),
    Left(String),
}

/// A name the folder may hold: what `clean_name` lets through, nothing else (the files of the
/// folder come from other processes, and a name is written to the terminal).
pub(crate) fn valid_name(name: &str) -> bool {
    !name.is_empty() && name.len() <= crate::render::versus::NAME_MAX
        && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '_' || c == '-')
        && !name.starts_with('.')
}

/// What changed between two looks at the folder.
pub(crate) fn notices(before: &[Player], after: &[Player]) -> Vec<Notice> {
    let joined = after.iter().filter(|p| before.iter().all(|b| b.name != p.name)).map(|p| Notice::Joined(p.name.clone()));
    let left = before.iter().filter(|b| after.iter().all(|p| p.name != b.name)).map(|b| Notice::Left(b.name.clone()));
    joined.chain(left).collect()
}

/// Where the players of a game's neighborhood are found.
enum Backend {
    /// The games of a server (`--lobby DIR`): a folder, and 127.0.0.1.
    Folder(Lobby),
    /// The games of a local network, announced over mDNS.
    Lan(crate::lan::Lan),
}

/// This game among the others, and the last arrival or departure to tell the player about.
pub(crate) struct Presence {
    backend: Backend,
    notice: Option<(String, Instant)>,
    /// What a friend without access is told to type (`--invite-command`).
    invite: Option<String>,
}

/// The pseudo at `start` in the list, wrapping around.
pub(crate) fn pseudo(start: usize) -> &'static str {
    let names: Vec<&str> = PSEUDOS.lines().filter(|l| valid_name(l)).collect();
    names[start % names.len()]
}

/// How long an arrival or a departure stays on the screen.
const NOTICE_FOR: Duration = Duration::from_secs(5);

impl Presence {
    /// Joins the lobby in `dir`, trying the pseudos from `start` on.
    pub(crate) fn join(dir: &std::path::Path, start: usize, name: Option<&str>) -> std::io::Result<Presence> {
        let lobby = match name {
            Some(name) => Lobby::join_as(dir, name)?,
            None => Lobby::join(dir, start)?,
        };
        Ok(Presence { backend: Backend::Folder(lobby), notice: None, invite: None })
    }

    /// Joins the games of the local network (mDNS), with the pseudo at `start`.
    pub(crate) fn join_lan(start: usize, name: Option<&str>) -> std::io::Result<Presence> {
        Ok(Presence { backend: Backend::Lan(crate::lan::Lan::join(start, name)?), notice: None, invite: None })
    }

    /// The text that tells a friend how to get in: printable ASCII only, at most 72 characters
    /// (it comes from the command line, and is written to the terminal and centered).
    pub(crate) fn set_invite(&mut self, text: &str) {
        let text: String = text.chars().filter(|c| c.is_ascii_graphic() || *c == ' ').take(72).collect();
        self.invite = Some(text.trim().to_string()).filter(|t| !t.is_empty());
    }

    /// Says what this player is doing.
    pub(crate) fn set_state(&self, state: &State) {
        match &self.backend {
            Backend::Folder(lobby) => lobby.set_state(state),
            Backend::Lan(lan) => lan.set_state(state),
        }
    }

    /// This player's pseudo.
    pub(crate) fn name(&self) -> &str {
        match &self.backend {
            Backend::Folder(lobby) => lobby.name(),
            Backend::Lan(lan) => lan.name(),
        }
    }

    /// The address a game that waits listens on: the machine itself for the games of a
    /// server (nothing outside can reach it), every interface on a network.
    pub(crate) fn listen_ip(&self) -> IpAddr {
        match &self.backend {
            Backend::Folder(_) => IpAddr::V4(Ipv4Addr::LOCALHOST),
            Backend::Lan(_) => IpAddr::V4(Ipv4Addr::UNSPECIFIED),
        }
    }

    fn others(&self) -> &[Player] {
        match &self.backend {
            Backend::Folder(lobby) => lobby.others(),
            Backend::Lan(lan) => lan.others(),
        }
    }

    /// The players waiting for a duel, with the address each listens on, by name.
    pub(crate) fn waiting(&self) -> Vec<(String, SocketAddr)> {
        self.others().iter()
            .filter_map(|p| match p.state {
                State::Waiting(port) => Some((p.name.clone(), SocketAddr::new(p.ip, port))),
                _ => None,
            })
            .collect()
    }

    /// Looks at the others (the folder is read once a second at most).
    pub(crate) fn poll(&mut self) {
        let news = match &mut self.backend {
            Backend::Folder(lobby) => lobby.poll(),
            Backend::Lan(lan) => lan.poll(),
        };
        for notice in news {
            let text = match notice {
                Notice::Joined(name) => format!("{} joined", name),
                Notice::Left(name) => format!("{} left", name),
            };
            self.notice = Some((text, Instant::now()));
        }
    }

    /// What the screen shows.
    pub(crate) fn view(&self) -> LobbyView {
        LobbyView {
            name: self.name().to_string(),
            others: self.others().iter().map(|p| (p.name.clone(), p.state.label())).collect(),
            notice: self.notice.as_ref().filter(|(_, at)| at.elapsed() < NOTICE_FOR).map(|(text, _)| text.clone()),
            invite: self.invite.clone(),
        }
    }
}

#[cfg(unix)]
pub(crate) use unix::Lobby;
#[cfg(not(unix))]
pub(crate) use other::Lobby;

#[cfg(unix)]
mod unix {
    use std::fs::{self, File, OpenOptions};
    use std::io::{self, Read};
    use std::net::{IpAddr, Ipv4Addr};
    use std::os::fd::AsRawFd;
    use std::os::unix::fs::{FileExt, MetadataExt};
    use std::path::{Path, PathBuf};
    use std::time::Instant;

    use super::{notices, valid_name, Notice, Player, State, PSEUDOS, SCAN_EVERY, STALE_AFTER, STATE_WIDTH};

    /// Takes the lock without waiting; `exclusive` for the player's own file, shared to find
    /// out whether somebody holds it.
    pub(super) fn try_lock(file: &File, exclusive: bool) -> bool {
        let operation = if exclusive { libc::LOCK_EX } else { libc::LOCK_SH } | libc::LOCK_NB;
        // SAFETY: `flock` only reads the descriptor of a file that is open for the whole call.
        unsafe { libc::flock(file.as_raw_fd(), operation) == 0 }
    }

    /// This game's place in the lobby. Dropping it leaves.
    pub(crate) struct Lobby {
        path: PathBuf,
        dir: PathBuf,
        name: String,
        /// Holds the lock: closed when the lobby is dropped.
        file: File,
        others: Vec<Player>,
        last_scan: Option<Instant>,
    }

    /// The file to remove when the terminal hangs up (see `leave_on_hangup`).
    static HANGUP_PATH: std::sync::OnceLock<std::ffi::CString> = std::sync::OnceLock::new();

    extern "C" fn on_hangup(_signal: libc::c_int) {
        if let Some(path) = HANGUP_PATH.get() {
            // SAFETY: `unlink` and `_exit` are safe to call from a signal handler, and `path`
            // is a C string that lives for the whole run.
            unsafe {
                libc::unlink(path.as_ptr());
                libc::_exit(129);
            }
        }
    }

    /// A game whose connection drops (ssh closing) is killed by SIGHUP, which skips `Drop`: its
    /// file would stay in the folder until another game sweeps it. The file is removed first.
    fn leave_on_hangup(path: &Path) {
        use std::os::unix::ffi::OsStrExt;
        if let Ok(path) = std::ffi::CString::new(path.as_os_str().as_bytes()) {
            if HANGUP_PATH.set(path).is_ok() {
                // SAFETY: installs a handler that only calls async-signal-safe functions.
                unsafe { libc::signal(libc::SIGHUP, on_hangup as extern "C" fn(libc::c_int) as libc::sighandler_t) };
            }
        }
    }

    impl Lobby {
        /// Joins the lobby in `dir` under the first free pseudo, trying them from `start`
        /// on. An error if the folder cannot be used, or every pseudo is taken.
        pub(crate) fn join(dir: &Path, start: usize) -> io::Result<Lobby> {
            let pseudos: Vec<&str> = PSEUDOS.lines().map(str::trim).filter(|p| valid_name(p)).collect();
            for i in 0..pseudos.len() {
                if let Some(lobby) = Lobby::take(dir, pseudos[(start + i) % pseudos.len()])? {
                    return Ok(lobby);
                }
            }
            Err(io::Error::other("every pseudo is taken"))
        }

        /// Joins the lobby in `dir` under this pseudo (`--name`). An error if it is taken.
        pub(crate) fn join_as(dir: &Path, name: &str) -> io::Result<Lobby> {
            Lobby::take(dir, name)?.ok_or_else(|| io::Error::other(format!("the pseudo {} is taken", name)))
        }

        /// The place of `name` in the folder, or `None` if somebody holds it.
        fn take(dir: &Path, name: &str) -> io::Result<Option<Lobby>> {
            let path = dir.join(name);
            let file = OpenOptions::new().read(true).write(true).create(true).truncate(false).open(&path)?;
            // Locked by another game, or by a scan looking at a file just created: next one.
            if !try_lock(&file, true) {
                return Ok(None);
            }
            // A scan may have removed the file between its creation and our lock.
            if fs::metadata(&path).map(|m| m.ino()).ok() != Some(file.metadata()?.ino()) {
                return Ok(None);
            }
            let lobby = Lobby { path, dir: dir.to_path_buf(), name: name.to_string(), file, others: Vec::new(), last_scan: None };
            lobby.write_state(&State::Menu)?;
            leave_on_hangup(&lobby.path);
            Ok(Some(lobby))
        }

        pub(crate) fn name(&self) -> &str {
            &self.name
        }

        /// The other players, by name, as of the last look.
        pub(crate) fn others(&self) -> &[Player] {
            &self.others
        }

        /// Says what this player is doing.
        pub(crate) fn set_state(&self, state: &State) {
            let _ = self.write_state(state);
        }

        fn write_state(&self, state: &State) -> io::Result<()> {
            // One write over the previous line, padded: a reader sees the old line or the new.
            let line = format!("{:<width$}\n", state.encode(), width = STATE_WIDTH - 1);
            self.file.write_all_at(line.as_bytes(), 0)
        }

        /// Looks at the folder (at most once a second) and tells who arrived or left.
        pub(crate) fn poll(&mut self) -> Vec<Notice> {
            self.poll_at(Instant::now())
        }

        pub(crate) fn poll_at(&mut self, now: Instant) -> Vec<Notice> {
            if self.last_scan.is_some_and(|last| now.duration_since(last) < SCAN_EVERY) {
                return Vec::new();
            }
            let first = self.last_scan.is_none();
            self.last_scan = Some(now);
            let after = self.scan();
            let news = if first { Vec::new() } else { notices(&self.others, &after) };
            self.others = after;
            news
        }

        /// The players whose file is locked; the files of killed games are removed.
        fn scan(&self) -> Vec<Player> {
            let Ok(entries) = fs::read_dir(&self.dir) else { return Vec::new() };
            let mut players = Vec::new();
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().into_owned();
                if name == self.name || !valid_name(&name) {
                    continue;
                }
                let Ok(mut file) = File::open(entry.path()) else { continue };
                if try_lock(&file, false) {
                    // Nobody holds it: a game that was killed (or one not yet locked: wait).
                    let old = file.metadata().and_then(|m| m.modified()).ok()
                        .and_then(|t| t.elapsed().ok()).is_some_and(|age| age > STALE_AFTER);
                    if old {
                        let _ = fs::remove_file(entry.path());
                    }
                    continue;
                }
                let mut line = String::new();
                let _ = (&mut file).take(STATE_WIDTH as u64).read_to_string(&mut line);
                players.push(Player { name, state: State::parse(&line), ip: IpAddr::V4(Ipv4Addr::LOCALHOST) });
            }
            players.sort_by(|a, b| a.name.cmp(&b.name));
            players
        }
    }

    impl Drop for Lobby {
        fn drop(&mut self) {
            // The file first, the lock with it when `file` closes.
            let _ = fs::remove_file(&self.path);
        }
    }
}

#[cfg(not(unix))]
mod other {
    use std::io;
    use std::path::Path;
    use std::time::Instant;

    use super::{Notice, Player, State};

    /// The lobby rests on file locks of Unix.
    pub(crate) struct Lobby;

    impl Lobby {
        pub(crate) fn join(_dir: &Path, _start: usize) -> io::Result<Lobby> {
            Err(io::Error::other("the lobby is not available on this system"))
        }
        pub(crate) fn join_as(_dir: &Path, _name: &str) -> io::Result<Lobby> {
            Err(io::Error::other("the lobby is not available on this system"))
        }
        pub(crate) fn name(&self) -> &str { "" }
        pub(crate) fn others(&self) -> &[Player] { &[] }
        pub(crate) fn set_state(&self, _state: &State) {}
        pub(crate) fn poll(&mut self) -> Vec<Notice> { Vec::new() }
        pub(crate) fn poll_at(&mut self, _now: Instant) -> Vec<Notice> { Vec::new() }
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::path::{Path, PathBuf};

    /// A folder of its own, removed at the end even when the test fails.
    struct TempDir(PathBuf);
    impl TempDir {
        fn new(tag: &str) -> Self {
            let path = std::env::temp_dir().join(format!("crackattack-lobby-{}-{}", tag, std::process::id()));
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir_all(&path).unwrap();
            TempDir(path)
        }
        fn path(&self) -> &Path { &self.0 }
    }
    impl Drop for TempDir {
        fn drop(&mut self) { let _ = std::fs::remove_dir_all(&self.0); }
    }

    /// A later look, past the one-second throttle.
    fn later(n: u64) -> Instant { Instant::now() + Duration::from_secs(10 * n) }

    #[test]
    fn two_games_never_get_the_same_pseudo_even_from_the_same_start() {
        let dir = TempDir::new("unique");
        let a = Lobby::join(dir.path(), 5).unwrap();
        let b = Lobby::join(dir.path(), 5).unwrap();
        assert_ne!(a.name(), b.name());
        assert!(dir.path().join(a.name()).exists() && dir.path().join(b.name()).exists());
    }

    #[test]
    fn players_see_each_other_arrive_and_leave() {
        let dir = TempDir::new("seen");
        let mut a = Lobby::join(dir.path(), 0).unwrap();
        assert!(a.poll_at(later(1)).is_empty() && a.others().is_empty());
        let b = Lobby::join(dir.path(), 40).unwrap();
        let b_name = b.name().to_string();
        assert_eq!(a.poll_at(later(2)), vec![Notice::Joined(b_name.clone())]);
        assert_eq!(a.others(), &[Player { name: b_name.clone(), state: State::Menu, ip: IpAddr::V4(Ipv4Addr::LOCALHOST) }]);
        // What the other does is published.
        b.set_state(&State::Waiting(4567));
        a.poll_at(later(3));
        assert_eq!(a.others()[0].state, State::Waiting(4567));
        // It leaves: its file goes with it.
        drop(b);
        assert_eq!(a.poll_at(later(4)), vec![Notice::Left(b_name.clone())]);
        assert!(!dir.path().join(&b_name).exists());
    }

    #[test]
    fn a_look_is_throttled_to_once_a_second() {
        let dir = TempDir::new("throttle");
        let mut a = Lobby::join(dir.path(), 0).unwrap();
        let start = later(1);
        a.poll_at(start);
        let _b = Lobby::join(dir.path(), 40).unwrap();
        assert!(a.poll_at(start + Duration::from_millis(500)).is_empty());
        assert_eq!(a.poll_at(start + Duration::from_millis(1100)).len(), 1);
    }

    #[test]
    fn the_file_of_a_killed_game_is_removed_and_its_pseudo_can_be_taken() {
        let dir = TempDir::new("stale");
        let mut a = Lobby::join(dir.path(), 0).unwrap();
        // A game that was killed: its file is there, nobody holds a lock on it, and it is old.
        let ghost = dir.path().join("GhostPlayer");
        std::fs::write(&ghost, "playing\n").unwrap();
        let old = std::time::SystemTime::now() - Duration::from_secs(60);
        std::fs::File::options().write(true).open(&ghost).unwrap().set_modified(old).unwrap();
        a.poll_at(later(1));
        assert!(a.others().is_empty(), "{:?}", a.others());
        assert!(!ghost.exists(), "the stale file was not removed");
        // A file just created and not locked yet is left alone.
        let fresh = dir.path().join("FreshPlayer");
        std::fs::write(&fresh, "menu\n").unwrap();
        a.poll_at(later(2));
        assert!(fresh.exists());
    }

    #[test]
    fn a_stale_file_of_a_pseudo_is_reused_by_a_new_game() {
        let dir = TempDir::new("reuse");
        let first = PSEUDOS.lines().next().unwrap().trim();
        std::fs::write(dir.path().join(first), "duel Someone\n").unwrap();
        let a = Lobby::join(dir.path(), 0).unwrap();
        assert_eq!(a.name(), first);
    }

    #[test]
    fn a_name_or_a_line_from_outside_cannot_reach_the_terminal() {
        let dir = TempDir::new("hostile");
        let mut a = Lobby::join(dir.path(), 0).unwrap();
        // A locked file whose name holds control characters is ignored; so is a dotfile.
        let held: Vec<std::fs::File> = ["bad\x1b[2Jname", ".hidden", "way-too-long-a-name-for-a-pseudo"].iter()
            .map(|name| {
                let file = std::fs::File::create(dir.path().join(name)).unwrap();
                assert!(unix::try_lock(&file, true));
                file
            })
            .collect();
        let b = Lobby::join(dir.path(), 40).unwrap();
        // An unreadable state line counts as the menu; a hostile duel name is cleaned.
        assert_eq!(State::parse("\x1b[2J???"), State::Menu);
        assert_eq!(State::parse("waiting x"), State::Menu);
        assert_eq!(State::parse("duel \x1b[2JEve"), State::Duel("[2JEve".to_string()));
        a.poll_at(later(1));
        assert_eq!(a.others().len(), 1);
        assert_eq!(a.others()[0].name, b.name());
        drop(held);
    }

    #[test]
    fn the_folder_must_exist_and_a_full_lobby_says_so() {
        assert!(Lobby::join(Path::new("/nonexistent/crackattack-lobby"), 0).is_err());
        let dir = TempDir::new("full");
        let held: Vec<Lobby> = (0..PSEUDOS.lines().filter(|l| !l.trim().is_empty()).count())
            .map(|i| Lobby::join(dir.path(), i).unwrap())
            .collect();
        let err = Lobby::join(dir.path(), 0).err().expect("a 101st player got in");
        assert!(err.to_string().contains("taken"));
        drop(held);
    }

    #[test]
    fn states_survive_the_file() {
        for state in [State::Menu, State::Playing, State::Waiting(1234), State::Duel("Eve".to_string())] {
            assert_eq!(State::parse(&state.encode()), state);
        }
    }
}
