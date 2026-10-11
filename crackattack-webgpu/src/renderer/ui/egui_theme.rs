//! Centralized egui theme for all UI panels.
//!
//! Applied once per frame before any egui rendering. Panels should NOT
//! call ctx.set_style() — they use local Frame/RichText/Stroke overrides.

/// Named palette for the arcade-style UI.
pub struct Palette;

impl Palette {
    pub const BG_APP: egui::Color32 = egui::Color32::from_rgb(17, 22, 28);
    pub const BG_PANEL: egui::Color32 = egui::Color32::from_rgb(23, 29, 36);
    pub const BG_SELECTED: egui::Color32 = egui::Color32::from_rgb(16, 40, 58);
    pub const BG_HOVERED: egui::Color32 = egui::Color32::from_rgb(28, 40, 52);
    pub const BG_INPUT: egui::Color32 = egui::Color32::from_rgb(10, 14, 20);

    pub const BORDER: egui::Color32 = egui::Color32::from_rgb(42, 52, 64);
    pub const BORDER_ACTIVE: egui::Color32 = egui::Color32::from_rgb(62, 193, 255);

    pub const TEXT: egui::Color32 = egui::Color32::from_rgb(230, 237, 245);
    pub const TEXT_DIM: egui::Color32 = egui::Color32::from_rgb(143, 161, 179);
    pub const TEXT_ACCENT: egui::Color32 = egui::Color32::from_rgb(62, 193, 255);
    pub const TEXT_UNAVAILABLE: egui::Color32 = egui::Color32::from_rgb(122, 110, 92);

    pub const DANGER: egui::Color32 = egui::Color32::from_rgb(255, 100, 100);
}

/// The single type scale for dialogs. Use it instead of literal sizes.
///
/// The default egui text styles (Heading/Body/Button/Small) are set to this scale in
/// `build_style`, so a label without an explicit `.size()` already gets the right value.
pub struct TextSize;

impl TextSize {
    /// Dialog title.
    pub const TITLE: f32 = 18.0;
    /// Body text, value, button label.
    pub const BODY: f32 = 15.0;
    /// Hint or warning.
    pub const HINT: f32 = 12.0;
}

/// Build the arcade egui style from the palette.
pub fn build_style() -> egui::Style {
    let mut style = egui::Style::default();

    style.visuals.panel_fill = Palette::BG_APP;
    style.visuals.window_fill = Palette::BG_APP;

    // Non-interactive (labels, separators)
    style.visuals.widgets.noninteractive.bg_stroke =
        egui::Stroke::new(1.0, Palette::BORDER);
    style.visuals.widgets.noninteractive.fg_stroke =
        egui::Stroke::new(1.0, Palette::TEXT);

    // Inactive widgets (buttons, text edits at rest)
    style.visuals.widgets.inactive.bg_fill =
        egui::Color32::from_rgb(30, 38, 48);
    style.visuals.widgets.inactive.bg_stroke =
        egui::Stroke::new(1.0, egui::Color32::from_rgb(55, 65, 78));
    style.visuals.widgets.inactive.fg_stroke =
        egui::Stroke::new(1.0, Palette::TEXT);

    style.visuals.widgets.hovered.bg_fill = Palette::BG_HOVERED;
    style.visuals.widgets.hovered.bg_stroke =
        egui::Stroke::new(1.0, Palette::BORDER_ACTIVE);

    style.visuals.widgets.active.bg_fill = Palette::BG_SELECTED;
    style.visuals.widgets.active.bg_stroke =
        egui::Stroke::new(2.0, Palette::BORDER_ACTIVE);

    // TextEdit background
    style.visuals.extreme_bg_color = Palette::BG_INPUT;

    // Selection highlight in text fields
    style.visuals.selection.bg_fill =
        egui::Color32::from_rgb(40, 80, 120);
    style.visuals.selection.stroke =
        egui::Stroke::new(1.0, Palette::TEXT_ACCENT);

    // Default text styles follow `TextSize`: any text without an explicit `.size()`
    // (the Options modal, for example) is already consistent.
    {
        use egui::{FontFamily::Proportional, FontId, TextStyle};
        let s = &mut style.text_styles;
        s.insert(TextStyle::Heading, FontId::new(TextSize::TITLE, Proportional));
        s.insert(TextStyle::Body, FontId::new(TextSize::BODY, Proportional));
        s.insert(TextStyle::Button, FontId::new(TextSize::BODY, Proportional));
        s.insert(TextStyle::Small, FontId::new(TextSize::HINT, Proportional));
    }

    style.visuals.window_corner_radius = 8.0.into();
    style.spacing.item_spacing = egui::vec2(8.0, 6.0);
    style.spacing.button_padding = egui::vec2(12.0, 8.0);

    // Labels are not selectable (game UI): this avoids the I-beam cursor over labels
    // and menu rows.
    style.interaction.selectable_labels = false;

    style
}

/// Apply the global arcade theme. Call once per frame, before
/// rendering any egui panels.
pub fn apply(ctx: &egui::Context) {
    ctx.set_style(build_style());
}

/// Frame for a selected row (keyboard navigation).
pub fn selected_row_frame() -> egui::Frame {
    egui::Frame::new()
        .fill(Palette::BG_SELECTED)
        .stroke(egui::Stroke::new(2.0, Palette::BORDER_ACTIVE))
        .corner_radius(6.0)
        .inner_margin(egui::Margin::symmetric(12, 8))
}

/// Frame for a normal (unselected) row.
pub fn row_frame() -> egui::Frame {
    egui::Frame::new()
        .fill(Palette::BG_PANEL)
        .stroke(egui::Stroke::new(1.0, Palette::BORDER))
        .corner_radius(6.0)
        .inner_margin(egui::Margin::symmetric(12, 8))
}
