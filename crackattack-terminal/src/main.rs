// Crack Attack! Terminal - Standalone Rust binary

mod danger;
mod duel;
mod lan;
mod lobby;
mod menu;
mod render;
mod screen;

use crackattack_core::ai_difficulty::AIDifficulty;
use crackattack_core::frame_data::{SessionEvent, SessionInterruption, SessionLifecycle, SessionResult};
use crackattack_core::vs::{GameSession, GridAutoplay, GridOpponent, SoloSession, VsComputerGameSession};
use crackattack_core::{FrameData, GameState, InputSnapshot};
use crackattack_core::input::*;
use std::io::{self, Write};
use std::time::{Duration, Instant};

use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind};
use crossterm::terminal;

/// Core game ticks per second (one step per frame).
pub(crate) const TICKS_PER_SECOND: i32 = crackattack_core::constants::STEPS_PER_SECOND;
/// How long the chain just reached stays in the side panel (ticks).
const CHAIN_SHOW_TICKS: i32 = 75;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|arg| arg == "--version" || arg == "-V") {
        println!("{}", version_line());
        return;
    }
    let options = match parse_args(&args) {
        Ok(Some(options)) => options,
        Ok(None) => {
            print_help();
            return;
        }
        Err(e) => {
            eprintln!("crackattack-terminal: {}", e);
            eprintln!("Try --help.");
            std::process::exit(2);
        }
    };
    render::SHOW_DEBUG.store(options.debug, std::sync::atomic::Ordering::Relaxed);
    render::SMOOTH_RISE.with(|smooth| smooth.set(options.smooth));
    render::COMPACT.with(|compact| compact.set(options.compact));

    let limits = Limits {
        idle: options.idle_timeout.map(Duration::from_secs),
        deadline: options.max_time.map(|seconds| Instant::now() + Duration::from_secs(seconds)),
    };
    // The menu comes up when nothing says what to play: no --solo, --vs or --autoplay.
    let use_menu = !options.headless && options.mode.is_none() && options.autoplay.is_none() && options.network.is_none();
    // The others, joined before the terminal is taken: a wrong folder is reported in the shell.
    // Without --lobby the game looks for the players of the local network (mDNS), unless
    // --no-lan says not to, or nothing is displayed; if that cannot be started, the menu
    // simply has no `Vs player`.
    let mut presence = match options.lobby.as_deref() {
        Some(dir) => Some(lobby::Presence::join(std::path::Path::new(dir), random_seed() as usize, options.name.as_deref()).unwrap_or_else(|e| {
            eprintln!("crackattack-terminal: --lobby {}: {}", dir, e);
            std::process::exit(2);
        })),
        None if (use_menu || options.network.is_some()) && !options.no_lan => lobby::Presence::join_lan(random_seed() as usize, options.name.as_deref()).ok(),
        None => None,
    };
    if let (Some(presence), Some(text)) = (presence.as_mut(), options.invite_command.as_deref()) {
        presence.set_invite(text);
    }
    // The terminal is taken once, for the menu and the games: it is given back before the
    // summary is printed.
    let mut guard = (!options.headless).then(|| {
        let guard = TerminalGuard::new();
        render::initialize();
        let _ = io::stdout().flush();
        guard
    });
    let mut start_network = options.network;
    let mut captures = Some(options.captures);
    let mut seed = options.seed.unwrap_or_else(random_seed);
    loop {
        // A duel against a player: who was picked in the list, or who picked this game.
        let (mut target, mut joined) = (None, None);
        let mode = if use_menu {
            match menu::run(&limits, options.compact, presence.as_mut(), seed) {
                menu::Outcome::Play(mode) => mode,
                menu::Outcome::Duel(chosen, room) => {
                    target = Some((chosen, room));
                    Mode::Player
                }
                menu::Outcome::Opponent(opponent) => {
                    joined = Some(opponent);
                    Mode::Player
                }
                menu::Outcome::Quit | menu::Outcome::Ending(Ending::Hangup) => return,
                menu::Outcome::Ending(ending) => {
                    drop(guard.take());
                    let _ = writeln!(io::stdout(), "{}", ending_title(ending));
                    return;
                }
            }
        } else if let Some(start) = start_network.take() {
            // A duel asked for on the command line: the same search as the menu's, until
            // somebody answers.
            let name = presence.as_ref().map_or_else(|| options.name.clone().unwrap_or_else(|| lobby::pseudo(random_seed() as usize).to_string()), |p| p.name().to_string());
            let (goal, room) = match start {
                NetworkStart::Create(port) => match duel::Waiting::bind(std::net::Ipv4Addr::UNSPECIFIED.into(), port) {
                    Ok(room) => {
                        if let Some(presence) = presence.as_ref() {
                            presence.set_state(&lobby::State::Waiting(room.port()));
                        }
                        (duel::Goal::Host, Some(room))
                    }
                    Err(e) => {
                        drop(guard.take());
                        eprintln!("crackattack-terminal: --vs network-create: port {}: {}", port, e);
                        std::process::exit(1);
                    }
                },
                NetworkStart::Join(address) => (duel::Goal::Address(address), None),
            };
            match duel::search(presence.as_mut(), &name, &limits, options.compact, seed, goal, room) {
                duel::Found::Opponent(opponent) => {
                    joined = Some(opponent);
                    Mode::Player
                }
                duel::Found::Ending(Ending::Hangup) => return,
                duel::Found::Ending(ending) => {
                    drop(guard.take());
                    let _ = writeln!(io::stdout(), "{}", ending_title(ending));
                    return;
                }
                duel::Found::Back => {
                    drop(guard.take());
                    eprintln!("crackattack-terminal: no duel started (nobody answered, or the invitation was refused)");
                    std::process::exit(1);
                }
            }
        } else {
            options.mode.unwrap_or(Mode::Solo)
        };
        let (mut session, state) = match mode {
            // Only the menu offers it, and only with a lobby.
            Mode::Player => match joined.take().map(duel::Found::Opponent).or_else(|| {
                let (presence, (chosen, room)) = (presence.as_mut()?, target.take()?);
                let name = presence.name().to_string();
                Some(duel::search(Some(presence), &name, &limits, options.compact, seed, duel::Goal::Player(&chosen), room))
            }) {
                Some(duel::Found::Opponent(opponent)) => {
                    let (session, name) = duel::session(opponent);
                    (session, lobby::State::Duel(name))
                }
                Some(duel::Found::Ending(Ending::Hangup)) => return,
                Some(duel::Found::Ending(ending)) => {
                    drop(guard.take());
                    let _ = writeln!(io::stdout(), "{}", ending_title(ending));
                    return;
                }
                // Back to the menu.
                _ => continue,
            },
            _ => (new_session(mode, seed), lobby::State::Playing),
        };
        if let Some(presence) = presence.as_ref() {
            presence.set_state(&state);
        }
        let autoplay = options.autoplay.map(|level| GridAutoplay::new(level, seed));
        let mut driver = Driver::new(autoplay, captures.take().unwrap_or_default(), matches!(mode, Mode::Versus(_) | Mode::Player));
        driver.compact_forced = options.compact;

        let (frame, ending) = if options.headless {
            run_headless(&mut *session, &mut driver, options.max_frames)
        } else {
            play(&mut *session, &mut driver, options.max_frames, &limits, presence.as_mut())
        };
        // From the menu, a game that ends (lost, or left with Q) leads back to the menu.
        if use_menu && matches!(ending, Ending::Lost | Ending::Quit) {
            seed = random_seed();
            continue;
        }
        // The connection is gone: nobody to tell, and writing to it would only fail.
        if ending == Ending::Hangup {
            return;
        }
        drop(guard.take());
        // The terminal is restored by now: the summary stays in the shell after the game.
        // Written without `println!`, which panics when the output is gone.
        let mut out = io::stdout();
        let _ = writeln!(out, "{}", summary(&frame, ending, seed));
        if let Some(written) = driver.finish(&frame) {
            match written {
                Ok((count, path)) => { let _ = writeln!(out, "  {} capture(s) written to {}", count, path); }
                Err(e) => eprintln!("  Captures not written: {}", e),
            }
        }
        return;
    }
}

/// Command-line options.
#[derive(Debug, Default)]
struct Options {
    seed: Option<u32>,
    max_frames: Option<i32>,
    debug: bool,
    smooth: bool,
    compact: bool,
    autoplay: Option<AIDifficulty>,
    headless: bool,
    captures: Captures,
    /// Seconds without a key press before the game stops (for a game served to someone else).
    idle_timeout: Option<u64>,
    /// Seconds of play, all games together, before the game stops.
    max_time: Option<u64>,
    /// `--lobby DIR`: the folder where the games of a server see each other.
    lobby: Option<String>,
    /// `--invite-command TEXT`: what to tell a friend who has no access yet, shown on the
    /// `Vs player` page (a game served over ssh, with `--lobby`).
    invite_command: Option<String>,
    /// `--name PSEUDO`: the pseudo, instead of one drawn at random.
    name: Option<String>,
    /// `--no-lan`: do not look for the players of the local network.
    no_lan: bool,
    /// `--solo` or `--vs`: play it at once, without the menu.
    mode: Option<Mode>,
    /// `--vs network-create` or `--vs network-join`: a duel against a player, at once.
    network: Option<NetworkStart>,
}

/// The port the graphical game listens on by default (`--vs network-create`).
const DEFAULT_PORT: u16 = 45678;

/// A duel started from the command line, as in the graphical game.
#[derive(Clone, Copy, Debug, PartialEq)]
enum NetworkStart {
    /// Listen on this port and wait to be invited.
    Create(u16),
    /// Invite the game at this address.
    Join(std::net::SocketAddr),
}

/// What a game is.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Mode {
    Solo,
    /// A duel against the computer, at this level.
    Versus(AIDifficulty),
    /// A duel against another player of the server (`--lobby`): chosen in the menu only.
    Player,
}

/// An optional port number after an option.
fn optional_port<'a>(args: &mut std::iter::Peekable<impl Iterator<Item = &'a str>>, option: &str) -> Result<Option<u16>, String> {
    match args.peek().copied() {
        Some(text) if !text.starts_with('-') => {
            let port = text.parse::<u16>().ok().filter(|&p| p > 0).ok_or_else(|| format!("{}: '{}' is not a port", option, text))?;
            args.next();
            Ok(Some(port))
        }
        _ => Ok(None),
    }
}

/// The duel of `network-create [PORT]` or `network-join HOST [PORT]`; `HOST` is an IP address
/// or a name that resolves to one.
fn network_start<'a>(join: bool, args: &mut std::iter::Peekable<impl Iterator<Item = &'a str>>, option: &str) -> Result<NetworkStart, String> {
    use std::net::ToSocketAddrs;
    if !join {
        let port = optional_port(args, option)?;
        return Ok(NetworkStart::Create(port.unwrap_or(DEFAULT_PORT)));
    }
    let host = args.next().filter(|h| !h.starts_with('-')).ok_or_else(|| format!("{}: joining expects a host", option))?;
    let port = optional_port(args, option)?.unwrap_or(DEFAULT_PORT);
    // The games listen on IPv4: a name that resolves to both (localhost) goes there first.
    let address = (host, port).to_socket_addrs().ok().and_then(|found| {
        let found: Vec<_> = found.collect();
        found.iter().copied().find(|a| a.is_ipv4()).or_else(|| found.first().copied())
    })
        .ok_or_else(|| format!("{}: '{}' is not an address that can be reached", option, host))?;
    Ok(NetworkStart::Join(address))
}

/// The graphical game's other ways to say the same things, written as the options of this
/// program: `--vs-computer`, `--vs computer`, `--open ID` (only the screens that make sense in a
/// terminal) and `--set ID=VALUE` (only the settings that have an option here).
fn expand_aliases(args: &[String]) -> Result<Vec<String>, String> {
    let mut out: Vec<String> = Vec::new();
    let (mut host, mut port): (Option<String>, Option<String>) = (None, None);
    let mut i = 0;
    while i < args.len() {
        let arg = args[i].as_str();
        let next = args.get(i + 1).map(String::as_str);
        match arg {
            "--vs-computer" => out.push("--vs".into()),
            "--vs" if next == Some("computer") => {
                out.push("--vs".into());
                i += 1;
            }
            "--open" => {
                let id = next.ok_or("--open expects an id (flow:play/solo, flow:play/vs-computer, flow:network/host)")?;
                match id {
                    "flow:play/solo" => out.push("--solo".into()),
                    "flow:play/vs-computer" => out.push("--vs".into()),
                    "flow:network/host" => out.extend(["--vs".into(), "network-create".into()]),
                    _ => match id.strip_prefix("flow:play/vs-computer:") {
                        Some(level) => out.extend(["--vs".into(), "--difficulty".into(), level.into()]),
                        None => return Err(format!("--open: '{}' has no meaning in a terminal (flow:play/solo, flow:play/vs-computer[:LEVEL], flow:network/host)", id)),
                    },
                }
                i += 1;
            }
            "--set" => {
                let spec = next.ok_or("--set expects ID=VALUE (e.g. config:player.name=Thib)")?;
                let (key, value) = spec.split_once('=').ok_or_else(|| format!("--set: '{}' is not ID=VALUE", spec))?;
                match key {
                    "config:player.name" => out.extend(["--name".into(), value.into()]),
                    "session:game.seed" => out.extend(["--seed".into(), value.into()]),
                    "session:vs.difficulty" => out.extend(["--difficulty".into(), value.into()]),
                    "session:qa.max_frames" => out.extend(["--max-frames".into(), value.into()]),
                    "session:autoplay" => match value {
                        "true" | "1" | "yes" | "on" => out.push("--autoplay".into()),
                        "false" | "0" | "no" | "off" => {}
                        _ => return Err(format!("--set session:autoplay: '{}' is not true or false", value)),
                    },
                    "session:network.host" => host = Some(value.to_string()),
                    "session:network.port" => port = Some(value.to_string()),
                    _ => return Err(format!("--set: '{}' has no meaning in this terminal game (config:player.name, session:game.seed, session:vs.difficulty, session:qa.max_frames, session:autoplay, session:network.host/port)", key)),
                }
                i += 1;
            }
            _ => out.push(arg.to_string()),
        }
        i += 1;
    }
    match (host, port) {
        (Some(host), port) => {
            out.extend(["--vs".into(), "network-join".into(), host]);
            out.extend(port);
        }
        (None, Some(_)) => return Err("--set session:network.port alone is ambiguous: add session:network.host".into()),
        (None, None) => {}
    }
    Ok(out)
}

/// The options, `None` for `--help`, or what is wrong with them.
fn parse_args(args: &[String]) -> Result<Option<Options>, String> {
    let args = &expand_aliases(args)?;
    let mut options = Options::default();
    let mut args = args.iter().map(String::as_str).peekable();
    // The levels come apart from the options they go with, as in the graphical game.
    let (mut difficulty, mut autoplay_ai): (Option<AIDifficulty>, Option<AIDifficulty>) = (None, None);
    while let Some(arg) = args.next() {
        // What comes next, for the options that take a word that may be an option.
        let upcoming = args.peek().copied();
        let mut value = |what: &str| args.next().ok_or_else(|| format!("{} expects {}", arg, what));
        match arg {
            "--seed" | "-s" => {
                let n = value("a number")?;
                options.seed = Some(n.parse().map_err(|_| format!("--seed: '{}' is not a number", n))?);
            }
            "--max-frames" | "-f" => {
                let n = value("a number of frames")?;
                options.max_frames = Some(
                    n.parse().ok().filter(|&n: &i32| n > 0)
                        .ok_or_else(|| format!("{}: '{}' is not a positive number", arg, n))?,
                );
            }
            "--debug" => options.debug = true,
            "--smooth" => options.smooth = true,
            "--compact" => options.compact = true,
            "--autoplay" => options.autoplay = Some(AIDifficulty::Hard),
            "--autoplay-ai" => {
                let level = value("easy, medium or hard")?;
                autoplay_ai = Some(parse_level(level).ok_or_else(|| format!("--autoplay-ai: '{}' is not easy, medium or hard", level))?);
            }
            "--difficulty" => {
                let level = value("easy, medium or hard")?;
                difficulty = Some(parse_level(level).ok_or_else(|| format!("--difficulty: '{}' is not easy, medium or hard", level))?);
            }
            "--headless" => options.headless = true,
            "--no-lan" => options.no_lan = true,
            "--name" | "-n" => {
                let name = value("a pseudo")?;
                if !lobby::valid_name(name) {
                    return Err(format!("--name: '{}' is not a pseudo (up to {} letters, digits, . _ -)", name, render::versus::NAME_MAX));
                }
                options.name = Some(name.to_string());
            }
            "--invite-command" => options.invite_command = Some(value("a text")?.to_string()),
            "--lobby" => options.lobby = Some(value("a folder")?.to_string()),
            "--solo" => options.mode = Some(Mode::Solo),
            "--vs" if matches!(upcoming, Some("network-create" | "network-join")) => {
                args.next();
                options.network = Some(network_start(upcoming == Some("network-join"), &mut args, "--vs")?);
            }
            // The combined shortcut of the graphical game: solo | vs-computer [LEVEL]
            // | net-host [PORT] | net-join HOST [PORT].
            "--preset" => {
                let name = args.next().ok_or("--preset expects solo, vs-computer, net-host or net-join")?;
                match name {
                    "solo" => options.mode = Some(Mode::Solo),
                    "vs-computer" => {
                        let level = match args.peek().copied() {
                            Some(text) if !text.starts_with('-') => {
                                args.next();
                                parse_level(text).ok_or_else(|| format!("--preset vs-computer: '{}' is not easy, medium or hard", text))?
                            }
                            _ => AIDifficulty::Hard,
                        };
                        options.mode = Some(Mode::Versus(level));
                    }
                    "net-host" | "net-join" => options.network = Some(network_start(name == "net-join", &mut args, "--preset")?),
                    other => return Err(format!("--preset: unknown '{}' (solo | vs-computer [LEVEL] | net-host [PORT] | net-join HOST [PORT])", other)),
                }
            }
            "--vs" => options.mode = Some(Mode::Versus(AIDifficulty::Hard)),
            "--idle-timeout" => {
                let n = value("a number of seconds")?;
                options.idle_timeout = Some(
                    n.parse().ok().filter(|&n: &u64| n > 0)
                        .ok_or_else(|| format!("--idle-timeout: '{}' is not a positive number of seconds", n))?,
                );
            }
            "--max-time" => {
                let n = value("a number of seconds")?;
                options.max_time = Some(
                    n.parse().ok().filter(|&n: &u64| n > 0)
                        .ok_or_else(|| format!("--max-time: '{}' is not a positive number of seconds", n))?,
                );
            }
            "--capture" => {
                let list = value("ticks such as 100,250,end")?;
                let (ticks, at_end) = parse_capture_ticks(list)
                    .ok_or_else(|| format!("--capture: '{}' is not a list such as 100,250,end", list))?;
                options.captures.ticks = ticks;
                options.captures.at_end = at_end;
            }
            "--capture-file" => options.captures.path = value("a path")?.to_string(),
            "--help" | "-h" => return Ok(None),
            _ => return Err(format!("unknown option '{}'", arg)),
        }
    }
    if let Some(level) = difficulty {
        match options.mode {
            Some(Mode::Versus(_)) => options.mode = Some(Mode::Versus(level)),
            _ => return Err("--difficulty goes with --vs".to_string()),
        }
    }
    if let Some(level) = autoplay_ai {
        if options.autoplay.is_none() {
            return Err("--autoplay-ai goes with --autoplay".to_string());
        }
        options.autoplay = Some(level);
    }
    if options.headless && (options.idle_timeout.is_some() || options.max_time.is_some()) {
        return Err("--idle-timeout and --max-time need a display (not --headless)".to_string());
    }
    if options.network.is_some() && (options.headless || options.lobby.is_some() || options.mode.is_some() || options.autoplay.is_some()) {
        return Err("--vs network-create and network-join stand alone (no --headless, --lobby, --solo, --vs or --autoplay)".to_string());
    }
    if options.invite_command.is_some() && options.lobby.is_none() {
        return Err("--invite-command needs --lobby".to_string());
    }
    if options.no_lan && options.lobby.is_some() {
        return Err("--no-lan and --lobby exclude each other".to_string());
    }
    if options.headless && options.lobby.is_some() {
        return Err("--lobby needs a display (not --headless)".to_string());
    }
    if options.headless && matches!(options.mode, Some(Mode::Versus(_))) && options.autoplay.is_none() && options.max_frames.is_none() {
        // Nobody would press Enter at the ready prompt: the duel would never start.
        return Err("--headless --vs needs --autoplay or --max-frames".to_string());
    }
    Ok(Some(options))
}

fn print_help() {
    println!("Crack Attack! Terminal (Rust)");
    println!();
    println!("Usage: crackattack-terminal [OPTIONS]");
    println!();
    println!("Options:");
    println!("  -s, --seed N  Use a specific seed (default: a new one each game)");
    println!("  -f, --max-frames N  Stop after N frames");
    println!("  --debug       Show the state of the cells under the cursor");
    println!("  --smooth      Move the stack by sixteenths of a row instead of halves");
    println!("                (smoother, but the gaps between blocks flicker)");
    println!("  --compact     One line per row: the screen fits 80x24, the shellcade canvas");
    println!("                (chosen by itself when the terminal is under 28 lines tall)");
    println!("  --solo        Play alone at once (no menu)");
    println!("  --vs          Duel against the computer at once (also --vs-computer)");
    println!("  --difficulty LEVEL  Level of the computer with --vs: easy, medium or hard (default)");
    println!("  --vs network-create [PORT]  Duel against a player: listen (port {} by default) and", DEFAULT_PORT);
    println!("                wait to be invited");
    println!("  --vs network-join IP [PORT]  Duel against a player: invite the game at this address");
    println!("  --open ID     As in the graphical game: flow:play/solo, flow:play/vs-computer[:LEVEL],");
    println!("                flow:network/host");
    println!("  --set ID=VALUE  As in the graphical game: config:player.name, session:game.seed,");
    println!("                session:vs.difficulty, session:qa.max_frames, session:autoplay,");
    println!("                session:network.host and session:network.port");
    println!("  --preset NAME [ARGS]  Same as the graphical game: solo | vs-computer [LEVEL]");
    println!("                | net-host [PORT] | net-join HOST [PORT]");
    println!("  --autoplay    The AI plays");
    println!("  --autoplay-ai LEVEL  Brain of the autoplay: hard (default), medium or easy");
    println!("  --idle-timeout SECS  Stop when no key was pressed for SECS seconds");
    println!("  --max-time SECS      Stop after SECS seconds of play, all games together");
    #[cfg(unix)]
    println!("  --lobby DIR          See the other players of a server: DIR is shared by all its games");
    println!("                (these are for a game served over ssh)");
    #[cfg(unix)]
    println!("  --invite-command TEXT  With --lobby: what a friend types to get in, shown on the page of players");
    println!("  -n, --name PSEUDO  Your pseudo in a duel (default: one drawn at random)");
    println!("  --no-lan      Do not look for the players of the local network (mDNS)");
    println!("  --headless    No display: the game runs at full speed, for tests");
    println!("                (with --autoplay; stops at game over, --max-frames or the");
    println!("                last capture)");
    println!("  --capture T1,T2,...  Text captures of the screen at these ticks;");
    println!("                'end' for the last frame");
    println!("  --capture-file PATH  Where captures go (default: {})", DEFAULT_CAPTURE_FILE);
    println!("  -V, --version  Show the version");
    println!("  -h, --help    Show this help");
    println!();
    println!("The screen is drawn in the middle of the terminal, which must be at least");
    println!("80 columns by 24 lines (80x28 or more for the full-size screen).");
    println!();
    println!("Controls:");
    for key in &render::KEYS {
        println!("  {:<19}{}", key.keys, key.action);
    }
}

/// What `--version` prints; the release pipeline checks it against the tag.
fn version_line() -> String {
    format!("crackattack-terminal {}", env!("CARGO_PKG_VERSION"))
}

const DEFAULT_CAPTURE_FILE: &str = "crackattack-captures.txt";

fn parse_level(arg: &str) -> Option<AIDifficulty> {
    match arg {
        "easy" | "logic" => Some(AIDifficulty::Easy),
        "medium" | "sigma" => Some(AIDifficulty::Medium),
        "hard" | "cortex" => Some(AIDifficulty::Hard),
        _ => None,
    }
}

/// `100,250,end` → ([100, 250], true).
fn parse_capture_ticks(list: &str) -> Option<(Vec<i32>, bool)> {
    let mut ticks = Vec::new();
    let mut at_end = false;
    for item in list.split(',').map(str::trim).filter(|i| !i.is_empty()) {
        if item == "end" {
            at_end = true;
        } else {
            ticks.push(item.parse().ok().filter(|&t: &i32| t > 0)?);
        }
    }
    ticks.sort_unstable();
    ticks.dedup();
    (!ticks.is_empty() || at_end).then_some((ticks, at_end))
}

/// Text captures requested on the command line.
#[derive(Clone, Debug)]
struct Captures {
    /// Ticks to capture, sorted.
    ticks: Vec<i32>,
    /// Also capture the last frame.
    at_end: bool,
    path: String,
}

impl Default for Captures {
    fn default() -> Self {
        Self { ticks: Vec::new(), at_end: false, path: DEFAULT_CAPTURE_FILE.to_string() }
    }
}

impl Captures {
    fn wanted(&self) -> bool {
        !self.ticks.is_empty() || self.at_end
    }
}

/// What runs each tick, with or without a display: the AI (if any), edge detection of
/// the keys, the side panel's state, and the screen captures.
struct Driver {
    ai: Option<GridAutoplay>,
    prev_state: u8,
    danger: danger::DangerAnimator,
    /// Chain just reached, and ticks left to show it.
    chain: Option<(i32, i32)>,
    ticks: i32,
    captures: Captures,
    /// Decodes every frame, so a capture shows exactly what the terminal holds.
    screen: Option<screen::Screen>,
    taken: String,
    taken_count: usize,
    hud: render::Hud,
    renderer: render::Renderer,
    /// `--compact`: compact whatever the terminal size.
    compact_forced: bool,
    /// A duel: the session has rounds, and the game over screen is the session's.
    versus: bool,
    /// How the round just played ended (`true` = the player won), until taken.
    round_result: Option<bool>,
    /// The terminal is smaller than 80x24: a message replaces the screen.
    too_small: bool,
    /// What the session said last about why it is not simply running (pause, waiting...).
    interruption: SessionInterruption,
    /// `--lobby`: what the screen shows of the other players.
    lobby: Option<render::LobbyView>,
    /// The result of a round (YOU WIN, YOU LOSE) stays on the screen until the player
    /// presses Enter, however long the session takes to go on: the message and the ticks
    /// it has been shown. Not with the autoplay, which presses nothing.
    /// The result of a round kept on screen: its message, the ticks it has been there, and the
    /// duel part of the screen as the round ended (the session moves on to the next round under
    /// it, whose starting boards would replace the final ones in the preview).
    held: Option<(render::Banner, i32, Option<render::Versus>)>,
}

/// An Enter pressed in the first moments of the result is not an answer to it: the player
/// was still pressing keys when the round ended.
const HOLD_MIN_TICKS: i32 = 40;

impl Driver {
    fn new(ai: Option<GridAutoplay>, captures: Captures, versus: bool) -> Self {
        let screen = captures.wanted().then(screen::Screen::new);
        Self {
            ai,
            prev_state: 0,
            danger: danger::DangerAnimator::new(),
            chain: None,
            ticks: 0,
            captures,
            screen,
            taken: String::new(),
            taken_count: 0,
            hud: render::Hud::default(),
            renderer: render::Renderer::new(),
            compact_forced: false,
            versus,
            round_result: None,
            too_small: false,
            interruption: SessionInterruption::None,
            lobby: None,
            held: None,
        }
    }

    /// One tick of the session from the keys held (`keys`, `INPUT_*` bits); the AI adds its own.
    ///
    /// The driver knows nothing of the opponent: it reads what the session publishes
    /// (`snapshot`, `drain_events`), whatever the mode.
    fn step(&mut self, session: &mut dyn GameSession, keys: u8) -> FrameData {
        let before = session.snapshot();
        let mut input = InputSnapshot::new();
        input.state = keys;
        // Enter answers the prompt, unless the opponent left: nobody to play again with.
        let ready_prompt = matches!(before.interruption, SessionInterruption::ReadyPrompt { opponent_present: true, .. });
        if let Some(ai) = &mut self.ai {
            if ready_prompt {
                session.signal_local_ready();
            } else if before.lifecycle == SessionLifecycle::Playing && before.interruption == SessionInterruption::None {
                if let Some(game) = session.game_state_mut() {
                    input.state |= ai.update(game);
                }
            }
        }
        // Edge detection: compute pressed_this_frame from state transitions.
        input.pressed_this_frame = input.state & !self.prev_state;
        input.prev_state = self.prev_state;
        self.prev_state = input.state;
        let enter = input.pressed_this_frame & INPUT_ADVANCE != 0;
        // Enter closes the result of the round, once it has been there a moment...
        let holding = self.held.is_some();
        if let Some((_, shown, _)) = &self.held {
            if enter && *shown >= HOLD_MIN_TICKS {
                self.held = None;
            }
        }
        // ...and answers the ready prompt after that (it raises the stack otherwise).
        if ready_prompt && enter && !holding {
            session.signal_local_ready();
        }

        let frame = session.tick(&input);
        for event in session.drain_events() {
            if let SessionEvent::RoundEnded { won } = event {
                self.round_result = Some(won);
            }
        }
        let snapshot = session.snapshot();
        let (mut versus, mut banner) = (render::Versus::from_snapshot(&snapshot), render::banner(&snapshot));
        if self.ai.is_none() {
            let result = snapshot.lifecycle == SessionLifecycle::ShowResult
                && matches!(snapshot.result, SessionResult::RoundWin | SessionResult::RoundLose);
            if let (None, true, Some(shown)) = (&self.held, result, &banner) {
                let mut held = shown.clone();
                held.below.extend([String::new(), "Enter: continue".to_string()]);
                self.held = Some((held, 0, None));
            }
            if let Some((held, shown, kept)) = &mut self.held {
                *shown += 1;
                banner = Some(held.clone());
                // The boards as the round ended: still followed while the result is the
                // session's (the opponent's last picture may come a moment after), then frozen.
                if result || kept.is_none() {
                    *kept = versus.clone();
                }
                versus = kept.clone();
            }
        }
        let countdown = render::Countdown::from_snapshot(&snapshot);
        self.interruption = snapshot.interruption;
        self.ticks += 1;
        self.danger.step(&frame.ui);
        self.chain = self.chain.map(|(n, left)| (n, left - 1)).filter(|&(_, left)| left > 0);
        for event in &frame.gameplay_events {
            if let crackattack_core::gameplay_event::GameplayEvent::ChainProgressed(e) = event {
                self.chain = Some((e.chain_level, CHAIN_SHOW_TICKS));
            }
        }
        self.hud = render::Hud { danger: self.danger.bars(), chain: self.chain.map(|(n, _)| n), game_over: None, versus, banner, countdown, lobby: self.lobby.clone() };
        frame
    }

    /// The text that brings the terminal to `frame` (or to the pause screen): only what
    /// changed. Captures decode it as the terminal would.
    fn draw(&mut self, frame: &FrameData) -> String {
        let paused = matches!(self.interruption, SessionInterruption::Paused { .. });
        let text = if paused { self.renderer.paused(frame, &self.hud) } else { self.renderer.frame(frame, &self.hud) };
        if let Some(screen) = &mut self.screen {
            screen.feed(&text);
            if !paused && self.captures.ticks.binary_search(&self.ticks).is_ok() {
                self.take(&format!("tick {}", self.ticks));
            }
        }
        text
    }

    /// Fits the screen to the terminal, at launch and after a resize of either side: the
    /// zone (80 columns, 24 or 28 lines) centered, the compact screen when the terminal is
    /// not tall enough for the normal one, a message when it is under 80x24.
    fn fit_terminal(&mut self) {
        let size = terminal::size().ok().map(|(columns, rows)| (columns as usize, rows as usize));
        let placement = render::place(size, self.compact_forced);
        match placement {
            render::Placement::Zone { origin, tall } => {
                render::COMPACT.with(|c| c.set(!tall));
                if let Some(screen) = &mut self.screen {
                    screen.set_origin(origin);
                }
            }
            render::Placement::TooSmall { .. } => {}
        }
        self.renderer.set_placement(placement);
        self.too_small = self.renderer.too_small();
    }

    /// One tick of the game-over animation: the game itself has stopped.
    fn step_game_over(&mut self) {
        self.ticks += 1;
        self.hud.game_over = Some(self.hud.game_over.map_or(0, |t| t + 1));
    }

    /// GAME OVER is fully shown.
    fn game_over_shown(&self) -> bool {
        self.hud.game_over.is_some_and(|t| t >= render::game_over_ticks())
    }

    /// Every capture tick is behind: nothing left to wait for.
    fn captures_done(&self) -> bool {
        self.captures.wanted() && !self.captures.at_end
            && self.captures.ticks.last().is_some_and(|&t| self.ticks >= t)
    }

    fn take(&mut self, title: &str) {
        if let Some(screen) = &self.screen {
            self.taken.push_str(&screen.capture(title));
            self.taken.push('\n');
            self.taken_count += 1;
        }
    }

    /// Takes the end capture if asked and writes the file; `None` without captures.
    fn finish(&mut self, last: &FrameData) -> Option<Result<(usize, String), io::Error>> {
        if !self.captures.wanted() {
            return None;
        }
        if self.captures.at_end && self.ticks > 0 {
            let title = format!("end, tick {}{}", self.ticks, if last.game_over { ", game over" } else { "" });
            self.take(&title);
        }
        let content = format!("{}{}", screen::legend(), self.taken);
        Some(std::fs::write(&self.captures.path, content).map(|_| (self.taken_count, self.captures.path.clone())))
    }
}

/// Runs the game without a display, as fast as it goes: for autoplay tests and captures.
fn run_headless(session: &mut dyn GameSession, driver: &mut Driver, max_frames: Option<i32>) -> (FrameData, Ending) {
    // Without a limit, a game nobody plays would last until the stack tops out.
    loop {
        let frame = driver.step(session, 0);
        if driver.captures.wanted() {
            driver.draw(&frame);
        }
        // A duel ends with its first round here: the series is open, nobody would stop it.
        if let Some(won) = driver.round_result.take() {
            return (frame, if won { Ending::Won } else { Ending::Lost });
        }
        if frame.game_over && !driver.versus {
            // The GAME OVER animation, for captures.
            while driver.captures.wanted() && !driver.game_over_shown() {
                driver.step_game_over();
                driver.draw(&frame);
            }
            return (frame, Ending::Lost);
        }
        if max_frames.is_some_and(|max| driver.ticks >= max) || driver.captures_done() {
            return (frame, Ending::FrameLimit);
        }
    }
}

/// A new seed for each game, from the clock: each launch deals a different board. The
/// summary prints it so a game can be replayed with `--seed`.
fn random_seed() -> u32 {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    // Fold the 128-bit count and mix it, so close launches give unrelated seeds.
    let mut x = (nanos as u64) ^ ((nanos >> 64) as u64) ^ u64::from(std::process::id());
    x ^= x >> 33;
    x = x.wrapping_mul(0xff51_afd7_ed55_8ccd);
    x ^= x >> 33;
    x as u32
}

/// Why the game loop stopped.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Ending {
    Lost,
    /// A round of a duel won (headless duels stop after one round).
    Won,
    Quit,
    FrameLimit,
    /// No key pressed for `--idle-timeout`.
    Idle,
    /// `--max-time` reached.
    TimeLimit,
    /// The terminal is gone (connection dropped).
    Hangup,
}

/// Writes a screen to the terminal; `false` when it cannot (the connection is gone).
fn present(screen: &str) -> bool {
    let mut out = io::stdout().lock();
    out.write_all(screen.as_bytes()).and_then(|()| out.flush()).is_ok()
}

/// Wall-clock limits for a game served to someone else; both are off by default.
struct Limits {
    idle: Option<Duration>,
    deadline: Option<Instant>,
}

impl Limits {
    /// The ending to apply now, if a limit is reached.
    fn reached(&self, last_activity: Instant, now: Instant) -> Option<Ending> {
        if self.deadline.is_some_and(|deadline| now >= deadline) {
            Some(Ending::TimeLimit)
        } else if self.idle.is_some_and(|idle| now.duration_since(last_activity) >= idle) {
            Some(Ending::Idle)
        } else {
            None
        }
    }
}

/// Runs the game until it ends; returns the last frame and why it stopped.
///
/// The terminal is already taken (see `main`): the menu and the games share it.
fn play(session: &mut dyn GameSession, driver: &mut Driver, max_frames: Option<i32>, limits: &Limits, mut presence: Option<&mut lobby::Presence>) -> (FrameData, Ending) {
    driver.fit_terminal();

    let frame_duration = Duration::from_millis(1000 / TICKS_PER_SECOND as u64);
    let mut last = FrameData::default();
    // Under 80x24 the game is paused if the session accepts it: we asked, so we resume.
    let mut size_pause = false;
    let mut last_activity = Instant::now();

    let ending = loop {
        let frame_start = Instant::now();

        let input = read_input();
        let keys = input.keys;
        if input.hangup {
            break Ending::Hangup;
        }
        if input.resized {
            driver.fit_terminal();
        }
        if input.activity {
            last_activity = frame_start;
        }
        // The pause belongs to the session, which accepts or refuses it, and in a network
        // game waits for both players: the loop keeps ticking it and draws what it says.
        if input.pause && !size_pause && (driver.versus || !last.game_over) {
            match driver.interruption {
                SessionInterruption::Paused { .. } => { session.request_resume(); }
                SessionInterruption::None | SessionInterruption::ReadyPrompt { .. } => { session.request_pause(); }
                _ => {}
            }
        }
        // A network session may refuse: the message then stays while the game goes on.
        let paused_by_player = matches!(driver.interruption, SessionInterruption::Paused { .. }) && !size_pause;
        if driver.too_small && !size_pause && !paused_by_player {
            size_pause = session.request_pause();
        } else if !driver.too_small && size_pause {
            size_pause = false;
            session.request_resume();
        }

        if input.quit {
            break Ending::Quit;
        }
        if let Some(ending) = limits.reached(last_activity, frame_start) {
            break ending;
        }

        if let Some(presence) = presence.as_deref_mut() {
            presence.poll();
            driver.lobby = Some(presence.view());
        }
        let frame = driver.step(session, keys);
        if !present(&driver.draw(&frame)) {
            break Ending::Hangup;
        }

        let game_over = frame.game_over;
        last = frame;

        if max_frames.is_some_and(|max| driver.ticks >= max) {
            break Ending::FrameLimit;
        }

        if game_over && !driver.versus {
            break show_game_over(driver, &last, limits).unwrap_or(Ending::Lost);
        }

        let elapsed = frame_start.elapsed();
        if elapsed < frame_duration {
            std::thread::sleep(frame_duration - elapsed);
        }
    };

    leave(session, ending);
    (last, ending)
}

/// Whether leaving the game this way is a forfeit the session must hear of: the player
/// quit, stopped answering, played too long, or the connection dropped.
fn forfeits(ending: Ending) -> bool {
    matches!(ending, Ending::Quit | Ending::Idle | Ending::TimeLimit | Ending::Hangup)
}

/// Tells the session the player is leaving, when that is a forfeit (a network opponent
/// then knows at once that it won; the other sessions do nothing).
fn leave(session: &mut dyn GameSession, ending: Ending) {
    if forfeits(ending) {
        session.abandon();
    }
}

/// Game over: the logo turns into GAME OVER, then the final board stays until a key is
/// pressed. `Some(ending)` when something else than the game ended the session meanwhile
/// (a limit, or the connection).
fn show_game_over(driver: &mut Driver, last: &FrameData, limits: &Limits) -> Option<Ending> {
    let frame_duration = Duration::from_millis(1000 / TICKS_PER_SECOND as u64);
    while !driver.game_over_shown() {
        driver.step_game_over();
        if !present(&driver.draw(last)) {
            return Some(Ending::Hangup);
        }
        std::thread::sleep(frame_duration);
    }
    // Keys pressed in the rush before the loss, or during the animation, do not count.
    while event::poll(Duration::ZERO).unwrap_or(false) {
        let _ = event::read();
    }
    let waiting_since = Instant::now();
    loop {
        // Woken up now and then to look at the limits.
        match event::poll(Duration::from_millis(250)) {
            Err(_) => return Some(Ending::Hangup),
            Ok(false) => {
                if let Some(ending) = limits.reached(waiting_since, Instant::now()) {
                    return Some(ending);
                }
                continue;
            }
            Ok(true) => {}
        }
        match event::read() {
            Ok(Event::Key(KeyEvent { kind: KeyEventKind::Press, .. })) => return None,
            Err(_) => return Some(Ending::Hangup),
            Ok(Event::Resize(..)) => {
                driver.fit_terminal();
                if !present(&driver.draw(last)) {
                    return Some(Ending::Hangup);
                }
            }
            Ok(_) => {}
        }
    }
}


/// One line saying why the game stopped.
fn ending_title(ending: Ending) -> &'static str {
    match ending {
        Ending::Lost => "Game over: the stack reached the top.",
        Ending::Won => "Round won: the computer's stack reached the top.",
        Ending::Quit => "Game stopped.",
        Ending::FrameLimit => "Frame limit reached.",
        Ending::Idle => "Game stopped: no key was pressed for a while.",
        Ending::TimeLimit => "Game stopped: time limit reached.",
        Ending::Hangup => "Game stopped: the terminal is gone.",
    }
}

/// The session that runs a game of this mode.
fn new_session(mode: Mode, seed: u32) -> Box<dyn GameSession> {
    match mode {
        Mode::Solo => Box::new(SoloSession::new(GameState::new(seed))),
        // Brought up by `duel::session`, once an opponent is found.
        Mode::Player => unreachable!("a duel against a player is not built from a seed"),
        Mode::Versus(level) => {
            let mut session = VsComputerGameSession::new(seed, GridOpponent::new(level, seed));
            // As in the graphical game: "Ready?" before round 1, and between rounds.
            session.set_needs_ready_prompt(true);
            Box::new(session)
        }
    }
}

/// End-of-game message printed in the shell: how the game ended, score, time, best chain
/// and the seed to replay the same game.
fn summary(frame: &crackattack_core::FrameData, ending: Ending, seed: u32) -> String {
    let title = ending_title(ending);
    let seconds = frame.ui.clock_time / TICKS_PER_SECOND;
    let mut text = format!(
        "{}\n  Score: {}   Time: {}:{:02}",
        title, frame.score, seconds / 60, seconds % 60,
    );
    if frame.stats.max_chain_level >= 2 {
        text.push_str(&format!("   Best chain: x{}", frame.stats.max_chain_level));
    }
    text.push_str(&format!("\n  Replay this game: crackattack-terminal --seed {}", seed));
    text
}

/// Map a crossterm KeyCode to an input bitmask. Returns 0 for unmapped keys.
fn map_key(code: KeyCode) -> u8 {
    // Letters work in either case (Caps Lock).
    let code = match code {
        KeyCode::Char(c) => KeyCode::Char(c.to_ascii_lowercase()),
        code => code,
    };
    match code {
        KeyCode::Up    | KeyCode::Char('w') => INPUT_UP,
        KeyCode::Down  | KeyCode::Char('s') => INPUT_DOWN,
        KeyCode::Left  | KeyCode::Char('a') => INPUT_LEFT,
        KeyCode::Right | KeyCode::Char('d') => INPUT_RIGHT,
        KeyCode::Char(' ')                  => INPUT_SWAP,
        KeyCode::Enter                      => INPUT_ADVANCE,
        _ => 0,
    }
}

fn is_quit_key(code: KeyCode) -> bool {
    matches!(code, KeyCode::Char('q') | KeyCode::Char('Q') | KeyCode::Esc)
}

fn is_pause_key(code: KeyCode) -> bool {
    matches!(code, KeyCode::Char('p') | KeyCode::Char('P'))
}

/// What the terminal gave since the last tick.
#[derive(Default)]
struct Input {
    /// Keys pressed, as `INPUT_*` bits.
    keys: u8,
    quit: bool,
    pause: bool,
    /// The terminal was resized: the screen must be redrawn whole.
    resized: bool,
    /// Any key was pressed, mapped or not (resets the idle timer).
    activity: bool,
    /// Reading the terminal failed: the connection is gone.
    hangup: bool,
}

fn read_input() -> Input {
    let mut input = Input::default();
    loop {
        match event::poll(Duration::ZERO) {
            Ok(true) => {}
            Ok(false) => break,
            Err(_) => {
                input.hangup = true;
                break;
            }
        }
        match event::read() {
            Ok(Event::Resize(..)) => input.resized = true,
            Ok(Event::Key(KeyEvent { code, kind: KeyEventKind::Press, .. })) => {
                input.activity = true;
                if is_quit_key(code) {
                    input.quit = true;
                }
                if is_pause_key(code) {
                    input.pause = true;
                }
                input.keys |= map_key(code);
            }
            Ok(_) => {}
            Err(_) => {
                input.hangup = true;
                break;
            }
        }
    }
    input
}

/// RAII guard: enables raw mode on construction and restores the FULL terminal state on drop:
/// raw mode, alternate screen and cursor visibility. Restoring in `Drop` means a panic in the
/// main loop still leaves the user's terminal usable.
struct TerminalGuard {
    active: bool,
}

impl TerminalGuard {
    fn new() -> Self {
        match terminal::enable_raw_mode() {
            Ok(()) => TerminalGuard { active: true },
            Err(e) => {
                eprintln!("Warning: could not enable raw mode: {}", e);
                TerminalGuard { active: false }
            }
        }
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        render::cleanup();
        let _ = io::stdout().flush();
        if self.active {
            let _ = terminal::disable_raw_mode();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(line: &str) -> Vec<String> {
        line.split_whitespace().map(String::from).collect()
    }

    #[test]
    fn options_are_parsed() {
        let options = parse_args(&args("-s 42 -f 300 --autoplay --autoplay-ai easy --headless --smooth"))
            .unwrap()
            .unwrap();
        assert_eq!(options.seed, Some(42));
        assert_eq!(options.max_frames, Some(300));
        assert_eq!(options.autoplay, Some(AIDifficulty::Easy));
        assert!(options.headless && options.smooth && !options.debug && !options.compact);
        assert!(parse_args(&args("--compact")).unwrap().unwrap().compact);
        assert_eq!(parse_args(&args("--lobby /srv/lobby")).unwrap().unwrap().lobby.as_deref(), Some("/srv/lobby"));
        assert!(parse_args(&args("")).unwrap().unwrap().lobby.is_none());
        assert!(parse_args(&args("--lobby")).is_err());
        assert_eq!(parse_args(&args("--lobby /x --invite-command ssh")).unwrap().unwrap().invite_command.as_deref(), Some("ssh"));
        assert!(parse_args(&args("--invite-command ssh")).is_err(), "the invitation belongs to a lobby");
        // Duels from the command line, as in the graphical game.
        let net = |line: &str| parse_args(&args(line)).unwrap().unwrap().network;
        assert_eq!(net("--vs network-create"), Some(NetworkStart::Create(45678)));
        assert_eq!(net("--vs network-create 5000"), Some(NetworkStart::Create(5000)));
        assert_eq!(net("--vs network-join 192.168.1.5"), Some(NetworkStart::Join("192.168.1.5:45678".parse().unwrap())));
        assert_eq!(net("--vs network-join ::1 7000"), Some(NetworkStart::Join("[::1]:7000".parse().unwrap())));
        assert_eq!(net("--vs --difficulty hard"), None);
        assert!(parse_args(&args("--vs network-join")).is_err());
        assert!(parse_args(&args("--vs network-join not-an-address.invalid")).is_err());
        assert_eq!(net("--preset net-join 127.0.0.1 7000"), Some(NetworkStart::Join("127.0.0.1:7000".parse().unwrap())));
        assert_eq!(net("--preset net-host"), Some(NetworkStart::Create(45678)));
        assert!(parse_args(&args("--preset net-join")).is_err());
        assert!(matches!(parse_args(&args("--preset vs-computer easy")).unwrap().unwrap().mode, Some(Mode::Versus(AIDifficulty::Easy))));
        assert!(matches!(parse_args(&args("--preset solo")).unwrap().unwrap().mode, Some(Mode::Solo)));
        assert!(parse_args(&args("--preset vs-computer extreme")).is_err());
        assert!(parse_args(&args("--preset nothing")).is_err());
        // The graphical game's other forms.
        assert!(matches!(parse_args(&args("--vs-computer --difficulty easy")).unwrap().unwrap().mode, Some(Mode::Versus(AIDifficulty::Easy))));
        assert!(matches!(parse_args(&args("--vs computer")).unwrap().unwrap().mode, Some(Mode::Versus(AIDifficulty::Hard))));
        assert!(matches!(parse_args(&args("--open flow:play/vs-computer:medium")).unwrap().unwrap().mode, Some(Mode::Versus(AIDifficulty::Medium))));
        assert!(matches!(parse_args(&args("--open flow:play/solo")).unwrap().unwrap().mode, Some(Mode::Solo)));
        assert_eq!(net("--open flow:network/host"), Some(NetworkStart::Create(45678)));
        assert!(parse_args(&args("--open menu:options")).is_err());
        let set = parse_args(&args("--set config:player.name=Zoe --set session:game.seed=9 --set session:qa.max_frames=50 --set session:autoplay=true")).unwrap().unwrap();
        assert_eq!((set.name.as_deref(), set.seed, set.max_frames, set.autoplay.is_some()), (Some("Zoe"), Some(9), Some(50), true));
        assert_eq!(net("--set session:network.host=127.0.0.1 --set session:network.port=7000"), Some(NetworkStart::Join("127.0.0.1:7000".parse().unwrap())));
        assert!(parse_args(&args("--set session:network.port=7000")).is_err());
        assert!(parse_args(&args("--set audio.music_volume=1")).is_err());
        assert!(parse_args(&args("--set session:autoplay=maybe")).is_err());
        assert!(parse_args(&args("--set noequals")).is_err());
        assert!(parse_args(&args("--vs network-create 0")).is_err());
        assert!(parse_args(&args("--vs network-create 99999")).is_err());
        assert!(parse_args(&args("--vs network-create --compact")).unwrap().unwrap().compact);
        assert!(parse_args(&args("--vs network-create --headless")).is_err());
        assert!(parse_args(&args("--vs network-create --lobby /x")).is_err());
        // The pseudo.
        assert_eq!(parse_args(&args("--name Zoe")).unwrap().unwrap().name.as_deref(), Some("Zoe"));
        assert_eq!(parse_args(&args("-n Zoe")).unwrap().unwrap().name.as_deref(), Some("Zoe"));
        assert!(parse_args(&args("--name")).is_err());
        assert!(parse_args(&args("--name a/b")).is_err());
        assert!(parse_args(&args("--name AVeryVeryLongPseudoName")).is_err());
        assert!(parse_args(&args("--headless --lobby /tmp/x")).is_err());
        // A level is optional: the next option is not taken for one.
        let options = parse_args(&args("--autoplay --debug")).unwrap().unwrap();
        assert_eq!(options.autoplay, Some(AIDifficulty::Hard));
        assert!(options.debug);
        assert!(parse_args(&args("--seed 1 --help")).unwrap().is_none());
    }

    #[test]
    fn version_line_names_the_workspace_version() {
        let line = version_line();
        assert!(line.starts_with("crackattack-terminal "), "{}", line);
        assert!(line.len() > "crackattack-terminal ".len());
    }

    /// One tick of a driver played by hand: `enter` presses Enter for this tick only.
    fn tick(driver: &mut Driver, session: &mut dyn GameSession, enter: bool) {
        driver.step(session, if enter { INPUT_ADVANCE } else { 0 });
    }

    #[test]
    fn the_result_of_a_round_stays_until_enter_and_enter_does_not_also_start_the_next() {
        let mut session = new_session(Mode::Versus(AIDifficulty::Hard), 5);
        let mut driver = Driver::new(None, Captures::default(), true);
        // Enter at the ready prompt, then nobody plays: the idle player's stack tops out.
        tick(&mut driver, &mut *session, true);
        let mut ticks = 0;
        while driver.held.is_none() {
            tick(&mut driver, &mut *session, false);
            ticks += 1;
            assert!(ticks < 30_000, "the round never ended");
        }
        // Far longer than the session shows it: still there, with the way out. The opponent's
        // board is the one the round ended with, not the start of the next round.
        for _ in 0..30 {
            tick(&mut driver, &mut *session, false);
        }
        let final_board = driver.hud.versus.as_ref().and_then(|v| v.preview.clone());
        assert!(final_board.is_some());
        for _ in 0..600 {
            tick(&mut driver, &mut *session, false);
            assert_eq!(driver.hud.versus.as_ref().and_then(|v| v.preview.clone()), final_board, "the preview moved under the result");
        }
        let shown = driver.hud.banner.clone().expect("the result went away on its own");
        assert!(shown.words.iter().any(|w| w == "YOU"), "{:?}", shown);
        assert!(shown.below.iter().any(|l| l.contains("Enter: continue")), "{:?}", shown);
        // Enter closes it, and shows the prompt of the next round: it does not answer it too.
        tick(&mut driver, &mut *session, true);
        tick(&mut driver, &mut *session, false);
        assert!(driver.held.is_none());
        let next = driver.hud.banner.clone().expect("no prompt for the next round");
        assert_eq!(next.above, vec!["Round".to_string()]);
        assert!(matches!(driver.interruption, SessionInterruption::ReadyPrompt { .. }), "{:?}", driver.interruption);
        // A second Enter starts it.
        tick(&mut driver, &mut *session, true);
        tick(&mut driver, &mut *session, false);
        assert!(!matches!(driver.interruption, SessionInterruption::ReadyPrompt { .. }), "{:?}", driver.interruption);
    }

    #[test]
    fn an_early_enter_does_not_dismiss_the_result_and_the_autoplay_holds_nothing() {
        let mut session = new_session(Mode::Versus(AIDifficulty::Hard), 5);
        let mut driver = Driver::new(None, Captures::default(), true);
        tick(&mut driver, &mut *session, true);
        while driver.held.is_none() {
            tick(&mut driver, &mut *session, false);
        }
        // Enter pressed over and over as the round ends (the player was still at it).
        for _ in 0..(HOLD_MIN_TICKS / 2) {
            tick(&mut driver, &mut *session, true);
            tick(&mut driver, &mut *session, false);
        }
        assert!(driver.held.is_some(), "the result went away under keys pressed at the end of the round");
        // The autoplay answers prompts by itself: nothing is held for it.
        let mut session = new_session(Mode::Versus(AIDifficulty::Easy), 9);
        let mut driver = Driver::new(Some(GridAutoplay::new(AIDifficulty::Easy, 9)), Captures::default(), true);
        for _ in 0..8000 {
            tick(&mut driver, &mut *session, false);
            assert!(driver.held.is_none());
        }
    }

    #[test]
    fn serving_options_are_parsed() {
        let options = parse_args(&args("--idle-timeout 120 --max-time 1800")).unwrap().unwrap();
        assert_eq!((options.idle_timeout, options.max_time), (Some(120), Some(1800)));
        let options = parse_args(&args("")).unwrap().unwrap();
        assert!(options.idle_timeout.is_none() && options.max_time.is_none());
        // The old --replay is gone: the menu brings a new game after a lost one.
        for line in ["--idle-timeout 0", "--idle-timeout x", "--idle-timeout", "--max-time -5", "--replay",
                     "--headless --idle-timeout 5", "--headless --max-time 5"] {
            assert!(parse_args(&args(line)).is_err(), "{}", line);
        }
    }

    #[test]
    fn limits_stop_an_idle_or_overlong_session() {
        let start = Instant::now();
        let later = |seconds| start + Duration::from_secs(seconds);
        let none = Limits { idle: None, deadline: None };
        assert_eq!(none.reached(start, later(100_000)), None);
        let idle = Limits { idle: Some(Duration::from_secs(60)), deadline: None };
        assert_eq!(idle.reached(start, later(59)), None);
        assert_eq!(idle.reached(start, later(60)), Some(Ending::Idle));
        // A key press moves the idle reference.
        assert_eq!(idle.reached(later(50), later(100)), None);
        let both = Limits { idle: Some(Duration::from_secs(60)), deadline: Some(later(300)) };
        assert_eq!(both.reached(later(290), later(299)), None);
        // The time limit wins even when keys keep coming.
        assert_eq!(both.reached(later(299), later(300)), Some(Ending::TimeLimit));
    }

    #[test]
    fn bad_options_are_errors() {
        for line in ["--seed abc", "--seed", "--max-frames abc", "--max-frames 0", "--max-frames", "--frames 5",
                     "--difficulty easy", "--difficulty extreme --vs", "--autoplay-ai easy", "--vs easy",
                     "--capture x", "--capture-file", "--sede 4", "extra"] {
            assert!(parse_args(&args(line)).is_err(), "{}", line);
        }
    }

    #[test]
    fn help_lists_every_key_of_the_panel() {
        // One table for both: the panel's short forms are among the keys --help gives.
        for key in &render::KEYS {
            assert!(key.keys.contains(key.hint_key), "{}", key.keys);
        }
    }

    #[test]
    fn letters_work_in_either_case() {
        for (lower, upper) in [('w', 'W'), ('a', 'A'), ('s', 'S'), ('d', 'D')] {
            assert_eq!(map_key(KeyCode::Char(lower)), map_key(KeyCode::Char(upper)));
        }
    }

    #[test]
    fn arrows_map_to_directions() {
        assert_eq!(map_key(KeyCode::Up), INPUT_UP);
        assert_eq!(map_key(KeyCode::Down), INPUT_DOWN);
        assert_eq!(map_key(KeyCode::Left), INPUT_LEFT);
        assert_eq!(map_key(KeyCode::Right), INPUT_RIGHT);
    }

    #[test]
    fn wasd_maps_to_directions() {
        assert_eq!(map_key(KeyCode::Char('w')), INPUT_UP);
        assert_eq!(map_key(KeyCode::Char('a')), INPUT_LEFT);
        assert_eq!(map_key(KeyCode::Char('s')), INPUT_DOWN);
        assert_eq!(map_key(KeyCode::Char('d')), INPUT_RIGHT);
    }

    #[test]
    fn action_keys() {
        assert_eq!(map_key(KeyCode::Char(' ')), INPUT_SWAP);
        assert_eq!(map_key(KeyCode::Enter), INPUT_ADVANCE);
    }

    #[test]
    fn unmapped_key_returns_zero() {
        assert_eq!(map_key(KeyCode::Char('z')), 0);
        assert_eq!(map_key(KeyCode::F(1)), 0);
    }

    #[test]
    fn summary_tells_how_it_ended_and_the_score() {
        let mut frame = crackattack_core::FrameData::default();
        frame.score = 1234;
        frame.ui.clock_time = 125 * TICKS_PER_SECOND;
        let text = summary(&frame, Ending::Lost, 42);
        assert!(text.starts_with("Game over"));
        assert!(text.contains("Score: 1234"));
        assert!(text.contains("Time: 2:05"));
        assert!(text.contains("--seed 42"));
        assert!(!text.contains("Best chain"));
        assert!(summary(&frame, Ending::Quit, 42).starts_with("Game stopped"));
        assert!(summary(&frame, Ending::Idle, 42).contains("no key"));
        assert!(summary(&frame, Ending::TimeLimit, 42).contains("time limit"));
    }

    #[test]
    fn capture_ticks_are_parsed_sorted_with_end() {
        assert_eq!(parse_capture_ticks("250,100,end"), Some((vec![100, 250], true)));
        assert_eq!(parse_capture_ticks("end"), Some((vec![], true)));
        assert_eq!(parse_capture_ticks("10, 10"), Some((vec![10], false)));
        assert_eq!(parse_capture_ticks("0"), None);
        assert_eq!(parse_capture_ticks("abc"), None);
        assert_eq!(parse_capture_ticks(""), None);
        assert_eq!(parse_level("medium"), Some(AIDifficulty::Medium));
        assert_eq!(parse_level("--seed"), None);
    }

    #[test]
    fn the_autoplay_plays_headless_and_captures_the_chosen_ticks() {
        let path = std::env::temp_dir().join(format!("crackattack-capture-test-{}.txt", std::process::id()));
        let captures = Captures { ticks: vec![50, 600], at_end: true, path: path.display().to_string() };
        let mut session = new_session(Mode::Solo, 3);
        let mut driver = Driver::new(Some(GridAutoplay::new(AIDifficulty::Hard, 3)), captures, false);
        // The session's countdown takes the first 200 ticks.
        let (frame, ending) = run_headless(&mut *session, &mut driver, Some(1000));
        let written = driver.finish(&frame).unwrap();
        let content = std::fs::read_to_string(&path);
        let _ = std::fs::remove_file(&path);
        assert_eq!(ending, Ending::FrameLimit);
        assert_eq!(written.unwrap().0, 3);
        let content = content.unwrap();
        for title in ["=== tick 50 ===", "=== tick 600 ===", "=== end, tick 1000 ==="] {
            assert!(content.contains(title), "missing {}", title);
        }
        // The AI did play: alone, the stack would not have scored.
        assert!(frame.score > 0);
        assert!(content.contains("Score"));
    }

    #[test]
    fn vs_and_solo_options_choose_the_mode_and_levels_have_two_names() {
        let parse = |args: &[&str]| parse_args(&args.iter().map(|a| a.to_string()).collect::<Vec<_>>());
        assert!(matches!(parse(&["--solo"]).unwrap().unwrap().mode, Some(Mode::Solo)));
        assert!(matches!(parse(&["--vs", "--difficulty", "medium"]).unwrap().unwrap().mode, Some(Mode::Versus(AIDifficulty::Medium))));
        assert!(matches!(parse(&["--difficulty", "easy", "--vs"]).unwrap().unwrap().mode, Some(Mode::Versus(AIDifficulty::Easy))));
        assert!(matches!(parse(&["--vs"]).unwrap().unwrap().mode, Some(Mode::Versus(AIDifficulty::Hard))));
        assert!(parse(&[]).unwrap().unwrap().mode.is_none());
        assert_eq!(parse_level("logic"), Some(AIDifficulty::Easy));
        assert_eq!(parse_level("cortex"), Some(AIDifficulty::Hard));
        // Nobody would answer the ready prompt of a headless duel.
        assert!(parse(&["--headless", "--vs"]).is_err());
        assert!(parse(&["--headless", "--vs", "--autoplay"]).is_ok());
    }

    #[test]
    fn a_duel_between_two_ais_ends_with_a_round_and_shows_the_opponent() {
        // Round 1 waits for Enter: the first frame of a player carries the ready prompt.
        let mut waiting = new_session(Mode::Versus(AIDifficulty::Easy), 7);
        let mut player = Driver::new(None, Captures::default(), true);
        let first = player.step(&mut *waiting, 0);
        player.draw(&first);
        assert!(player.hud.banner.as_ref().is_some_and(|b| b.words == ["1"] && b.above == ["Round"]), "no ready prompt");
        // The AI answers it by itself.
        let mut session = new_session(Mode::Versus(AIDifficulty::Easy), 7);
        let mut driver = Driver::new(Some(GridAutoplay::new(AIDifficulty::Hard, 7)), Captures::default(), true);
        // Then the countdown, and the opponent's stack in the side panel while the round is played.
        let mut saw_preview = false;
        let mut ending = None;
        for _ in 0..(50 * 60 * 12) {
            let frame = driver.step(&mut *session, 0);
            let text = driver.draw(&frame);
            saw_preview |= text.contains("▒▒") || text.contains("██") && driver.hud.versus.as_ref().is_some_and(|v| v.preview.is_some());
            if let Some(won) = driver.round_result.take() {
                ending = Some(won);
                break;
            }
        }
        assert!(ending.is_some(), "the round did not end");
        assert!(saw_preview, "the opponent's board was never shown");
        // The tally is the session's: counted once the result has been shown, when the
        // prompt for round 2 comes up (the AI is not asked to play again: it waits there).
        let mut player = Driver::new(None, Captures::default(), true);
        let mut round_two = false;
        for i in 0..1000 {
            // The result stays until Enter: pressed now and then, it brings the prompt.
            let frame = player.step(&mut *session, if i % 2 == 0 { INPUT_ADVANCE } else { 0 });
            player.draw(&frame);
            round_two |= player.hud.banner.as_ref().is_some_and(|b| b.words == ["2"]);
            if round_two {
                break;
            }
        }
        assert!(round_two, "no prompt for round 2");
        let versus = player.hud.versus.as_ref().unwrap();
        assert_eq!(versus.wins + versus.losses, 1);
    }

    #[test]
    fn the_enter_key_answers_the_ready_prompt_of_a_duel() {
        let mut session = new_session(Mode::Versus(AIDifficulty::Easy), 7);
        let mut driver = Driver::new(None, Captures::default(), true);
        driver.step(&mut *session, 0);
        assert!(matches!(session.snapshot().interruption, SessionInterruption::ReadyPrompt { .. }));
        driver.step(&mut *session, INPUT_ADVANCE);
        for _ in 0..3 {
            driver.step(&mut *session, 0);
        }
        assert!(!matches!(session.snapshot().interruption, SessionInterruption::ReadyPrompt { .. }), "Enter did not start the round");
    }

    /// A duel that counts the times it is told the player left.
    struct Spy {
        inner: VsComputerGameSession,
        abandoned: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    }

    impl GameSession for Spy {
        fn tick(&mut self, input: &InputSnapshot) -> FrameData { self.inner.tick(input) }
        fn frame_data(&self) -> FrameData { self.inner.frame_data() }
        fn signal_local_ready(&mut self) { self.inner.signal_local_ready() }
        fn snapshot(&self) -> crackattack_core::frame_data::SessionSnapshot { self.inner.snapshot() }
        fn drain_events(&mut self) -> Vec<SessionEvent> { self.inner.drain_events() }
        fn request_pause(&mut self) -> bool { self.inner.request_pause() }
        fn request_resume(&mut self) -> bool { self.inner.request_resume() }
        fn abandon(&mut self) {
            self.abandoned.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }
        fn game_state_mut(&mut self) -> Option<&mut GameState> { self.inner.game_state_mut() }
    }

    #[test]
    fn leaving_the_game_tells_the_session_unless_the_game_itself_ended() {
        use std::sync::{atomic::{AtomicUsize, Ordering}, Arc};
        let count = Arc::new(AtomicUsize::new(0));
        let mut spy = Spy {
            inner: VsComputerGameSession::new(7, GridOpponent::new(AIDifficulty::Easy, 7)),
            abandoned: count.clone(),
        };
        // Quit, no key pressed, too long, connection dropped: a forfeit each time.
        for ending in [Ending::Quit, Ending::Idle, Ending::TimeLimit, Ending::Hangup] {
            leave(&mut spy, ending);
        }
        assert_eq!(count.load(Ordering::SeqCst), 4);
        // The game ending by itself, or a test cut-off, is not.
        for ending in [Ending::Lost, Ending::Won, Ending::FrameLimit] {
            leave(&mut spy, ending);
        }
        assert_eq!(count.load(Ordering::SeqCst), 4);
    }

    #[test]
    fn the_pause_is_the_sessions_and_the_loop_keeps_ticking_it() {
        let mut session = new_session(Mode::Versus(AIDifficulty::Easy), 7);
        let mut driver = Driver::new(Some(GridAutoplay::new(AIDifficulty::Hard, 7)), Captures::default(), true);
        for _ in 0..300 {
            driver.step(&mut *session, 0);
        }
        // A round in progress: the opponent's stack is in the panel.
        let frame = driver.step(&mut *session, 0);
        assert!(driver.draw(&frame).contains("██"), "no preview before the pause");
        // The session is paused: the driver sees it from the snapshot, with no state of its own.
        assert!(session.request_pause());
        let frame = driver.step(&mut *session, 0);
        let screen = driver.draw(&frame);
        assert!(matches!(driver.interruption, SessionInterruption::Paused { .. }));
        assert!(screen.contains("PAUSE") && screen.contains("Q: quit"));
        // The paused screen hides what could be planned with: no danger bars, no opponent.
        assert!(driver.hud.versus.is_some(), "the hud is kept, the screen hides it");
        let mut hidden = crate::screen::Screen::new();
        hidden.feed(&screen);
        assert!(hidden.text().iter().all(|line| !line.contains('▒')));
        // But the panel is still the duel's: the opponent's name stays, the logo does not come back.
        let name = driver.hud.versus.as_ref().unwrap().name.clone();
        assert!(hidden.text().iter().any(|line| line.contains(&name)), "the duel panel is gone");
        // Resuming is also the session's: the next tick runs again.
        assert!(session.request_resume());
        driver.step(&mut *session, 0);
        assert_eq!(driver.interruption, SessionInterruption::None);
    }

    #[test]
    fn solo_shows_its_countdown_while_the_keys_do_nothing() {
        let mut session = new_session(Mode::Solo, 3);
        let mut driver = Driver::new(None, Captures::default(), false);
        let first = driver.step(&mut *session, 0);
        let screen = driver.draw(&first);
        // The digit is painted with its own color over the board, which is drawn too.
        assert!(driver.hud.countdown.is_some() && driver.hud.banner.is_none());
        assert!(screen.contains("38;5;196"), "no red 3");
        assert!(screen.contains('˙'), "the grid is hidden");
        // After the 200 ticks the board is back and GO is gone.
        let mut last = first;
        for _ in 0..200 {
            last = driver.step(&mut *session, 0);
        }
        assert!(driver.hud.banner.is_none() && driver.hud.countdown.is_none(), "the countdown outlived its time");
        // Only the lines that changed are written: the board, now that the message is gone.
        assert!(!driver.draw(&last).is_empty(), "the board did not come back");
    }

    #[test]
    fn random_seeds_differ() {
        let a = random_seed();
        std::thread::sleep(Duration::from_millis(2));
        assert_ne!(a, random_seed());
    }

    #[test]
    fn pause_key() {
        assert!(is_pause_key(KeyCode::Char('p')));
        assert!(is_pause_key(KeyCode::Char('P')));
        assert_eq!(map_key(KeyCode::Char('p')), 0);
    }

    #[test]
    fn quit_keys() {
        assert!(is_quit_key(KeyCode::Char('q')));
        assert!(is_quit_key(KeyCode::Char('Q')));
        assert!(is_quit_key(KeyCode::Esc));
        assert!(!is_quit_key(KeyCode::Char('a')));
    }

    /// What the game writes to the terminal (`--ignored --nocapture` to see it): bytes per
    /// second, lines rewritten and the biggest frame, in a game the AI plays, with
    /// `--smooth`, and paused.
    #[test]
    #[ignore]
    fn output_bandwidth() {
        for (name, smooth, paused) in [("normal", false, false), ("smooth", true, false), ("paused", false, true)] {
            render::SMOOTH_RISE.with(|s| s.set(smooth));
            let (mut bytes, mut lines, mut peak, mut frames) = (0, 0, 0, 0);
            for seed in [1, 2, 3] {
                let mut session = new_session(Mode::Solo, seed);
                let mut driver = Driver::new(Some(GridAutoplay::new(AIDifficulty::Hard, seed)), Captures::default(), false);
                if paused {
                    session.request_pause();
                }
                for _ in 0..5000 {
                    let frame = driver.step(&mut *session, 0);
                    let text = driver.draw(&frame);
                    bytes += text.len();
                    lines += text.matches(";1H").count();
                    peak = peak.max(text.len());
                    frames += 1;
                    if frame.game_over {
                        break;
                    }
                }
            }
            let per_second = bytes as f64 / frames as f64 * TICKS_PER_SECOND as f64 / 1024.0;
            println!(
                "{:<6} {:>6.1} KB/s  {:>5.0} B/frame  {:.1} lines/frame  peak {} B",
                name, per_second, bytes as f64 / frames as f64, lines as f64 / frames as f64, peak
            );
        }
    }
}
