//! Miniscript: scripted, deterministic input for tests.
//!
//! A `.mini` file describes a timeline of ordered actions: key press/release, screenshot
//! capture and quit. Each step has a gate: absolute (`@f`), relative (`wait N`) or
//! conditional (`wait until flow=playing`). Gates count frames. A script runs on the fixed-step
//! QA clock (`session:qa.fixed_step`, enabled automatically when a script is loaded), so one
//! frame is one sim tick and a run replays byte for byte.
//!
//! This module belongs to the binary crate (like `keycodes` and `platform_shell`). It forges
//! `InputEvent::Key` events at the `poll_events` seam and reads the flow state. The core
//! (parser and `ScriptRunner`) is pure and needs no GPU or window. Startup options go through
//! the CLI flags (`--preset`, `--set`, `--open`); the `.mini` holds only the timeline.

use crate::keycodes::{key_from_name, name_from_key, Key};
use crate::platform_shell::InputEvent;
use crate::app::meta_state::FlowState;

/// Default timeout of a conditional gate (`wait until`), in frames.
const DEFAULT_UNTIL_TIMEOUT: u32 = 600; // 12 s at 50 Hz

/// State condition checked by a gate or an assert. Only the current flow for now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cond {
    Flow(FlowKind),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FlowKind { Menu, Playing, PostGame }

impl FlowKind {
    fn matches(self, flow: FlowState) -> bool {
        matches!((self, flow),
            (FlowKind::Menu, FlowState::Menu)
            | (FlowKind::Playing, FlowState::Playing)
            | (FlowKind::PostGame, FlowState::PostGame))
    }
    fn parse(s: &str) -> Option<FlowKind> {
        match s {
            "menu" => Some(FlowKind::Menu),
            "playing" => Some(FlowKind::Playing),
            "postgame" | "post_game" => Some(FlowKind::PostGame),
            _ => None,
        }
    }
}

impl Cond {
    fn parse(s: &str) -> Result<Cond, String> {
        let (key, val) = s.split_once('=').ok_or_else(|| format!("condition \"{s}\" (expected flow=…)"))?;
        match key.trim() {
            "flow" => FlowKind::parse(val.trim()).map(Cond::Flow)
                .ok_or_else(|| format!("flow \"{val}\" (menu|playing|postgame)")),
            _ => Err(format!("unknown condition \"{key}\" (flow=…)")),
        }
    }
    fn eval(self, flow: FlowState) -> bool {
        match self { Cond::Flow(f) => f.matches(flow) }
    }
}

/// When a step fires.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Gate {
    Absolute(u32),          // @f: absolute frame
    Relative(u32),          // wait N: wait N frames from the previous step
    Until(Cond, u32),       // wait until <cond>: until the condition holds, bounded by a timeout (frames)
}

/// What a step does (`Noop` is a pure gate, e.g. `wait`).
#[derive(Debug, Clone, PartialEq)]
enum Verb {
    Noop,
    Press(Key),
    Release(Key),
    Tap(Key),
    Hold(Key, u32),
    Capture(String),
    Quit,
    Assert(Cond),
}

#[derive(Debug, Clone)]
struct Step { gate: Gate, verb: Verb, line: usize }

/// Effect the caller applies (in `drain_inputs`).
pub enum Effect {
    Input(InputEvent),
    /// Capture name: a slug validated at parse time, with no extension and no script prefix.
    Capture(String),
    Quit,
    AssertFailed(String),
}

/// A parsed script and its execution cursor.
#[derive(Debug)]
pub struct ScriptRunner {
    steps: Vec<Step>,
    cursor: usize,
    gate_base: u32,                     // frame where the previous step resolved its gate
    pending_releases: Vec<(u32, Key)>,  // automatic releases (tap/hold), independent of the cursor
}

impl ScriptRunner {
    /// Parses a `.mini` source. An `Err` message carries the line number.
    pub fn parse(src: &str) -> Result<ScriptRunner, String> {
        // Tolerate a UTF-8 BOM: Windows editors often add one. `trim()` does not strip
        // U+FEFF (it is not Unicode whitespace), so the first token would fail to parse.
        let src = src.strip_prefix('\u{feff}').unwrap_or(src);
        let mut steps = Vec::new();
        for (i, raw) in src.lines().enumerate() {
            let line = i + 1;
            let text = raw.split('#').next().unwrap_or("").trim();
            if text.is_empty() { continue; }
            steps.push(parse_line(text, line)?);
        }
        // A duplicate capture name is an error: the two files would overwrite each other.
        let mut seen: Vec<&str> = Vec::new();
        for st in &steps {
            if let Verb::Capture(name) = &st.verb {
                if seen.contains(&name.as_str()) {
                    return Err(format!("script L{}: duplicate capture name \"{name}\"", st.line));
                }
                seen.push(name);
            }
        }
        Ok(ScriptRunner { steps, cursor: 0, gate_base: 0, pending_releases: Vec::new() })
    }

    /// Capture names of the timeline, in script order. Used by the seed lock (a script
    /// that captures without an explicit seed is not reproducible) and by the summary.
    pub fn capture_names(&self) -> Vec<&str> {
        self.steps.iter()
            .filter_map(|s| match &s.verb { Verb::Capture(n) => Some(n.as_str()), _ => None })
            .collect()
    }


    /// Whether no step or release is left pending (for tests).
    #[cfg(test)]
    pub fn is_done(&self) -> bool {
        self.cursor >= self.steps.len() && self.pending_releases.is_empty()
    }

    /// Static description of the timeline (for `--script-dry-run`).
    pub fn describe(&self) -> Vec<String> {
        self.steps.iter().map(|s| format!("L{:<3} {}{}", s.line, gate_str(&s.gate), verb_str(&s.verb))).collect()
    }

    /// Call once per frame. Emits the releases that are due, then every step whose gate
    /// opens at this frame and state. `flow` is the current flow (for conditions).
    pub fn tick(&mut self, frame: u32, flow: FlowState) -> Vec<Effect> {
        let mut out = Vec::new();

        // 1. Automatic releases (tap/hold) that are due.
        let mut still = Vec::new();
        for (f, k) in std::mem::take(&mut self.pending_releases) {
            if frame >= f { out.push(Effect::Input(key_event(k, false))); }
            else { still.push((f, k)); }
        }
        self.pending_releases = still;

        // 2. Sequential steps.
        while self.cursor < self.steps.len() {
            let step = self.steps[self.cursor].clone();
            let resolved = match step.gate {
                Gate::Absolute(f) => frame >= f,
                Gate::Relative(d) => frame >= self.gate_base.saturating_add(d),
                Gate::Until(cond, timeout) => {
                    if cond.eval(flow) { true }
                    else if frame >= self.gate_base.saturating_add(timeout) {
                        // A timeout is an implicit assertion ("the state is reached within N
                        // frames"). The scenario has diverged, so the script fails (exit 1).
                        // A mere warning would let the rest run in a different world and
                        // still end green.
                        out.push(Effect::AssertFailed(format!(
                            "script L{}: \"until\" gate expired at frame {frame} (timeout {timeout})",
                            step.line)));
                        true
                    } else { false }
                }
            };
            if !resolved { break; }

            match &step.verb {
                Verb::Noop => {}
                Verb::Press(k) => out.push(Effect::Input(key_event(*k, true))),
                Verb::Release(k) => out.push(Effect::Input(key_event(*k, false))),
                Verb::Tap(k) => {
                    out.push(Effect::Input(key_event(*k, true)));
                    self.pending_releases.push((frame + 1, *k));
                }
                Verb::Hold(k, d) => {
                    out.push(Effect::Input(key_event(*k, true)));
                    self.pending_releases.push((frame + d.max(&1), *k));
                }
                Verb::Capture(name) => out.push(Effect::Capture(name.clone())),
                Verb::Quit => out.push(Effect::Quit),
                Verb::Assert(cond) => {
                    if !cond.eval(flow) {
                        out.push(Effect::AssertFailed(
                            format!("script L{}: assert failed at frame {frame} ({:?} != current state {:?})",
                                step.line, cond, flow)));
                    }
                }
            }
            self.gate_base = frame;
            self.cursor += 1;
        }
        out
    }
}

/// Recorder, the inverse of replay. It captures the real `InputEvent::Key` events at the
/// `poll_events` seam, stamps them with the frame, and serializes them into a `.mini` that
/// `ScriptRunner` can replay. Like replay, recording runs on the fixed step (one frame is
/// one tick) so the captured frames are reproducible. Keyboard only (down/up edges, no
/// repeats); modifiers are ignored.
pub struct Recorder {
    path: String,
    seed: u64,
    /// Command line of this run, written in the header. The `.mini` must be replayed with
    /// the same startup options, so they are recorded.
    cmdline: String,
    lines: Vec<String>,
    /// User inputs that are not captured (mouse, gamepad, text). Counted so the header
    /// states that the recording is incomplete.
    dropped_inputs: u32,
}

impl Recorder {
    pub fn new(path: String, seed: u64, cmdline: String) -> Self {
        Recorder { path, seed, cmdline, lines: Vec::new(), dropped_inputs: 0 }
    }

    /// Captures an input event (keyboard only, edges without repeats). Other user inputs
    /// (mouse, gamepad, text) are counted as dropped. Lifecycle events (resize, focus...)
    /// are neither captured nor counted.
    pub fn capture(&mut self, frame: u32, ev: &InputEvent) {
        match ev {
            InputEvent::Key { key, pressed, repeat: false, .. } => {
                if let Some(name) = name_from_key(*key) {
                    self.lines.push(format!("@{frame} {} {name}", if *pressed { "press" } else { "release" }));
                }
            }
            InputEvent::PointerButton { .. } | InputEvent::Scroll { .. }
            | InputEvent::Text(_) | InputEvent::Gamepad(_) => self.dropped_inputs += 1,
            _ => {}
        }
    }

    pub fn dropped_inputs(&self) -> u32 { self.dropped_inputs }

    pub fn len(&self) -> usize { self.lines.len() }
    pub fn path(&self) -> &str { &self.path }

    /// Returns the `.mini` content: header, timeline and a closing `quit`. Kept apart from
    /// the file write so it can be tested. `end_frame` is the last frame of the session. The
    /// file ends with an explicit `quit`; without it, replaying the export would never end,
    /// because `quit` and `capture` are effects, not input events, and are not captured.
    pub fn render(&self, end_frame: u32) -> String {
        let mut out = String::new();
        out.push_str("# RECORDED miniscript: replay with the SAME startup options.\n");
        // Informational only: the quoting is POSIX. Under cmd or PowerShell, adjust the
        // quotes by hand.
        out.push_str(&format!("# Launched with (informational, POSIX quoting): crackattack {}\n", self.cmdline));
        out.push_str(&format!("# Effective seed: {}; replay: … --script <this-file>\n", self.seed));
        out.push_str(&format!("# {} keyboard events\n", self.lines.len()));
        if self.dropped_inputs > 0 {
            out.push_str(&format!(
                "# WARNING: {} non-keyboard input(s) (mouse/gamepad/text) NOT captured; replay incomplete.\n",
                self.dropped_inputs));
        }
        for l in &self.lines { out.push('\n'); out.push_str(l); }
        // Closing `quit`: the recorded session ended here.
        out.push_str(&format!("\n@{end_frame} quit\n"));
        out
    }

    pub fn write(&self, end_frame: u32) -> std::io::Result<()> {
        std::fs::write(&self.path, self.render(end_frame))
    }
}

/// Parses one line. Grammar: one line is one step, and `wait` takes no inline action.
/// - `@f <action>`          absolute gate, the action is required;
/// - `wait N`               relative gate alone (`Noop` action);
/// - `wait until <cond>`    conditional gate alone (`Noop` action, implicit timeout);
/// - `<action>`             action alone, immediate gate.
fn parse_line(text: &str, line: usize) -> Result<Step, String> {
    let err = |m: String| format!("script L{line}: {m}");

    // Explicit gate?
    if let Some(rest) = text.strip_prefix('@') {
        let mut it = rest.splitn(2, char::is_whitespace);
        let num = it.next().unwrap_or("");
        let f: u32 = num.parse().map_err(|_| err(format!("frame \"{num}\" (integer)")))?;
        let action = it.next().unwrap_or("").trim();
        if action.is_empty() { return Err(err("\"@f\" requires an action".to_string())); }
        return Ok(Step { gate: Gate::Absolute(f), verb: parse_verb(action, line)?, line });
    }
    if let Some(rest) = text.strip_prefix("wait ").map(str::trim) {
        if let Some(cond_s) = rest.strip_prefix("until ") {
            let cond = Cond::parse(cond_s.trim()).map_err(|m| err(m))?;
            return Ok(Step { gate: Gate::Until(cond, DEFAULT_UNTIL_TIMEOUT), verb: Verb::Noop, line });
        }
        let d: u32 = rest.parse().map_err(|_| err(format!("duration \"{rest}\" (frames)")))?;
        return Ok(Step { gate: Gate::Relative(d), verb: Verb::Noop, line });
    }

    // Otherwise: action alone, immediate gate.
    Ok(Step { gate: Gate::Relative(0), verb: parse_verb(text, line)?, line })
}

fn parse_verb(text: &str, line: usize) -> Result<Verb, String> {
    let err = |m: String| format!("script L{line}: {m}");
    let mut it = text.split_whitespace();
    let verb = it.next().unwrap_or("");
    let key = |name: Option<&str>| -> Result<Key, String> {
        let n = name.ok_or_else(|| err(format!("\"{verb}\" requires a key")))?;
        key_from_name(n).ok_or_else(|| err(format!("unknown key \"{n}\" (e.g. Left, Enter, Space, KeyZ)")))
    };
    match verb {
        "press" => Ok(Verb::Press(key(it.next())?)),
        "release" => Ok(Verb::Release(key(it.next())?)),
        "tap" => Ok(Verb::Tap(key(it.next())?)),
        "hold" => {
            let k = key(it.next())?;
            let d_s = it.next().ok_or_else(|| err("\"hold\" requires a duration (frames)".to_string()))?;
            let d: u32 = d_s.parse().map_err(|_| err(format!("duration \"{d_s}\" (frames)")))?;
            Ok(Verb::Hold(k, d))
        }
        "capture" => {
            // The name is required: an anonymous capture would fall back to timestamp
            // naming, which brings back matching by position. Strict slug: no path can
            // get through.
            let name = it.next().ok_or_else(|| err(
                "\"capture\" requires a name (slug [A-Za-z0-9._-], e.g. capture solo-rideau)".to_string()))?;
            if let Some(extra) = it.next() {
                return Err(err(format!("\"capture\" takes a single name (extra \"{extra}\")")));
            }
            if name.is_empty()
                || !name.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
                || name.contains("..")
            {
                return Err(err(format!("invalid capture name \"{name}\" (slug [A-Za-z0-9._-], no \"..\")")));
            }
            if name.to_ascii_lowercase().ends_with(".png") {
                return Err(err(format!("capture name \"{name}\": \".png\" is added automatically")));
            }
            // DOS reserved names: `con.png` and the like are forbidden on Windows. Rejected
            // at parse time, case-insensitive, on the first segment before the first '.'.
            let stem = name.split('.').next().unwrap_or("").to_ascii_lowercase();
            const DOS_RESERVED: [&str; 22] = [
                "con", "prn", "aux", "nul",
                "com1", "com2", "com3", "com4", "com5", "com6", "com7", "com8", "com9",
                "lpt1", "lpt2", "lpt3", "lpt4", "lpt5", "lpt6", "lpt7", "lpt8", "lpt9",
            ];
            if DOS_RESERVED.contains(&stem.as_str()) {
                return Err(err(format!(
                    "capture name \"{name}\": reserved name on Windows (con/prn/aux/nul/comN/lptN)")));
            }
            Ok(Verb::Capture(name.to_string()))
        }
        "quit" => Ok(Verb::Quit),
        "assert" => Ok(Verb::Assert(Cond::parse(text.strip_prefix("assert").unwrap_or("").trim()).map_err(|m| err(m))?)),
        // Startup options go through the CLI flags (--preset, --set, --open).
        "open" | "set" | "preset" =>
            Err(err(format!("\"{verb}\" (startup): use the CLI flags, not the .mini (MVP)"))),
        other => Err(err(format!("unknown verb \"{other}\""))),
    }
}

fn key_event(key: Key, pressed: bool) -> InputEvent {
    InputEvent::Key { key, pressed, repeat: false, modifiers: egui::Modifiers::default() }
}

fn gate_str(g: &Gate) -> String {
    match g {
        Gate::Absolute(f) => format!("@{f} "),
        Gate::Relative(0) => String::new(),
        Gate::Relative(d) => format!("+{d} "),
        Gate::Until(c, t) => format!("until {c:?}(≤{t}) "),
    }
}
fn verb_str(v: &Verb) -> String { format!("{v:?}") }

#[cfg(test)]
mod tests {
    /// The QA miniscripts under version control must always parse (same safety net as the
    /// naga test for the WGSL sources). A grammar drift or a broken script fails in tests,
    /// not when a QA campaign starts.
    #[test]
    fn qa_scripts_parse() {
        for (name, src) in [
            ("shader-solo", include_str!("../qa/scripts/shader-solo.mini")),
            ("shader-solo-panic", include_str!("../qa/scripts/shader-solo-panic.mini")),
            ("shader-vs-ia", include_str!("../qa/scripts/shader-vs-ia.mini")),
            ("shader-vs-ia-shatter", include_str!("../qa/scripts/shader-vs-ia-shatter.mini")),
            ("shader-vs-ia-anim", include_str!("../qa/scripts/shader-vs-ia-anim.mini")),
            ("shader-vs-ia-round3", include_str!("../qa/scripts/shader-vs-ia-round3.mini")),
        ] {
            let r = ScriptRunner::parse(src).unwrap_or_else(|e| panic!("{name}: {e}"));
            assert!(!r.capture_names().is_empty(), "{name}: QA script without capture: suspect");
        }
    }

    use super::*;

    #[test]
    fn parses_and_desugars_basic_timeline() {
        let r = ScriptRunner::parse("
            # commentaire
            wait until flow=playing
            hold Left 20
            wait 10
            tap Space
            capture shot
            quit
        ").unwrap();
        assert_eq!(r.steps.len(), 6);
        assert!(matches!(r.steps[0].gate, Gate::Until(Cond::Flow(FlowKind::Playing), _)));
        assert!(matches!(r.steps[0].verb, Verb::Noop));
        assert!(matches!(r.steps[1].verb, Verb::Hold(_, 20)));
        assert!(matches!(r.steps[2].gate, Gate::Relative(10)));
    }

    #[test]
    fn parse_tolerates_utf8_bom() {
        // A .mini written on Windows with a BOM must parse.
        assert!(ScriptRunner::parse("\u{feff}wait 5").is_ok());
    }

    #[test]
    fn parse_rejects_dos_reserved_capture_names() {
        // con.png and nul.png are reserved on Windows.
        assert!(ScriptRunner::parse("capture con").is_err());
        assert!(ScriptRunner::parse("capture NUL").is_err());
        assert!(ScriptRunner::parse("capture lpt3").is_err());
        assert!(ScriptRunner::parse("capture com1.before").is_err());
        assert!(ScriptRunner::parse("capture console").is_ok()); // not a reserved name
    }

    #[test]
    fn parse_errors_carry_line_numbers() {
        assert!(ScriptRunner::parse("tap Nonexistentkey").unwrap_err().contains("L1"));
        assert!(ScriptRunner::parse("\n\n@x quit").unwrap_err().contains("L3"));
        assert!(ScriptRunner::parse("frobnicate").unwrap_err().contains("unknown verb"));
        assert!(ScriptRunner::parse("hold Left").is_err()); // missing duration
    }

    #[test]
    fn absolute_gate_fires_at_frame() {
        let mut r = ScriptRunner::parse("@3 press Space").unwrap();
        for f in 0..3 { assert!(r.tick(f, FlowState::Playing).is_empty(), "frame {f}"); }
        let e = r.tick(3, FlowState::Playing);
        assert!(matches!(e.as_slice(), [Effect::Input(InputEvent::Key { pressed: true, .. })]));
    }

    #[test]
    fn tap_auto_releases_next_frame() {
        let mut r = ScriptRunner::parse("tap Space").unwrap();
        let down = r.tick(0, FlowState::Playing);
        assert!(matches!(down.as_slice(), [Effect::Input(InputEvent::Key { pressed: true, .. })]));
        let up = r.tick(1, FlowState::Playing);
        assert!(matches!(up.as_slice(), [Effect::Input(InputEvent::Key { pressed: false, .. })]));
        assert!(r.is_done());
    }

    #[test]
    fn hold_releases_after_duration() {
        let mut r = ScriptRunner::parse("hold Left 5").unwrap();
        assert!(matches!(r.tick(0, FlowState::Playing).as_slice(),
            [Effect::Input(InputEvent::Key { pressed: true, .. })]));
        for f in 1..5 { assert!(r.tick(f, FlowState::Playing).is_empty(), "frame {f}"); }
        assert!(matches!(r.tick(5, FlowState::Playing).as_slice(),
            [Effect::Input(InputEvent::Key { pressed: false, .. })]));
    }

    #[test]
    fn until_gate_waits_for_state_then_relative_counts_from_there() {
        let mut r = ScriptRunner::parse("wait until flow=playing\nwait 2\ncapture x").unwrap();
        // Stays in the menu: nothing resolves.
        for f in 0..5 { assert!(r.tick(f, FlowState::Menu).is_empty(), "menu frame {f}"); }
        // Switches to playing at frame 5: the until gate resolves, then `wait 2` counts from 5.
        assert!(r.tick(5, FlowState::Playing).is_empty());     // until resolves (Noop), wait 2 not yet
        assert!(r.tick(6, FlowState::Playing).is_empty());
        assert!(matches!(r.tick(7, FlowState::Playing).as_slice(), [Effect::Capture(n)] if n == "x"));
    }

    #[test]
    fn until_gate_times_out_as_failure() {
        // Default timeout is 600: after 600 frames in the menu the gate expires. That is a
        // failed implicit assertion (exit 1), and the script still moves on instead of hanging.
        let mut r = ScriptRunner::parse("wait until flow=playing\nquit").unwrap();
        assert!(r.tick(0, FlowState::Menu).is_empty());
        let e = r.tick(600, FlowState::Menu);
        assert!(matches!(e.as_slice(), [Effect::AssertFailed(_), Effect::Quit]));
    }

    #[test]
    fn recorder_captures_edges_and_roundtrips() {
        let mut rec = Recorder::new("/tmp/x.mini".into(), 42, "--preset solo --record x.mini".to_string());
        let left = key_from_name("Left").unwrap();
        rec.capture(3, &InputEvent::Key { key: left, pressed: true, repeat: false, modifiers: egui::Modifiers::default() });
        // Repeats are ignored.
        rec.capture(4, &InputEvent::Key { key: left, pressed: true, repeat: true, modifiers: egui::Modifiers::default() });
        rec.capture(10, &InputEvent::Key { key: left, pressed: false, repeat: false, modifiers: egui::Modifiers::default() });
        // Non-keyboard events are ignored.
        rec.capture(11, &InputEvent::CloseRequested);
        assert_eq!(rec.len(), 2);
        let mini = rec.render(99);
        assert!(mini.contains("@3 press ArrowLeft"));
        assert!(mini.contains("@10 release ArrowLeft"));
        // The recording re-parses as a valid script (2 keys plus the closing quit).
        let r = ScriptRunner::parse(&mini).unwrap();
        assert_eq!(r.describe().len(), 3);
        // The re-export must always end with a quit.
        assert!(mini.trim_end().ends_with("@99 quit"), "{mini}");
    }

    #[test]
    fn capture_naming_rules() {
        // The name is required (an anonymous capture would fall back to timestamp naming,
        // so to matching by position), the slug is strict, `.png` is added automatically,
        // and a duplicate is rejected.
        assert!(ScriptRunner::parse("capture").is_err(), "name required");
        assert!(ScriptRunner::parse("capture shot.png").is_err(), ".png is automatic");
        assert!(ScriptRunner::parse("capture a/b").is_err(), "no path");
        assert!(ScriptRunner::parse("capture a..b").is_err(), "no parent dir");
        assert!(ScriptRunner::parse("capture curtain extra").is_err(), "a single argument");
        assert!(ScriptRunner::parse("capture curtain\ncapture curtain").is_err(), "duplicate");
        let r = ScriptRunner::parse("capture curtain\nwait 2\ncapture static-2").unwrap();
        assert_eq!(r.capture_names(), vec!["curtain", "static-2"]);
    }

    #[test]
    fn recorder_counts_dropped_non_key_inputs() {
        let mut rec = Recorder::new("/tmp/x.mini".into(), 1, String::new());
        rec.capture(2, &InputEvent::Text("a".into()));
        rec.capture(3, &InputEvent::Resized { pixels: (800, 600) }); // lifecycle event: neither captured nor counted
        assert_eq!(rec.dropped_inputs(), 1);
        assert!(rec.render(9).contains("WARNING: 1 non-keyboard input(s)"));
    }

    #[test]
    fn assert_flow_fails_when_mismatch() {
        let mut r = ScriptRunner::parse("assert flow=playing").unwrap();
        let e = r.tick(0, FlowState::Menu);
        assert!(matches!(e.as_slice(), [Effect::AssertFailed(_)]));
        let mut r2 = ScriptRunner::parse("assert flow=playing").unwrap();
        assert!(r2.tick(0, FlowState::Playing).is_empty());
    }
}
