// Danger bars: the final countdown, as in the graphical version.
//
// When the stack touches the top (`creep_freeze`), a bar rises on each side of the board:
// purple while the loss countdown runs from 7 s down to 1 s, then red during the last
// second. It fades out when the stack is freed.

use crackattack_core::constants::{LOSS_DELAY, LOSS_DELAY_ELIMINATION};

/// Fade-out duration (ticks), as in the graphical version.
const FADE_TIME: i32 = 20;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State {
    Inactive,
    LowAlert,
    HighAlert,
    FadeLow,
    FadeHigh,
}

/// What to draw this frame.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct DangerBars {
    /// Height, as a fraction of the board (0 = nothing).
    pub height: f32,
    /// Red (last second) rather than purple.
    pub red: bool,
    /// Opacity: 1 while the alert lasts, down to 0 while fading out.
    pub alpha: f32,
}

pub(crate) struct DangerAnimator {
    state: State,
    bar: f32,
    fade: i32,
}

impl DangerAnimator {
    pub(crate) fn new() -> Self {
        Self { state: State::Inactive, bar: 0.0, fade: 0 }
    }

    /// One game tick.
    pub(crate) fn step(&mut self, creep_freeze: bool, loss_countdown: i32) {
        match self.state {
            State::Inactive | State::FadeLow | State::FadeHigh => {
                if self.state != State::Inactive {
                    self.fade -= 1;
                    if self.fade <= 0 {
                        self.state = State::Inactive;
                    }
                }
                if creep_freeze {
                    self.state = State::LowAlert;
                    self.fade = 0;
                }
            }
            State::LowAlert => {
                if !creep_freeze {
                    self.fade = FADE_TIME;
                    self.state = State::FadeLow;
                } else if loss_countdown <= LOSS_DELAY_ELIMINATION {
                    self.state = State::HighAlert;
                }
            }
            State::HighAlert => {
                if !creep_freeze {
                    self.fade = FADE_TIME;
                    self.state = State::FadeHigh;
                }
            }
        }
        // The bar fills once over the purple phase, then again over the red one.
        let fill = match self.state {
            State::LowAlert => Some(
                (LOSS_DELAY - loss_countdown) as f32 / (LOSS_DELAY - LOSS_DELAY_ELIMINATION) as f32,
            ),
            State::HighAlert => Some(
                (LOSS_DELAY_ELIMINATION - loss_countdown) as f32 / LOSS_DELAY_ELIMINATION as f32,
            ),
            _ => None,
        };
        if let Some(fill) = fill {
            self.bar = fill.clamp(0.0, 1.0);
        }
    }

    pub(crate) fn bars(&self) -> DangerBars {
        let alpha = match self.state {
            State::Inactive => return DangerBars::default(),
            State::LowAlert | State::HighAlert => 1.0,
            State::FadeLow | State::FadeHigh => self.fade as f32 / FADE_TIME as f32,
        };
        DangerBars {
            height: self.bar,
            red: matches!(self.state, State::HighAlert | State::FadeHigh),
            alpha,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn purple_then_red_as_the_countdown_runs_out() {
        let mut danger = DangerAnimator::new();
        assert_eq!(danger.bars(), DangerBars::default());
        let mut countdown = LOSS_DELAY;
        let mut seen_purple = false;
        while countdown > 0 {
            danger.step(true, countdown);
            let bars = danger.bars();
            if countdown > LOSS_DELAY_ELIMINATION {
                assert!(!bars.red);
                seen_purple |= bars.height > 0.5;
            } else if countdown < LOSS_DELAY_ELIMINATION {
                assert!(bars.red);
            }
            countdown -= 1;
        }
        assert!(seen_purple);
        assert!(danger.bars().height > 0.9);
    }

    #[test]
    fn the_bars_fade_out_when_the_stack_is_freed() {
        let mut danger = DangerAnimator::new();
        for countdown in (LOSS_DELAY - 100..LOSS_DELAY).rev() {
            danger.step(true, countdown);
        }
        danger.step(false, LOSS_DELAY);
        danger.step(false, LOSS_DELAY);
        let fading = danger.bars();
        assert!(fading.alpha < 1.0 && fading.alpha > 0.0);
        for _ in 0..FADE_TIME - 1 {
            danger.step(false, LOSS_DELAY);
        }
        assert_eq!(danger.bars(), DangerBars::default());
    }
}
