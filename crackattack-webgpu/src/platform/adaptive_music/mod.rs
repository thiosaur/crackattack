//! Adaptive music: during a game, the music follows the height of the local player's stack
//! along a ladder of precomputed rungs (arrangement x tempo) described by `rungs.json`.
//!
//! - `plan`: loads and validates `rungs.json`.
//! - `controller`: smoothed stress and rung state machine (pure, driven by the mixer clock).
//! - `mixer`: the rodio `Source` that mixes the voices sample by sample.
//! - `track`: streaming readers for the tracks, WAV (`wav`) or Ogg Vorbis (`ogg`).

pub mod controller;
pub mod mixer;
pub mod ogg;
pub mod plan;
pub mod track;
pub mod wav;

use std::path::Path;

use crackattack_core::frame_data::UIState;
use std::sync::Arc;

use controller::{Change, Controller, Stress, VoicePlan};
use mixer::{AdaptiveMusic, Command, Envelope, MixerHandle, Voice};
use plan::Plan;

/// Interval between diagnostic log lines (`--music-debug`), in seconds of music.
const DEBUG_PERIOD_SECONDS: f64 = 1.0;

/// One game of adaptive music: the controller, and the handle of the mixer plugged into the
/// music sink.
pub struct AdaptiveSession {
    plan: Arc<Plan>,
    mixer: MixerHandle,
    controller: Controller,
    stress: Stress,
    voice_id: u32,
    next_id: u32,
    debug: bool,
    next_debug: u64,
}

impl AdaptiveSession {
    /// Loads the rung ladder from `dir`. On error the game falls back to `game.ogg`.
    pub fn load(dir: &Path) -> Result<Plan, String> {
        Plan::load(dir)
    }

    /// Starts a game: returns the session and the `Source` to plug into the sink.
    pub fn start(plan: Plan, debug: bool) -> (AdaptiveSession, AdaptiveMusic) {
        let plan = Arc::new(plan);
        let (music, mixer) = AdaptiveMusic::new(plan.sample_rate);
        // The first voice also starts after the prepare delay: opening an Ogg track and
        // seeking in it takes longer than the mixer's first buffer.
        let (controller, voice) = Controller::start(plan.clone(), plan.frames(controller::PREPARE_SECONDS) as u64);
        let session = AdaptiveSession {
            stress: Stress::new(&plan),
            plan,
            mixer,
            controller,
            voice_id: 1,
            next_id: 2,
            debug,
            next_debug: 0,
        };
        session.mixer.send(Command::Start(Box::new(session.spawn_voice(1, &voice))));
        (session, music)
    }

    /// Called on every game tick (not while paused). The local player's stack state is mapped
    /// to [0, 1] by the `rungs.json` model; `dt` is the time step in seconds.
    pub fn update(&mut self, ui: &UIState, dt: f32) {
        let height = self.plan.height(ui);
        self.stress.update(height, dt);
        let clock = self.mixer.clock();
        let elapsed = ui.clock_time as f64 / crackattack_core::constants::STEPS_PER_SECOND as f64;
        let available = self.plan.available(elapsed);
        if let Some(change) = self.controller.update(self.stress.value, clock, available) {
            self.apply(change);
        }
        if self.debug && clock >= self.next_debug {
            self.next_debug = clock + self.plan.frames(DEBUG_PERIOD_SECONDS) as u64;
            let pending = self.controller.pending()
                .map(|(to, at)| format!(", to {} in {:.1} s", self.plan.rungs[to].id,
                    self.plan.seconds(at.saturating_sub(clock) as f64)))
                .unwrap_or_default();
            log::info!("Music: stress {:.2} (stack {:.2}: row {}, {:.1} rows left at {:.1} s/row{}), \
                rung {} ({available} reachable){pending}, t {:.1} s, missed frames {}",
                self.stress.value, height, ui.top_effective_row, ui.rows_to_loss, ui.seconds_per_row,
                if ui.creep_freeze { ", frozen" } else { "" },
                self.plan.rungs[self.controller.rung()].id,
                self.plan.seconds(clock as f64), self.mixer.late_frames());
        }
    }

    fn apply(&mut self, change: Change) {
        let (from, to) = (&self.plan.rungs[change.from], &self.plan.rungs[change.to]);
        log::info!("Music: {:?} {} → {} at {:.2} s (decided at {:.2} s, stress {:.2})",
            change.kind, from.id, to.id, self.plan.seconds(change.at as f64),
            self.plan.seconds(change.decided_at as f64), self.stress.value);
        let id = self.next_id;
        self.next_id += 1;
        self.mixer.send(Command::Start(Box::new(self.spawn_voice(id, &change.voice))));
        self.mixer.send(Command::Envelope {
            id: self.voice_id,
            master: change.old_master,
            rhythm: change.old_rhythm,
            stop_at: Some(change.old_stop_at),
        });
        self.voice_id = id;
    }

    fn spawn_voice(&self, id: u32, v: &VoicePlan) -> Voice {
        let rung = &self.plan.rungs[v.rung];
        let (core, rhythm) = (rung.core.clone(), rung.rhythm.clone());
        Voice::spawn(
            id,
            v.timeline.start_clock,
            v.timeline.cursor(),
            Envelope::new(v.master.0, v.master.1.clone()),
            Envelope::new(v.rhythm.0, v.rhythm.1.clone()),
            move || {
                Ok((track::open(&core)?, track::open(&rhythm)?))
            },
        )
    }
}
