//! Pause menu modal — Resume / Ready / Quit with anti-reflex protection.
//!
//! In network mode, when the remote opponent initiates the pause, all
//! interaction is blocked for 1 second (splash title only) so stale
//! key presses don't accidentally trigger Resume or Quit. When the
//! local player initiated the pause, the full menu appears immediately.

use super::egui_theme::{Palette, TextSize};

/// Outcome of the pause modal for this frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PauseOutcome {
    /// Modal still open, no action taken.
    StillOpen,
    /// Player chose Resume (solo/vs-computer: immediate resume).
    Resume,
    /// Player signaled ready to resume (network: marks local ready).
    ResumeReady,
    /// Player chose Quit.
    Quit,
}

/// Context passed to the pause modal each frame.
pub struct PauseModalContext {
    /// Local player is ready to resume (network only).
    pub local_ready: bool,
    /// Remote player is ready to resume (network only).
    pub remote_ready: bool,
    /// True for network games (shows ready state), false for solo/vs-computer.
    pub is_network: bool,
    /// Seconds since the modal was opened (for anti-reflex delay).
    pub seconds_open: f32,
    /// True when the anti-reflex delay should apply (network + remote-initiated pause).
    /// During the delay, only the title is shown — no buttons, no interaction.
    pub needs_delay: bool,
}

const QUIT_DELAY_SECS: f32 = 1.0;

/// Render the pause modal. Returns the outcome for this frame.
///
/// `selection` is the currently selected item (0=Resume, 1=Quit),
/// stored in SessionState and reset to 0 when the menu opens.
/// Uses `Cell` so the render pass can update it without &mut self.
pub fn render_pause_modal(
    ctx: &egui::Context,
    pctx: &PauseModalContext,
    selection: &std::cell::Cell<usize>,
) -> PauseOutcome {
    let mut outcome = PauseOutcome::StillOpen;

    // Anti-reflex: when the delay applies and hasn't elapsed, show only the title.
    let in_protection = in_protection(pctx.needs_delay, pctx.seconds_open);

    let t = crate::app::locale::t();

    if in_protection {
        // Keyboard and gamepad input for the pause menu goes through `dispatch_input` ->
        // `handle_pause_menu`, which also applies the anti-reflex protection. Only the title
        // is drawn here.
        super::helpers::modal_area("pause_menu")
            .show(ctx, |ui| {
                egui::Frame::window(&ctx.style())
                    .inner_margin(egui::Margin::same(20))
                    .show(ui, |ui| {
                        ui.vertical_centered(|ui| {
                            ui.label(egui::RichText::new(t.pause_title)
                                .size(TextSize::TITLE).strong().color(Palette::TEXT_DIM));
                        });
                    });
            });
        return PauseOutcome::StillOpen;
    }

    // Full interactive menu

    // For network: if local already ready, the resume row is informational (no action).
    // For solo: resume is always available.
    let can_resume = if pctx.is_network {
        !pctx.local_ready
    } else {
        true
    };

    // Selection is driven by the keyboard/gamepad handler (`handle_pause_menu` ->
    // `pause_menu_step`). Here it is read-only, for display and mouse clicks/hover.
    let sel = selection.get();
    // Mouse: row hovered this frame (moves the shared selection, see after the Area).
    let mut hovered_now: Option<usize> = None;

    super::helpers::modal_area("pause_menu")
        .show(ctx, |ui| {
            egui::Frame::window(&ctx.style())
                .inner_margin(egui::Margin::same(12))
                .show(ui, |ui| {
                    // Fixed width — fits "Ready to resume" without growing.
                    let modal_w = 190.0;
                    ui.set_min_width(modal_w);
                    ui.set_max_width(modal_w);

                    // Row inner width (modal minus frame stroke/padding)
                    let row_w = modal_w - 4.0;

                    ui.vertical_centered(|ui| {
                        ui.label(egui::RichText::new(t.pause_title)
                            .size(TextSize::TITLE).strong().color(Palette::TEXT_DIM));
                    });
                    ui.add_space(4.0);

                    if pctx.is_network {
                        if pctx.local_ready && !pctx.remote_ready {
                            ui.add(
                                egui::Label::new(
                                    egui::RichText::new(t.waiting_opponent_resume)
                                        .size(12.0)
                                        .color(Palette::TEXT_DIM),
                                )
                                .wrap(),
                            );
                            ui.add_space(4.0);
                        } else if pctx.remote_ready && !pctx.local_ready {
                            ui.add(
                                egui::Label::new(
                                    egui::RichText::new(t.opponent_ready)
                                        .size(12.0)
                                        .color(Palette::TEXT_DIM),
                                )
                                .wrap(),
                            );
                            ui.add_space(4.0);
                        }
                    }

                    // Resume / Ready row
                    {
                        let is_sel = sel == 0;
                        let available = can_resume;

                        let label_text = if pctx.is_network {
                            if pctx.local_ready {
                                t.ready_check // "Ready ✓"
                            } else {
                                t.ready_to_resume // "Ready to resume"
                            }
                        } else {
                            t.resume // "Resume"
                        };

                        // Clickable only when the action is available (else informational).
                        let resp = super::helpers::selectable_row(
                            ui, is_sel && available, available,
                            super::helpers::RowGauge::ModalThin, row_w,
                            |ui, text_color| {
                                ui.label(egui::RichText::new(label_text)
                                    .color(text_color).size(16.0));
                            });
                        if available {
                            if resp.clicked() {
                                outcome = if pctx.is_network {
                                    PauseOutcome::ResumeReady
                                } else {
                                    PauseOutcome::Resume
                                };
                            } else if resp.hovered() {
                                hovered_now = Some(0);
                            }
                        }
                        ui.add_space(2.0);
                    }

                    // Quit row
                    {
                        let is_sel = sel == 1;
                        let resp = super::helpers::selectable_row(
                            ui, is_sel, true, super::helpers::RowGauge::ModalThin, row_w,
                            |ui, text_color| {
                                ui.label(egui::RichText::new(t.quit)
                                    .color(text_color).size(16.0));
                            });
                        if resp.clicked() {
                            outcome = PauseOutcome::Quit;
                        } else if resp.hovered() {
                            hovered_now = Some(1);
                        }
                    }
                });
        });

    // Mouse: a hover transition moves the selection shared with the keyboard.
    if let Some(i) = super::helpers::hover_select_transition(ctx, "pause_menu_hover", hovered_now) {
        selection.set(i);
    }

    outcome
}

/// Whether the anti-reflex delay is active (network pause started by the opponent, less
/// than 1 s elapsed). Shared by rendering and `handle_pause_menu` (input).
pub fn in_protection(needs_delay: bool, seconds_open: f32) -> bool {
    needs_delay && seconds_open < QUIT_DELAY_SECS
}

/// Input state of the pause menu (the part of `PauseModalContext` needed for decisions).
#[derive(Debug, Clone, Copy)]
pub struct PauseMenuInput {
    pub is_network: bool,
    pub local_ready: bool,
    /// Anti-reflex delay is active: all input is ignored.
    pub in_protection: bool,
}

/// Pure input logic of the pause menu (selection 0=Resume, 1=Quit).
///
/// Returns `(new_selection, outcome)`. It does not use egui, so it is testable.
/// Called by `handle_pause_menu` (keyboard and gamepad). `Escape`/`Cancel` and the pause
/// key resume; `Enter`/`Confirm` activates the selected row.
pub fn pause_menu_step(
    input: &PauseMenuInput,
    selection: usize,
    intent: crate::app::ui_intent::UiIntent,
) -> (usize, PauseOutcome) {
    use crate::app::ui_intent::{UiDir, UiIntent};
    // Anti-reflex: ignore everything (the render shows only the title).
    if input.in_protection {
        return (selection, PauseOutcome::StillOpen);
    }
    let can_resume = if input.is_network { !input.local_ready } else { true };
    let resume = if input.is_network { PauseOutcome::ResumeReady } else { PauseOutcome::Resume };
    // Two entries (Resume/Ready, Quit). Navigation wraps like the main menu
    // (`MenuState::move_selection`): Down on the last row goes to the top, and Up on the
    // first row goes to the bottom.
    match intent {
        UiIntent::Navigate(UiDir::Up) | UiIntent::Navigate(UiDir::Down) =>
            (crate::app::focus::ring_move(selection, 2, 1), PauseOutcome::StillOpen),
        UiIntent::Confirm => match selection {
            0 if can_resume => (0, resume),
            0 => (0, PauseOutcome::StillOpen),
            _ => (1, PauseOutcome::Quit),
        },
        // Escape (Cancel) and the pause key/button (Pause) resume when allowed.
        UiIntent::Cancel | UiIntent::Pause if can_resume => (selection, resume),
        _ => (selection, PauseOutcome::StillOpen),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::ui_intent::{UiDir, UiIntent};

    fn solo() -> PauseMenuInput {
        PauseMenuInput { is_network: false, local_ready: false, in_protection: false }
    }

    #[test]
    fn navigate_sets_selection() {
        assert_eq!(pause_menu_step(&solo(), 1, UiIntent::Navigate(UiDir::Up)), (0, PauseOutcome::StillOpen));
        assert_eq!(pause_menu_step(&solo(), 0, UiIntent::Navigate(UiDir::Down)), (1, PauseOutcome::StillOpen));
    }

    #[test]
    fn navigate_wraps_like_menu() {
        // Down on the last row wraps to the top; Up on the first wraps to the bottom.
        assert_eq!(pause_menu_step(&solo(), 1, UiIntent::Navigate(UiDir::Down)), (0, PauseOutcome::StillOpen));
        assert_eq!(pause_menu_step(&solo(), 0, UiIntent::Navigate(UiDir::Up)), (1, PauseOutcome::StillOpen));
    }

    #[test]
    fn confirm_resume_or_quit_solo() {
        assert_eq!(pause_menu_step(&solo(), 0, UiIntent::Confirm), (0, PauseOutcome::Resume));
        assert_eq!(pause_menu_step(&solo(), 1, UiIntent::Confirm), (1, PauseOutcome::Quit));
    }

    #[test]
    fn cancel_and_pause_resume() {
        assert_eq!(pause_menu_step(&solo(), 0, UiIntent::Cancel), (0, PauseOutcome::Resume));
        assert_eq!(pause_menu_step(&solo(), 1, UiIntent::Pause), (1, PauseOutcome::Resume));
    }

    #[test]
    fn network_uses_resume_ready_and_blocks_when_ready() {
        let net = PauseMenuInput { is_network: true, local_ready: false, in_protection: false };
        assert_eq!(pause_menu_step(&net, 0, UiIntent::Confirm), (0, PauseOutcome::ResumeReady));
        // Already ready: resume is no longer available, but Quit is.
        let ready = PauseMenuInput { is_network: true, local_ready: true, in_protection: false };
        assert_eq!(pause_menu_step(&ready, 0, UiIntent::Confirm), (0, PauseOutcome::StillOpen));
        assert_eq!(pause_menu_step(&ready, 1, UiIntent::Confirm), (1, PauseOutcome::Quit));
        assert_eq!(pause_menu_step(&ready, 0, UiIntent::Cancel), (0, PauseOutcome::StillOpen));
    }

    #[test]
    fn protection_absorbs_everything() {
        let prot = PauseMenuInput { is_network: false, local_ready: false, in_protection: true };
        assert_eq!(pause_menu_step(&prot, 1, UiIntent::Confirm), (1, PauseOutcome::StillOpen));
        assert_eq!(pause_menu_step(&prot, 0, UiIntent::Navigate(UiDir::Down)), (0, PauseOutcome::StillOpen));
    }
}
