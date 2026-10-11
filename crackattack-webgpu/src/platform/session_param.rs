//! Boot parameters: `--set session:…` and `--set profile:…`.
//!
//! Unlike `config:` (an overlay on a persistable option, see `persist::ConfigKey`), these are
//! ephemeral launch parameters. They shadow nothing in `state.json`. They configure startup
//! (seed, autoplay, QA, window, network, platform profile).
//!
//! `boot_config::resolve(cli, overrides, env)` merges these overrides with the legacy flags
//! into a single `BootConfig`; the generic `--set` form wins. `main` passes the result on
//! explicitly. `--set session:game.seed=42` is equivalent to `--seed 42`.
//!
//! This module is pure (no globals, no I/O). It parses a list of specs into a typed
//! `BootOverrides`, which can be tested in isolation. `cli::apply()` applies the result.

use super::cli::NetworkMode;

/// Accepted `session:` keys, used in error messages and help. Keep in sync with
/// `set_session` below.
pub const SESSION_KEYS: &[&str] = &[
    "game.seed", "autoplay", "autoplay.max_rounds", "qa.max_frames", "qa.render_dump",
    "qa.fixed_step", "qa.script", "qa.record", "qa.capture_dir", "window.position", "vs.difficulty",
    "network.host", "network.port", "audio.sounds_dir", "audio.music_dir", "audio.rungs_dir",
    "audio.music_debug", "audio.preview",
];

/// AI difficulty: `easy|medium|hard` maps to `0|1|2`.
fn parse_difficulty(v: &str) -> Result<u8, String> {
    match v {
        "easy" => Ok(0),
        "medium" => Ok(1),
        "hard" => Ok(2),
        _ => Err(format!("difficulty \"{v}\" (expected easy|medium|hard)")),
    }
}

pub(crate) fn parse_bool(v: &str) -> Result<bool, String> {
    match v {
        "true" | "1" | "on" | "yes" => Ok(true),
        "false" | "0" | "off" | "no" => Ok(false),
        _ => Err(format!("boolean \"{v}\" (expected true/false)")),
    }
}

/// Boot parameters accumulated from the `--set session:…` / `profile:…` specs.
/// `None` means not provided: the legacy flag or the default applies.
#[derive(Debug, Default, PartialEq)]
pub struct BootOverrides {
    /// `profile:platform`: `Some(true)` is Steam Deck, `Some(false)` is Desktop.
    pub platform_deck: Option<bool>,
    pub seed: Option<u64>,
    pub autoplay: Option<bool>,
    pub autoplay_rounds: Option<u32>,
    pub max_frames: Option<u32>,
    /// `qa.fixed_step`: fixed-step clock (1 tick per frame) for reproducible runs.
    pub fixed_step: Option<bool>,
    pub render_dump: Option<String>,
    /// `qa.script`: path of a test miniscript (`.mini`). Enables the fixed-step clock.
    pub script: Option<String>,
    /// `qa.record`: path where the session inputs are recorded as `.mini`. Fixed step.
    pub record: Option<String>,
    /// `qa.capture_dir`: folder for the named miniscript screenshots (relative to the cwd).
    /// Default: `<Pictures>/Crack Attack/qa/`.
    pub capture_dir: Option<String>,
    pub window_position: Option<(i32, i32)>,
    pub vs_difficulty: Option<u8>,
    pub net_host: Option<String>,
    pub net_port: Option<u16>,
    /// `audio.sounds_dir`: sound set to test. Sound effects found in this folder replace the
    /// default ones, file by file (shorthand: `--sounds DIR`).
    pub sounds_dir: Option<String>,
    /// `audio.music_dir`: music to test. Tracks found in this folder (`prelude.ogg`,
    /// `game.ogg`, …) replace the default ones (shorthand: `--music DIR`).
    pub music_dir: Option<String>,
    /// `audio.rungs_dir`: adaptive music ladder to test (a folder with `rungs.json` and the
    /// rung tracks), used instead of `music/adaptive/` (shorthand: `--rungs DIR`).
    pub rungs_dir: Option<String>,
    /// `audio.music_debug`: logs the stress and the rung every second
    /// (shorthand: `--music-debug`).
    pub music_debug: Option<bool>,
    /// `audio.preview`: plays every sound effect once before the window opens
    /// (shorthand: `--sound-preview`).
    pub sound_preview: Option<bool>,
    /// Parse errors (unknown key, badly typed value). They are reported and never applied.
    pub errors: Vec<String>,
}

impl BootOverrides {
    /// Accumulates the `session:` and `profile:` specs of a `--set` list. `config:` specs are
    /// ignored here (the overlay consumes them, see `persist::UserConfig::apply_cli_set_overrides`).
    /// An unknown namespace is left to the overlay, which reports it.
    pub fn from_specs(specs: &[String]) -> Self {
        let mut o = BootOverrides::default();
        for spec in specs {
            let Some((id, value)) = spec.split_once('=') else { continue };
            match id.split_once(':') {
                Some(("session", key)) => o.set_session(key, value),
                Some(("profile", key)) => o.set_profile(key, value),
                _ => {} // config: or unknown namespace: not handled here
            }
        }
        o
    }

    fn set_session(&mut self, key: &str, value: &str) {
        let res: Result<(), String> = (|| {
            match key {
                "game.seed" => self.seed = Some(value.parse().map_err(|_| format!("integer \"{value}\""))?),
                "autoplay" => self.autoplay = Some(parse_bool(value)?),
                "autoplay.max_rounds" => self.autoplay_rounds = Some(value.parse().map_err(|_| format!("integer \"{value}\""))?),
                "qa.max_frames" => self.max_frames = Some(value.parse().map_err(|_| format!("integer \"{value}\""))?),
                "qa.render_dump" => self.render_dump = Some(value.to_string()),
                "qa.fixed_step" => self.fixed_step = Some(parse_bool(value)?),
                "qa.script" => self.script = Some(value.to_string()),
                "qa.record" => self.record = Some(value.to_string()),
                "qa.capture_dir" => self.capture_dir = Some(value.to_string()),
                "window.position" => self.window_position = Some(parse_position(value)?),
                "vs.difficulty" => self.vs_difficulty = Some(parse_difficulty(value)?),
                "network.host" => self.net_host = Some(value.to_string()),
                "network.port" => self.net_port = Some(value.parse().map_err(|_| format!("port \"{value}\""))?),
                "audio.sounds_dir" => self.sounds_dir = Some(value.to_string()),
                "audio.music_dir" => self.music_dir = Some(value.to_string()),
                "audio.rungs_dir" => self.rungs_dir = Some(value.to_string()),
                "audio.music_debug" => self.music_debug = Some(parse_bool(value)?),
                "audio.preview" => self.sound_preview = Some(parse_bool(value)?),
                _ => return Err(format!("unknown session key \"{key}\" (keys: {})", SESSION_KEYS.join(", "))),
            }
            Ok(())
        })();
        if let Err(e) = res {
            self.errors.push(format!("session:{key} : {e}"));
        }
    }

    fn set_profile(&mut self, key: &str, value: &str) {
        match key {
            "platform" => match value {
                "steamdeck" | "deck" => self.platform_deck = Some(true),
                "desktop" => self.platform_deck = Some(false),
                _ => self.errors.push(format!("profile:platform: value \"{value}\" (expected steamdeck|desktop)")),
            },
            _ => self.errors.push(format!("unknown profile:{key}")),
        }
    }

    /// Network mode derived from the `session:network.*` parameters, when they are enough alone.
    /// - `network.host` present: **Join** (port is `network.port` or `preset::DEFAULT_NET_PORT`).
    /// - `network.port` alone: ambiguous (join or create?), so this returns an error. Give a
    ///   host, or use `--vs network-create PORT` (hosting on a fixed port is a preset shortcut).
    /// - neither: `Ok(None)`.
    pub fn network_mode(&self) -> Result<Option<NetworkMode>, String> {
        match (&self.net_host, self.net_port) {
            (Some(host), port) => Ok(Some(NetworkMode::Join {
                host: host.clone(),
                port: port.unwrap_or(crate::platform::preset::DEFAULT_NET_PORT),
            })),
            (None, Some(_)) => Err(
                "session:network.port alone is ambiguous: set session:network.host (join) or \
                 use --vs network-create PORT (host)".to_string()),
            (None, None) => Ok(None),
        }
    }
}

fn parse_position(v: &str) -> Result<(i32, i32), String> {
    let (x, y) = v.split_once(',').ok_or_else(|| format!("position \"{v}\" (expected X,Y)"))?;
    let x = x.trim().parse().map_err(|_| format!("X \"{x}\""))?;
    let y = y.trim().parse().map_err(|_| format!("Y \"{y}\""))?;
    Ok((x, y))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn specs(v: &[&str]) -> Vec<String> { v.iter().map(|s| s.to_string()).collect() }

    #[test]
    fn parses_scalar_session_params() {
        let o = BootOverrides::from_specs(&specs(&[
            "session:game.seed=42",
            "session:autoplay=true",
            "session:autoplay.max_rounds=3",
            "session:qa.max_frames=100",
            "session:qa.render_dump=/tmp/dump.jsonl",
            "session:window.position=60,80",
            "session:vs.difficulty=hard",
        ]));
        assert!(o.errors.is_empty(), "{:?}", o.errors);
        assert_eq!(o.seed, Some(42));
        assert_eq!(o.autoplay, Some(true));
        assert_eq!(o.autoplay_rounds, Some(3));
        assert_eq!(o.max_frames, Some(100));
        assert_eq!(o.render_dump.as_deref(), Some("/tmp/dump.jsonl"));
        assert_eq!(o.window_position, Some((60, 80)));
        assert_eq!(o.vs_difficulty, Some(2));
    }

    #[test]
    fn fixed_step_flag_parses() {
        assert_eq!(BootOverrides::from_specs(&specs(&["session:qa.fixed_step=true"])).fixed_step, Some(true));
        assert_eq!(BootOverrides::from_specs(&specs(&["session:qa.fixed_step=0"])).fixed_step, Some(false));
    }

    #[test]
    fn profile_platform_maps_both_ways() {
        assert_eq!(BootOverrides::from_specs(&specs(&["profile:platform=steamdeck"])).platform_deck, Some(true));
        assert_eq!(BootOverrides::from_specs(&specs(&["profile:platform=deck"])).platform_deck, Some(true));
        assert_eq!(BootOverrides::from_specs(&specs(&["profile:platform=desktop"])).platform_deck, Some(false));
        assert!(!BootOverrides::from_specs(&specs(&["profile:platform=phone"])).errors.is_empty());
    }

    #[test]
    fn config_specs_are_ignored_here() {
        // Config overrides belong to the overlay, not to the boot parameters.
        let o = BootOverrides::from_specs(&specs(&["config:player.name=Thib", "config:audio.music_volume=0.2"]));
        assert_eq!(o, BootOverrides::default());
    }

    #[test]
    fn network_host_becomes_join() {
        let o = BootOverrides::from_specs(&specs(&["session:network.host=192.168.1.5", "session:network.port=45000"]));
        match o.network_mode().unwrap() {
            Some(NetworkMode::Join { host, port }) => { assert_eq!(host, "192.168.1.5"); assert_eq!(port, 45000); }
            other => panic!("expected Join, got {other:?}"),
        }
    }

    #[test]
    fn network_host_default_port() {
        let o = BootOverrides::from_specs(&specs(&["session:network.host=10.0.0.1"]));
        assert!(matches!(o.network_mode().unwrap(), Some(NetworkMode::Join { port: 45678, .. })));
    }

    #[test]
    fn network_port_alone_is_ambiguous() {
        let o = BootOverrides::from_specs(&specs(&["session:network.port=45000"]));
        assert!(o.network_mode().is_err());
    }

    #[test]
    fn bad_values_are_collected_not_applied() {
        let o = BootOverrides::from_specs(&specs(&[
            "session:game.seed=abc",
            "session:vs.difficulty=insane",
            "session:window.position=nope",
            "session:unknown.key=1",
        ]));
        assert_eq!(o.seed, None);
        assert_eq!(o.vs_difficulty, None);
        assert_eq!(o.window_position, None);
        assert_eq!(o.errors.len(), 4, "{:?}", o.errors);
    }
}
