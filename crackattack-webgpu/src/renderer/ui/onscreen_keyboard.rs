//! egui rendering of the on-screen keyboard. The model (layout, navigation, applying keys)
//! lives in `app::onscreen_keyboard`; this module only draws the grid, highlights the focus
//! and handles mouse clicks. Keyboard and gamepad navigation goes through the router in
//! `main.rs` (`route_options_intent`).
//!
//! `render_osk_grid` is generic: it edits a `String` according to an `InputFormat`. The name
//! editor (`render_onscreen_keyboard`) and the network editor (IPv4/IPv6/port fields) both
//! use it, so there is one renderer for several adapted keyboards.

use crate::app::meta_state::NameInputState;
use crate::app::onscreen_keyboard::{
    apply, apply_edit, done_focus, layout, osk_activate, osk_focus_to, osk_move, InputFormat, KbKey,
    KbOutcome, NameEditOp,
};
use crate::renderer::ui::egui_theme::Palette;

fn key_label(key: KbKey) -> String {
    // Text labels (ASCII) from `Locale`. The default egui font does not draw the symbols
    // ⎵/⌫/✕ (they show as tofu), so the keys use text.
    let l = crate::app::locale::t();
    match key {
        KbKey::Char(' ') => l.key_space.to_string(),
        KbKey::Char(c) => c.to_string(),
        KbKey::Backspace => l.osk_backspace.to_string(),
        KbKey::Clear => l.osk_clear.to_string(),
        KbKey::Done => "OK".to_string(),
    }
}

/// Total width of the keyboard (px). Fixed, so all rows have the same width (each row
/// divides it equally with `ui.columns`).
const KB_WIDTH: f32 = 520.0;
/// Height of the keys (px). Generous, to be easy to hit with touch.
const KEY_H: f32 = 46.0;

/// Result of one frame of the on-screen keyboard.
pub struct OskFrame {
    /// The text changed this frame (physical typing, click, or activation).
    pub changed: bool,
    /// Closing was requested this frame (OK key, Enter on OK, or Escape).
    pub closed: bool,
}

/// Draws a generic on-screen keyboard that edits `text` according to `format`, with a
/// persistent focus `(focus, goal_x)`. It reads the physical keyboard (egui events) and
/// clicks; both edit the same `String`. Call it only while the keyboard is visible. The
/// caller owns the "open" state and the finalization (`format.finalize`).
pub fn render_osk_grid(
    ctx: &egui::Context,
    text: &mut String,
    format: InputFormat,
    focus: &mut (usize, usize),
    goal_x: &mut f32,
) -> OskFrame {
    let mut changed = false;
    let mut closed = false;
    // Typing on the physical keyboard moves the focus to OK, so pressing Enter at the end of
    // the input confirms it directly.
    let mut typed_physical = false;
    let rows = layout(format.charset);
    let charset = format.charset;

    // While the keyboard is open, physical keys are routed to egui (see
    // `keyboard_goes_to_egui`), and this reads the event stream. `text` is the single source
    // of truth, fed the same way by both inputs: direct typing (`Event::Text`, Backspace) and
    // navigation of the virtual keys (arrows + Enter). Enter activates the focused key (OK
    // closes); Escape closes.
    ctx.input(|i| {
        for ev in &i.events {
            match ev {
                egui::Event::Text(s) => {
                    for c in s.chars() {
                        if apply_edit(text, NameEditOp::Insert(c), format) {
                            changed = true;
                            typed_physical = true;
                        }
                    }
                }
                egui::Event::Key { key, pressed: true, .. } => match key {
                    egui::Key::Backspace => {
                        if apply_edit(text, NameEditOp::Backspace, format) {
                            changed = true;
                            typed_physical = true;
                        }
                    }
                    egui::Key::ArrowLeft => osk_move(focus, goal_x, charset, -1, 0),
                    egui::Key::ArrowRight => osk_move(focus, goal_x, charset, 1, 0),
                    egui::Key::ArrowUp => osk_move(focus, goal_x, charset, 0, -1),
                    egui::Key::ArrowDown => osk_move(focus, goal_x, charset, 0, 1),
                    egui::Key::Enter => {
                        if osk_activate(text, *focus, format) == KbOutcome::Close {
                            closed = true;
                        }
                        changed = true;
                    }
                    egui::Key::Escape => closed = true,
                    _ => {}
                },
                _ => {}
            }
        }
    });

    // After physical typing, move the focus to OK (Enter will confirm). Not needed if the
    // keyboard just closed (Enter/Escape).
    if typed_physical && !closed {
        let (r, c) = done_focus(charset);
        osk_focus_to(focus, goal_x, charset, r, c);
    }

    // If a physical key closed the keyboard (Escape/OK/Enter), do not draw the window this frame.
    if closed {
        return OskFrame { changed, closed };
    }

    let foc = *focus;
    // Key hovered this frame (encoded `r*32+c`); the focus moves after the grid is drawn.
    let mut hovered_now: Option<usize> = None;
    egui::Window::new("onscreen_keyboard")
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .resizable(false)
        .collapsible(false)
        .title_bar(false)
        .order(egui::Order::Foreground)
        .show(ctx, |ui| {
            egui::Frame::NONE
                .inner_margin(egui::Margin::same(16))
                .show(ui, |ui| {
                    ui.set_width(KB_WIDTH);

                    // Preview of the current text, with a fake cursor "|" (ASCII, safe to render).
                    ui.vertical_centered(|ui| {
                        ui.label(
                            egui::RichText::new(format!("{}|", text))
                                .size(24.0)
                                .strong()
                                .color(Palette::TEXT_ACCENT),
                        );
                    });
                    ui.add_space(8.0);
                    ui.add(egui::Separator::default().spacing(4.0));
                    ui.add_space(8.0);

                    ui.spacing_mut().item_spacing = egui::vec2(6.0, 6.0);
                    for (r, row) in rows.iter().enumerate() {
                        // A row is N equal columns, so every row has the same total width
                        // whatever its number of keys.
                        ui.columns(row.len(), |cols| {
                            for (c, key) in row.iter().enumerate() {
                                let ui_c = &mut cols[c];
                                let w = ui_c.available_width();
                                let mut resp = ui_c.add_sized(
                                    egui::vec2(w, KEY_H),
                                    egui::Button::new(
                                        egui::RichText::new(key_label(*key))
                                            .size(18.0)
                                            .color(Palette::TEXT),
                                    ),
                                ).on_hover_cursor(egui::CursorIcon::PointingHand);
                                if (r, c) == foc {
                                    resp = resp.highlight();
                                }
                                if resp.clicked() {
                                    osk_focus_to(focus, goal_x, charset, r, c);
                                    match apply(text, *key, format) {
                                        KbOutcome::Edited => changed = true,
                                        KbOutcome::Close => closed = true,
                                    }
                                } else if resp.hovered() {
                                    hovered_now = Some(r * 32 + c);
                                }
                            }
                        });
                    }
                });
        });

    // Hover transition on a key moves the focus shared by keyboard and mouse.
    if let Some(idx) = super::helpers::hover_select_transition(ctx, "osk_hover", hovered_now) {
        osk_focus_to(focus, goal_x, charset, idx / 32, idx % 32);
    }

    OskFrame { changed, closed }
}

/// On-screen keyboard of the name editor (options): a wrapper around `render_osk_grid` that
/// also handles the modal's `keyboard_open` state and finalizes the text on close. Returns
/// `true` if the text changed this frame (for the live nameplate preview).
pub fn render_onscreen_keyboard(ctx: &egui::Context, modal: &mut NameInputState) -> bool {
    let frame = render_osk_grid(
        ctx,
        &mut modal.text,
        modal.format,
        &mut modal.kb_focus,
        &mut modal.kb_goal_x,
    );
    let mut changed = frame.changed;
    if frame.closed {
        modal.keyboard_open = false;
    }

    // Validate on close: trim, and fall back to the default value if empty (see the format).
    // This covers every way out (OK, Enter, Escape, click).
    if !modal.keyboard_open {
        let finalized = modal.format.finalize(&modal.text);
        if finalized != modal.text {
            modal.text = finalized;
            changed = true;
        }
    }

    changed
}
