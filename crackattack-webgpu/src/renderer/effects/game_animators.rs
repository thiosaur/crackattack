
const DANGER_BAR_FADE_TIME: i32 = 20;
// Loss delays come from the core so they cannot diverge from the engine.
use crackattack_core::constants::{LOSS_DELAY, LOSS_DELAY_ELIMINATION, PLAY_WIDTH};
use crackattack_core::vs::board_view::BOARD_VIEW_ROWS;
use crackattack_core::vs::{BoardCell, OpponentBoardView};

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

/// Time a new garbage slab of the opponent preview takes to fade from bright red to its normal
/// color.
const OPPONENT_GARBAGE_FLASH_SECONDS: f32 = 1.0;

/// Opponent board preview: holds the last received picture and makes each new garbage slab
/// redden when it appears, then fade.
///
/// A slab is new when its identifier was absent from the previous picture. The first picture
/// (session start) reddens nothing; a new round starts from a board without garbage, so it does
/// not either.
pub struct OpponentBoardAnimator {
    view: Option<OpponentBoardView>,
    /// Remaining flash per garbage identifier, 1.0 (just appeared) to 0.0.
    flash: [f32; 256],
}

impl Default for OpponentBoardAnimator {
    fn default() -> Self {
        Self::new()
    }
}

impl OpponentBoardAnimator {
    pub fn new() -> Self {
        Self { view: None, flash: [0.0; 256] }
    }

    pub fn time_step(&mut self, view: Option<&OpponentBoardView>, dt: f32) {
        let fade = dt / OPPONENT_GARBAGE_FLASH_SECONDS;
        for f in &mut self.flash {
            *f = (*f - fade).max(0.0);
        }
        let Some(view) = view else {
            self.view = None;
            return;
        };
        if self.view.as_ref() == Some(view) {
            return;
        }
        if let Some(previous) = &self.view {
            for row in 0..BOARD_VIEW_ROWS {
                for x in 0..PLAY_WIDTH {
                    if let BoardCell::Garbage(id) = view.cell(x, row) {
                        if !previous.has_garbage(id) {
                            self.flash[id as usize] = 1.0;
                        }
                    }
                }
            }
        }
        self.view = Some(view.clone());
    }

    /// The picture to draw, if the opponent publishes one.
    pub fn view(&self) -> Option<&OpponentBoardView> {
        self.view.as_ref()
    }

    /// Flash strength of garbage slab `id`, 1.0 when it just appeared, eased toward 0.0.
    pub fn flash(&self, id: u8) -> f32 {
        let f = self.flash[id as usize];
        f * f
    }
}

#[cfg(test)]
mod opponent_board_tests {
    use super::*;
    use crackattack_core::grid::{CellRef, CellType};
    use crackattack_core::{GameState, ScoringMode};

    fn view_with_garbage(ids: &[i16]) -> OpponentBoardView {
        let mut game = GameState::with_scoring(1, ScoringMode::Classic);
        for (i, &id) in ids.iter().enumerate() {
            game.grid.set(i as i32, 12, CellRef { id, cell_type: CellType::Garbage, phase: 0 });
        }
        OpponentBoardView::from_game(&game)
    }

    #[test]
    fn first_view_reddens_nothing() {
        let mut anim = OpponentBoardAnimator::new();
        anim.time_step(Some(&view_with_garbage(&[3])), 0.02);
        assert_eq!(anim.flash(3), 0.0);
    }

    #[test]
    fn new_slab_reddens_then_fades() {
        let mut anim = OpponentBoardAnimator::new();
        anim.time_step(Some(&view_with_garbage(&[3])), 0.02);
        anim.time_step(Some(&view_with_garbage(&[3, 5])), 0.02);
        assert_eq!(anim.flash(5), 1.0);
        assert_eq!(anim.flash(3), 0.0, "a slab already shown must not redden again");
        anim.time_step(Some(&view_with_garbage(&[3, 5])), 0.5);
        assert!(anim.flash(5) > 0.0 && anim.flash(5) < 1.0);
        anim.time_step(Some(&view_with_garbage(&[3, 5])), 0.6);
        assert_eq!(anim.flash(5), 0.0);
    }
}

/// Logo shake when garbage lands on the board: a short tremor that dies out.
pub struct LogoShakeAnimator {
    /// Strength of the current tremor (0 = none, 1 = the largest).
    strength: f32,
    /// Seconds since the tremor started.
    elapsed: f32,
}

/// Duration of a tremor (seconds).
const LOGO_SHAKE_DURATION: f32 = 0.5;
/// Largest offset (world units; the logo is about 6.7 wide) and tilt (degrees).
const LOGO_SHAKE_OFFSET: f32 = 0.38;
const LOGO_SHAKE_TILT_DEG: f32 = 2.4;

impl Default for LogoShakeAnimator {
    fn default() -> Self {
        Self::new()
    }
}

impl LogoShakeAnimator {
    pub fn new() -> Self {
        Self { strength: 0.0, elapsed: LOGO_SHAKE_DURATION }
    }

    /// Garbage of `width` x `height` cells landed: starts a tremor, stronger for bigger
    /// slabs. A landing during a stronger tremor keeps the stronger one.
    pub fn garbage_landed(&mut self, width: i32, height: i32) {
        let strength = (0.45 + 0.05 * width as f32 + 0.1 * height as f32).min(1.0);
        if strength >= self.current_strength() {
            self.strength = strength;
            self.elapsed = 0.0;
        }
    }

    pub fn time_step(&mut self, dt: f32) {
        self.elapsed = (self.elapsed + dt).min(LOGO_SHAKE_DURATION);
    }

    /// Strength left, decaying to 0 over the duration.
    fn current_strength(&self) -> f32 {
        let left = 1.0 - self.elapsed / LOGO_SHAKE_DURATION;
        self.strength * left * left
    }

    /// Current offset (world units, x and y) and tilt (radians, counterclockwise).
    pub fn offset(&self) -> (f32, f32, f32) {
        let s = self.current_strength();
        if s <= 0.0 {
            return (0.0, 0.0, 0.0);
        }
        // Three unrelated frequencies, so the tremor does not look like a swing.
        let t = self.elapsed * std::f32::consts::TAU;
        (
            s * LOGO_SHAKE_OFFSET * (19.0 * t).sin(),
            s * LOGO_SHAKE_OFFSET * 0.7 * (23.0 * t + 1.0).sin(),
            s * LOGO_SHAKE_TILT_DEG.to_radians() * (16.0 * t + 2.0).sin(),
        )
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn logo_shake_dies_out() {
        let mut shake = LogoShakeAnimator::new();
        assert_eq!(shake.offset(), (0.0, 0.0, 0.0));
        shake.garbage_landed(6, 2);
        shake.time_step(0.03);
        assert_ne!(shake.offset(), (0.0, 0.0, 0.0));
        shake.time_step(1.0);
        assert_eq!(shake.offset(), (0.0, 0.0, 0.0));
    }

    #[test]
    fn bigger_garbage_shakes_harder_and_a_small_one_does_not_cut_it_short() {
        let mut small = LogoShakeAnimator::new();
        small.garbage_landed(3, 1);
        let mut big = LogoShakeAnimator::new();
        big.garbage_landed(6, 3);
        assert!(big.current_strength() > small.current_strength());
        big.time_step(0.05);
        let before = big.current_strength();
        big.garbage_landed(3, 1);
        assert_eq!(big.current_strength(), before);
    }
}
