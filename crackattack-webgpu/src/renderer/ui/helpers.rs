//! UI helper types and functions (easing, message shapes, level light state).


// ─────────────────────────────────────────────────────────────────────────────────────────────
// **Contract for screens with shared focus** (keyboard, gamepad and mouse converge on one
// focus). Every navigable dialog follows these 4 rules:
//
//   1. **Pure rendering**: rendering reads the focus state, it does not own it.
//   2. **Hover requests focus**: through `hover_select_transition`, which reacts to
//      **transitions** only.
//   3. **A click produces an outcome or a command**, never a hidden mutation in rendering.
//   4. **The focus state lives outside rendering.** It is propagated in one of 3 ways, in
//      increasing order of robustness (choose by who owns the state):
//        a. **direct mutation** of a `Cell`/`&mut` held by the session (pause, ready, LAN list);
//        b. an **`Outcome`** returned to the caller, which mutates (`confirm_modal`, `nav_view`);
//        c. an **`AppCommand`** applied by the runtime (Options, whose persistent state is a
//           render copy, so (a) cannot work). This is the most robust way and the one to use
//           for complex persistent state (see `OptionsNavFocus`).
// ─────────────────────────────────────────────────────────────────────────────────────────────

/// Marks an egui response as a clickable button or icon.
///
/// Adds click detection (`Sense::click()`) and the pointing-hand cursor on hover. Use it for
/// every button or icon drawn with a `Frame` or `Label`, which otherwise detect neither
/// clicks nor hover.
pub fn clickable(resp: egui::Response) -> egui::Response {
    resp.interact(egui::Sense::click())
        .on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// Visual style of a selectable row (see `selectable_row`).
///
/// Two styles coexist on purpose (1 px and 2 px strokes):
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RowGauge {
    /// Compact modals (pause, ready, confirmation, reconnect buttons): thin **1 px**
    /// stroke, border **visible** at rest (`Palette::BORDER`), so rows read as bounded
    /// buttons in a small box.
    ModalThin,
    /// Full-page lists (main menu, LAN peers): **2 px** stroke, border **invisible** at
    /// rest (same color as the `BG_PANEL` background). The size stays constant when focus
    /// changes, and no border shows without focus.
    ListFlush,
}

/// Selectable row shared by all dialogs (menu, pause, ready, confirmation, LAN).
///
/// Draws the selected or normal frame, a chevron (accent color when selected, transparent
/// otherwise so the width stays constant), passes the conventional text color to the
/// content, and handles click and pointing-hand cursor when `enabled`.
///
/// - `selected`: the row has focus (accented frame and chevron). The caller applies its own
///   policy (for example the pause menu passes `is_sel && available`, so an unavailable row
///   never shows the focus frame).
/// - `enabled`: the row can be activated (click and `PointingHand`). `false` = inert,
///   informational row (the raw response is returned, without `Sense::click`).
/// - `min_width`: minimum inner width (0.0 = fit the content, as in `confirm_modal`).
/// - `add_contents(ui, text_color)`: renders the content to the right of the chevron.
///   `text_color` follows 3 cases (disabled: `TEXT_UNAVAILABLE`, selected: `TEXT`,
///   otherwise `TEXT_DIM`). The caller may ignore it (`peer_row` keeps `TEXT` at rest).
pub fn selectable_row(
    ui: &mut egui::Ui,
    selected: bool,
    enabled: bool,
    gauge: RowGauge,
    min_width: f32,
    add_contents: impl FnOnce(&mut egui::Ui, egui::Color32),
) -> egui::Response {
    selectable_row_marked(ui, selected, enabled, gauge, min_width, None, add_contents)
}

/// [`selectable_row`] with an optional `marker` icon that replaces the chevron and is always
/// shown (for a row that cannot take the focus, so it never shows the chevron).
pub fn selectable_row_marked(
    ui: &mut egui::Ui,
    selected: bool,
    enabled: bool,
    gauge: RowGauge,
    min_width: f32,
    marker: Option<char>,
    add_contents: impl FnOnce(&mut egui::Ui, egui::Color32),
) -> egui::Response {
    use super::egui_theme::{row_frame, selected_row_frame, Palette};
    let frame = match (gauge, selected) {
        (RowGauge::ModalThin, true) => {
            let mut f = selected_row_frame();
            f.stroke.width = 1.0;
            f
        }
        (RowGauge::ModalThin, false) => {
            let mut f = row_frame();
            f.stroke.width = 1.0;
            f
        }
        (RowGauge::ListFlush, true) => selected_row_frame(),
        // Invisible border at rest (same as the background): keeps the 2 px size without a visible line.
        (RowGauge::ListFlush, false) => {
            let mut f = row_frame();
            f.stroke = egui::Stroke::new(2.0, Palette::BG_PANEL);
            f
        }
    };
    let text_color = if !enabled {
        Palette::TEXT_UNAVAILABLE
    } else if selected {
        Palette::TEXT
    } else {
        Palette::TEXT_DIM
    };
    let chevron_color = if selected { Palette::TEXT_ACCENT } else { egui::Color32::TRANSPARENT };
    let resp = frame
        .show(ui, |ui| {
            if min_width > 0.0 {
                ui.set_min_width(min_width);
            }
            ui.horizontal(|ui| {
                match marker {
                    Some(icon) => ui.label(
                        super::icons::icon(icon).color(Palette::TEXT_UNAVAILABLE).size(14.0)),
                    None => ui.label(egui::RichText::new("\u{25b6}").color(chevron_color).size(14.0)),
                };
                add_contents(ui, text_color);
            });
        })
        .response;
    if enabled { clickable(resp) } else { resp }
}

/// Hover-to-select, as in the main menu.
///
/// egui dialogs show a visual focus that follows a selection index shared by keyboard and
/// mouse. To let hover move that focus without blocking keyboard or gamepad navigation, this
/// reacts only to a **transition** (the pointer just entered a new item). A still mouse does
/// not reselect its item every frame, which would keep overriding the arrow keys.
///
/// - `id_salt`: unique id per dialog (remembers the hovered item between frames).
/// - `hovered_now`: index hovered this frame (`None` if no button is hovered).
/// - Returns `Some(i)` only when `i` was just hovered (transition), so the caller updates its
///   selection. Returns `None` otherwise (still mouse, or pointer outside the buttons), so
///   keyboard navigation keeps control.
pub fn hover_select_transition(
    ctx: &egui::Context,
    id_salt: &str,
    hovered_now: Option<usize>,
) -> Option<usize> {
    let id = egui::Id::new(id_salt);
    let prev: Option<usize> = ctx.data(|d| d.get_temp::<Option<usize>>(id)).flatten();
    ctx.data_mut(|d| d.insert_temp(id, hovered_now));
    match hovered_now {
        Some(i) if prev != Some(i) => Some(i),
        _ => None,
    }
}

/// Quadratic ease-out: 1 - (1-t)^2
#[inline]
pub fn ease_out_quad(t: f32) -> f32 {
    1.0 - (1.0 - t) * (1.0 - t)
}


/// Common scaffold for centered modals: an `Area` anchored at the center, not movable, on the
/// foreground layer.
///
/// Areas with a specific anchor or order (menu, `nav_view`, scrims) stay local.
pub fn modal_area(id: &str) -> egui::Area {
    egui::Area::new(egui::Id::new(id))
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .movable(false)
        .order(egui::Order::Foreground)
}
