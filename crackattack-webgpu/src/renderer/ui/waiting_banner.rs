//! Waiting banner: informational overlay during waiting states.
//!
//! Displayed when `SessionInterruption::WaitingRemoteReady` or a lost connection is
//! active. Shows a text message, and a Quit button appears after a timeout.

use super::egui_theme::{Palette, TextSize};

/// Outcome of the waiting banner for this frame.
pub enum WaitingOutcome {
    /// Still waiting, no action.
    StillWaiting,
    /// Player chose Quit (after timeout).
    Quit,
    /// Player pressed Escape (open pause menu).
    Escape,
}

/// Render the waiting banner. Returns the outcome for this frame.
///
/// - `text`: message to display (for example, waiting for the opponent)
/// - `elapsed_secs`: wall-clock seconds since the waiting started
/// - `quit_timeout_secs`: seconds before Quit button appears (e.g. 15.0)
pub fn render_waiting_banner(
    ctx: &egui::Context,
    text: &str,
    elapsed_secs: f32,
    quit_timeout_secs: f32,
) -> WaitingOutcome {
    let mut outcome = WaitingOutcome::StillWaiting;

    let quit_visible = quit_timeout_secs > 0.0 && elapsed_secs >= quit_timeout_secs;

    if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        outcome = WaitingOutcome::Escape;
    }

    if quit_visible && ctx.input(|i| i.key_pressed(egui::Key::Enter)) {
        outcome = WaitingOutcome::Quit;
    }

    egui::Window::new("waiting_banner")
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .resizable(false)
        .collapsible(false)
        .title_bar(false)
        .show(ctx, |ui| {
            ui.set_min_width(280.0);
            egui::Frame::NONE
                .inner_margin(egui::Margin::same(16))
                .show(ui, |ui| {
                    ui.vertical_centered(|ui| {
                        ui.label(
                            egui::RichText::new(text)
                                .size(15.0)
                                .color(Palette::TEXT),
                        );
                    });

                    if quit_visible {
                        ui.add_space(12.0);
                        let frame = super::egui_theme::selected_row_frame();
                        let resp = frame.show(ui, |ui| {
                            ui.set_min_width(180.0);
                            ui.horizontal(|ui| {
                                ui.label(egui::RichText::new("▶").color(Palette::TEXT_ACCENT).size(14.0));
                                ui.label(egui::RichText::new(crate::app::locale::t().quit).color(Palette::TEXT).size(TextSize::BODY));
                            });
                        }).response;
                        // Hand cursor + click (single button, so no hover-to-focus).
                        if super::helpers::clickable(resp).clicked() {
                            outcome = WaitingOutcome::Quit;
                        }
                    }
                });
        });

    outcome
}
