//! Minimal Markdown renderer for egui, limited to what the game needs (Rules and Credits
//! screens).
//!
//! The supported subset is small on purpose:
//! - `# title` and `## subtitle`
//! - `- bullet`
//! - inline **bold** with `**…**` (drawn in the bright `TEXT` color instead of `TEXT_DIM`)
//! - paragraphs (single lines); an empty line adds spacing
//!
//! There are no links (the Credits page handles its buttons separately), images or tables.
//! It has no dependency: a full Markdown crate is not needed for this.

use super::egui_theme::Palette;

const BODY: f32 = 13.0;
const H1: f32 = 18.0;
const H2: f32 = 15.0;

/// Renders a markdown document (the subset above) into `ui`, line by line.
pub fn render_markdown(ui: &mut egui::Ui, md: &str) {
    let avail = ui.available_width();
    for raw in md.lines() {
        if let Some(h) = raw.strip_prefix("## ") {
            ui.add_space(6.0);
            ui.label(egui::RichText::new(h).size(H2).strong().color(Palette::TEXT));
        } else if let Some(h) = raw.strip_prefix("# ") {
            ui.add_space(8.0);
            ui.label(egui::RichText::new(h).size(H1).strong().color(Palette::TEXT_ACCENT));
        } else if let Some(b) = raw.strip_prefix("- ") {
            ui.label(line_job(&format!("•  {}", b), avail));
        } else if raw.trim().is_empty() {
            ui.add_space(6.0);
        } else {
            ui.label(line_job(raw, avail));
        }
    }
}

/// Builds a `LayoutJob` for a body line and handles `**…**` bold. Even segments are normal
/// (`TEXT_DIM`), odd segments are bold (bright `TEXT`). The text wraps at `max_width`.
fn line_job(text: &str, max_width: f32) -> egui::text::LayoutJob {
    use egui::{text::LayoutJob, FontId, TextFormat};
    let mut job = LayoutJob::default();
    job.wrap.max_width = max_width;
    let mut bold = false;
    for seg in text.split("**") {
        if !seg.is_empty() {
            let color = if bold { Palette::TEXT } else { Palette::TEXT_DIM };
            job.append(seg, 0.0, TextFormat {
                font_id: FontId::proportional(BODY),
                color,
                ..Default::default()
            });
        }
        bold = !bold;
    }
    job
}
