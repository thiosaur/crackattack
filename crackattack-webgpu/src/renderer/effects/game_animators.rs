
const DANGER_BAR_FADE_TIME: i32 = 20;
// Loss delays come from the core so they cannot diverge from the engine.
use crackattack_core::constants::{LOSS_DELAY, LOSS_DELAY_ELIMINATION};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DangerBarState {
    Inactive,
    LowAlert,
    HighAlert,
    FadeLowToInactive,
    FadeHighToInactive,
}

pub struct DangerBarSnapshot {
    pub visible: bool,
    pub bar: f32,
    pub alpha: f32,
    pub is_low_alert: bool,
    pub is_high_alert: bool,
}

pub struct DangerBarAnimator {
    state: DangerBarState,
    bar: f32,
    fade_countdown: i32,
}

impl Default for DangerBarAnimator {
    fn default() -> Self {
        Self::new()
    }
}

impl DangerBarAnimator {
    pub fn new() -> Self {
        Self {
            state: DangerBarState::Inactive,
            bar: 0.0,
            fade_countdown: 0,
        }
    }

    pub fn time_step(&mut self, creep_freeze: bool, loss_countdown: i32) {
        match self.state {
            DangerBarState::Inactive | DangerBarState::FadeLowToInactive | DangerBarState::FadeHighToInactive => {
                if matches!(self.state, DangerBarState::FadeLowToInactive | DangerBarState::FadeHighToInactive) {
                    self.fade_countdown -= 1;
                    if self.fade_countdown <= 0 {
                        self.state = DangerBarState::Inactive;
                    }
                }
                if creep_freeze {
                    self.state = DangerBarState::LowAlert;
                    self.fade_countdown = 0;
                }
            }
            DangerBarState::LowAlert => {
                if !creep_freeze {
                    self.fade_countdown = DANGER_BAR_FADE_TIME;
                    self.state = DangerBarState::FadeLowToInactive;
                } else if loss_countdown <= LOSS_DELAY_ELIMINATION {
                    self.state = DangerBarState::HighAlert;
                }
            }
            DangerBarState::HighAlert => {
                if !creep_freeze {
                    self.fade_countdown = DANGER_BAR_FADE_TIME;
                    self.state = DangerBarState::FadeHighToInactive;
                }
            }
        }

        if matches!(self.state, DangerBarState::LowAlert | DangerBarState::HighAlert) {
            self.bar = if self.state == DangerBarState::LowAlert {
                (LOSS_DELAY - loss_countdown) as f32
                    / (LOSS_DELAY - LOSS_DELAY_ELIMINATION) as f32
            } else {
                (LOSS_DELAY_ELIMINATION - loss_countdown) as f32
                    / LOSS_DELAY_ELIMINATION as f32
            };
            self.bar = self.bar.clamp(0.0, 1.0);
        }
    }

    pub fn snapshot(&self) -> DangerBarSnapshot {
        let alpha = match self.state {
            DangerBarState::FadeLowToInactive | DangerBarState::FadeHighToInactive => {
                self.fade_countdown as f32 / DANGER_BAR_FADE_TIME as f32
            }
            DangerBarState::LowAlert | DangerBarState::HighAlert => 1.0,
            DangerBarState::Inactive => 0.0,
        };
        DangerBarSnapshot {
            visible: self.state != DangerBarState::Inactive,
            bar: self.bar,
            alpha,
            is_low_alert: self.state == DangerBarState::LowAlert,
            is_high_alert: self.state == DangerBarState::HighAlert,
        }
    }
}

pub const LEVEL_LIGHT_COUNT: usize = 12;
const LEVEL_LIGHT_FADE_TIME: i32 = 150;
const LEVEL_LIGHT_SLIDE_DURATION: i32 = 45;
pub const LEVEL_LIGHT_SLIDE_OFFSET: f32 = 10.0;

#[derive(Clone, Copy, Debug)]
#[derive(Default)]
struct LightAnimState {
    is_red: bool,
    fading: Option<bool>, // Some(true) = fading to red, Some(false) = fading to blue
    fade_countdown: i32,
}


impl LightAnimState {
    fn set_red(&mut self) {
        if self.fading == Some(false) {
            self.fade_countdown = LEVEL_LIGHT_FADE_TIME - self.fade_countdown;
        } else {
            self.fade_countdown = LEVEL_LIGHT_FADE_TIME;
        }
        self.is_red = false;
        self.fading = Some(true);
    }

    fn set_blue(&mut self) {
        if self.fading == Some(true) {
            self.fade_countdown = LEVEL_LIGHT_FADE_TIME - self.fade_countdown;
        } else {
            self.fade_countdown = LEVEL_LIGHT_FADE_TIME;
        }
        self.is_red = false;
        self.fading = Some(false);
    }


    fn is_red_or_fading_to_red(&self) -> bool {
        self.is_red || self.fading == Some(true)
    }

    fn is_blue_or_fading_to_blue(&self) -> bool {
        !self.is_red && self.fading.is_none() || self.fading == Some(false)
    }
}

pub struct LevelLightsSnapshot {
    pub slide_progress: f32,
    pub slide_offset_right: f32,
    pub slide_offset_left: f32,
    pub panic_pulse: f32,
    pub local_level: i32,
}

pub struct LevelLightAnimator {
    lights: [LightAnimState; LEVEL_LIGHT_COUNT],
    slide_progress: f32,
    slide_alarm: i32,
    prev_top_effective_row: i32,
    initialized: bool,
    /// Slide-out progress during game over (0.0 = fully visible, 1.0 = hidden).
    pub slide_out_progress: f32,
}

impl Default for LevelLightAnimator {
    fn default() -> Self {
        Self::new()
    }
}

impl LevelLightAnimator {
    pub fn new() -> Self {
        Self {
            lights: [LightAnimState::default(); LEVEL_LIGHT_COUNT],
            slide_progress: 0.0,
            slide_alarm: 0,
            prev_top_effective_row: 0,
            initialized: false,
            slide_out_progress: 0.0,
        }
    }

    pub fn time_step(&mut self, top_effective_row: i32) {
        if !self.initialized {
            self.game_start(top_effective_row);
            self.initialized = true;
            return;
        }

        // Slide-in animation (ease-out cubic).
        if self.slide_alarm > 0 {
            self.slide_alarm -= 1;
            let t = 1.0 - (self.slide_alarm as f32 / LEVEL_LIGHT_SLIDE_DURATION as f32);
            let t1 = 1.0 - t;
            self.slide_progress = 1.0 - (t1 * t1 * t1);
        }

        // Fade timers.
        for n in 0..LEVEL_LIGHT_COUNT {
            let light = &mut self.lights[n];
            if let Some(to_red) = light.fading {
                light.fade_countdown -= 1;
                if light.fade_countdown <= 0 {
                    light.is_red = to_red;
                    light.fading = None;
                }
            }
        }

        // Light up or clear lights when the level changes.
        if top_effective_row > self.prev_top_effective_row {
            self.level_raise(top_effective_row);
        } else if top_effective_row < self.prev_top_effective_row {
            self.level_lower(top_effective_row);
        }
        self.prev_top_effective_row = top_effective_row;

    }

    fn game_start(&mut self, initial_level: i32) {
        let top_row = initial_level.min(LEVEL_LIGHT_COUNT as i32) as usize;
        for n in 0..top_row {
            self.lights[n].is_red = true;
            self.lights[n].fading = None;
        }
        for n in top_row..LEVEL_LIGHT_COUNT {
            self.lights[n].is_red = false;
            self.lights[n].fading = None;
        }
        self.prev_top_effective_row = initial_level;
        self.slide_alarm = LEVEL_LIGHT_SLIDE_DURATION;
    }

    fn level_raise(&mut self, top_effective_row: i32) {
        let mut row = (top_effective_row - 1).min(LEVEL_LIGHT_COUNT as i32 - 1);
        while row >= 0 {
            let light = &mut self.lights[row as usize];
            if light.is_red_or_fading_to_red() { break; }
            light.set_red();
            row -= 1;
        }
    }

    fn level_lower(&mut self, top_effective_row: i32) {
        let mut row = top_effective_row;
        while (row as usize) < LEVEL_LIGHT_COUNT {
            let light = &mut self.lights[row as usize];
            if light.is_blue_or_fading_to_blue() { break; }
            light.set_blue();
            row += 1;
        }
    }


    /// Advances the slide-out by one tick. Call each tick during the Finished phase.
    pub fn slide_out_step(&mut self) {
        // 100 ticks to slide out.
        self.slide_out_progress = (self.slide_out_progress + 0.01).min(1.0);
    }

    pub fn snapshot(&self) -> LevelLightsSnapshot {
        // Both progress values go 0 to 1: slide-in, then slide-out.
        let effective = self.slide_progress * (1.0 - self.slide_out_progress);
        let hidden = 1.0 - effective;

        let mut local_level = 0;
        for n in 0..LEVEL_LIGHT_COUNT {
            if self.lights[n].is_red_or_fading_to_red() {
                local_level = (n + 1) as i32;
            }
        }

        LevelLightsSnapshot {
            slide_progress: self.slide_progress,
            slide_offset_right: LEVEL_LIGHT_SLIDE_OFFSET * hidden,
            slide_offset_left: -LEVEL_LIGHT_SLIDE_OFFSET * hidden,
            panic_pulse: 0.0,
            local_level,
        }
    }
}
