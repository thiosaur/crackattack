//! About / Info screen: shows one section (Rules, Credits or Updates) of the Info submenu.
//!
//! The content is embedded markdown files (under `assets/text/`), localized with a fallback to
//! English, and rendered by the small parser in `markdown.rs`. The page is read-only and
//! scrollable. The Credits page also has a separate contribute button.

use super::egui_theme::Palette;
use crate::app::locale::{current_language, Language};
use crate::app::menu_tree::InfoSection;

/// What the Updates page shows. The session derives it from the check state (persisted
/// results and the attempt in progress); the renderer only draws it.
// Without the `update-check` feature only `Unavailable` and `Managed` are ever built.
#[cfg_attr(not(feature = "update-check"), allow(dead_code))]
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum UpdateView {
    /// This binary cannot check for updates (built from source).
    #[default]
    Unavailable,
    /// Updates come from the store or the distribution (already-localized note).
    Managed(&'static str),
    Checking,
    NeverChecked,
    /// Nothing newer. `checked` is the date of the last successful check.
    UpToDate { checked: String },
    /// A newer version exists, with the page to open to get it.
    Available { version: String, url: String },
    /// The last attempt failed. `last_success` is the date of the last successful check.
    Failed { last_success: Option<String> },
}

impl UpdateView {
    /// Whether the player can start a check from this page.
    fn can_check(&self) -> bool {
        !matches!(self, Self::Unavailable | Self::Managed(_))
    }
}

/// Data for the Credits and Updates pages, coming from the session. It is resolved outside
/// the renderer from the distribution profile and the public identity, so the renderer never
/// decides on a URL. Everything is `None` / `Unavailable` while the public identity is not set.
#[derive(Default)]
pub struct InfoExtras {
    /// "Contribute" link (repository), if the distribution channel allows it.
    pub contribute_url: Option<String>,
    /// State of the Updates page.
    pub update_view: UpdateView,
    /// Is the daily automatic check enabled? `Some` means this binary can check for updates,
    /// and the checkbox is shown (a checkbox that does nothing would be worse than none).
    pub update_checks: Option<bool>,
}

/// Action of an Info page (Credits, Updates), listed in display order. The same list serves
/// rendering (highlight, click) and the router (Up/Down, Confirm), so an action reachable
/// with the mouse is also reachable with the keyboard and the gamepad.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InfoAction {
    /// "Download version X". Opens the channel page.
    ViewDownload(String),
    /// "Check now".
    CheckNow,
    /// "Check for updates" checkbox (daily automatic check).
    ToggleUpdateChecks,
    /// "Contribute": opens the repository page.
    Contribute(String),
}

impl InfoAction {
    /// Command triggered by the action. URLs are opened through a command, never by the
    /// renderer.
    pub fn command(&self) -> crate::app::command::AppCommand {
        use crate::app::command::AppCommand;
        match self {
            InfoAction::ViewDownload(url) | InfoAction::Contribute(url) => {
                AppCommand::OpenUrl(url.clone())
            }
            InfoAction::CheckNow => AppCommand::CheckUpdatesNow,
            InfoAction::ToggleUpdateChecks => AppCommand::ToggleUpdateChecks,
        }
    }
}

impl InfoExtras {
    /// Actions present on the `section` page, in display order.
    pub fn actions(&self, section: InfoSection) -> Vec<InfoAction> {
        let mut out = Vec::new();
        match section {
            InfoSection::Credits => {
                if let Some(url) = &self.contribute_url {
                    out.push(InfoAction::Contribute(url.clone()));
                }
            }
            InfoSection::Updates => {
                if let UpdateView::Available { url, .. } = &self.update_view {
                    out.push(InfoAction::ViewDownload(url.clone()));
                }
                if self.update_view.can_check() {
                    out.push(InfoAction::CheckNow);
                }
                if self.update_checks.is_some() {
                    out.push(InfoAction::ToggleUpdateChecks);
                }
            }
            InfoSection::Rules | InfoSection::NetworkRank | InfoSection::NetworkOpponents => {}
        }
        out
    }
}

/// Result of the Info screen for this frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InfoOutcome {
    /// Network Rank page: an opponent row is hovered, so select it.
    HoverRow(usize),
    /// Network Rank page: an opponent row is clicked, so reconnect quickly.
    RowClicked(usize),
    StillOpen,
    Close,
    /// Credits / Updates: an action was clicked. The caller turns it into a command
    /// ([`InfoAction::command`]), the same as when it is confirmed with keyboard or gamepad.
    InfoActionActivated(InfoAction),
    /// Credits / Updates: the mouse starts hovering an action, so select it (1-based).
    HoverInfoAction(usize),
    /// The mouse starts hovering the Back button, so focus it (the caller emits the command).
    HoverBack,
}

/// Embedded markdown of a section in the current language (each file is included once with
/// `include_str!`; the match is exhaustive, so a new language must bring its own files).
fn section_md(section: InfoSection, lang: Language) -> &'static str {
    match section {
        InfoSection::Rules => match lang {
            Language::French => include_str!("../../../assets/text/rules.fr.md"),
            Language::German => include_str!("../../../assets/text/rules.de.md"),
            Language::Spanish => include_str!("../../../assets/text/rules.es.md"),
            Language::Russian => include_str!("../../../assets/text/rules.ru.md"),
            Language::Chinese => include_str!("../../../assets/text/rules.zh.md"),
            Language::Japanese => include_str!("../../../assets/text/rules.ja.md"),
            Language::Korean => include_str!("../../../assets/text/rules.ko.md"),
            Language::Polish => include_str!("../../../assets/text/rules.pl.md"),
            Language::Turkish => include_str!("../../../assets/text/rules.tr.md"),
            Language::Italian => include_str!("../../../assets/text/rules.it.md"),
            Language::English => include_str!("../../../assets/text/rules.en.md"),
        },
        InfoSection::Credits => match lang {
            Language::French => include_str!("../../../assets/text/credits.fr.md"),
            Language::German => include_str!("../../../assets/text/credits.de.md"),
            Language::Spanish => include_str!("../../../assets/text/credits.es.md"),
            Language::Russian => include_str!("../../../assets/text/credits.ru.md"),
            Language::Chinese => include_str!("../../../assets/text/credits.zh.md"),
            Language::Japanese => include_str!("../../../assets/text/credits.ja.md"),
            Language::Korean => include_str!("../../../assets/text/credits.ko.md"),
            Language::Polish => include_str!("../../../assets/text/credits.pl.md"),
            Language::Turkish => include_str!("../../../assets/text/credits.tr.md"),
            Language::Italian => include_str!("../../../assets/text/credits.it.md"),
            Language::English => include_str!("../../../assets/text/credits.en.md"),
        },
        // The Updates page has no markdown (localized title, generated content). The network
        // pages are not markdown either: `rank_page` draws them, wired in before this
        // function in `frame_submit`. This arm is only defensive.
        InfoSection::Updates | InfoSection::NetworkRank | InfoSection::NetworkOpponents => "",
    }
}

/// Renders the Info page for `section` (full surface, not a dialog box). Returns `Close` when
/// the Back button is activated. `scroll_delta` (px, set by the navigation) is applied once;
/// `back_focused` is true when the Back button has focus.
///
/// Navigation (focus, scrolling, closing) is driven by UI intents from keyboard and gamepad
/// (see `route_info_intent`); this function only handles mouse clicks on the buttons.
pub fn render_info_screen(
    ctx: &egui::Context,
    section: InfoSection,
    scroll_delta: f32,
    back_focused: bool,
    extras: &InfoExtras,
    info_action_selected: usize,
    action_scroll_to_selected: bool,
) -> InfoOutcome {
    use super::nav_view::{render_nav_view, NavFocus, NavView, NavOutcome};
    let md = section_md(section, current_language());
    // The markdown `# Title` becomes the pinned title; the rest scrolls under it.
    let (title, body) = match section {
        InfoSection::Updates => (crate::app::locale::t().menu_updates, ""),
        _ => split_title(md),
    };
    let id_salt = match section {
        InfoSection::Rules => "info_rules",
        InfoSection::Credits => "info_credits",
        InfoSection::Updates => "info_updates",
        InfoSection::NetworkRank => "info_rank", // defensive: `rank_page` draws it in practice
        InfoSection::NetworkOpponents => "info_opponents", // defensive: same
    };

    let view = NavView {
        id_salt,
        title,
        // Labels under the side buttons: every `nav_view` view names its actions.
        left_label: Some(crate::app::locale::t().back),
        right_label: None,
        left_icon: super::icons::BACK,
        right_icon: None, // content page: no "next" button
        focus: if back_focused { NavFocus::Left } else { NavFocus::Content },
        scroll_delta,
        opaque_bg: true,
        center_content: false, // reading page: aligned to the top (the Updates page pads itself)
    };
    let actions = extras.actions(section);
    let mut clicked_action: Option<usize> = None;
    let mut hovered_action: Option<usize> = None;
    // Credits: the music theme is inserted before the first subsection ("License").
    let out = render_nav_view(ctx, &view, |ui| {
        super::markdown::render_markdown(ui, body);

        // Actions of the Credits and Updates pages, in the order of `InfoExtras::actions`.
        // `info_action_selected` (1-based, 0 = none) is set by keyboard/gamepad navigation or
        // by hovering. Each control exists only if `extras` provides it.
        let t = crate::app::locale::t();
        let mut action_button = |ui: &mut egui::Ui, action: &InfoAction, text: egui::RichText| {
            let i = actions.iter().position(|a| a == action).unwrap_or(usize::MAX);
            let selected = i + 1 == info_action_selected;
            // Visible selection: the egui selection frame (`highlight()` draws nothing on a
            // button without a frame).
            let r = ui.add(egui::Button::new(text).frame(selected).selected(selected))
                .on_hover_cursor(egui::CursorIcon::PointingHand);
            if selected && action_scroll_to_selected {
                r.scroll_to_me(Some(egui::Align::Center));
            }
            if r.hovered() {
                hovered_action = Some(i);
            }
            if r.clicked() {
                clicked_action = Some(i);
            }
        };
        let dim = |ui: &mut egui::Ui, text: &str| {
            ui.label(egui::RichText::new(text).size(12.0).color(Palette::TEXT_DIM));
        };
        match section {
            InfoSection::Credits => {
                ui.add_space(10.0);
                dim(ui, &format!("v{}", env!("CARGO_PKG_VERSION")));
                if let Some(url) = &extras.contribute_url {
                    action_button(ui, &InfoAction::Contribute(url.clone()),
                        egui::RichText::new(t.contribute).size(14.0).color(Palette::TEXT_ACCENT));
                }
            }
            InfoSection::Updates => {
                let (clicked, hovered) = render_updates_page(
                    ui, extras, &actions, info_action_selected, action_scroll_to_selected);
                clicked_action = clicked;
                hovered_action = hovered;
            }
            _ => {}
        }
    });

    if let Some(i) = clicked_action {
        return InfoOutcome::InfoActionActivated(actions[i].clone());
    }
    // Hover transition selects the action, without scroll-follow (mouse only).
    if let Some(i) = super::helpers::hover_select_transition(ctx, "info_action_hover", hovered_action) {
        return InfoOutcome::HoverInfoAction(i + 1);
    }
    match out {
        NavOutcome::LeftActivated => InfoOutcome::Close,
        // Hovering the Back button with the mouse gives it focus.
        NavOutcome::HoverLeft => InfoOutcome::HoverBack,
        _ => InfoOutcome::StillOpen,
    }
}

/// Width of the Updates page buttons (they shrink with the window).
const PAGE_BUTTON_W: f32 = 340.0;

/// Look of an Updates page button: the main action of the page is filled, the others are
/// outlined. Every button keeps its frame at rest, so it reads as a button before it is
/// selected or hovered.
#[derive(Clone, Copy)]
enum ButtonLook {
    Primary,
    Secondary,
}

fn page_button(
    ui: &mut egui::Ui,
    glyph: char,
    text: &str,
    look: ButtonLook,
    selected: bool,
    enabled: bool,
) -> egui::Response {
    use super::egui_theme::TextSize;
    let (fill, stroke, ink) = match look {
        ButtonLook::Primary => (
            Palette::TEXT_ACCENT,
            if selected { Palette::TEXT } else { Palette::TEXT_ACCENT },
            Palette::BG_APP,
        ),
        ButtonLook::Secondary => (
            if selected { Palette::BG_SELECTED } else { Palette::BG_PANEL },
            if selected { Palette::BORDER_ACTIVE } else { Palette::BORDER },
            Palette::TEXT,
        ),
    };
    let width = PAGE_BUTTON_W.min(ui.available_width());
    let button = egui::Button::new(super::icons::icon_text(glyph, text, TextSize::BODY, ink))
        .fill(fill)
        .stroke(egui::Stroke::new(2.0, stroke))
        .corner_radius(8.0)
        .min_size(egui::vec2(width, 40.0));
    let response = ui.add_enabled(enabled, button).on_hover_cursor(egui::CursorIcon::PointingHand);
    // A filled button hides its own border, so the focus shows as a ring around it.
    if selected && matches!(look, ButtonLook::Primary) {
        ui.painter().rect_stroke(
            response.rect.expand(4.0),
            11.0,
            egui::Stroke::new(2.0, Palette::BORDER_ACTIVE),
            egui::StrokeKind::Outside,
        );
    }
    response
}

/// Icon, headline and detail line of the Updates page, from its state.
fn updates_header(ui: &mut egui::Ui, view: &UpdateView) {
    let t = crate::app::locale::t();
    let installed = t.update_installed_fmt.replace("{}", env!("CARGO_PKG_VERSION"));
    let (glyph, color, headline, detail) = match view {
        UpdateView::Unavailable => (
            super::icons::INFO, Palette::TEXT_DIM, t.update_unavailable.to_string(), installed),
        UpdateView::Managed(note) => (
            super::icons::STORE, Palette::TEXT_DIM, note.to_string(), installed),
        UpdateView::Checking => (
            super::icons::REFRESH, Palette::TEXT_ACCENT, t.update_status_checking.to_string(), installed),
        UpdateView::NeverChecked => (
            super::icons::INFO, Palette::TEXT_DIM, t.update_status_never.to_string(), installed),
        UpdateView::UpToDate { checked } => (
            super::icons::CHECK_CIRCLE, Palette::SUCCESS, t.update_uptodate.to_string(),
            format!("{installed} · {}", t.update_checked_fmt.replace("{}", checked))),
        UpdateView::Available { version, .. } => (
            super::icons::DOWNLOAD, Palette::TEXT_ACCENT,
            t.update_status_available_fmt.replace("{}", version), installed),
        UpdateView::Failed { last_success } => (
            super::icons::WIFI_OFF, Palette::WARNING, t.update_status_failed.to_string(),
            match last_success {
                Some(date) => format!("{installed} · {}", t.update_last_success_fmt.replace("{}", date)),
                None => installed,
            }),
    };
    ui.vertical_centered(|ui| {
        if matches!(view, UpdateView::Checking) {
            ui.add(egui::Spinner::new().size(44.0).color(color));
        } else {
            ui.label(super::icons::icon(glyph).size(44.0).color(color));
        }
        ui.add_space(6.0);
        ui.label(egui::RichText::new(headline).size(20.0).strong().color(Palette::TEXT));
        ui.label(egui::RichText::new(detail).size(14.0).color(Palette::TEXT_DIM));
    });
}

/// The Updates page: state first (icon and headline), then the actions as visible buttons,
/// then the automatic-check switch. Returns the action clicked and the action hovered this
/// frame (indexes into `actions`).
fn render_updates_page(
    ui: &mut egui::Ui,
    extras: &InfoExtras,
    actions: &[InfoAction],
    info_action_selected: usize,
    scroll_to_selected: bool,
) -> (Option<usize>, Option<usize>) {
    let t = crate::app::locale::t();
    let mut clicked = None;
    let mut hovered = None;
    // `selected` follows the same 1-based index as keyboard and gamepad navigation.
    let index_of = |a: &InfoAction| actions.iter().position(|x| x == a);
    let mut track = |r: &egui::Response, i: Option<usize>, selected: bool, also_hovered: bool, also_clicked: bool| {
        if selected && scroll_to_selected {
            r.scroll_to_me(Some(egui::Align::Center));
        }
        if let Some(i) = i {
            if r.hovered() || also_hovered {
                hovered = Some(i);
            }
            if r.clicked() || also_clicked {
                clicked = Some(i);
            }
        }
    };

    // The page is short: push it down so it sits in the middle of the frame, whatever the
    // window height (the frame itself aligns its content to the top).
    const PAGE_HEIGHT: f32 = 330.0;
    ui.add_space(((ui.available_height().min(900.0) - PAGE_HEIGHT) / 2.0).max(0.0));
    ui.vertical_centered(|ui| {
        updates_header(ui, &extras.update_view);
        ui.add_space(14.0);

        let view = &extras.update_view;
        let check_look = match view {
            UpdateView::Failed { .. } | UpdateView::NeverChecked => ButtonLook::Primary,
            _ => ButtonLook::Secondary,
        };
        if let UpdateView::Available { version, url } = view {
            let action = InfoAction::ViewDownload(url.clone());
            let i = index_of(&action);
            let selected = i.is_some_and(|i| i + 1 == info_action_selected);
            let label = t.update_download_fmt.replace("{}", version);
            let r = page_button(ui, super::icons::DOWNLOAD, &label, ButtonLook::Primary, selected, true);
            track(&r, i, selected, false, false);
            ui.add_space(8.0);
        }
        if view.can_check() {
            let i = index_of(&InfoAction::CheckNow);
            let selected = i.is_some_and(|i| i + 1 == info_action_selected);
            let label = if matches!(view, UpdateView::Failed { .. }) { t.update_retry } else { t.update_check_now };
            // No second check while one is running (the command would do nothing).
            let enabled = !matches!(view, UpdateView::Checking);
            let r = page_button(ui, super::icons::REFRESH, label, check_look, selected, enabled);
            track(&r, i, selected, false, false);
        }
        if let Some(on) = extras.update_checks {
            ui.add_space(14.0);
            let i = index_of(&InfoAction::ToggleUpdateChecks);
            let selected = i.is_some_and(|i| i + 1 == info_action_selected);
            // Same on/off component as the Options screens: the egui checkbox, focus shown
            // with `highlight()` and no frame around it.
            let mut value = on;
            let r = ui.checkbox(&mut value, egui::RichText::new(t.update_checks_option).size(14.0));
            let r = if selected { r.highlight() } else { r };
            track(&r, i, selected, false, false);
        }
    });
    (clicked, hovered)
}

/// Splits the leading `# Title` (pinned) from the rest (scrollable body). Without a title it
/// returns `("", md)`.
fn split_title(md: &str) -> (&str, &str) {
    if let Some(rest) = md.strip_prefix("# ") {
        if let Some(nl) = rest.find('\n') {
            return (rest[..nl].trim_end(), &rest[nl + 1..]);
        }
        return (rest.trim_end(), "");
    }
    ("", md)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::command::AppCommand;

    /// The Updates page offers a check only when this binary can check, and the download only
    /// when a newer version exists. A store-managed or source build has no action at all.
    #[test]
    fn updates_actions_follow_the_state() {
        let view = |update_view| InfoExtras { update_view, ..Default::default() }
            .actions(InfoSection::Updates);
        assert!(view(UpdateView::Unavailable).is_empty());
        assert!(view(UpdateView::Managed("store")).is_empty());
        for v in [
            UpdateView::Checking,
            UpdateView::NeverChecked,
            UpdateView::UpToDate { checked: "2026-10-28".into() },
            UpdateView::Failed { last_success: None },
        ] {
            assert_eq!(view(v), vec![InfoAction::CheckNow]);
        }
    }

    /// Actions follow the display order, page by page. Each exists only if its control
    /// exists, and links go through a command (never opened by the renderer).
    #[test]
    fn info_actions_order_and_commands() {
        let none = InfoExtras::default();
        assert!(none.actions(InfoSection::Updates).is_empty());
        assert!(none.actions(InfoSection::Credits).is_empty());
        assert!(none.actions(InfoSection::Rules).is_empty());

        let extras = InfoExtras {
            contribute_url: Some("https://example.test/repo".into()),
            update_view: UpdateView::Available {
                version: "2.1.0".into(),
                url: "https://example.test/releases".into(),
            },
            update_checks: Some(true),
        };
        let updates = extras.actions(InfoSection::Updates);
        assert_eq!(updates, vec![
            InfoAction::ViewDownload("https://example.test/releases".into()),
            InfoAction::CheckNow,
            InfoAction::ToggleUpdateChecks,
        ]);
        assert!(matches!(updates[0].command(), AppCommand::OpenUrl(u) if u.ends_with("/releases")));
        assert!(matches!(updates[1].command(), AppCommand::CheckUpdatesNow));
        assert!(matches!(updates[2].command(), AppCommand::ToggleUpdateChecks));

        let credits = extras.actions(InfoSection::Credits);
        assert_eq!(credits, vec![InfoAction::Contribute("https://example.test/repo".into())]);
        assert!(matches!(credits[0].command(), AppCommand::OpenUrl(u) if u.ends_with("/repo")));
    }
}
