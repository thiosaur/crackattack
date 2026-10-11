//! Reusable navigation view, for content pages and post-game screens.
//!
//! Shared layout: a large pinned title that is always visible, a scrollable body, and side
//! buttons outside the dialog. The left button (back/menu) is always present; the right
//! button (next) is optional and used by the post-game screens. The background is opaque
//! (content page) or transparent (post-game, scene visible behind).
//!
//! Navigation (focus left / content / right, scrolling, activation) is driven by UI intents
//! in the router, for keyboard and gamepad alike. This module only draws and handles mouse
//! clicks. Focus and scroll delta are passed in as parameters; the pinned title stays visible
//! while only the body scrolls.

use super::egui_theme::Palette;

/// Scroll speed while a direction is held (px applied per frame). Together with continuous
/// repaint during a session, this scrolls smoothly as long as the key is held.
pub const SCROLL_SPEED: f32 = 12.0;

/// Size of the pinned title. It is large on purpose (about twice the body text) so it
/// dominates the content and marks the title / content hierarchy.
const NAV_TITLE_SIZE: f32 = 34.0;

/// Focused element of the view (cycle: left, content, right).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NavFocus {
    Left,
    Content,
    Right,
}

/// Result of the view for this frame (mouse clicks; keyboard and gamepad go through the router).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NavOutcome {
    StillOpen,
    LeftActivated,
    RightActivated,
    /// The mouse starts hovering the left button: focus it, without activating it.
    HoverLeft,
    /// The mouse starts hovering the right button: focus it, without activating it.
    HoverRight,
}

/// Configuration of a navigation view.
pub struct NavView<'a> {
    /// Identity of the scroll area, so each screen scrolls independently.
    pub id_salt: &'a str,
    /// Pinned title (large, outside the scroll area).
    pub title: &'a str,
    /// Icon of the left button (back/menu). Always present.
    pub left_icon: char,
    /// Optional label under the left icon (for example "Quit"), for when the action must be
    /// explicit (post-round screen, where the choice matters).
    pub left_label: Option<&'a str>,
    /// Icon of the right button (next). `None` means no right button (content pages).
    pub right_icon: Option<char>,
    /// Optional label under the right icon (for example "Play again").
    pub right_label: Option<&'a str>,
    /// Focused element (`Left` and `Right` can only be focused if they exist).
    pub focus: NavFocus,
    /// Scroll to apply to the body this frame (px). While a direction is held, this is a
    /// per-frame delta (see `SCROLL_SPEED`). The body stays under the pinned title.
    pub scroll_delta: f32,
    /// Opaque full-screen background (content page) or transparent (post-game, scene behind).
    pub opaque_bg: bool,
    /// Center the content vertically in the available space (post-game: short content fills
    /// the space instead of sitting at the top). `false` aligns to the top (reading pages).
    pub center_content: bool,
}

/// Renders the view. Returns which button was clicked (mouse), or `StillOpen`. `content`
/// draws the body in the scroll area, under the pinned title.
pub fn render_nav_view(
    ctx: &egui::Context,
    view: &NavView,
    content: impl FnOnce(&mut egui::Ui),
) -> NavOutcome {
    let mut outcome = NavOutcome::StillOpen;
    // Side button hovered this frame (0 = left, 1 = right); focus is set after the buttons.
    let mut hovered_side: Option<usize> = None;
    let screen = ctx.screen_rect();

    // Opaque full-screen background (page); otherwise what is behind stays visible (post-game).
    if view.opaque_bg {
        egui::Area::new(egui::Id::new("nav_view_bg"))
            .order(egui::Order::Middle)
            .fixed_pos(screen.min)
            .show(ctx, |ui| {
                ui.painter().rect_filled(screen, 0.0, Palette::BG_APP);
            });
    }

    // Pinned title at the top (centered, always visible while scrolling).
    egui::Area::new(egui::Id::new("nav_view_title"))
        .anchor(egui::Align2::CENTER_TOP, [0.0, 26.0])
        .order(egui::Order::Foreground)
        .show(ctx, |ui| {
            // `Extend`: never wrap the title. Otherwise it collapses to the width of the
            // `Area` and a word such as "Credits" breaks into two lines.
            ui.add(egui::Label::new(egui::RichText::new(view.title)
                .size(NAV_TITLE_SIZE).strong().color(Palette::TEXT))
                .wrap_mode(egui::TextWrapMode::Extend));
        });

    // Left button (back/menu): left edge, vertically centered; accent color when focused.
    egui::Area::new(egui::Id::new("nav_view_left"))
        .anchor(egui::Align2::LEFT_CENTER, [30.0, 0.0])
        .order(egui::Order::Foreground)
        .show(ctx, |ui| {
            let col = if view.focus == NavFocus::Left { Palette::TEXT_ACCENT } else { Palette::TEXT_DIM };
            ui.vertical_centered(|ui| {
                let r = ui.add(egui::Button::new(super::icons::icon(view.left_icon).size(34.0).color(col)).frame(false))
                    .on_hover_cursor(egui::CursorIcon::PointingHand);
                if let Some(label) = view.left_label {
                    // `Extend`: the `Area` collapses to the icon width, so without it the label
                    // would wrap in the middle of a word (same trap as the pinned title).
                    ui.add(egui::Label::new(egui::RichText::new(label).size(13.0).color(col))
                        .wrap_mode(egui::TextWrapMode::Extend));
                }
                if r.clicked() {
                    outcome = NavOutcome::LeftActivated;
                } else if r.hovered() {
                    hovered_side = Some(0);
                }
            });
        });

    // Optional right button (next): right edge, vertically centered; accent color when focused.
    if let Some(right_icon) = view.right_icon {
        egui::Area::new(egui::Id::new("nav_view_right"))
            .anchor(egui::Align2::RIGHT_CENTER, [-30.0, 0.0])
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                let col = if view.focus == NavFocus::Right { Palette::TEXT_ACCENT } else { Palette::TEXT_DIM };
                ui.vertical_centered(|ui| {
                    let r = ui.add(egui::Button::new(super::icons::icon(right_icon).size(34.0).color(col)).frame(false))
                        .on_hover_cursor(egui::CursorIcon::PointingHand);
                    if let Some(label) = view.right_label {
                        // `Extend`: same reason as the left label.
                        ui.add(egui::Label::new(egui::RichText::new(label).size(13.0).color(col))
                            .wrap_mode(egui::TextWrapMode::Extend));
                    }
                    if r.clicked() {
                        outcome = NavOutcome::RightActivated;
                    } else if r.hovered() {
                        hovered_side = Some(1);
                    }
                });
            });
    }

    // Body: a frame of constant size, equal to the usable content area (not the area actually
    // occupied). For a given window size this rectangle is identical on every content page;
    // only its border (focus) and its inner content change.
    const MARGIN_X: f32 = 16.0;
    const MARGIN_Y: f32 = 12.0;
    // Side gutter: wide enough that the frame clears the side buttons (anchored at ±30 px,
    // about 50 px wide) without overlapping them, hence a margin of about 120 px on each side.
    let content_w = (screen.width() - 240.0).clamp(240.0, 580.0);
    let max_h = (screen.height() - 150.0).clamp(120.0, 4000.0);
    // Area inside the frame (without margins) has a fixed size; frame = inside + margins, so
    // the frame is always `content_w × max_h`.
    let inner_w = (content_w - 2.0 * MARGIN_X).max(120.0);
    let inner_h = (max_h - 2.0 * MARGIN_Y).max(80.0);
    egui::Area::new(egui::Id::new("nav_view_body"))
        .anchor(egui::Align2::CENTER_TOP, [0.0, 92.0])
        .order(egui::Order::Foreground)
        .show(ctx, |ui| {
            // The area must be distinguishable even without focus, styled according to what is
            // behind it. The border is always 2 px, so the geometry never changes; only the
            // colors do:
            //  - page on a black background (`opaque_bg`, Credits/Rules): lighter panel
            //    (`BG_PANEL`) and a dim gray border, accent color when focused;
            //  - in front of a colored scene or curtain (post-game): a very subtle light veil
            //    and a neutral translucent white border, accent color when focused.
            let focused = view.focus == NavFocus::Content;
            let (fill, border) = if view.opaque_bg {
                let b = if focused { Palette::BORDER_ACTIVE } else { Palette::BORDER };
                (Palette::BG_PANEL, b)
            } else {
                // In front of a colored curtain: background and border both lighten. The white
                // veil is very subtle (about as light as the Credits panel) and the border a
                // little stronger; accent color when focused.
                let b = if focused { Palette::BORDER_ACTIVE } else { egui::Color32::from_white_alpha(30) };
                (egui::Color32::from_white_alpha(8), b)
            };
            let frame = egui::Frame::new()
                .fill(fill)
                .stroke(egui::Stroke::new(2.0, border))
                .corner_radius(10.0)
                .inner_margin(egui::Margin::symmetric(MARGIN_X as i8, MARGIN_Y as i8));
            // Alignment inside the fixed frame: Center (post-game, short content centered) or Min
            // (reading pages, aligned to the top). Content larger than the area scrolls (the
            // `ScrollArea` is capped at `inner_h`).
            let valign = if view.center_content { egui::Align::Center } else { egui::Align::Min };
            frame.show(ui, |ui| {
                // Fixed width (`set_width` sets both min and max). Without it, a `ScrollArea`
                // with `auto_shrink([false, …])` fills the available width of the unbounded
                // `Area`, and the frame overflows onto the side icons. The height is forced to
                // the usable minimum (`set_min_height`), so the frame is always
                // `content_w × max_h`.
                ui.set_width(inner_w);
                ui.set_min_height(inner_h);
                ui.with_layout(
                    egui::Layout::top_down(egui::Align::Center).with_main_align(valign),
                    |ui| {
                        // Margin between the content and the scroll bar. The egui scroll bar is
                        // floating by default (drawn OVER the content), and then
                        // `bar_inner_margin` is ignored, so the bar touches the text. Make it
                        // non-floating: it reserves a gutter on the right, separated from the
                        // text by `bar_inner_margin`.
                        {
                            let sc = &mut ui.spacing_mut().scroll;
                            sc.floating = false;
                            sc.bar_width = 8.0;
                            sc.bar_inner_margin = 10.0;
                            sc.bar_outer_margin = 2.0;
                        }
                        egui::ScrollArea::vertical()
                            .id_salt(view.id_salt)
                            .max_height(inner_h)
                            .auto_shrink([false, true])
                            .show(ui, |ui| {
                                if view.scroll_delta != 0.0 {
                                    ui.scroll_with_delta(egui::vec2(0.0, -view.scroll_delta));
                                }
                                content(ui);
                            });
                    },
                );
            });
        });

    // Hover transition on a side button focuses it (unless a click already activated it).
    // The id is unique per screen (`id_salt`), so each screen tracks transitions independently.
    if matches!(outcome, NavOutcome::StillOpen) {
        let hover_id = format!("{}_nav_side_hover", view.id_salt);
        if let Some(i) = super::helpers::hover_select_transition(ctx, &hover_id, hovered_side) {
            outcome = if i == 0 { NavOutcome::HoverLeft } else { NavOutcome::HoverRight };
        }
    }

    outcome
}
