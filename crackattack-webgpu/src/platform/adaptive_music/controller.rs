//! The game-side controller: smooths the stress, picks the rung and schedules the changes,
//! on the mixer clock. Pure functions, tested without sound.
//!
//! Rules:
//! - One rung at a time: go up if stress >= `up_at`, go down if stress < `down_at`. The
//!   thresholds are spread over the available rungs (`Plan::thresholds`).
//! - No change before `min_seconds` on the rung, and no arrangement transition before
//!   `min_seconds_before_transition` in the arrangement.
//! - A scheduled change always completes.

use std::collections::VecDeque;
use std::sync::Arc;

use super::mixer::{LoopCursor, Ramp, Shape};
use super::plan::{Plan, Rung};

/// Minimum lead time between a decision and its execution: the time to open the incoming
/// voice and seek it in its thread.
pub(super) const PREPARE_SECONDS: f64 = 0.3;
/// Crossfade length of a tempo change, centered on the bar line.
const TEMPO_FADE_SECONDS: f64 = 0.06;

/// Stress is the mean danger over the last `window_seconds` of play, so the music changes
/// rarely and slowly. At the start of a game the window is filled with zeros.
#[derive(Debug, Clone)]
pub struct Stress {
    pub value: f32,
    window: f64,
    /// (duration, danger) of the recent steps, and their sums.
    samples: VecDeque<(f64, f64)>,
    span: f64,
    sum: f64,
}

impl Stress {
    pub fn new(plan: &Plan) -> Self {
        let window = plan.window_seconds as f64;
        let mut samples = VecDeque::new();
        samples.push_back((window, 0.0));
        Self { value: 0.0, window, samples, span: window, sum: 0.0 }
    }

    /// `h` is the stack danger in [0, 1]; `dt` is the game time step in seconds.
    pub fn update(&mut self, h: f32, dt: f32) {
        let (dt, h) = (dt as f64, h as f64);
        self.samples.push_back((dt, h));
        self.span += dt;
        self.sum += dt * h;
        // Drop what leaves the window (the oldest step is cut if needed).
        while self.span > self.window {
            let excess = self.span - self.window;
            let front = self.samples.front_mut().expect("the window is never empty");
            if front.0 <= excess {
                self.span -= front.0;
                self.sum -= front.0 * front.1;
                self.samples.pop_front();
            } else {
                front.0 -= excess;
                self.span -= excess;
                self.sum -= excess * front.1;
            }
        }
        self.value = (self.sum / self.span).clamp(0.0, 1.0) as f32;
    }
}

/// Where a voice is in its file, as a function of the mixer clock.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Timeline {
    pub start_clock: u64,
    pub start_pos: u64,
    pub loop_start: u64,
    pub loop_end: u64,
}

impl Timeline {
    pub fn pos_at(&self, clock: u64) -> u64 {
        let played = clock.saturating_sub(self.start_clock);
        LoopCursor::new(self.start_pos + played, self.loop_start, self.loop_end).pos
    }

    pub fn cursor(&self) -> LoopCursor {
        LoopCursor::new(self.start_pos, self.loop_start, self.loop_end)
    }
}

/// A voice to start: its rung, position and gains.
#[derive(Debug, Clone)]
pub struct VoicePlan {
    pub rung: usize,
    pub timeline: Timeline,
    pub master: (f32, Vec<Ramp>),
    pub rhythm: (f32, Vec<Ramp>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangeKind {
    Tempo,
    Transition,
}

/// A scheduled change: the incoming voice, and what to do with the outgoing one.
#[derive(Debug, Clone)]
pub struct Change {
    pub kind: ChangeKind,
    pub from: usize,
    pub to: usize,
    pub decided_at: u64,
    /// Clock of the bar line (or join point) where the change happens.
    pub at: u64,
    pub voice: VoicePlan,
    pub old_master: Vec<Ramp>,
    pub old_rhythm: Vec<Ramp>,
    pub old_stop_at: u64,
}

#[derive(Debug, Clone, Copy)]
struct Pending {
    to: usize,
    at: u64,
    timeline: Timeline,
}

pub struct Controller {
    plan: Arc<Plan>,
    rung: usize,
    timeline: Timeline,
    /// Clock of arrival on the rung, and in the arrangement.
    t_rung: u64,
    t_arr: u64,
    pending: Option<Pending>,
}

fn timeline_for(rung: &Rung, start_clock: u64, start_pos: u64) -> Timeline {
    Timeline { start_clock, start_pos, loop_start: rung.loop_start(), loop_end: rung.loop_end() }
}

/// Next bar start at or after clock `c_min` whose number satisfies `want`, as
/// (clock, number). The end of the loop counts as the start of bar 1.
pub fn next_bar(rung: &Rung, tl: &Timeline, c_min: u64, want: impl Fn(u32) -> bool) -> Option<(u64, u32)> {
    let p = tl.pos_at(c_min) as f64;
    let label = |b: u32| if b > rung.loop_bars { 1 } else { b };
    for b in 1..=rung.loop_bars + 1 {
        let s = rung.bar_start(b);
        if s >= p && want(label(b)) {
            return Some((c_min + (s - p).round() as u64, label(b)));
        }
    }
    // After wrapping back to the loop start (bar 1 is covered by the loop end).
    let to_wrap = tl.loop_end as f64 - p;
    (2..=rung.loop_bars).find(|&b| want(b)).map(|b| {
        let s = rung.bar_start(b) - tl.loop_start as f64;
        (c_min + (to_wrap + s).round() as u64, b)
    })
}

/// Start position of a voice that must be at the start of bar `b` in `back` frames. Bar 1 is
/// taken as the continuation of the last bar of the loop.
fn pos_before_bar(rung: &Rung, b: u32, back: u64) -> u64 {
    let s = if b <= 1 || b > rung.loop_bars { rung.bar_start(rung.loop_bars + 1) } else { rung.bar_start(b) };
    (s.round() as u64).saturating_sub(back)
}

impl Controller {
    /// Start of a game: rung 0, at the beginning of the file (pickup included).
    pub fn start(plan: Arc<Plan>, clock: u64) -> (Controller, VoicePlan) {
        let timeline = timeline_for(&plan.rungs[0], clock, 0);
        let voice = VoicePlan { rung: 0, timeline, master: (1.0, vec![]), rhythm: (1.0, vec![]) };
        let c = Controller { plan, rung: 0, timeline, t_rung: clock, t_arr: clock, pending: None };
        (c, voice)
    }

    pub fn rung(&self) -> usize {
        self.rung
    }

    #[cfg(test)]
    pub fn timeline(&self) -> Timeline {
        self.timeline
    }

    /// Scheduled change as (target rung, clock).
    pub fn pending(&self) -> Option<(usize, u64)> {
        self.pending.map(|p| (p.to, p.at))
    }

    /// `available` is the number of rungs reachable at this point of the game
    /// (`Plan::available`).
    pub fn update(&mut self, stress: f32, clock: u64, available: usize) -> Option<Change> {
        if let Some(p) = self.pending {
            if clock < p.at {
                return None;
            }
            let arrangement_changed = self.plan.rungs[p.to].arrangement != self.plan.rungs[self.rung].arrangement;
            self.rung = p.to;
            self.timeline = p.timeline;
            self.t_rung = p.at;
            if arrangement_changed {
                self.t_arr = p.at;
            }
            self.pending = None;
        }

        let plan = self.plan.clone();
        let r = &plan.rungs[self.rung];
        let (up_at, down_at) = plan.thresholds(self.rung, available.max(self.rung + 1));
        let target = if up_at.is_some_and(|u| stress >= u) {
            self.rung + 1
        } else if down_at.is_some_and(|d| stress < d) {
            self.rung - 1
        } else {
            return None;
        };
        if clock.saturating_sub(self.t_rung) < r.min_frames {
            return None;
        }
        let to = &plan.rungs[target];
        let change = if to.arrangement == r.arrangement {
            self.tempo_change(r, target, to, clock)
        } else {
            if clock.saturating_sub(self.t_arr) < r.min_frames_before_transition {
                return None;
            }
            self.transition(r, target, to, clock)?
        };
        self.pending = Some(Pending { to: target, at: change.at, timeline: change.voice.timeline });
        Some(change)
    }

    /// Same arrangement: at the next bar line, play the same bar from the other file, with a
    /// short crossfade centered on the bar line.
    fn tempo_change(&self, r: &Rung, target: usize, to: &Rung, clock: u64) -> Change {
        let half = (self.plan.frames(TEMPO_FADE_SECONDS) / 2.0).round() as u64;
        let c_min = clock + self.plan.frames(PREPARE_SECONDS) as u64 + half;
        let (at, bar) = next_bar(r, &self.timeline, c_min, |_| true)
            .expect("a loop always has a next bar");
        let (fade_start, fade_end) = (at - half, at + half);
        Change {
            kind: ChangeKind::Tempo,
            from: self.rung,
            to: target,
            decided_at: clock,
            at,
            voice: VoicePlan {
                rung: target,
                timeline: timeline_for(to, fade_start, pos_before_bar(to, bar, half)),
                master: (0.0, vec![Ramp { start: fade_start, end: fade_end, from: 0.0, to: 1.0, shape: Shape::EqualPowerIn }]),
                rhythm: (1.0, vec![]),
            },
            old_master: vec![Ramp { start: fade_start, end: fade_end, from: 1.0, to: 0.0, shape: Shape::EqualPowerOut }],
            old_rhythm: vec![],
            old_stop_at: fade_end,
        }
    }

    /// Arrangement change: at the next join point, with a centered crossfade. The rhythm track
    /// fades out before a downward change and fades in after an upward one.
    fn transition(&self, r: &Rung, target: usize, to: &Rung, clock: u64) -> Option<Change> {
        let t = self.plan.transition(r, to)?;
        let crossfade = t.crossfade_beats * self.plan.frames(60.0 / r.bpm);
        let half = (crossfade / 2.0).round() as u64;
        let out_frames = (t.rhythm_out_bars as f64 * r.bar).round() as u64;
        let in_frames = (t.rhythm_in_bars as f64 * to.bar).round() as u64;
        let c_min = clock + self.plan.frames(PREPARE_SECONDS) as u64 + half.max(out_frames);
        let (at, entry) = t.points.iter()
            .filter_map(|&(exit, entry)| next_bar(r, &self.timeline, c_min, |b| b == exit).map(|(at, _)| (at, entry)))
            .min_by_key(|&(at, _)| at)?;
        let (fade_start, fade_end) = (at - half, at + half);
        let rhythm = if in_frames > 0 {
            (0.0, vec![Ramp { start: at, end: at + in_frames, from: 0.0, to: 1.0, shape: Shape::Linear }])
        } else {
            (1.0, vec![])
        };
        let old_rhythm = if out_frames > 0 {
            vec![Ramp { start: at - out_frames, end: at, from: 1.0, to: 0.0, shape: Shape::Linear }]
        } else {
            vec![]
        };
        Some(Change {
            kind: ChangeKind::Transition,
            from: self.rung,
            to: target,
            decided_at: clock,
            at,
            voice: VoicePlan {
                rung: target,
                timeline: timeline_for(to, fade_start, pos_before_bar(to, entry, half)),
                master: (0.0, vec![Ramp { start: fade_start, end: fade_end, from: 0.0, to: 1.0, shape: Shape::EqualPowerIn }]),
                rhythm,
            },
            old_master: vec![Ramp { start: fade_start, end: fade_end, from: 1.0, to: 0.0, shape: Shape::EqualPowerOut }],
            old_rhythm,
            old_stop_at: fade_end,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::platform::adaptive_music::plan::test_plan;

    const SR: f64 = 44100.0;
    const TICK: u64 = 882; // 20 ms

    /// Runs the controller for `seconds` seconds with a stress given as a function of time.
    /// Returns the scheduled changes and, for each, the timeline being left.
    fn run(c: &mut Controller, from_s: f64, seconds: f64, stress: impl Fn(f64) -> f32) -> Vec<(Change, Timeline)> {
        let mut out = Vec::new();
        let mut clock = (from_s * SR) as u64;
        let end = ((from_s + seconds) * SR) as u64;
        while clock < end {
            let before = c.timeline();
            if let Some(ch) = c.update(stress(clock as f64 / SR), clock, 6) {
                out.push((ch, before));
            }
            clock += TICK;
        }
        out
    }

    fn secs(frames: u64) -> f64 {
        frames as f64 / SR
    }

    /// Checks that the timeline is at the start of a bar at the given clock and returns its number.
    fn bar_at(rung: &Rung, tl: &Timeline, clock: u64) -> u32 {
        let x = (tl.pos_at(clock) as f64 - rung.first_downbeat) / rung.bar;
        assert!((x - x.round()).abs() * rung.bar < 2.0, "not on a bar line: {x}");
        let b = x.round() as u32 + 1;
        if b > rung.loop_bars { 1 } else { b }
    }

    #[test]
    fn stress_is_the_mean_of_the_last_30_seconds() {
        let plan = test_plan(400.0);
        let mut s = Stress::new(&plan);
        let run = |s: &mut Stress, h: f32, seconds: u32| for _ in 0..seconds * 50 { s.update(h, 0.02); };
        run(&mut s, 1.0, 15); // game starting at maximum danger: half of the window
        assert!((s.value - 0.5).abs() < 1e-3, "{}", s.value);
        run(&mut s, 1.0, 15);
        assert!((s.value - 1.0).abs() < 1e-3);
        run(&mut s, 0.0, 25); // a peak is gone after 30 s
        assert!((s.value - 5.0 / 30.0).abs() < 1e-3, "{}", s.value);
        run(&mut s, 0.0, 5);
        assert!(s.value.abs() < 1e-3);
        run(&mut s, 1.0, 5); // 5 s of danger only weigh 5/30
        assert!((s.value - 5.0 / 30.0).abs() < 1e-3);
    }

    #[test]
    fn rapid_rise_climbs_one_rung_at_a_time_with_minimum_times() {
        let plan = Arc::new(test_plan(400.0));
        let (mut c, _) = Controller::start(plan.clone(), 0);
        let changes = run(&mut c, 0.0, 600.0, |_| 1.0);
        let path: Vec<(usize, usize)> = changes.iter().map(|(ch, _)| (ch.from, ch.to)).collect();
        assert_eq!(path, vec![(0, 1), (1, 2), (2, 3), (3, 4), (4, 5)]);

        let mut arrived = 0u64; // arrival on the current rung
        let mut arrived_arr = 0u64;
        for (ch, _) in &changes {
            assert!(ch.decided_at >= arrived, "only one change at a time");
            assert!(secs(ch.decided_at - arrived) >= 15.0, "minimum time at a tempo");
            assert!(ch.at >= ch.decided_at + (0.3 * SR) as u64);
            if ch.kind == ChangeKind::Transition {
                assert!(secs(ch.decided_at - arrived_arr) >= 30.0, "minimum time before a transition");
                arrived_arr = ch.at;
            }
            arrived = ch.at;
        }
        // The intermediate rung lasts at least 30 s.
        assert!(secs(changes[3].0.at - changes[1].0.at) >= 30.0);
        assert!(secs(changes[4].0.at) < 600.0);
        assert_eq!(c.rung(), 5);
    }

    /// At the start only three rungs are available: maximum stress stops at the intermediate
    /// rung 130.
    #[test]
    fn heavy_stress_at_start_stays_intermediate() {
        let plan = Arc::new(test_plan(400.0));
        let (mut c, _) = Controller::start(plan.clone(), 0);
        let mut path = Vec::new();
        let mut clock = 0u64;
        while clock < (300.0 * SR) as u64 {
            if let Some(ch) = c.update(1.0, clock, 3) { path.push((ch.from, ch.to)); }
            clock += TICK;
        }
        assert_eq!(path, vec![(0, 1), (1, 2)]);
    }

    #[test]
    fn stress_around_a_threshold_does_not_oscillate() {
        let plan = Arc::new(test_plan(400.0));
        let (mut c, _) = Controller::start(plan, 0);
        // Around the up threshold of rung 0 (0.18): it goes up once, and the down threshold
        // of rung 1 (0.12) prevents it from going back down.
        let changes = run(&mut c, 0.0, 300.0, |t| if (t as u64) % 2 == 0 { 0.17 } else { 0.19 });
        assert_eq!(changes.len(), 1);
        assert_eq!((changes[0].0.from, changes[0].0.to), (0, 1));
    }

    #[test]
    fn tempo_change_lands_on_the_same_bar() {
        let plan = Arc::new(test_plan(400.0));
        let (mut c, _) = Controller::start(plan.clone(), 0);
        let (ch, old) = run(&mut c, 0.0, 20.0, |_| 1.0).remove(0);
        assert_eq!(ch.kind, ChangeKind::Tempo);
        let b_old = bar_at(&plan.rungs[0], &old, ch.at);
        let b_new = bar_at(&plan.rungs[1], &ch.voice.timeline, ch.at);
        assert_eq!(b_old, b_new);
        // 60 ms crossfade centered on the bar line.
        let fade = &ch.voice.master.1[0];
        assert_eq!((fade.start + fade.end) / 2, ch.at);
        assert!((secs(fade.end - fade.start) - 0.06).abs() < 1e-3);
        assert_eq!(ch.old_stop_at, fade.end);
    }

    #[test]
    fn transition_uses_the_join_points_and_rhythm_ramps() {
        let plan = Arc::new(test_plan(400.0));
        let (mut c, _) = Controller::start(plan.clone(), 0);
        let changes = run(&mut c, 0.0, 120.0, |_| 1.0);
        let (up, old) = changes.iter().find(|(ch, _)| ch.kind == ChangeKind::Transition).unwrap();
        assert_eq!((up.from, up.to), (1, 2));
        assert_eq!(bar_at(&plan.rungs[1], old, up.at), 11);
        assert_eq!(bar_at(&plan.rungs[2], &up.voice.timeline, up.at), 11);
        // Going up: the incoming rhythm track starts at 0 and ramps up over 2 bars after the join point.
        let (g0, ramps) = &up.voice.rhythm;
        assert_eq!(*g0, 0.0);
        assert_eq!(ramps[0].start, up.at);
        assert_eq!(ramps[0].end - ramps[0].start, (2.0 * plan.rungs[2].bar).round() as u64);
        assert!(up.old_rhythm.is_empty());

        // Then stress drops: mid to normal, with a 2-bar rhythm fade-out.
        c.update(1.0, up.at, 6);
        assert_eq!(c.rung(), 2);
        let downs = run(&mut c, secs(up.at), 200.0, |_| 0.0);
        let (down, old) = &downs[0];
        assert_eq!((down.kind, down.from, down.to), (ChangeKind::Transition, 2, 1));
        assert_eq!(bar_at(&plan.rungs[2], old, down.at), 8);
        assert_eq!(bar_at(&plan.rungs[1], &down.voice.timeline, down.at), 8);
        let out = &down.old_rhythm[0];
        assert_eq!(out.end, down.at);
        assert_eq!(out.end - out.start, (2.0 * plan.rungs[2].bar).round() as u64);
        assert!(out.start >= down.decided_at, "de-escalation scheduled early enough");
        assert_eq!(down.voice.rhythm, (1.0, vec![]));
    }

    #[test]
    fn transition_takes_the_nearest_join_point() {
        use crate::platform::adaptive_music::plan::{parse_test_plan, TEST_PLAN_JSON};
        let json = TEST_PLAN_JSON.replace(
            r#""points": [ { "exit_bar": 11, "entry_bar": 11 } ]"#,
            r#""points": [ { "exit_bar": 11, "entry_bar": 11 }, { "exit_bar": 44, "entry_bar": 46, "source": "auto" },
                          { "exit_bar": 24, "entry_bar": 24, "source": "auto" } ]"#);
        let plan = Arc::new(parse_test_plan(&json, 400.0).unwrap());
        let (mut c, _) = Controller::start(plan.clone(), 0);
        let (up, old) = run(&mut c, 0.0, 120.0, |_| 1.0).into_iter()
            .find(|(ch, _)| ch.kind == ChangeKind::Transition).unwrap();
        let exit = bar_at(&plan.rungs[1], &old, up.at);
        let entry = bar_at(&plan.rungs[2], &up.voice.timeline, up.at);
        assert!(matches!((exit, entry), (11, 11) | (24, 24) | (44, 46)), "{exit}→{entry}");
        // No other point came earlier.
        for b in [11, 24, 44] {
            let (at, _) = next_bar(&plan.rungs[1], &old, up.decided_at + (0.3 * SR) as u64 + 1000, |x| x == b).unwrap();
            assert!(at >= up.at);
        }
        assert!(secs(up.at - up.decided_at) < 45.0, "wait bounded by the gap between points");
    }

    #[test]
    fn next_bar_wraps_around_the_loop() {
        let plan = test_plan(400.0);
        let mid = &plan.rungs[2];
        // Intermediate-rung voice positioned at the start of bar 90 at clock 0.
        let tl = timeline_for(mid, 0, mid.bar_start(90).round() as u64);
        let (at, b) = next_bar(mid, &tl, 0, |b| b == 85).unwrap();
        assert_eq!(b, 85);
        // 11 bars up to the end of the loop (90..=100), then 84 from bar 1.
        let expected = 11.0 * mid.bar + 84.0 * mid.bar;
        assert!((at as f64 - expected).abs() <= 2.0);
        assert_eq!(bar_at(mid, &tl, at), 85);
        // The loop end counts as bar 1.
        let (at1, b1) = next_bar(mid, &tl, 0, |b| b == 1).unwrap();
        assert_eq!(b1, 1);
        assert!((at1 as f64 - 11.0 * mid.bar).abs() <= 2.0);
    }
}
