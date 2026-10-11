//! Network rank page and Opponents page (local ladder).
//!
//! Each screen shows the player's own rank once:
//!  - Network rank: the player's card (a league chip, shown large) + the match history.
//!  - Opponents: a compact "Me" reminder on top, then the opponents (last self-announced
//!    rank + quick reconnect).
//!
//! The design is visual: a league is a color, drawn as a chip everywhere; wins and losses
//! are trophy and skull icons; rating deltas are colored gain/loss chips. Text is only
//! used for names and dates.
//!
//! The opponents page is not a ladder of opponents. Their rank is self-announced and stale
//! by nature, so it is labeled as the last known rank. The data is built upstream
//! (`app/ladder_view::build`, pure and tested). This module only renders and handles mouse
//! clicks. Keyboard and gamepad navigation goes through `route_info_intent`.

use super::egui_theme::Palette;
use super::icons;
use super::info_modal::InfoOutcome;
use crate::app::ladder_view::RankPageData;
use crate::app::locale::t;
use crackattack_rating::League;

/// Color of a league. Single source of the ladder's visual identity.
pub fn league_color(l: League) -> egui::Color32 {
    use egui::Color32;
    match l {
        League::Rookie => Color32::from_rgb(128, 140, 152),      // gray (provisional)
        League::Copper => Color32::from_rgb(196, 118, 78),       // copper
        League::Bronze => Color32::from_rgb(205, 145, 62),       // bronze
        League::Silver => Color32::from_rgb(190, 200, 212),      // silver
        League::Gold => Color32::from_rgb(238, 190, 60),         // gold
        League::Platinum => Color32::from_rgb(110, 220, 210),    // platinum
        League::Diamond => Color32::from_rgb(112, 178, 255),     // diamond
        League::Master => Color32::from_rgb(196, 122, 255),      // master
        League::Grandmaster => Color32::from_rgb(255, 96, 96),   // grandmaster
    }
}

/// League chip such as "Gold · 1512": translucent fill and outline in the league color.
/// `value` is optional (league name only). The same chip is used for every rank display.
pub fn league_chip(ui: &mut egui::Ui, l: League, value: Option<i32>, size: f32) {
    let col = league_color(l);
    let text = match value {
        Some(v) => format!("{} · {}", t().league_name(l), v),
        None => t().league_name(l).to_string(),
    };
    egui::Frame::new()
        .fill(col.gamma_multiply(0.18))
        .stroke(egui::Stroke::new(1.0, col.gamma_multiply(0.7)))
        .corner_radius(size)
        .inner_margin(egui::Margin::symmetric((size * 0.7) as i8, (size * 0.25) as i8))
        .show(ui, |ui| {
            ui.label(egui::RichText::new(text).size(size).strong().color(col));
        });
}

/// Win/loss record as icons, for example `🏆 9   💀 5`.
pub fn record_icons(ui: &mut egui::Ui, wins: u32, losses: u32, size: f32) {
    ui.label(icons::icon(icons::TROPHY).size(size).color(league_color(League::Gold)));
    ui.label(egui::RichText::new(format!("{wins}")).size(size).strong().color(Palette::TEXT));
    ui.add_space(6.0);
    ui.label(icons::icon(icons::SKULL).size(size).color(Palette::TEXT_DIM));
    ui.label(egui::RichText::new(format!("{losses}")).size(size).strong().color(Palette::TEXT_DIM));
}

/// Rating delta chip: `+12` (accent) or `−8` (muted red), translucent fill.
pub fn delta_chip(ui: &mut egui::Ui, delta: f64, size: f32) {
    let rounded = delta.round() as i64;
    let (text, col) = if rounded >= 0 {
        (format!("+{rounded}"), Palette::TEXT_ACCENT)
    } else {
        (format!("\u{2212}{}", -rounded), Palette::DANGER)
    };
    egui::Frame::new()
        .fill(col.gamma_multiply(0.15))
        .corner_radius(size)
        .inner_margin(egui::Margin::symmetric((size * 0.5) as i8, (size * 0.15) as i8))
        .show(ui, |ui| {
            ui.label(egui::RichText::new(text).size(size).strong().color(col));
        });
}

/// Gray "?" chip: the opponent was never seen placed (no self-announced rank recorded).
fn unknown_chip(ui: &mut egui::Ui, size: f32) {
    let col = Palette::TEXT_DIM;
    egui::Frame::new()
        .fill(col.gamma_multiply(0.12))
        .stroke(egui::Stroke::new(1.0, col.gamma_multiply(0.5)))
        .corner_radius(size)
        .inner_margin(egui::Margin::symmetric((size * 0.7) as i8, (size * 0.25) as i8))
        .show(ui, |ui| {
            ui.label(egui::RichText::new("?").size(size).strong().color(col));
        });
}

fn section_header(ui: &mut egui::Ui, label: &str) {
    ui.add_space(14.0);
    ui.label(egui::RichText::new(label).size(13.0).strong().color(Palette::TEXT_DIM));
    ui.add(egui::Separator::default().spacing(4.0));
    ui.add_space(2.0);
}

fn empty_hint(ui: &mut egui::Ui) {
    ui.add_space(12.0);
    ui.label(egui::RichText::new(t().rank_no_matches).color(Palette::TEXT_DIM));
}

/// Network rank screen: the player's card + the match history. The content has no
/// interaction except scrolling. Reconnecting is done on the Opponents screen.
pub fn render_rank_page(
    ctx: &egui::Context,
    data: &RankPageData,
    rating_cfg: &crackattack_rating::RatingConfig,
    now_ts: u64,
    scroll_delta: f32,
    back_focused: bool,
) -> InfoOutcome {
    use super::nav_view::{render_nav_view, NavFocus, NavOutcome, NavView};
    let tr = t();
    let _ = rating_cfg;

    let view = NavView {
        id_salt: "rank_page",
        title: tr.menu_network_rank,
        left_label: Some(tr.back),
        right_label: None,
        left_icon: icons::BACK,
        right_icon: None,
        focus: if back_focused { NavFocus::Left } else { NavFocus::Content },
        scroll_delta,
        opaque_bg: true,
        center_content: false,
    };

    let out = render_nav_view(ctx, &view, |ui| {
        let Some(me) = &data.me else {
            empty_hint(ui);
            return;
        };

        // ── Player card: the one large display of the player's rank ──────────────────────
        ui.add_space(10.0);
        ui.vertical_centered(|ui| {
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing.x = 10.0;
                league_chip(ui, me.league, Some(me.rating_value.round() as i32), 20.0);
                record_icons(ui, me.wins, me.losses, 16.0);
            });
            ui.add_space(2.0);
            ui.label(
                egui::RichText::new(format!("±{:.0}", me.rd))
                    .size(12.0)
                    .color(Palette::TEXT_DIM),
            );
        });

        // ── Match history (consecutive rounds against the same opponent = one match) ─────
        if !data.parties.is_empty() {
            section_header(ui, tr.rank_last_parties);
            for p in &data.parties {
                super::egui_theme::row_frame().show(ui, |ui| {
                    ui.set_min_width(ui.available_width().max(320.0));
                    ui.horizontal(|ui| {
                        ui.label(egui::RichText::new(&p.opponent).size(14.0)
                            .strong().color(Palette::TEXT));
                        record_icons(ui, p.wins, p.losses, 13.0);
                        delta_chip(ui, p.delta, 13.0);
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(egui::RichText::new(tr.ago(p.last_ts, now_ts))
                                .size(12.0).color(Palette::TEXT_DIM));
                        });
                    });
                });
                ui.add_space(2.0);
            }
        }
    });

    match out {
        NavOutcome::LeftActivated => InfoOutcome::Close,
        NavOutcome::HoverLeft => InfoOutcome::HoverBack,
        _ => InfoOutcome::StillOpen,
    }
}

/// Opponents screen: a compact "Me" reminder, then one card per opponent. A card shows the
/// last known rank (chip), the player's record against them (trophy/skull) and a Reconnect
/// action when an address is stored. Rows can be activated; the keyboard selection is shared
/// with mouse hover.
pub fn render_opponents_page(
    ctx: &egui::Context,
    data: &RankPageData,
    rating_cfg: &crackattack_rating::RatingConfig,
    now_ts: u64,
    scroll_delta: f32,
    back_focused: bool,
    selected: usize,
    scroll_to_selected: bool,
) -> InfoOutcome {
    use super::nav_view::{render_nav_view, NavFocus, NavOutcome, NavView};
    let tr = t();

    let view = NavView {
        id_salt: "opponents_page",
        title: tr.menu_network_opponents,
        left_label: Some(tr.back),
        right_label: None,
        left_icon: icons::BACK,
        right_icon: None,
        focus: if back_focused { NavFocus::Left } else { NavFocus::Content },
        scroll_delta,
        opaque_bg: true,
        center_content: false,
    };

    // Mouse: row hovered or clicked this frame (resolved after rendering).
    let mut hovered_row: Option<usize> = None;
    let mut clicked_row: Option<usize> = None;

    let out = render_nav_view(ctx, &view, |ui| {
        if data.opponents.is_empty() {
            empty_hint(ui);
            return;
        }

        // ── Compact "Me" reminder: the only display of the player's rank here ─────────
        if let Some(me) = &data.me {
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(tr.rank_me_prefix)
                    .size(13.0).color(Palette::TEXT_DIM));
                league_chip(ui, me.league, Some(me.rating_value.round() as i32), 13.0);
            });
            ui.add(egui::Separator::default().spacing(6.0));
        }

        for (i, o) in data.opponents.iter().enumerate() {
            let is_sel = i == selected;
            let mut frame = if is_sel && o.reachable {
                super::egui_theme::selected_row_frame()
            } else {
                super::egui_theme::row_frame()
            };
            frame.stroke.width = 1.0;
            let inner = frame.show(ui, |ui| {
                ui.set_min_width(ui.available_width().max(320.0));
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new(&o.name).size(15.0)
                        .strong().color(Palette::TEXT));
                    // Online: its mDNS announcement (uuid) is currently visible on the
                    // network, so reconnecting is almost immediate.
                    if o.online {
                        let green = egui::Color32::from_rgb(96, 216, 128);
                        ui.label(egui::RichText::new("\u{25cf}").size(11.0).color(green));
                        ui.label(egui::RichText::new(t().online_label).size(12.0).color(green));
                    }
                    // Last known (self-announced) rank as a league chip. If never seen
                    // placed, a gray "?" chip keeps the slot visible.
                    match o.last_rt {
                        Some(rt) => {
                            let lg = crate::app::ladder_view::league_of_value(rt as f64, rating_cfg);
                            league_chip(ui, lg, Some(rt), 13.0);
                        }
                        None => unknown_chip(ui, 13.0),
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(egui::RichText::new(tr.ago(o.last_ts, now_ts))
                            .size(12.0).color(Palette::TEXT_DIM));
                    });
                });
                ui.horizontal(|ui| {
                    // The player's record against this opponent. The trophy and skull icons
                    // need no text prefix.
                    record_icons(ui, o.wins, o.losses, 13.0);
                    if o.reachable {
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            let col = if is_sel { Palette::TEXT_ACCENT } else { Palette::TEXT_DIM };
                            ui.label(icons::icon_text(
                                super::icons::WIFI, tr.reconnect, 13.0, col,
                            ));
                        });
                    }
                });
            });
            if is_sel && scroll_to_selected {
                inner.response.scroll_to_me(Some(egui::Align::Center));
            }
            if o.reachable {
                let resp = super::helpers::clickable(inner.response);
                if resp.clicked() {
                    clicked_row = Some(i);
                } else if resp.hovered() {
                    hovered_row = Some(i);
                }
            }
            ui.add_space(3.0);
        }
    });

    if let Some(i) = clicked_row {
        return InfoOutcome::RowClicked(i);
    }
    // A hover transition selects the row, without scrolling to it (mouse only).
    if let Some(i) = super::helpers::hover_select_transition(ctx, "opp_row_hover", hovered_row) {
        return InfoOutcome::HoverRow(i);
    }
    match out {
        NavOutcome::LeftActivated => InfoOutcome::Close,
        NavOutcome::HoverLeft => InfoOutcome::HoverBack,
        _ => InfoOutcome::StillOpen,
    }
}
