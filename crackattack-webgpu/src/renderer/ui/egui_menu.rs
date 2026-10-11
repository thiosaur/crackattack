//! Main menu (egui).

use crate::app::meta_state::MenuState;
use crate::app::menu_tree::LabelKey;
use super::egui_theme::Palette;

/// Outcome of the egui menu for this frame.
pub enum MenuOutcome {
    /// No action this frame.
    StillOpen,
    /// Mouse: the pointer entered row `i`; select it (absolute selection).
    Select(usize),
    /// Mouse or touch: click or tap on row `i`; select and activate it.
    Activate(usize),
}

/// One item of the footer **ticker** of the root menu, shown on two lines: a discreet label
/// on top and the **value in large accent text** below.
///
/// The list is open-ended (best solo, best combo, network rank, support link, update...).
/// Items without data are simply absent from the list the caller provides.
pub struct TickerItem {
    /// Top line: short label, shown small and dimmed (uppercased when rendered).
    pub top: String,
    /// Bottom line: value (score, "x7"...) or call to action, shown large in the accent color.
    pub bottom: String,
    /// Action on click when the item is fully displayed. An item with an action is drawn
    /// smaller (its text is longer than a value). The action is resolved outside rendering:
    /// rendering only opens a URL that was already decided.
    pub action: TickerAction,
}

/// Typed action of a ticker item.
#[derive(Clone, PartialEq, Eq, Default)]
pub enum TickerAction {
    #[default]
    None,
    /// Project link: the issue tracker (beta versions).
    Support { url: String },
    /// Update available: the releases page of the channel. Only constructed with the
    /// `update-check` feature; the variant itself is always part of the UI contract.
    #[cfg_attr(not(feature = "update-check"), allow(dead_code))]
    Update { url: String },
}

impl TickerAction {
    fn url(&self) -> Option<&str> {
        match self {
            TickerAction::None => None,
            TickerAction::Support { url } | TickerAction::Update { url } => Some(url),
        }
    }
}

/// Load the logo as an egui texture (cached after first call).
fn logo_texture(ctx: &egui::Context) -> Option<egui::TextureHandle> {
    let id = egui::Id::new("menu_logo_tex");
    if let Some(handle) = ctx.data_mut(|d| d.get_temp::<egui::TextureHandle>(id)) {
        return Some(handle);
    }
    let path = crate::renderer::init::find_assets_dir()
        .join("textures/ui/logo.png");
    let image_data = match std::fs::read(&path) {
        Ok(d) => d,
        Err(e) => {
            log::warn!("Failed to read logo: {}", e);
            return None;
        }
    };
    let image = match image::load_from_memory(&image_data) {
        Ok(img) => img.to_rgba8(),
        Err(e) => {
            log::warn!("Failed to decode logo: {}", e);
            return None;
        }
    };
    let size = [image.width() as usize, image.height() as usize];
    let pixels = image.into_raw();
    let color_image = egui::ColorImage::from_rgba_unmultiplied(size, &pixels);
    let handle = ctx.load_texture("menu_logo", color_image, egui::TextureOptions::LINEAR);
    ctx.data_mut(|d| d.insert_temp(id, handle.clone()));
    Some(handle)
}

/// Render the egui main menu. Returns the outcome for this frame.
pub fn render_egui_menu(
    ctx: &egui::Context,
    menu: &MenuState,
    ticker: &[TickerItem],
) -> MenuOutcome {
    // Keyboard input for the bare menu goes through `dispatch_input` and `route_menu_input`,
    // not through egui. When an overlay is open, this menu is not rendered (see the gate in
    // `frame_submit`). Here egui only handles the **pointer** (hover = Select, click = Activate).
    let mut outcome = MenuOutcome::StillOpen;

    let labels = menu.labels();
    let selected = menu.selected;

    // Three zones with a **fixed** anchor (instead of one centered block), so the logo
    // does not move when the number of items changes:
    //   - logo: pinned to the top
    //   - items: anchored at a fixed Y below the logo
    //   - footer (ticker): pinned to the bottom

    // 1. Logo: fixed zone at the top.
    egui::Area::new(egui::Id::new("egui_menu_logo"))
        .anchor(egui::Align2::CENTER_TOP, [0.0, 56.0])
        .movable(false)
        .order(egui::Order::Middle)
        .show(ctx, |ui| {
            ui.vertical_centered(|ui| {
                if let Some(tex) = logo_texture(ctx) {
                    let aspect = tex.aspect_ratio();
                    let logo_width = 192.0;
                    let logo_height = logo_width / aspect;
                    ui.image(egui::load::SizedTexture::new(tex.id(), egui::vec2(logo_width, logo_height)));
                }
            });
        });

    // Event-style hover: select on hover only when the hovered item CHANGES. A still mouse
    // would otherwise return `Select(i)` every frame and override keyboard and gamepad
    // navigation. The last hovered index is kept in egui temporary memory (transient UI
    // state), and `Select` is emitted only on a hover transition.
    let hover_id = egui::Id::new("egui_menu_hovered_index");
    let prev_hovered: Option<usize> = ctx.data(|d| d.get_temp::<Option<usize>>(hover_id)).flatten();
    let mut new_hovered: Option<usize> = None;

    // 2. Items.
    egui::Area::new(egui::Id::new("egui_menu_items"))
        // Anchored at the TOP at a fixed position (not centered): the block height does not
        // trigger vertical re-centering, so items do not shift when focus changes. The fixed
        // Y also keeps a constant gap below the logo (logo bottom is about 219 px).
        .anchor(egui::Align2::CENTER_TOP, [0.0, 250.0])
        .movable(false)
        .order(egui::Order::Middle)
        .show(ctx, |ui| {
            ui.set_min_width(280.0);

            // Partition: **text** items (game modes) first, then a trailing **icon row**
            // (options, info and quit at the root; back in a submenu). `selected` stays a
            // **linear** cursor over all entries (Left/Right = +-1, see `handle_menu_input`),
            // so the icon row is navigated horizontally.
            let icon_for = |key: &LabelKey| -> Option<char> {
                match key {
                    LabelKey::Quit => Some(super::icons::POWER),
                    LabelKey::Back => Some(super::icons::BACK),
                    LabelKey::Options => Some(super::icons::SETTINGS),
                    LabelKey::Info => Some(super::icons::INFO),
                    _ => None,
                }
            };
            let icon_start = labels.iter().position(|(k, _)| icon_for(k).is_some())
                .unwrap_or(labels.len());

            // --- Text items: vertical rows (same geometry with or without focus;
            // `ListFlush` style: 2 px, border invisible at rest) ---
            for i in 0..icon_start {
                let (key, enabled) = &labels[i];
                let enabled = *enabled;
                let is_selected = i == selected;

                let resp = super::helpers::selectable_row(
                    ui, is_selected && enabled, enabled,
                    super::helpers::RowGauge::ListFlush, 250.0,
                    |ui, text_color| {
                        ui.label(egui::RichText::new(key.localized())
                            .color(text_color).size(18.0).strong());
                    });
                if enabled {
                    if resp.clicked() {
                        outcome = MenuOutcome::Activate(i);
                    } else if resp.hovered() {
                        new_hovered = Some(i);
                        if prev_hovered != Some(i) {
                            outcome = MenuOutcome::Select(i);
                        }
                    }
                }
                ui.add_space(2.0);
            }

            // --- Icon row: **centered horizontal** row (focus = accent color, otherwise dimmed).
            // Centering is measured by hand: `vertical_centered` does not center a row narrower
            // than the text items above it, so the width (glyphs + gaps) is measured and a
            // leading margin is added. Icons are clickable `Label`s without padding.
            if icon_start < labels.len() {
                ui.add_space(14.0);
                const ICON_SIZE: f32 = 26.0;
                const ICON_GAP: f32 = 22.0;
                let icon_font = egui::FontId::new(
                    ICON_SIZE, egui::FontFamily::Name(super::icons::FAMILY.into()));
                let n = labels.len() - icon_start;
                let mut row_w = ICON_GAP * n.saturating_sub(1) as f32;
                for i in icon_start..labels.len() {
                    let glyph = icon_for(&labels[i].0).unwrap_or(super::icons::POWER);
                    row_w += ui.fonts(|f| f.layout_no_wrap(
                        glyph.to_string(), icon_font.clone(), egui::Color32::WHITE).size().x);
                }
                let lead = ((ui.available_width() - row_w) * 0.5).max(0.0);
                ui.horizontal(|ui| {
                    ui.add_space(lead);
                    for i in icon_start..labels.len() {
                        if i > icon_start { ui.add_space(ICON_GAP); }
                        let (key, enabled) = &labels[i];
                        let enabled = *enabled;
                        let is_selected = i == selected;
                        let glyph = icon_for(key).unwrap_or(super::icons::POWER);
                        let color = if is_selected && enabled {
                            Palette::TEXT_ACCENT
                        } else {
                            Palette::TEXT_DIM
                        };
                        let resp = ui.add(
                            egui::Label::new(super::icons::icon(glyph).size(ICON_SIZE).color(color))
                                .sense(egui::Sense::click()),
                        ).on_hover_cursor(egui::CursorIcon::PointingHand);
                        if enabled {
                            if resp.clicked() {
                                outcome = MenuOutcome::Activate(i);
                            } else if resp.hovered() {
                                new_hovered = Some(i);
                                if prev_hovered != Some(i) {
                                    outcome = MenuOutcome::Select(i);
                                }
                            }
                        }
                    }
                });
            }
        });

    // Remember the hovered item for the next frame (`None` if the mouse is over no item).
    ctx.data_mut(|d| d.insert_temp(hover_id, new_hovered));

    // 3. Footer: info **ticker** at the root (fixed zone at the bottom). The Quit (root) and
    // Back (submenu) icons in the list serve as the exit affordance; there is no text hint.
    if menu.depth() == 0 {
        render_ticker(ctx, ticker);
    }

    outcome
}

/// Footer ticker: cycles through `items` one at a time, each on two lines (discreet
/// UPPERCASE label and large accent value), with a horizontal slide and a crossfade every
/// ~3.9 s.
///
/// Cheap on CPU: it repaints continuously only during the transition (~0.45 s) and otherwise
/// schedules a wake-up for the next change. An item with an action is clickable while fully
/// displayed (static phase).
fn render_ticker(ctx: &egui::Context, items: &[TickerItem]) {
    if items.is_empty() {
        return;
    }
    const PERIOD: f64 = 3.9;
    const TRANSITION: f64 = 0.45;
    const TOP_SIZE: f32 = 12.0;
    const VALUE_SIZE: f32 = 30.0;
    const GITHUB_SIZE: f32 = 17.0;
    const TOP_DY: f32 = -13.0; // label line, above the center
    const BOT_DY: f32 = 12.0;  // value line, below

    let n = items.len();
    let time = ctx.input(|i| i.time);
    let pos = time / PERIOD;
    let idx = (pos.floor() as usize) % n;
    let frac = pos.fract();
    let t_start = 1.0 - TRANSITION / PERIOD;
    let transitioning = n > 1 && frac >= t_start;

    let fade = |c: egui::Color32, a: f32| {
        egui::Color32::from_rgba_unmultiplied(c.r(), c.g(), c.b(), (a.clamp(0.0, 1.0) * 255.0) as u8)
    };

    egui::Area::new(egui::Id::new("egui_menu_ticker"))
        .anchor(egui::Align2::CENTER_BOTTOM, [0.0, -52.0])
        .movable(false)
        .order(egui::Order::Middle)
        .show(ctx, |ui| {
            // Fixed area (tall enough for two lines). Its painter clips the slide, so text
            // leaving through the sides is cut cleanly.
            let size = egui::vec2(440.0, 56.0);
            let (rect, _) = ui.allocate_exact_size(size, egui::Sense::hover());
            let painter = ui.painter_at(rect);
            let w = rect.width();
            let center = rect.center();

            // Draws an item (two lines) shifted horizontally by `dx`, with opacity `a`.
            let draw = |item: &TickerItem, dx: f32, a: f32| {
                let cx = center.x + dx;
                painter.text(
                    egui::pos2(cx, center.y + TOP_DY),
                    egui::Align2::CENTER_CENTER,
                    crate::app::locale::uppercase(&item.top),
                    egui::FontId::proportional(TOP_SIZE),
                    fade(Palette::TEXT_DIM, a),
                );
                let size = if item.action != TickerAction::None { GITHUB_SIZE } else { VALUE_SIZE };
                painter.text(
                    egui::pos2(cx, center.y + BOT_DY),
                    egui::Align2::CENTER_CENTER,
                    &item.bottom,
                    egui::FontId::proportional(size),
                    fade(Palette::TEXT_ACCENT, a),
                );
            };

            if transitioning {
                // p goes 0 to 1 over the transition. The current item leaves to the left,
                // the next one enters from the right; the two crossfade.
                let p = ((frac - t_start) / (1.0 - t_start)) as f32;
                let next = (idx + 1) % n;
                draw(&items[idx], -p * w, 1.0 - p);
                draw(&items[next], (1.0 - p) * w, p);
                ui.ctx().request_repaint(); // smooth animation during the transition
            } else {
                let item = &items[idx];
                draw(item, 0.0, 1.0);
                // Actionable item (support, update): interaction area while it is fully
                // displayed. A click opens the URL resolved by the caller.
                if let Some(url) = item.action.url() {
                    let resp = ui.interact(rect, ui.id().with("ticker_action"), egui::Sense::click());
                    if resp.hovered() {
                        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                    }
                    #[cfg(feature = "native")]
                    if resp.clicked() {
                        let _ = open::that(url);
                    }
                    #[cfg(not(feature = "native"))]
                    let _ = url;
                }
                // Wake up when the next transition starts (avoids useless repaints).
                let until = ((t_start - frac).max(0.0)) * PERIOD;
                ui.ctx().request_repaint_after(std::time::Duration::from_secs_f64(until.max(0.05)));
            }
        });
}
