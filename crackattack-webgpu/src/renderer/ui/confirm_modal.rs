//! Generic confirmation modal (Cancel / Confirm), for example to confirm abandoning a game
//! in progress. Two buttons side by side; the focused button uses the accent color. Rendering
//! handles **clicks** only. The caller handles the keyboard (Left/Right to choose, Enter to
//! confirm, Escape/Space to cancel). There is no scrim: the modal is drawn on the `Foreground`
//! layer, above everything, like the pause menu, which does not dim the background either.

use super::egui_theme::{Palette, TextSize};

/// Outcome of the confirmation modal for this frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfirmOutcome {
    /// Still open, no action.
    StillOpen,
    /// The user confirmed (right button).
    Confirmed,
    /// The user cancelled (left button, Escape, Space).
    Cancelled,
    /// Mouse: the pointer just entered a button, so focus moves to it (`true` = confirm
    /// button, `false` = cancel button). The caller updates its focus state.
    HoverFocus(bool),
}

/// Renders a centered confirmation modal with two buttons (Cancel on the left, Confirm on
/// the right).
///
/// `confirm_focused`: `true` = confirm button selected, `false` = cancel button selected.
/// Returns the outcome of a **click**, or `StillOpen` otherwise.
pub fn render_confirm_modal(
    ctx: &egui::Context,
    title: &str,
    confirm_label: &str,
    cancel_label: &str,
    confirm_focused: bool,
) -> ConfirmOutcome {
    let mut outcome = ConfirmOutcome::StillOpen;
    // Mouse: button hovered this frame (0 = cancel, 1 = confirm); focus is updated after the Area.
    let mut hovered_now: Option<usize> = None;

    super::helpers::modal_area("confirm_modal")
        .show(ctx, |ui| {
            egui::Frame::window(&ctx.style())
                .inner_margin(egui::Margin::same(20))
                .show(ui, |ui| {
                    ui.vertical_centered(|ui| {
                        ui.label(
                            egui::RichText::new(title)
                                .size(TextSize::TITLE)
                                .strong()
                                .color(Palette::TEXT),
                        );
                        ui.add_space(16.0);
                        // Buttons use the same style as the other dialogs (pause, ready):
                        // `selected_row_frame`/`row_frame`, a chevron and `Palette` colors,
                        // side by side (Cancel on the left, Confirm on the right).
                        ui.horizontal(|ui| {
                            for (idx, label) in [(0usize, cancel_label), (1usize, confirm_label)] {
                                let is_sel = if idx == 1 { confirm_focused } else { !confirm_focused };
                                let resp = super::helpers::selectable_row(
                                    ui, is_sel, true, super::helpers::RowGauge::ModalThin, 0.0,
                                    |ui, text_color| {
                                        ui.label(egui::RichText::new(label)
                                            .color(text_color).size(16.0));
                                    });
                                if resp.clicked() {
                                    outcome = if idx == 0 { ConfirmOutcome::Cancelled } else { ConfirmOutcome::Confirmed };
                                } else if resp.hovered() {
                                    hovered_now = Some(idx);
                                }
                                if idx == 0 {
                                    ui.add_space(10.0);
                                }
                            }
                        });
                    });
                });
        });

    // Mouse: a hover transition moves the focus, unless a click already decided the outcome.
    if outcome == ConfirmOutcome::StillOpen {
        if let Some(i) = super::helpers::hover_select_transition(ctx, "confirm_hover", hovered_now) {
            outcome = ConfirmOutcome::HoverFocus(i == 1);
        }
    }

    outcome
}
