//! Ready modal: the question that starts each round.
//!
//! It asks "Ready?" before round 1 and "Ready for a new round?" from round 2. Being ready
//! means consenting to the next round.
//!
//! Shown when `SessionInterruption::ReadyPrompt` is active and the round summary is closed.
//! The new grid is visible behind it, so the player sees what they consent to.
//!
//! It has two navigable buttons, Ready and Quit. Keyboard and gamepad input goes through the
//! router (ready sub-state, `route_ready_prompt`). The mouse supports click, hover to focus
//! and a hand cursor. The shared selection is `session.ready_selection`. Quit leaves
//! directly to the menu, because nothing is committed at the prompt. A confirmation is only
//! needed when leaving costs a rated round.
//!
//! The opponent status ("X is ready" / "X is not ready yet", network only) is shown here,
//! when the player answers, and never on the round summary. When the opponent has left,
//! the offer is reduced to Quit alone: no question, no status, no message.

use super::egui_theme::{Palette, TextSize};

/// Outcome of the ready modal for this frame (mouse click only; keyboard and gamepad go
/// through the router).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReadyOutcome {
    /// No action this frame.
    StillOpen,
    /// Ready button: signal ready (consent to the round).
    Ready,
    /// Quit button: leave directly to the menu (nothing committed).
    Quit,
}

/// Display context of the modal.
pub struct ReadyModalContext {
    /// Round about to be played (1-based).
    pub round_number: i32,
    /// Opponent is still present. `false` reduces the offer to Quit alone.
    pub opponent_present: bool,
    /// Opponent status `(text, is_ready)`, network only. `None` for the computer, which
    /// consents instantly, and when the offer is reduced to Quit.
    pub opp_status: Option<(String, bool)>,
}

pub fn render_ready_modal(
    ctx: &egui::Context,
    mctx: &ReadyModalContext,
    selection: &std::cell::Cell<usize>,
) -> ReadyOutcome {
    let mut outcome = ReadyOutcome::StillOpen;
    let t = crate::app::locale::t();
    let sel = selection.get();
    // Mouse: button hovered this frame (0 = Ready, 1 = Quit); moves the selection after the Area.
    let mut hovered_now: Option<usize> = None;

    egui::Window::new("ready_prompt")
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .resizable(false)
        .collapsible(false)
        .title_bar(false)
        .show(ctx, |ui| {
            let modal_w = 230.0;
            ui.set_min_width(modal_w);
            egui::Frame::NONE
                .inner_margin(egui::Margin::same(16))
                .show(ui, |ui| {
                    // The question: round 1 shows the round label, later rounds ask for a
                    // new round. Not shown when the opponent has left.
                    if mctx.opponent_present {
                        let subtitle = if mctx.round_number > 1 {
                            t.ready_new_round.to_string()
                        } else {
                            t.round_label(mctx.round_number)
                        };
                        ui.label(
                            egui::RichText::new(subtitle)
                                .size(14.0)
                                .color(Palette::TEXT_DIM),
                        );
                        ui.add_space(8.0);
                    }

                    let row_w = modal_w - 4.0;
                    // One row per button, with accent and chevron when selected (same as
                    // `pause_modal`). Full offer: Ready and Quit. Reduced offer: Quit alone.
                    let rows: &[(usize, &str)] = if mctx.opponent_present {
                        &[(0usize, t.ready), (1usize, t.quit)]
                    } else {
                        &[(1usize, t.quit)]
                    };
                    for &(idx, label) in rows {
                        let is_sel = sel == idx || !mctx.opponent_present;
                        let resp = super::helpers::selectable_row(
                            ui, is_sel, true, super::helpers::RowGauge::ModalThin, row_w,
                            |ui, text_color| {
                                ui.label(egui::RichText::new(label)
                                    .color(text_color).size(TextSize::BODY));
                            });
                        if resp.clicked() {
                            outcome = if idx == 0 { ReadyOutcome::Ready } else { ReadyOutcome::Quit };
                        } else if resp.hovered() {
                            hovered_now = Some(idx);
                        }
                        ui.add_space(2.0);
                    }

                    // Opponent status: has it already answered? (network)
                    if let Some((status, is_ready)) = &mctx.opp_status {
                        ui.add_space(6.0);
                        let col = if *is_ready { Palette::TEXT_ACCENT } else { Palette::TEXT_DIM };
                        ui.label(egui::RichText::new(status).size(12.0).color(col));
                    }
                });
        });

    // Mouse: a hover transition moves the selection shared with the keyboard.
    if let Some(i) = super::helpers::hover_select_transition(ctx, "ready_prompt_hover", hovered_now) {
        selection.set(i);
    }

    outcome
}
