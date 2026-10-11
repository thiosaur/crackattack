//! What a player can send when something goes wrong: the log file and the fatal error dialog.
//!
//! The log goes to `<config>/crackattack.log` on every platform (and to stderr), so a game started
//! from the Finder or the Explorer still leaves a trace. The previous session is kept as
//! `crackattack.log.1`, so relaunching after a crash does not erase it.

use std::io::{IsTerminal, Write};

/// Log file name, in the config directory.
const LOG_FILE: &str = "crackattack.log";

/// Writes every log line both to stderr and to the log file.
struct Tee(std::fs::File);

impl Write for Tee {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let _ = std::io::stderr().write_all(buf);
        self.0.write_all(buf)?;
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.0.flush()
    }
}

/// Initializes logging (level `info`, `RUST_LOG` overrides it), then writes the version, the
/// distribution channel and the system as the first line.
pub fn init_logging(version_line: &str) {
    let env = env_logger::Env::default().default_filter_or("info");
    let mut builder = env_logger::Builder::from_env(env);
    if let Some(file) = open_log_file() {
        builder.target(env_logger::Target::Pipe(Box::new(Tee(file))));
    }
    builder.init();
    log::info!("{version_line} on {} {}", std::env::consts::OS, std::env::consts::ARCH);
}

/// Opens a fresh log file, after keeping the previous one as `.log.1`.
fn open_log_file() -> Option<std::fs::File> {
    let dir = super::persist::config_dir()?;
    std::fs::create_dir_all(&dir).ok()?;
    let path = dir.join(LOG_FILE);
    let _ = std::fs::rename(&path, dir.join(format!("{LOG_FILE}.1")));
    std::fs::File::create(&path).ok()
}

/// Stops the game on an error the player must see (no usable GPU, missing game data, SDL init).
/// The message is logged and printed on stderr; when the game was not started from a terminal
/// (Finder, Explorer, desktop launcher), a dialog shows it too, with the log location.
pub fn fatal(message: &str) -> ! {
    log::error!("Fatal error: {message}");
    eprintln!("Fatal error: {message}");
    if !std::io::stderr().is_terminal() {
        let log_hint = super::persist::config_dir()
            .map(|d| format!("\n\nDetails: {}", d.join(LOG_FILE).display()))
            .unwrap_or_default();
        let _ = sdl3::messagebox::show_simple_message_box(
            sdl3::messagebox::MessageBoxFlag::ERROR,
            crate::GAME_NAME,
            &format!("{message}{log_hint}"),
            None,
        );
    }
    std::process::exit(1);
}
