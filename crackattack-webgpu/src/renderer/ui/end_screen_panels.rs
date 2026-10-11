//! End screen egui panels (high scores, game stats).
//!
//! Shared table style for the post-game screens (round summary, game stats, high scores):
//!  - dimmed labels on the left, bold values aligned to the right;
//!  - subtle row striping, no borders;
//!  - comparisons: headers are the player names (own name in the accent color), and the best
//!    value of a row is highlighted when "more is better";
//!  - records: a medal and the accent color when a record is beaten.

use crate::app::post_game::PostGameScreen;
use crate::platform::persist::{ScoreEntry, RecordEntry, RecordKey};
use crate::renderer::ui::egui_theme::Palette;
use crate::renderer::ui::icons;

/// Title of a solo results screen (pinned by `nav_view`).
pub fn results_title(step: PostGameScreen) -> &'static str {
    let t = crate::app::locale::t();
    match step {
        PostGameScreen::HighScores => t.high_scores,
        PostGameScreen::Stats => t.game_stats,
        PostGameScreen::ResultMessage => "",
    }
}

// ─── Shared table building blocks ───────────────────────────────────────────────────────
//
// No striped `egui::Grid`: it paints its stripes at the CURRENT grid width (the columns
// measured so far), not the panel width, so the bands are truncated.
// Instead, a row is a full-width frame with an alternating background, a label on the left
// and value columns on the right with a FIXED width (rows stay aligned with each other).

/// Width of a value column (right side).
const VAL_COL_W: f32 = 88.0;
/// Usable height of a cell.
const CELL_H: f32 = 18.0;

/// Width of the icon column (record medal). Fixed so the medals of several rows line up
/// vertically.
const ICON_COL_W: f32 = 24.0;

/// One cell: text OR an icon, with its column width.
struct Cell {
    text: String,
    icon: Option<char>,
    size: f32,
    color: egui::Color32,
    strong: bool,
    width: f32,
}

impl Cell {
    fn value(text: String, emphasized: bool) -> Self {
        Self {
            text,
            icon: None,
            size: 15.0,
            color: if emphasized { Palette::TEXT_ACCENT } else { Palette::TEXT },
            strong: true,
            width: VAL_COL_W,
        }
    }
    fn dim(text: String) -> Self {
        Self { text, icon: None, size: 14.0, color: Palette::TEXT_DIM, strong: false, width: VAL_COL_W }
    }
    fn header(text: String, is_me: bool) -> Self {
        Self {
            text,
            icon: None,
            size: 12.0,
            color: if is_me { Palette::TEXT_ACCENT } else { Palette::TEXT_DIM },
            strong: true,
            width: VAL_COL_W,
        }
    }
    /// Fixed icon column: a gold medal when a record is beaten, empty otherwise (keeps alignment).
    fn medal(present: bool) -> Self {
        Self {
            text: String::new(),
            icon: present.then_some(icons::MEDAL),
            size: 14.0,
            color: egui::Color32::from_rgb(238, 190, 60),
            strong: false,
            width: ICON_COL_W,
        }
    }
}

/// Full-width table row: alternating background (odd `idx`), dimmed label on the left,
/// fixed-width cells on the right (drawn right to left, last cell first).
fn table_row(ui: &mut egui::Ui, idx: usize, label: &str, cells: &[Cell]) {
    let fill = if idx % 2 == 1 {
        egui::Color32::from_white_alpha(6)
    } else {
        egui::Color32::TRANSPARENT
    };
    egui::Frame::new()
        .fill(fill)
        .corner_radius(4.0)
        .inner_margin(egui::Margin::symmetric(10, 5))
        .show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(label).size(13.0).color(Palette::TEXT_DIM));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    for cell in cells.iter().rev() {
                        ui.allocate_ui_with_layout(
                            egui::vec2(cell.width, CELL_H),
                            egui::Layout::right_to_left(egui::Align::Center),
                            |ui| {
                                // `allocate_ui_with_layout` only CONSUMES the width of its
                                // content (the requested size is just a maximum). Without
                                // this the columns collapse and run together.
                                ui.set_min_width(cell.width);
                                if let Some(glyph) = cell.icon {
                                    ui.label(icons::icon(glyph).size(cell.size).color(cell.color));
                                } else if !cell.text.is_empty() {
                                    let mut rt = egui::RichText::new(&cell.text)
                                        .size(cell.size)
                                        .color(cell.color);
                                    if cell.strong {
                                        rt = rt.strong();
                                    }
                                    ui.label(rt);
                                }
                            },
                        );
                    }
                });
            });
        });
}

// ─── High scores ────────────────────────────────────────────────────────────────────────

/// Body of the High Scores screen (rank / name / score rows). The window, title and
/// scroll area are provided by `nav_view`. Rows are striped, and the view scrolls to the
/// player's rank when it changes.
pub fn draw_high_scores_body(ui: &mut egui::Ui, scores: &[ScoreEntry], current_rank: Option<usize>) {
    if scores.is_empty() {
        ui.label(crate::app::locale::t().no_scores_yet);
        return;
    }
    const ROW: f32 = 20.0;
    let total = scores.len().min(30);
    let last_rank_id = egui::Id::new("hs_last_rank");
    let last_rank: Option<usize> =
        ui.ctx().data_mut(|d| d.get_temp::<Option<usize>>(last_rank_id).flatten());
    for i in 0..total {
        let entry = &scores[i];
        let is_current = current_rank == Some(i);
        // Three levels: default entries (the original table) are dimmed, the player's own
        // earlier scores are drawn normally, and the score just obtained uses the accent
        // color plus a ◀ marker (besides the auto-scroll).
        let is_default = crate::platform::persist::is_default_score(entry);
        let color = if is_current {
            Palette::TEXT_ACCENT
        } else if is_default {
            Palette::TEXT_DIM
        } else {
            Palette::TEXT
        };
        let rank_color = if is_current { Palette::TEXT_ACCENT } else { Palette::TEXT_DIM };
        let fill = if i % 2 == 1 {
            egui::Color32::from_white_alpha(6)
        } else {
            egui::Color32::TRANSPARENT
        };
        let resp = egui::Frame::new()
            .fill(fill)
            .corner_radius(4.0)
            .inner_margin(egui::Margin::symmetric(10, 6))
            .show(ui, |ui| {
                ui.set_min_width(ui.available_width());
                ui.horizontal(|ui| {
                    // Rank in a fixed column (right-aligned), name on the left, score on the right.
                    ui.allocate_ui_with_layout(
                        egui::vec2(44.0, ROW + 2.0),
                        egui::Layout::right_to_left(egui::Align::Center),
                        |ui| {
                            ui.set_min_width(44.0); // as in `table_row`: consume the whole column
                            ui.label(egui::RichText::new(format!("{}.", i + 1))
                                .size(ROW).strong().color(rank_color));
                        },
                    );
                    ui.add_space(10.0);
                    let mut name_rt = egui::RichText::new(&entry.name).size(ROW).color(color);
                    if !is_default {
                        name_rt = name_rt.strong();
                    }
                    ui.label(name_rt);
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        // Marker for the fresh result. Fixed column, so the scores stay
                        // aligned between rows.
                        ui.allocate_ui_with_layout(
                            egui::vec2(20.0, ROW + 2.0),
                            egui::Layout::right_to_left(egui::Align::Center),
                            |ui| {
                                ui.set_min_width(20.0); // as in `table_row`
                                if is_current {
                                    ui.label(egui::RichText::new("\u{25c0}")
                                        .size(14.0).color(Palette::TEXT_ACCENT));
                                }
                            },
                        );
                        ui.label(egui::RichText::new(format!("{}", entry.score)).size(ROW).color(color));
                    });
                });
            })
            .response;
        if is_current && current_rank != last_rank {
            resp.scroll_to_me(Some(egui::Align::Center));
        }
    }
    ui.ctx().data_mut(|d| d.insert_temp(last_rank_id, current_rank));
}

// ─── Stats screen bodies ─────────────────────────────────────────────────────────────

/// Body of the solo Stats screen: a comparison if there is an opponent, otherwise a single
/// column plus records. `names` is (me, opponent) for the comparison headers; without it the
/// generic locale labels are used.
pub fn draw_final_stats_body(
    ui: &mut egui::Ui,
    stats: Option<&crackattack_core::stats::GameStats>,
    opponent_stats: Option<&crackattack_core::stats::GameStats>,
    score: i32,
    show_score: bool,
    records: &[RecordEntry],
    beaten: &[RecordKey],
    names: Option<(&str, &str)>,
) {
    if let Some(stats) = stats {
        let t = crate::app::locale::t();
        if let Some(opp) = opponent_stats {
            draw_comparative_stats(ui, stats, opp, names, t);
        } else {
            draw_single_stats(ui, stats, score, show_score, records, beaten, t);
        }
    }
}

/// Rank change shown in the banner of the round summary (network ladder).
pub struct RatingSummary {
    pub league: crackattack_rating::League,
    pub value: i32,
    /// Rating delta of the round just played.
    pub round_delta: f64,
    /// Session total, if it differs from the round delta (otherwise redundant).
    pub session_delta: Option<f64>,
}

/// Body of the VS round summary, in two blocks:
///  1. a full-width Result banner: session tally as trophy/skull icons on the left, rank
///     change (league chip + delta chips) on the right;
///  2. a Details block: the comparison table (headers are the player names).
///
/// The caller pins the "Round N" title through `nav_view`.
pub fn draw_round_results_body(
    ui: &mut egui::Ui,
    rs: &crate::app::post_game::RoundEndStats,
    opp_override: Option<&crackattack_core::stats::GameStats>,
    names: Option<(&str, &str)>,
    rating: Option<&RatingSummary>,
) {
    use crate::renderer::ui::rank_page::{delta_chip, league_chip, record_icons};
    let t = crate::app::locale::t();
    let opp = rs.opponent.as_ref().or(opp_override);

    // ── Result banner ──────────────────────────────────────────────────────────────
    egui::Frame::new()
        .fill(Palette::BG_PANEL)
        .stroke(egui::Stroke::new(1.0, crate::renderer::ui::egui_theme::Palette::BORDER))
        .corner_radius(8.0)
        .inner_margin(egui::Margin::symmetric(14, 10))
        .show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            ui.horizontal(|ui| {
                // Session tally as icons.
                record_icons(
                    ui,
                    rs.session_wins.max(0) as u32,
                    rs.session_losses.max(0) as u32,
                    20.0,
                );
                // Rank change on the right: league chip + round delta + session total (Σ).
                if let Some(r) = rating {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if let Some(total) = r.session_delta {
                            delta_chip(ui, total, 13.0);
                            ui.label(egui::RichText::new("\u{03a3}")
                                .size(13.0).color(Palette::TEXT_DIM));
                        }
                        delta_chip(ui, r.round_delta, 14.0);
                        league_chip(ui, r.league, Some(r.value), 14.0);
                    });
                }
            });
        });
    ui.add_space(8.0);

    // ── Details block ──────────────────────────────────────────────────────────────────
    if let Some(opp) = opp {
        draw_comparative_stats(ui, &rs.local, opp, names, t);
    } else {
        draw_single_stats_no_records(ui, &rs.local, t);
    }
}

/// Format a record value for display.
fn format_record(key: RecordKey, value: i32) -> String {
    match key {
        RecordKey::LongestGame => format!("{}:{:02}", value / 60, value % 60),
        RecordKey::BestChain => format!("x{}", value),
        _ => format!("{}", value),
    }
}

/// Single-column stats display with optional Record column.
fn draw_single_stats(
    ui: &mut egui::Ui,
    stats: &crackattack_core::stats::GameStats,
    score: i32,
    show_score: bool,
    records: &[RecordEntry],
    beaten: &[RecordKey],
    t: &crate::app::locale::Locale,
) {
    let has_records = !records.is_empty();
    let record_for = |key: RecordKey| -> Option<&RecordEntry> {
        records.iter().find(|r| r.key == key)
    };

    if has_records {
        table_row(ui, 0, "", &[
            Cell::medal(false), // alignment column
            Cell::header(t.stat_col_you.to_string(), true),
            Cell::header(t.stat_col_record.to_string(), false),
        ]);
    }

    let mut idx = 1usize;
    let mut row = |ui: &mut egui::Ui, label: &str, value: String, key: RecordKey| {
        let is_beaten = beaten.contains(&key);
        // The medal (record beaten) has its own fixed column, so all medals line up. The
        // trophy stays reserved for victories (one icon, one meaning).
        let mut cells = vec![Cell::medal(is_beaten), Cell::value(value, is_beaten)];
        if has_records {
            cells.push(match record_for(key) {
                Some(rec) => Cell::dim(format_record(key, rec.value)),
                None => Cell::dim("\u{2014}".to_string()),
            });
        }
        table_row(ui, idx, label, &cells);
        idx += 1;
    };

    if show_score {
        row(ui, t.stat_score, format!("{}", score), RecordKey::BestScore);
    }
    row(ui, t.stat_duration, stats.duration_formatted(), RecordKey::LongestGame);
    row(ui, t.stat_blocks_eliminated, format!("{}", stats.blocks_eliminated), RecordKey::MostBlocks);
    if stats.max_chain_level > 0 {
        row(ui, t.stat_best_chain, format!("x{}", stats.max_chain_level), RecordKey::BestChain);
    }
    row(ui, t.stat_swaps, format!("{}", stats.swaps_performed), RecordKey::MostSwaps);
    row(ui, t.stat_blocks_per_min, format!("{:.0}", stats.blocks_per_minute()), RecordKey::BestBlocksPerMin);
}

/// Two-column comparative stats display (VS mode with opponent stats).
/// The best value of each "more is better" row is highlighted (accent color), so you can see
/// who won what without reading the numbers.
fn draw_comparative_stats(
    ui: &mut egui::Ui,
    local: &crackattack_core::stats::GameStats,
    opponent: &crackattack_core::stats::GameStats,
    names: Option<(&str, &str)>,
    t: &crate::app::locale::Locale,
) {
    let fmt_duration = |ticks: u32| -> String {
        let secs = ticks / 50;
        format!("{}:{:02}", secs / 60, secs % 60)
    };
    let (me_name, opp_name) = names.unwrap_or((t.stat_col_you, t.stat_col_opponent));

    table_row(ui, 0, "", &[
        Cell::header(me_name.to_string(), true),
        Cell::header(opp_name.to_string(), false),
    ]);

    let mut idx = 1usize;
    // `emphasize`: highlight the larger value. `None` marks a neutral row where "more" is not
    // "better" (duration, garbage received, swaps).
    let mut row = |ui: &mut egui::Ui, label: &str, lv: String, ov: String, emphasize: Option<(u32, u32)>| {
        let (me_best, opp_best) = match emphasize {
            Some((l, o)) if l != o => (l > o, o > l),
            _ => (false, false),
        };
        table_row(ui, idx, label, &[Cell::value(lv, me_best), Cell::value(ov, opp_best)]);
        idx += 1;
    };

    row(ui, t.stat_duration,
        local.duration_formatted(), fmt_duration(opponent.duration_ticks), None);
    row(ui, t.stat_blocks_eliminated,
        format!("{}", local.blocks_eliminated), format!("{}", opponent.blocks_eliminated),
        Some((local.blocks_eliminated, opponent.blocks_eliminated)));
    row(ui, t.stat_chains,
        format!("{}", local.chains_count), format!("{}", opponent.chains_count),
        Some((local.chains_count, opponent.chains_count)));
    if local.max_chain_level > 0 || opponent.max_chain_level > 0 {
        row(ui, t.stat_best_chain,
            format!("x{}", local.max_chain_level), format!("x{}", opponent.max_chain_level),
            Some((local.max_chain_level, opponent.max_chain_level)));
    }
    row(ui, t.stat_garbage_sent,
        format!("{}", local.garbage_sent), format!("{}", opponent.garbage_sent),
        Some((local.garbage_sent, opponent.garbage_sent)));
    row(ui, t.stat_garbage_received,
        format!("{}", local.garbage_received), format!("{}", opponent.garbage_received), None);
    row(ui, t.stat_garbage_shattered,
        format!("{}", local.garbage_shattered), format!("{}", opponent.garbage_shattered),
        Some((local.garbage_shattered, opponent.garbage_shattered)));
    row(ui, t.stat_swaps,
        format!("{}", local.swaps_performed), format!("{}", opponent.swaps_performed), None);
}

/// Single-column stats display without Record column (for VS Computer round stats).
fn draw_single_stats_no_records(
    ui: &mut egui::Ui,
    stats: &crackattack_core::stats::GameStats,
    t: &crate::app::locale::Locale,
) {
    let mut idx = 0usize;
    let mut row = |ui: &mut egui::Ui, label: &str, value: String| {
        table_row(ui, idx, label, &[Cell::value(value, false)]);
        idx += 1;
    };
    row(ui, t.stat_duration, stats.duration_formatted());
    row(ui, t.stat_blocks_eliminated, format!("{}", stats.blocks_eliminated));
    if stats.max_chain_level > 0 {
        row(ui, t.stat_best_chain, format!("x{}", stats.max_chain_level));
    }
    row(ui, t.stat_garbage_sent, format!("{}", stats.garbage_sent));
    row(ui, t.stat_garbage_received, format!("{}", stats.garbage_received));
    row(ui, t.stat_swaps, format!("{}", stats.swaps_performed));
}
