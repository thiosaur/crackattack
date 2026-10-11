//! `rungs.json`: the rung ladder, produced by the crackattack-sounds tool
//! (`tools/build_rungs.py`). All musical values come from this file; the game converts them
//! here to frames (stereo samples) and validates them once.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crackattack_core::frame_data::UIState;

use super::track::{self, TrackInfo};

pub const RUNGS_FILE: &str = "rungs.json";

#[derive(Deserialize)]
struct RawPlan {
    version: u32,
    #[serde(default = "default_rate")]
    sample_rate: u32,
    stress: RawStress,
    arrangements: HashMap<String, RawArrangement>,
    rungs: Vec<RawRung>,
    transitions: HashMap<String, RawTransition>,
}

fn default_rate() -> u32 { 44100 }

#[derive(Deserialize)]
struct RawStress {
    /// Length of the sliding mean of the danger.
    #[serde(default = "default_window")]
    window_seconds: f32,
    /// Stack height (`top_effective_row`) that gives zero stress, and the one that gives 1.
    #[serde(default)]
    calm_row: f32,
    #[serde(default = "default_top_row")]
    top_row: f32,
    /// Time model (takes priority when given): the felt time before defeat that gives 1, and
    /// the weight of the creep speed (0: only rows count; 1: real time). Zero stress is
    /// still set by `calm_row`.
    panic_seconds: Option<f32>,
    #[serde(default = "default_speed_exponent")]
    speed_exponent: f32,
}

fn default_speed_exponent() -> f32 { 0.5 }

fn default_window() -> f32 { 30.0 }

fn default_top_row() -> f32 { (crackattack_core::constants::SAFE_HEIGHT - 1) as f32 }

#[derive(Deserialize)]
struct RawArrangement { min_seconds_before_transition: f64 }

#[derive(Deserialize)]
struct RawRung {
    id: String,
    arrangement: String,
    bpm: f64,
    core: String,
    rhythm: String,
    first_downbeat: f64,
    bar_seconds: f64,
    loop_bars: u32,
    min_seconds: f64,
    up_at: Option<f32>,
    down_at: Option<f32>,
    /// Game time before the rung becomes available.
    #[serde(default)]
    unlock_seconds: f64,
}

#[derive(Deserialize)]
struct RawTransition {
    points: Vec<RawPoint>,
    crossfade_beats: f64,
    rhythm_out_bars: u32,
    rhythm_in_bars: u32,
}

#[derive(Deserialize)]
struct RawPoint { exit_bar: u32, entry_bar: u32 }

/// A rung: one arrangement at one tempo, with two tracks (core, rhythm) of the same length.
/// Durations are in frames. Bars are numbered from 1.
#[derive(Debug, Clone)]
pub struct Rung {
    pub id: String,
    pub arrangement: String,
    pub bpm: f64,
    pub core: PathBuf,
    pub rhythm: PathBuf,
    /// Start of bar 1 (what precedes it is the pickup, played only at the start).
    pub first_downbeat: f64,
    pub bar: f64,
    pub loop_bars: u32,
    pub min_frames: u64,
    /// Minimum time in the arrangement before leaving it.
    pub min_frames_before_transition: u64,
    pub up_at: Option<f32>,
    pub down_at: Option<f32>,
    pub unlock_seconds: f64,
}

impl Rung {
    /// Start of bar `b` (`b = loop_bars + 1` is the end of the loop).
    pub fn bar_start(&self, b: u32) -> f64 {
        self.first_downbeat + (b as f64 - 1.0) * self.bar
    }
    pub fn loop_start(&self) -> u64 {
        self.bar_start(1).round() as u64
    }
    pub fn loop_end(&self) -> u64 {
        self.bar_start(self.loop_bars + 1).round() as u64
    }
}

#[derive(Debug, Clone)]
pub struct Transition {
    /// (exit bar, entry bar).
    pub points: Vec<(u32, u32)>,
    pub crossfade_beats: f64,
    pub rhythm_out_bars: u32,
    pub rhythm_in_bars: u32,
}

/// How the stack becomes a danger in [0, 1].
#[derive(Debug, Clone, PartialEq)]
pub enum HeightModel {
    /// Height only: `calm_row` gives 0, `top_row` gives 1.
    Rows { calm_row: f32, top_row: f32 },
    /// Felt time before defeat: rows to loss x creep speed, where the speed weighs less than
    /// the rows (`speed_exponent`). The result is on a logarithmic scale between the time of
    /// a calm stack (`calm_rows` rows to loss, at the current speed: 0) and `panic_seconds`
    /// (1). A calm stack gives 0 at any point of the game; the faster the stack rises, the
    /// more a high stack weighs. A stack frozen at the top gives 1.
    Time { calm_rows: f32, panic_seconds: f32, speed_exponent: f32 },
}

impl HeightModel {
    pub fn height(&self, ui: &UIState) -> f32 {
        match *self {
            HeightModel::Rows { calm_row, top_row } =>
                ((ui.top_effective_row as f32 - calm_row) / (top_row - calm_row)).clamp(0.0, 1.0),
            HeightModel::Time { calm_rows, panic_seconds, speed_exponent } => {
                if ui.creep_freeze {
                    return 1.0;
                }
                if ui.rows_to_loss >= calm_rows {
                    return 0.0;
                }
                let t = felt_seconds(ui.rows_to_loss, ui.seconds_per_row, speed_exponent);
                let calm = felt_seconds(calm_rows, ui.seconds_per_row, speed_exponent);
                if t <= panic_seconds || calm <= panic_seconds {
                    return 1.0;
                }
                ((calm / t).ln() / (calm / panic_seconds).ln()).clamp(0.0, 1.0)
            }
        }
    }
}

/// Felt time before defeat. At the start of a game it is the real time (rows x speed). When
/// the stack speeds up, the speed only counts to the power `speed_exponent`.
pub fn felt_seconds(rows_to_loss: f32, seconds_per_row: f32, speed_exponent: f32) -> f32 {
    let start = crackattack_core::constants::CREEP_START_SECONDS_PER_ROW;
    rows_to_loss * start * (seconds_per_row / start).powf(speed_exponent)
}

#[derive(Debug, Clone)]
pub struct Plan {
    pub sample_rate: u32,
    /// Stress is the mean danger over this duration.
    pub window_seconds: f32,
    pub height_model: HeightModel,
    pub rungs: Vec<Rung>,
    /// Key: "source arrangement>target arrangement".
    transitions: HashMap<String, Transition>,
}

impl Plan {
    /// Loads `dir/rungs.json` and checks the tracks (WAV or Ogg headers).
    pub fn load(dir: &Path) -> Result<Plan, String> {
        let path = dir.join(RUNGS_FILE);
        let text = std::fs::read_to_string(&path).map_err(|e| format!("{} : {e}", path.display()))?;
        Plan::parse(&text, dir, |p| track::probe(p))
    }

    /// Parses and validates. `probe` reads a track header (replaced in tests).
    pub fn parse(text: &str, dir: &Path, probe: impl Fn(&Path) -> Result<TrackInfo, String>) -> Result<Plan, String> {
        let raw: RawPlan = serde_json::from_str(text).map_err(|e| format!("{RUNGS_FILE} : {e}"))?;
        if raw.version != 1 {
            return Err(format!("unknown version {} (expected 1)", raw.version));
        }
        if raw.rungs.is_empty() {
            return Err("no rungs".into());
        }
        if !(raw.stress.window_seconds > 0.0) {
            return Err("stress: window_seconds must be > 0".into());
        }
        let height_model = match raw.stress.panic_seconds {
            Some(panic) => {
                let calm_rows = default_top_row() - raw.stress.calm_row;
                if !(panic > 0.0 && calm_rows > 0.0) {
                    return Err("stress: panic_seconds must be > 0 and calm_row below the safe height".into());
                }
                HeightModel::Time { calm_rows, panic_seconds: panic,
                    speed_exponent: raw.stress.speed_exponent.clamp(0.0, 1.0) }
            }
            None => {
                if !(raw.stress.top_row > raw.stress.calm_row) {
                    return Err("stress: top_row must exceed calm_row".into());
                }
                HeightModel::Rows { calm_row: raw.stress.calm_row, top_row: raw.stress.top_row }
            }
        };
        let sr = raw.sample_rate as f64;
        let mut rungs = Vec::with_capacity(raw.rungs.len());
        for r in &raw.rungs {
            let arr = raw.arrangements.get(&r.arrangement)
                .ok_or_else(|| format!("{}: arrangement \"{}\" not described", r.id, r.arrangement))?;
            if !(r.bar_seconds > 0.0 && r.first_downbeat >= 0.0 && r.loop_bars >= 1 && r.bpm > 0.0) {
                return Err(format!("{}: invalid bar_seconds, first_downbeat, loop_bars or bpm", r.id));
            }
            let rung = Rung {
                id: r.id.clone(),
                arrangement: r.arrangement.clone(),
                bpm: r.bpm,
                core: dir.join(&r.core),
                rhythm: dir.join(&r.rhythm),
                first_downbeat: r.first_downbeat * sr,
                bar: r.bar_seconds * sr,
                loop_bars: r.loop_bars,
                min_frames: (r.min_seconds.max(0.0) * sr) as u64,
                min_frames_before_transition: (arr.min_seconds_before_transition.max(0.0) * sr) as u64,
                up_at: r.up_at,
                down_at: r.down_at,
                unlock_seconds: r.unlock_seconds.max(0.0),
            };
            let core = probe(&rung.core)?;
            let rhythm = probe(&rung.rhythm)?;
            for (info, path) in [(core, &rung.core), (rhythm, &rung.rhythm)] {
                if info.sample_rate != raw.sample_rate {
                    return Err(format!("{}: {} Hz, expected {}", path.display(), info.sample_rate, raw.sample_rate));
                }
            }
            if core.frames != rhythm.frames {
                return Err(format!("{}: core and rhythm differ in length ({} / {} frames)",
                    r.id, core.frames, rhythm.frames));
            }
            if core.frames < rung.loop_end() {
                return Err(format!("{}: the track ends before the end of bar {}", r.id, r.loop_bars));
            }
            rungs.push(rung);
        }

        let mut transitions = HashMap::new();
        for (key, t) in &raw.transitions {
            transitions.insert(key.clone(), Transition {
                points: t.points.iter().map(|p| (p.exit_bar, p.entry_bar)).collect(),
                crossfade_beats: t.crossfade_beats.max(0.0),
                rhythm_out_bars: t.rhythm_out_bars,
                rhythm_in_bars: t.rhythm_in_bars,
            });
        }
        let plan = Plan {
            sample_rate: raw.sample_rate,
            window_seconds: raw.stress.window_seconds,
            height_model,
            rungs,
            transitions,
        };

        // Thresholds: an up threshold on every rung but the last, a down threshold on every
        // rung but the first (they also define the curve used to spread them).
        let last = plan.rungs.len() - 1;
        for (i, r) in plan.rungs.iter().enumerate() {
            if (i < last) != r.up_at.is_some() || (i > 0) != r.down_at.is_some() {
                return Err(format!("{}: up_at on every rung except the last, down_at on every rung except the first", r.id));
            }
        }
        // Unlocking: the first rung from the start, then in order.
        if plan.rungs[0].unlock_seconds > 0.0
            || plan.rungs.windows(2).any(|w| w[1].unlock_seconds < w[0].unlock_seconds)
        {
            return Err("unlock_seconds: 0 for the first rung, then increasing".into());
        }

        // Each pair of neighboring rungs must be crossable in both directions.
        for w in plan.rungs.windows(2) {
            let (a, b) = (&w[0], &w[1]);
            if a.arrangement == b.arrangement {
                if a.loop_bars != b.loop_bars {
                    return Err(format!("{} / {}: same arrangement, different loop_bars", a.id, b.id));
                }
                continue;
            }
            for (from, to) in [(a, b), (b, a)] {
                let t = plan.transition(from, to)
                    .ok_or_else(|| format!("transition \"{}>{}\" missing", from.arrangement, to.arrangement))?;
                if t.points.is_empty() {
                    return Err(format!("transition \"{}>{}\" has no point", from.arrangement, to.arrangement));
                }
                for &(exit, entry) in &t.points {
                    if !(1..=from.loop_bars).contains(&exit) || !(1..=to.loop_bars).contains(&entry) {
                        return Err(format!("transition \"{}>{}\": point {exit}→{entry} outside the loop",
                            from.arrangement, to.arrangement));
                    }
                }
            }
        }
        Ok(plan)
    }

    /// Number of rungs available after `elapsed` seconds of play.
    pub fn available(&self, elapsed: f64) -> usize {
        self.rungs.iter().filter(|r| r.unlock_seconds <= elapsed).count().max(1)
    }

    /// Thresholds (up, down) of `rung` when only the first `available` rungs are reachable.
    /// The whole stress scale is spread over them: the thresholds in `rungs.json` describe a
    /// curve (threshold by position of the step in the ladder), and step i sits at
    /// i / (available - 1) of the scale. When everything is available, the thresholds are
    /// those of the file, unchanged.
    pub fn thresholds(&self, rung: usize, available: usize) -> (Option<f32>, Option<f32>) {
        let n = self.rungs.len();
        if available < 2 || n < 2 {
            return (None, None);
        }
        let span = (available - 1) as f32;
        let full = (n - 1) as f32;
        // Piecewise linear curve through (0, 0) and (j / full, threshold of step j).
        let curve = |p: f32, at: &dyn Fn(usize) -> f32| -> f32 {
            let x = (p * full).clamp(0.0, full);
            let j = (x.ceil() as usize).max(1);
            let (x0, y0) = if j == 1 { (0.0, 0.0) } else { ((j - 1) as f32, at(j - 1)) };
            y0 + (at(j) - y0) * (x - x0) / (j as f32 - x0)
        };
        // Step j (numbered from 1) joins rung j-1 and rung j; going up crosses it from below.
        let up_curve = |j: usize| self.rungs[j - 1].up_at.unwrap_or(1.0);
        let down_curve = |j: usize| self.rungs[j].down_at.unwrap_or(0.0);
        let up = (rung + 1 < available).then(|| curve((rung + 1) as f32 / span, &up_curve));
        let down = (rung > 0).then(|| curve(rung as f32 / span, &down_curve));
        (up, down)
    }

    pub fn transition(&self, from: &Rung, to: &Rung) -> Option<&Transition> {
        self.transitions.get(&format!("{}>{}", from.arrangement, to.arrangement))
    }

    /// Instantaneous danger of the local player's stack, in [0, 1] (before smoothing).
    pub fn height(&self, ui: &UIState) -> f32 {
        self.height_model.height(ui)
    }

    pub fn seconds(&self, frames: f64) -> f64 {
        frames / self.sample_rate as f64
    }

    pub fn frames(&self, seconds: f64) -> f64 {
        seconds * self.sample_rate as f64
    }
}

/// Test plan: three arrangements and six rungs, without files (`probe` returns a long
/// enough length).
#[cfg(test)]
pub(crate) const TEST_PLAN_JSON: &str = r#"{
      "version": 1, "sample_rate": 44100,
      "stress": { "window_seconds": 30, "calm_row": 6, "top_row": 12 },
      "arrangements": {
        "normal": { "min_seconds_before_transition": 30 },
        "mid":    { "min_seconds_before_transition": 30 },
        "stress": { "min_seconds_before_transition": 30 }
      },
      "rungs": [
        { "id": "normal_121", "arrangement": "normal", "bpm": 121.125, "core": "a", "rhythm": "b",
          "first_downbeat": 0.23787, "bar_seconds": 1.981421, "loop_bars": 67, "min_seconds": 15,
          "up_at": 0.18, "down_at": null },
        { "id": "normal_130", "arrangement": "normal", "bpm": 130.0, "core": "a", "rhythm": "b",
          "first_downbeat": 0.22163, "bar_seconds": 1.846154, "loop_bars": 67, "min_seconds": 15,
          "up_at": 0.33, "down_at": 0.12 },
        { "id": "mid_130", "arrangement": "mid", "bpm": 130.0, "core": "a", "rhythm": "b",
          "first_downbeat": 0.47978, "bar_seconds": 1.846154, "loop_bars": 100, "min_seconds": 15,
          "up_at": 0.47, "down_at": 0.25 },
        { "id": "mid_138", "unlock_seconds": 90, "arrangement": "mid", "bpm": 138.0, "core": "a", "rhythm": "b",
          "first_downbeat": 0.45196, "bar_seconds": 1.73913, "loop_bars": 100, "min_seconds": 15,
          "up_at": 0.6, "down_at": 0.4 },
        { "id": "stress_138", "unlock_seconds": 150, "arrangement": "stress", "bpm": 138.0, "core": "a", "rhythm": "b",
          "first_downbeat": 0.69378, "bar_seconds": 1.73913, "loop_bars": 43, "min_seconds": 15,
          "up_at": 0.78, "down_at": 0.5 },
        { "id": "stress_150", "unlock_seconds": 240, "arrangement": "stress", "bpm": 150.04, "core": "a", "rhythm": "b",
          "first_downbeat": 0.6381, "bar_seconds": 1.59957, "loop_bars": 43, "min_seconds": 15,
          "up_at": null, "down_at": 0.7 }
      ],
      "transitions": {
        "normal>mid": { "points": [ { "exit_bar": 11, "entry_bar": 11 } ], "crossfade_beats": 1,
                        "rhythm_out_bars": 0, "rhythm_in_bars": 2 },
        "mid>normal": { "points": [ { "exit_bar": 8, "entry_bar": 8 } ], "crossfade_beats": 1,
                        "rhythm_out_bars": 2, "rhythm_in_bars": 0 },
        "mid>stress": { "points": [ { "exit_bar": 85, "entry_bar": 26 } ], "crossfade_beats": 1,
                        "rhythm_out_bars": 0, "rhythm_in_bars": 4 },
        "stress>mid": { "points": [ { "exit_bar": 17, "entry_bar": 18 } ], "crossfade_beats": 1,
                        "rhythm_out_bars": 2, "rhythm_in_bars": 0 }
      }
    }"#;

#[cfg(test)]
pub(crate) fn parse_test_plan(json: &str, len_seconds: f64) -> Result<Plan, String> {
    let frames = (len_seconds * 44100.0) as u64;
    Plan::parse(json, Path::new("/test"), |_| Ok(TrackInfo { sample_rate: 44100, frames }))
}

#[cfg(test)]
pub(crate) fn test_plan(len_seconds: f64) -> Plan {
    parse_test_plan(TEST_PLAN_JSON, len_seconds).unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plan_converts_to_frames() {
        let plan = test_plan(400.0);
        let r = &plan.rungs[0];
        assert_eq!(r.loop_start(), (0.23787f64 * 44100.0).round() as u64);
        assert!((r.bar - 1.981421 * 44100.0).abs() < 1e-6);
        assert_eq!(r.min_frames, 15 * 44100);
        assert_eq!(r.min_frames_before_transition, 30 * 44100);
        let t = plan.transition(&plan.rungs[3], &plan.rungs[4]).unwrap();
        assert_eq!((t.points[0], t.rhythm_in_bars), ((85, 26), 4));
        // Starting stack (7) just under the first threshold (0.18); 12 and above: 1.
        let row = |r| UIState { top_effective_row: r, ..Default::default() };
        assert!((plan.height(&row(7)) - 1.0 / 6.0).abs() < 1e-6);
        assert_eq!((plan.height(&row(3)), plan.height(&row(12)), plan.height(&row(14))), (0.0, 1.0, 1.0));
    }

    /// Time model: a stack of 5 rows never stresses; above that, a high stack weighs more
    /// when the stack rises fast; the extremes stay reachable.
    #[test]
    fn time_model_shifts_with_creep_speed() {
        let json = TEST_PLAN_JSON.replace(r#""calm_row": 6, "top_row": 12"#,
            r#""calm_row": 5, "panic_seconds": 24, "speed_exponent": 0.5"#);
        let plan = parse_test_plan(&json, 400.0).unwrap();
        let ui = |rows: f32, spr: f32| UIState { rows_to_loss: rows, seconds_per_row: spr, ..Default::default() };
        let h = |rows, spr| plan.height(&ui(rows, spr));
        let (start, two_min) = (72.0, 1440.0 / 260.0); // creep speed at the start, and after 2 min
        // 5 rows of stack (top at row 5): 7 rows to loss, zero stress, even at maximum
        // speed.
        for spr in [start, two_min, 0.6] {
            assert_eq!(h(7.0, spr), 0.0);
            assert_eq!(h(7.4, spr), 0.0);
        }
        assert!(h(5.0, start) < 0.18, "start, medium stack: normal ({})", h(5.0, start));
        assert!((0.33..0.6).contains(&h(1.5, start)), "start, high stack: intermediate ({})", h(1.5, start));
        assert!(h(5.0, two_min) < 0.33, "2 min, medium stack: normal ({})", h(5.0, two_min));
        assert!(h(1.5, two_min) > 0.78, "2 min, high stack: stress ({})", h(1.5, two_min));
        assert!(h(1.5, two_min) > h(1.5, start) + 0.2, "speed makes a high stack weigh more");
        // Limits: at the edge of defeat even early on, almost empty stack even late.
        assert_eq!(h(0.2, start), 1.0);
        assert!(h(11.0, two_min) < 0.33);
        assert_eq!(plan.height(&UIState { creep_freeze: true, rows_to_loss: 5.0, seconds_per_row: start,
            ..Default::default() }), 1.0);
        // `calm_row` above the safe height: rejected.
        let bad = json.replace(r#""calm_row": 5"#, r#""calm_row": 12"#);
        assert!(parse_test_plan(&bad, 400.0).is_err());
    }

    /// At the start (3 rungs available), heavy stress leads to the intermediate rung; once
    /// everything is unlocked, the thresholds are those of the file.
    #[test]
    fn thresholds_spread_over_available_rungs() {
        let plan = test_plan(400.0);
        assert_eq!((plan.available(0.0), plan.available(89.9), plan.available(90.0)), (3, 3, 4));
        assert_eq!(plan.available(1000.0), 6);
        let close = |a: Option<f32>, b: f32| (a.unwrap() - b).abs() < 1e-4;
        let (up0, down0) = plan.thresholds(0, 3);
        assert!(close(up0, 0.40) && down0.is_none(), "{up0:?}");
        let (up1, down1) = plan.thresholds(1, 3);
        assert!(close(up1, 0.78) && close(down1, 0.325), "{up1:?} {down1:?}");
        let (up2, down2) = plan.thresholds(2, 3);
        assert!(up2.is_none() && close(down2, 0.70));
        // Everything available: thresholds of the file.
        for (i, r) in plan.rungs.iter().enumerate() {
            let (u, d) = plan.thresholds(i, 6);
            assert_eq!((u.map(|x| (x * 1e4).round()), d.map(|x| (x * 1e4).round())),
                (r.up_at.map(|x| (x * 1e4).round()), r.down_at.map(|x| (x * 1e4).round())));
        }
        // Hysteresis holds: each down threshold is below the up threshold that leads to it.
        for k in 2..=6 {
            for i in 0..k - 1 {
                assert!(plan.thresholds(i + 1, k).1.unwrap() < plan.thresholds(i, k).0.unwrap());
            }
        }
        let bad = TEST_PLAN_JSON.replace(r#""unlock_seconds": 150"#, r#""unlock_seconds": 60"#);
        assert!(parse_test_plan(&bad, 400.0).unwrap_err().contains("unlock_seconds"));
    }

    #[test]
    fn plan_rejects_short_tracks_and_bad_points() {
        // 60 s: the loop of normal_121 (67 bars, about 133 s) does not fit.
        let e = parse_test_plan(TEST_PLAN_JSON, 60.0).unwrap_err();
        assert!(e.contains("before the end of bar"), "{e}");
        // Exit bar beyond the loop of mid (100 bars).
        let bad = TEST_PLAN_JSON.replace(r#""exit_bar": 85"#, r#""exit_bar": 101"#);
        let e = parse_test_plan(&bad, 400.0).unwrap_err();
        assert!(e.contains("outside the loop"), "{e}");
        // Missing transition.
        let bad = TEST_PLAN_JSON.replace(r#""stress>mid""#, r#""stress>x""#);
        assert!(parse_test_plan(&bad, 400.0).unwrap_err().contains("stress>mid"));
        assert!(parse_test_plan("not json", 400.0).is_err());
    }
}
