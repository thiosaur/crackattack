//! Command-line parsing and application.
//!
//! `parse_from` (pure) produces `CliArgs`. `boot_config::resolve` (pure) decides
//! precedence. `apply` runs the named effects and returns the `BootConfig`, which `main`
//! passes explicitly to the rest of the app (AppState, SchedulingState, DebugState,
//! ScreenshotCapture, SessionConfig). No global state is involved.

/// Network mode
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NetworkMode {
    /// Host a game (server), optional port
    Create { port: u16 },
    /// Join a game (client), host + optional port
    Join { host: String, port: u16 },
}

#[derive(Default)]
pub struct CliArgs {
    pub render_dump: Option<String>,
    pub max_frames: Option<u32>,
    pub seed: Option<u64>,
    /// `--autoplay`.
    pub autoplay: bool,
    /// `--autoplay-rounds N`.
    pub autoplay_rounds: Option<u32>,
    /// `--autoplay-ai classic|easy|medium|hard`: which brain plays under `--autoplay`.
    pub autoplay_ai: Option<String>,
    /// `--script-dry-run`.
    pub script_dry_run: bool,
    pub vs_computer: bool,
    pub ai_difficulty: Option<String>,
    pub network: Option<NetworkMode>,
    pub window_position: Option<(i32, i32)>,
    /// `--open <id>` targets (menu:, screen:, flow:). Validated at resolution time (bootstrap).
    pub open_targets: Vec<String>,
    /// Raw `--set <id>=<value>` overrides. Validated at resolution time (bootstrap).
    pub set_overrides: Vec<String>,
    /// Forces the platform profile: `Some(true)` = Steam Deck (`--deck`), `Some(false)` = Desktop (`--desktop`).
    pub platform_override: Option<bool>,
    pub help: bool,
    /// `--version`: prints the version and the stamped origin on stdout. The release
    /// pipeline checks this output on the final artifact.
    pub version: bool,
}

/// Checks a legacy difficulty value. An unknown value is fatal: silently falling back to
/// "medium" would change a QA scenario without saying so. The generic
/// `session:vs.difficulty` path (session_param.rs) also rejects unknown values.
fn check_difficulty(s: &str) -> Result<(), String> {
    if !matches!(s, "easy" | "medium" | "hard") {
        return Err(format!("--difficulty: '{s}' unknown (values: easy, medium, hard)"));
    }
    Ok(())
}

/// Merges a preset expansion into the accumulated arguments.
fn merge_preset(cli: &mut CliArgs, exp: crate::platform::preset::PresetExpansion) {
    cli.open_targets.extend(exp.open_targets);
    cli.set_overrides.extend(exp.set_overrides);
    if let Some(net) = exp.network {
        cli.network = Some(net);
    }
}

pub fn parse_args() -> CliArgs {
    let args: Vec<String> = std::env::args().collect();
    match parse_from(&args[1..]) {
        Ok(cli) => cli,
        Err(e) => {
            eprintln!("Error: {e}");
            std::process::exit(1);
        }
    }
}

/// Pure parser: no global writes, no I/O. A list of arguments (without argv[0]) becomes a
/// complete `CliArgs` or an error. Testable in isolation, like `BootOverrides::from_specs`.
pub fn parse_from(args: &[String]) -> Result<CliArgs, String> {
    let mut cli = CliArgs::default();
    let mut i = 0;

    while i < args.len() {
        match args[i].as_str() {
            "--render-dump" | "-d" => {
                if i + 1 < args.len() {
                    cli.render_dump = Some(args[i + 1].clone());
                    i += 2;
                } else {
                    return Err("--render-dump requires a path argument".into());
                }
            }
            // Invalid values are fatal, like --set and --open: a silently ignored option
            // would skew a QA scenario without saying so.
            "--max-frames" | "-f" => {
                if i + 1 < args.len() {
                    match args[i + 1].parse() {
                        Ok(n) => cli.max_frames = Some(n),
                        Err(_) => {
                            return Err(format!("--max-frames: '{}' is not a number", args[i + 1]));
                        }
                    }
                    i += 2;
                } else {
                    return Err("--max-frames requires a number argument".into());
                }
            }
            "--seed" | "-s" => {
                if i + 1 < args.len() {
                    match args[i + 1].parse() {
                        Ok(n) => cli.seed = Some(n),
                        Err(_) => {
                            return Err(format!("--seed: '{}' is not a number (u32)", args[i + 1]));
                        }
                    }
                    i += 2;
                } else {
                    return Err("--seed requires a number argument".into());
                }
            }
            "--autoplay" => {
                cli.autoplay = true;
                i += 1;
            }
            "--autoplay-rounds" => {
                if i + 1 < args.len() {
                    match args[i + 1].parse::<u32>() {
                        // A typo must not fall back to 0 (unlimited): that is the opposite
                        // of the intent.
                        Ok(n) => cli.autoplay_rounds = Some(n),
                        Err(_) => {
                            return Err(format!("--autoplay-rounds: '{}' is not a number (0 = unlimited)", args[i + 1]));
                        }
                    }
                    i += 2;
                } else {
                    return Err("--autoplay-rounds requires a number argument".into());
                }
            }
            "--autoplay-ai" => {
                if i + 1 < args.len() {
                    match args[i + 1].as_str() {
                        "classic" | "easy" | "medium" | "hard" => {
                            cli.autoplay_ai = Some(args[i + 1].clone());
                        }
                        other => {
                            return Err(format!("--autoplay-ai: '{other}' is not classic, easy, medium or hard"));
                        }
                    }
                    i += 2;
                } else {
                    return Err("--autoplay-ai requires classic, easy, medium or hard".into());
                }
            }
            "--vs-computer" => {
                cli.vs_computer = true;
                i += 1;
            }
            "--vs" if i + 1 < args.len() && args[i + 1] == "computer" => {
                cli.vs_computer = true;
                i += 2; // skip "--vs" and "computer"
                // Check for difficulty
                if i < args.len() && !args[i].starts_with('-') {
                    check_difficulty(&args[i])?;
                    cli.ai_difficulty = Some(args[i].clone());
                    i += 1;
                }
            }
            "--difficulty" => {
                if i + 1 < args.len() {
                    check_difficulty(&args[i + 1])?;
                    cli.ai_difficulty = Some(args[i + 1].clone());
                    i += 2;
                } else {
                    return Err("--difficulty requires an argument (easy, medium, hard)".into());
                }
            }
            // Legacy aliases reuse the preset expansion (one source for host and port).
            "--vs" if i + 1 < args.len() && args[i + 1] == "network-create" => {
                match crate::platform::preset::expand("net-host", &args[i + 2..]) {
                    Ok((exp, consumed)) => { merge_preset(&mut cli, exp); i += 2 + consumed; }
                    Err(e) => return Err(format!("--vs network-create: {e}")),
                }
            }
            "--vs" if i + 1 < args.len() && args[i + 1] == "network-join" => {
                match crate::platform::preset::expand("net-join", &args[i + 2..]) {
                    Ok((exp, consumed)) => { merge_preset(&mut cli, exp); i += 2 + consumed; }
                    Err(e) => return Err(format!("--vs network-join: {e}")),
                }
            }
            "--preset" => {
                if i + 1 < args.len() {
                    match crate::platform::preset::expand(&args[i + 1], &args[i + 2..]) {
                        Ok((exp, consumed)) => { merge_preset(&mut cli, exp); i += 2 + consumed; }
                        Err(e) => return Err(format!("--preset: {e}")),
                    }
                } else {
                    return Err("--preset requires a name (solo|vs-computer|net-host|net-join)".into());
                }
            }
            "--name" | "-n" => {
                if i + 1 < args.len() {
                    // Shorthand for `--set config:player.name=...`.
                    cli.set_overrides.push(format!("config:player.name={}", args[i + 1]));
                    i += 2;
                } else {
                    return Err("--name requires a player name".into());
                }
            }
            "--position" | "-p" => {
                if i + 1 < args.len() {
                    let parts: Vec<&str> = args[i + 1].split(',').collect();
                    match (parts.len(), parts.first().and_then(|p| p.parse::<i32>().ok()),
                           parts.get(1).and_then(|p| p.parse::<i32>().ok())) {
                        (2, Some(x), Some(y)) => cli.window_position = Some((x, y)),
                        _ => {
                            return Err(format!("--position: '{}' is not in x,y format (e.g. 40,80)", args[i + 1]));
                        }
                    }
                    i += 2;
                } else {
                    return Err("--position requires an x,y value (e.g. 40,80)".into());
                }
            }
            "--deck" => {
                cli.platform_override = Some(true);
                i += 1;
            }
            "--desktop" => {
                cli.platform_override = Some(false);
                i += 1;
            }
            "--open" => {
                if i + 1 < args.len() {
                    cli.open_targets.push(args[i + 1].clone());
                    i += 2;
                } else {
                    return Err("--open requires an id (menu:/screen:/flow:…)".into());
                }
            }
            "--set" => {
                if i + 1 < args.len() {
                    cli.set_overrides.push(args[i + 1].clone());
                    i += 2;
                } else {
                    return Err("--set requires <id>=<value> (e.g. config:player.name=Thib)".into());
                }
            }
            "--script" => {
                if i + 1 < args.len() {
                    // Shorthand for `--set session:qa.script=...`.
                    cli.set_overrides.push(format!("session:qa.script={}", args[i + 1]));
                    i += 2;
                } else {
                    return Err("--script requires a .mini file path".into());
                }
            }
            "--script-dry-run" => {
                cli.script_dry_run = true;
                i += 1;
            }
            "--sounds" => {
                if i + 1 < args.len() {
                    // Shorthand for `--set session:audio.sounds_dir=...`.
                    cli.set_overrides.push(format!("session:audio.sounds_dir={}", args[i + 1]));
                    i += 2;
                } else {
                    return Err("--sounds requires a sounds folder".into());
                }
            }
            "--music" => {
                if i + 1 < args.len() {
                    // Shorthand for `--set session:audio.music_dir=...`.
                    cli.set_overrides.push(format!("session:audio.music_dir={}", args[i + 1]));
                    i += 2;
                } else {
                    return Err("--music requires a music folder".into());
                }
            }
            "--rungs" => {
                if i + 1 < args.len() {
                    // Shorthand for `--set session:audio.rungs_dir=...`.
                    cli.set_overrides.push(format!("session:audio.rungs_dir={}", args[i + 1]));
                    i += 2;
                } else {
                    return Err("--rungs requires the folder of an adaptive music ladder".into());
                }
            }
            "--music-debug" => {
                // Shorthand for `--set session:audio.music_debug=true`.
                cli.set_overrides.push("session:audio.music_debug=true".into());
                i += 1;
            }
            "--sound-preview" => {
                // Shorthand for `--set session:audio.preview=true`.
                cli.set_overrides.push("session:audio.preview=true".into());
                i += 1;
            }
            "--record" => {
                if i + 1 < args.len() {
                    // Shorthand for `--set session:qa.record=...`.
                    cli.set_overrides.push(format!("session:qa.record={}", args[i + 1]));
                    i += 2;
                } else {
                    return Err("--record requires a .mini file path".into());
                }
            }
            "--help" | "-h" => {
                cli.help = true;
                i += 1;
            }
            "--version" | "-V" => {
                cli.version = true;
                i += 1;
            }
            // An unknown argument is fatal: a lost option would skew a QA scenario without
            // saying so, and printing help with exit 0 would look like success to any
            // automation. The error suggests --help.
            _ => {
                return Err(format!("unknown argument: {} (see --help)", args[i]));
            }
        }
    }

    Ok(cli)
}

/// Validates all `--set` and `--open` entries (presets already expanded) and collects the
/// errors. `main()` calls it before any init (window, GPU, logger). An error is a clear
/// failure (stderr and exit 1), never a silently ignored option. This matters in QA
/// (`--script`, `--record`, `--render-dump`), where a lost option skews the scenario.
/// Pure: a scratch config validates `config:` values without touching the disk or globals.
pub fn validate(cli: &CliArgs) -> Vec<String> {
    let mut errors = Vec::new();

    // `--set`: form, namespace, key, value.
    let mut scratch = crate::platform::persist::UserConfig::default();
    for spec in &cli.set_overrides {
        let Some((id, value)) = spec.split_once('=') else {
            errors.push(format!("--set '{spec}': expected <ns:id>=<value>"));
            continue;
        };
        match id.split_once(':') {
            Some(("config", key_id)) => match crate::platform::persist::ConfigKey::parse(key_id) {
                Some(key) => {
                    if let Err(e) = scratch.apply_override(key, value) {
                        errors.push(format!("--set config:{key_id} : {e}"));
                    }
                }
                None => errors.push(format!(
                    "--set config:{key_id}: unknown key (keys: {})",
                    crate::platform::persist::ConfigKey::PUBLIC_IDS.join(", ")
                )),
            },
            Some(("session", _)) | Some(("profile", _)) => {} // validated as a block below
            _ => errors.push(format!("--set '{id}': unknown namespace (config:/session:/profile:)")),
        }
    }
    let boot = crate::platform::session_param::BootOverrides::from_specs(&cli.set_overrides);
    for e in &boot.errors {
        errors.push(format!("--set {e}"));
    }
    // Sound set to test: a missing folder is a clear error, not a silent fallback to the
    // default sounds.
    if let Some(dir) = &boot.sounds_dir {
        if !std::path::Path::new(dir).is_dir() {
            errors.push(format!("--sounds '{dir}': not a folder"));
        }
    }
    if let Some(dir) = &boot.music_dir {
        if !std::path::Path::new(dir).is_dir() {
            errors.push(format!("--music '{dir}': not a folder"));
        }
    }
    if let Some(dir) = &boot.rungs_dir {
        let file = std::path::Path::new(dir).join(crate::platform::adaptive_music::plan::RUNGS_FILE);
        if !file.is_file() {
            errors.push(format!("--rungs '{dir}': no {}", file.display()));
        }
    }
    if let Err(e) = boot.network_mode() {
        errors.push(format!("--set {e}"));
    }

    // `--open`: every target must parse (a silent fallback to the menu would run a
    // different scenario from the one requested).
    for id in &cli.open_targets {
        if let Err(e) = crate::platform::boot_target::BootTarget::parse(id) {
            errors.push(format!("--open {id} : {e}"));
        }
    }

    // `--script-dry-run` requires a script (so the late guard in main() is unreachable).
    if cli.script_dry_run && boot.script.is_none() {
        errors.push("--script-dry-run without --script <file.mini>".to_string());
    }

    errors
}

pub fn print_help() {
    println!("{}", crate::GAME_NAME_FULL);
    println!();
    println!("Usage: crackattack [OPTIONS]");
    println!();
    println!("With no arguments, the game starts at the main menu. All game options (mode, name,");
    println!("sound, controls, language, fullscreen) are in the menu. The shortcuts below perform");
    println!("the same actions as the menu and follow exactly the same flow.");
    println!();
    println!("Launch shortcuts:");
    println!("  --vs, --vs-computer           Start a VS Computer game");
    println!("  --difficulty <LEVEL>          AI difficulty with --vs: easy, medium or hard");
    println!("  --vs network-create [PORT]    Open VS Network as host (listens, port 45678 by default)");
    println!("  --vs network-join IP [PORT]   Open VS Network and invite this address");
    println!("  --preset <NAME> [ARGS]        Combined shortcut: solo | vs-computer [LEVEL]");
    println!("                                | net-host [PORT] | net-join HOST [PORT]");
    println!("  --open <ID>                   Start on a screen (e.g. flow:play/solo,");
    println!("                                menu:network/opponents, screen:lan)");
    println!("  --set <ID>=<VALUE>            Temporary override, never saved. Three families:");
    println!("                                config:  saved option (player.name, audio.music_volume,");
    println!("                                         video.fullscreen, i18n.language)");
    println!("                                session: launch parameter (game.seed, autoplay,");
    println!("                                         autoplay.max_rounds, qa.max_frames, qa.render_dump,");
    println!("                                         qa.fixed_step, qa.script, qa.record, window.position,");
    println!("                                         vs.difficulty, network.host, network.port)");
    println!("                                profile: platform profile (platform=steamdeck|desktop)");
    println!();
    println!("Platform:");
    println!("  -p, --position <X,Y>          Initial window position");
    println!("  --deck                        Force the Steam Deck profile (fullscreen by default)");
    println!("  --desktop                     Force the Desktop profile (overrides auto-detection)");
    println!("  -h, --help                    Show this help");
    println!("  -V, --version                 Show the version and build origin");
    println!();
    println!("Test & QA (developer tooling):");
    println!("  --autoplay                    The AI plays the game (soak/perf; Solo by default)");
    println!("  --autoplay-rounds N           Autoplay exits at the prompt after N rounds");
    println!("  --autoplay-ai <BRAIN>         Brain of the autoplay: hard (default), medium, easy or classic");
    println!("                                (all but classic are the grid AI of the VS Computer opponent)");
    println!("  -n, --name <NAME>             Force the player name (several instances, separate HOMEs)");
    println!("  -s, --seed <N>                Fixed random seed");
    println!("  -f, --max-frames <N>          Quit after N frames (with --autoplay/--render-dump)");
    println!("  -d, --render-dump <PATH>      Dump the render commands as JSONL");
    println!("  --script <PATH.mini>          Replay a test miniscript (enables the fixed step)");
    println!("  --script-dry-run              Print the script timeline without starting the game");
    println!("  --record <PATH.mini>          Record the session inputs as a replayable .mini");
    println!("  --sounds <FOLDER>             Try a sound set: the sound effects present in the");
    println!("                                folder replace the default ones, file by file");
    println!("                                (same names as crackattack-webgpu/assets/sounds/).");
    println!("                                Example: --sounds my-sounds --autoplay");
    println!("  --music <FOLDER>              Try a music set: the tracks present in the folder");
    println!("                                (prelude = menu, game, gameover, youwin; .ogg or .wav)");
    println!("                                replace the default ones");
    println!("  --rungs <FOLDER>              Try an adaptive music ladder: rungs.json and");
    println!("                                the rung tracks (default: assets/music/adaptive/)");
    println!("  --music-debug                 Log the adaptive music state (stress, rung) every second");
    println!("  --sound-preview               Play the 14 sound effects before the window opens");
}

/// Resolves the configuration (pure), applies the named effects, logs the effective
/// configuration and returns it. `main` passes it on explicitly.
pub fn apply(cli: &CliArgs) -> crate::platform::boot_config::BootConfig {
    let boot = crate::platform::session_param::BootOverrides::from_specs(&cli.set_overrides);
    for e in &boot.errors {
        // Defensive: unreachable from the binary (`validate()` has already failed as fatal).
        log::warn!("CLI: --set {e}");
    }
    // Single trace of the config overrides. They are applied on each
    // `UserConfig::load()` in debug builds (see `persist::apply_cli_set_overrides`).
    for spec in cli.set_overrides.iter().filter(|s| s.starts_with("config:")) {
        log::info!("CLI: override {spec}");
    }

    let config = crate::platform::boot_config::resolve(
        cli,
        &boot,
        &crate::platform::boot_config::EnvOverrides::from_process_env(),
    );
    apply_effects(&config);

    // Log of the effective configuration.
    if let Some(max) = config.max_frames {
        log::info!("Max frames: {}", max);
    }
    if let Some(seed) = config.seed {
        log::info!("Using seed: {}", seed);
    }
    if config.fixed_step {
        log::info!("QA fixed-step clock: 1 tick/frame (deterministic)");
    }
    if let Some(ref path) = config.script {
        log::info!("Test miniscript: {path}");
    }
    if let Some(ref path) = config.record {
        log::info!("Input recording: {path}");
    }
    match &config.network {
        Some(NetworkMode::Create { port }) => log::info!("Network mode: hosting on port {}", port),
        Some(NetworkMode::Join { host, port }) => log::info!("Network mode: joining {}:{}", host, port),
        None => {}
    }
    if config.vs_computer {
        log::info!("VS Computer mode: difficulty={}", match config.ai_difficulty {
            0 => "easy (Logic)", 2 => "hard (Cortex)", _ => "medium (Sigma)"
        });
    }
    config
}

/// The boot effects, isolated and named: platform profile initialization (it must happen
/// before the profile is first resolved) and render-dump activation.
fn apply_effects(config: &crate::platform::boot_config::BootConfig) {
    if let Some(is_deck) = config.platform_deck {
        crate::platform::profile::set_override(if is_deck {
            crate::platform::profile::PlatformProfile::SteamDeck
        } else {
            crate::platform::profile::PlatformProfile::Desktop
        });
    }
    crate::platform::profile::init();

    if let Some(ref dump_path) = config.render_dump {
        crate::renderer::gpu::render_dump::enable(dump_path);
        log::info!("RenderDump enabled: {}", dump_path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Result<CliArgs, String> {
        let v: Vec<String> = args.iter().map(|s| s.to_string()).collect();
        parse_from(&v)
    }

    // Tests of the pure parser.
    #[test]
    fn parse_from_fills_every_field() {
        let cli = parse(&[
            "--seed", "42", "--max-frames", "100", "--autoplay", "--autoplay-rounds", "3",
            "--script-dry-run", "--vs-computer", "--difficulty", "hard",
            "--render-dump", "d.jsonl", "--position", "40,80", "--deck",
            "--open", "flow:play/solo", "--set", "config:player.name=Thib",
        ]).unwrap();
        assert_eq!(cli.seed, Some(42));
        assert_eq!(cli.max_frames, Some(100));
        assert!(cli.autoplay);
        assert_eq!(cli.autoplay_rounds, Some(3));
        assert!(cli.script_dry_run);
        assert!(cli.vs_computer);
        assert_eq!(cli.ai_difficulty.as_deref(), Some("hard"));
        assert_eq!(cli.render_dump.as_deref(), Some("d.jsonl"));
        assert_eq!(cli.window_position, Some((40, 80)));
        assert_eq!(cli.platform_override, Some(true));
        assert_eq!(cli.open_targets, vec!["flow:play/solo"]);
        assert_eq!(cli.set_overrides, vec!["config:player.name=Thib"]);
        assert!(!cli.help);
    }

    /// Sound set testing: `--sounds DIR` and `--sound-preview` are shorthands for
    /// `--set session:audio.*`, resolved into the startup config.
    #[test]
    fn sound_set_aliases() {
        let cli = parse(&["--sounds", "my-sounds", "--music", "my-music", "--rungs", "ladder",
            "--music-debug", "--sound-preview", "--autoplay"]).unwrap();
        assert_eq!(cli.set_overrides, vec![
            "session:audio.sounds_dir=my-sounds",
            "session:audio.music_dir=my-music",
            "session:audio.rungs_dir=ladder",
            "session:audio.music_debug=true",
            "session:audio.preview=true",
        ]);
        let boot = crate::platform::session_param::BootOverrides::from_specs(&cli.set_overrides);
        assert!(boot.errors.is_empty(), "{:?}", boot.errors);
        assert_eq!(boot.sounds_dir.as_deref(), Some("my-sounds"));
        assert_eq!(boot.music_dir.as_deref(), Some("my-music"));
        assert_eq!(boot.sound_preview, Some(true));
        assert_eq!(boot.rungs_dir.as_deref(), Some("ladder"));
        assert_eq!(boot.music_debug, Some(true));
        assert!(parse(&["--rungs"]).is_err());
        assert!(parse(&["--sounds"]).is_err());
        assert!(parse(&["--music"]).is_err());
    }

    #[test]
    fn parse_from_sugar_aliases_become_set_overrides() {
        let cli = parse(&["--script", "a.mini", "--record", "b.mini", "--name", "Zoé"]).unwrap();
        assert_eq!(cli.set_overrides, vec![
            "session:qa.script=a.mini",
            "session:qa.record=b.mini",
            "config:player.name=Zoé",
        ]);
    }

    #[test]
    fn parse_from_errors_are_fatal_not_silent() {
        assert!(parse(&["--seed"]).is_err());
        assert!(parse(&["--seed", "abc"]).is_err());
        assert!(parse(&["--autoplay-rounds", "many"]).is_err());
        assert!(parse(&["--difficulty", "impossible"]).is_err());
        assert!(parse(&["--position", "40x80"]).is_err());
        assert!(parse(&["--preset", "nonexistent"]).is_err());
    }

    #[test]
    fn autoplay_ai_accepts_known_brains_only() {
        assert_eq!(parse(&["--autoplay-ai", "hard"]).unwrap().autoplay_ai.as_deref(), Some("hard"));
        assert!(parse(&["--autoplay-ai", "genius"]).is_err());
        assert!(parse(&["--autoplay-ai"]).is_err());
    }

    #[test]
    fn parse_from_unknown_arg_is_fatal_not_false_success() {
        // Printing help with exit 0 would make a typo look like a successful automation run.
        assert!(parse(&["--frombulateur"]).is_err());
    }

    fn cli_with_sets(sets: &[&str]) -> CliArgs {
        CliArgs { set_overrides: sets.iter().map(|s| s.to_string()).collect(), ..Default::default() }
    }

    #[test]
    fn validate_accepts_correct_sets_and_opens() {
        let mut cli = cli_with_sets(&[
            "config:player.name=Thib",
            "session:game.seed=42",
            "profile:platform=desktop",
        ]);
        cli.open_targets.push("flow:play/solo".to_string());
        assert!(validate(&cli).is_empty(), "{:?}", validate(&cli));
    }

    #[test]
    fn validate_rejects_unknown_keys_listing_candidates() {
        // Common mistake: `session:seed` instead of `session:game.seed`. The error must list
        // the valid keys so the typo is obvious.
        let errs = validate(&cli_with_sets(&["session:seed=42"]));
        assert_eq!(errs.len(), 1, "{errs:?}");
        assert!(errs[0].contains("game.seed"), "{errs:?}");

        let errs = validate(&cli_with_sets(&["config:player.nam=x"]));
        assert_eq!(errs.len(), 1, "{errs:?}");
        assert!(errs[0].contains("player.name"), "{errs:?}");
    }

    #[test]
    fn validate_rejects_bad_form_namespace_and_value() {
        assert_eq!(validate(&cli_with_sets(&["config:player.name"])).len(), 1); // no '='
        assert_eq!(validate(&cli_with_sets(&["bogus:x=1"])).len(), 1); // namespace
        assert_eq!(validate(&cli_with_sets(&["config:audio.music_volume=fort"])).len(), 1); // value
        assert_eq!(validate(&cli_with_sets(&["session:network.port=45000"])).len(), 1); // a lone port is ambiguous
    }

    #[test]
    fn validate_rejects_bad_open_target() {
        let mut cli = CliArgs::default();
        cli.open_targets.push("menu:nowhere".to_string());
        let errs = validate(&cli);
        assert_eq!(errs.len(), 1, "{errs:?}");
        assert!(errs[0].contains("--open"), "{errs:?}");
    }
}
