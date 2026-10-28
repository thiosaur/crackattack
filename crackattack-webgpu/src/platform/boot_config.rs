//! Resolved boot configuration.
//!
//! `resolve()` is the single place that decides precedence between the three sources:
//! legacy flags (`CliArgs`), generic overrides (`--set session:...` parsed into
//! `BootOverrides`) and the environment (`EnvOverrides`). The rule: **a generic override
//! wins over a legacy flag, and the environment is a last resort**.
//!
//! The module is pure: no globals, no I/O. The caller builds `EnvOverrides`.

use super::cli::{CliArgs, NetworkMode};
use super::session_param::BootOverrides;

/// Environment values read at boot. The caller supplies them so `resolve` stays pure.
#[derive(Debug, Default)]
pub struct EnvOverrides {
    /// `RENDER_DUMP` variable (JSONL path). Last resort after `--render-dump`
    /// and `session:qa.render_dump`.
    pub render_dump: Option<String>,
}

impl EnvOverrides {
    pub fn from_process_env() -> Self {
        Self { render_dump: std::env::var("RENDER_DUMP").ok() }
    }
}

/// The launch configuration, resolved once.
#[derive(Debug, Default, PartialEq)]
pub struct BootConfig {
    /// Quit after N frames (`None` = unlimited).
    pub max_frames: Option<u32>,
    /// Explicit seed (`None` = not provided). `Some(0)` is a valid seed.
    pub seed: Option<u64>,
    pub autoplay: bool,
    /// 0 = unlimited.
    pub autoplay_rounds: u32,
    /// `None` = the classic autoplay; otherwise the level of the grid AI.
    pub autoplay_ai: Option<crackattack_core::ai_difficulty::AIDifficulty>,
    pub vs_computer: bool,
    /// 0 = easy, 1 = medium (default), 2 = hard.
    pub ai_difficulty: u8,
    /// QA fixed-step clock. A script or a recording implies it,
    /// unless it is refused explicitly (`session:qa.fixed_step=false`).
    pub fixed_step: bool,
    pub render_dump: Option<String>,
    pub script: Option<String>,
    pub script_dry_run: bool,
    pub record: Option<String>,
    pub capture_dir: Option<String>,
    /// Sound set to test (`--sounds DIR`). Replaces the sounds found in DIR.
    pub sounds_dir: Option<String>,
    /// Music to test (`--music DIR`). Replaces the tracks found in DIR.
    pub music_dir: Option<String>,
    /// Adaptive music rung set to test (`--rungs DIR`).
    pub rungs_dir: Option<String>,
    /// Adaptive music log (`--music-debug`).
    pub music_debug: bool,
    /// Play the sounds before the window opens (`--sound-preview`).
    pub sound_preview: bool,
    pub network: Option<NetworkMode>,
    pub window_position: Option<(i32, i32)>,
    pub open_targets: Vec<String>,
    pub set_overrides: Vec<String>,
    /// `Some(true)` = Steam Deck, `Some(false)` = Desktop, `None` = auto-detect.
    pub platform_deck: Option<bool>,
}

/// Resolves the full precedence. `boot` is the `BootOverrides` already parsed from
/// `cli.set_overrides`. The caller builds it. `validate()` has already rejected its
/// errors as fatal, so they are ignored here.
pub fn resolve(cli: &CliArgs, boot: &BootOverrides, env: &EnvOverrides) -> BootConfig {
    let script = boot.script.clone();
    let record = boot.record.clone();

    // Fixed step: explicit value, otherwise implied by a script or a recording (determinism).
    let fixed_step = match boot.fixed_step {
        Some(v) => v,
        None => script.is_some() || record.is_some(),
    };

    // AI difficulty: the generic override wins. The legacy flag only counts with
    // vs computer (`--difficulty` alone has no effect).
    let legacy_diff = match cli.ai_difficulty.as_deref() {
        Some("easy") => 0,
        Some("hard") => 2,
        _ => 1,
    };
    let ai_difficulty = boot
        .vs_difficulty
        .or(if cli.vs_computer { Some(legacy_diff) } else { None })
        .unwrap_or(1);

    // Network: the generic form wins. Its error is already fatal through `validate()`.
    let network = match boot.network_mode() {
        Ok(Some(net)) => Some(net),
        _ => cli.network.clone(),
    };

    BootConfig {
        max_frames: boot.max_frames.or(cli.max_frames),
        seed: boot.seed.or(cli.seed),
        autoplay: boot.autoplay.unwrap_or(cli.autoplay),
        autoplay_rounds: boot.autoplay_rounds.or(cli.autoplay_rounds).unwrap_or(0),
        autoplay_ai: match cli.autoplay_ai.as_deref() {
            // The old autoplay, kept for comparisons until it is retired.
            Some("classic") => None,
            Some("easy") => Some(crackattack_core::ai_difficulty::AIDifficulty::Easy),
            Some("medium") => Some(crackattack_core::ai_difficulty::AIDifficulty::Medium),
            _ => Some(crackattack_core::ai_difficulty::AIDifficulty::Hard),
        },
        vs_computer: cli.vs_computer,
        ai_difficulty,
        fixed_step,
        render_dump: boot
            .render_dump
            .clone()
            .or_else(|| cli.render_dump.clone())
            .or_else(|| env.render_dump.clone()),
        script,
        script_dry_run: cli.script_dry_run,
        record,
        capture_dir: boot.capture_dir.clone(),
        sounds_dir: boot.sounds_dir.clone(),
        music_dir: boot.music_dir.clone(),
        rungs_dir: boot.rungs_dir.clone(),
        music_debug: boot.music_debug.unwrap_or(false),
        sound_preview: boot.sound_preview.unwrap_or(false),
        network,
        window_position: boot.window_position.or(cli.window_position),
        open_targets: cli.open_targets.clone(),
        set_overrides: cli.set_overrides.clone(),
        platform_deck: boot.platform_deck.or(cli.platform_override),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn boot_of(specs: &[&str]) -> BootOverrides {
        let v: Vec<String> = specs.iter().map(|s| s.to_string()).collect();
        BootOverrides::from_specs(&v)
    }

    fn rsl(cli: &CliArgs, specs: &[&str]) -> BootConfig {
        resolve(cli, &boot_of(specs), &EnvOverrides::default())
    }

    // Each field is tested three ways: flag alone, --set alone, both (generic wins).
    #[test]
    fn seed_generic_wins_over_legacy() {
        let cli = CliArgs { seed: Some(1), ..Default::default() };
        assert_eq!(rsl(&cli, &[]).seed, Some(1));
        assert_eq!(rsl(&CliArgs::default(), &["session:game.seed=2"]).seed, Some(2));
        assert_eq!(rsl(&cli, &["session:game.seed=2"]).seed, Some(2));
        // `--seed 0` is an explicit seed.
        assert_eq!(rsl(&CliArgs { seed: Some(0), ..Default::default() }, &[]).seed, Some(0));
        assert_eq!(rsl(&CliArgs::default(), &[]).seed, None);
    }

    #[test]
    fn max_frames_generic_wins() {
        let cli = CliArgs { max_frames: Some(10), ..Default::default() };
        assert_eq!(rsl(&cli, &[]).max_frames, Some(10));
        assert_eq!(rsl(&CliArgs::default(), &["session:qa.max_frames=20"]).max_frames, Some(20));
        assert_eq!(rsl(&cli, &["session:qa.max_frames=20"]).max_frames, Some(20));
    }

    #[test]
    fn autoplay_and_rounds_generic_wins() {
        let cli = CliArgs { autoplay: true, autoplay_rounds: Some(3), ..Default::default() };
        let c = rsl(&cli, &[]);
        assert!(c.autoplay);
        assert_eq!(c.autoplay_rounds, 3);
        // A generic override can turn a legacy flag off.
        assert!(!rsl(&cli, &["session:autoplay=false"]).autoplay);
        assert_eq!(rsl(&cli, &["session:autoplay.max_rounds=9"]).autoplay_rounds, 9);
        assert_eq!(rsl(&CliArgs::default(), &[]).autoplay_rounds, 0);
    }

    #[test]
    fn autoplay_plays_with_the_grid_ai_at_cortex_level_by_default() {
        use crackattack_core::ai_difficulty::AIDifficulty;
        assert_eq!(rsl(&CliArgs::default(), &[]).autoplay_ai, Some(AIDifficulty::Hard));
        let easy = CliArgs { autoplay_ai: Some("easy".into()), ..Default::default() };
        assert_eq!(rsl(&easy, &[]).autoplay_ai, Some(AIDifficulty::Easy));
        let classic = CliArgs { autoplay_ai: Some("classic".into()), ..Default::default() };
        assert_eq!(rsl(&classic, &[]).autoplay_ai, None);
    }

    #[test]
    fn fixed_step_implied_by_script_unless_refused() {
        assert!(!rsl(&CliArgs::default(), &[]).fixed_step);
        assert!(rsl(&CliArgs::default(), &["session:qa.fixed_step=true"]).fixed_step);
        assert!(rsl(&CliArgs::default(), &["session:qa.script=a.mini"]).fixed_step);
        assert!(rsl(&CliArgs::default(), &["session:qa.record=b.mini"]).fixed_step);
        // Explicit refusal: the script does not force the fixed step.
        assert!(!rsl(&CliArgs::default(),
            &["session:qa.script=a.mini", "session:qa.fixed_step=false"]).fixed_step);
    }

    #[test]
    fn render_dump_env_is_last_resort() {
        let env = EnvOverrides { render_dump: Some("env.jsonl".into()) };
        let cli = CliArgs { render_dump: Some("flag.jsonl".into()), ..Default::default() };
        assert_eq!(resolve(&CliArgs::default(), &boot_of(&[]), &env).render_dump.as_deref(),
                   Some("env.jsonl"));
        assert_eq!(resolve(&cli, &boot_of(&[]), &env).render_dump.as_deref(),
                   Some("flag.jsonl"));
        assert_eq!(resolve(&cli, &boot_of(&["session:qa.render_dump=set.jsonl"]), &env)
                   .render_dump.as_deref(), Some("set.jsonl"));
    }

    #[test]
    fn difficulty_legacy_counts_only_with_vs_computer() {
        let vs = CliArgs { vs_computer: true, ai_difficulty: Some("hard".into()), ..Default::default() };
        assert_eq!(rsl(&vs, &[]).ai_difficulty, 2);
        assert_eq!(rsl(&vs, &["session:vs.difficulty=easy"]).ai_difficulty, 0);
        // `--difficulty` without --vs-computer has no effect (default medium).
        let solo = CliArgs { ai_difficulty: Some("hard".into()), ..Default::default() };
        assert_eq!(rsl(&solo, &[]).ai_difficulty, 1);
        // The generic override is kept even without vs computer (for a later --open).
        assert_eq!(rsl(&CliArgs::default(), &["session:vs.difficulty=hard"]).ai_difficulty, 2);
    }

    #[test]
    fn network_generic_wins() {
        let cli = CliArgs {
            network: Some(NetworkMode::Create { port: 1111 }),
            ..Default::default()
        };
        assert_eq!(rsl(&cli, &[]).network, Some(NetworkMode::Create { port: 1111 }));
        let c = rsl(&cli, &["session:network.host=10.0.0.2", "session:network.port=2222"]);
        assert_eq!(c.network, Some(NetworkMode::Join { host: "10.0.0.2".into(), port: 2222 }));
    }

    #[test]
    fn position_and_platform_generic_wins() {
        let cli = CliArgs {
            window_position: Some((1, 2)),
            platform_override: Some(false),
            ..Default::default()
        };
        assert_eq!(rsl(&cli, &[]).window_position, Some((1, 2)));
        assert_eq!(rsl(&cli, &["session:window.position=30,40"]).window_position, Some((30, 40)));
        assert_eq!(rsl(&cli, &[]).platform_deck, Some(false));
        assert_eq!(rsl(&cli, &["profile:platform=steamdeck"]).platform_deck, Some(true));
    }
}
