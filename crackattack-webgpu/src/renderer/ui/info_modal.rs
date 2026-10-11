//! About / Info screen: shows one section (Rules, Credits or Updates) of the Info submenu.
//!
//! The content is embedded markdown files (under `assets/text/`), localized with a fallback to
//! English, and rendered by the small parser in `markdown.rs`. The page is read-only and
//! scrollable. The Credits page also has a separate contribute button.

use super::egui_theme::Palette;
use crate::app::locale::{current_language, Language};
use crate::app::menu_tree::InfoSection;

/// Data for the Credits and Updates pages, coming from the session. It is resolved outside
/// the renderer from the distribution profile and the public identity, so the renderer never
/// decides on a URL. Everything is `None` while the public identity is not set.
#[derive(Default)]
pub struct InfoExtras {
    /// "Contribute" link (repository), if the distribution channel allows it.
    pub contribute_url: Option<String>,
    /// Available update: (version, URL to open).
    pub update: Option<(String, String)>,
    /// Already-localized status line of the update check. `Some` means this binary can check
    /// for updates: the "Check now" button and this line are shown.
    pub update_status: Option<String>,
    /// Already-localized note that updates are managed by the store or distribution.
    pub managed_info: Option<&'static str>,
    /// Is the daily automatic check enabled? `Some` means this binary can check for updates,
    /// and the checkbox is shown (a checkbox that does nothing would be worse than none).
    pub update_checks: Option<bool>,
}

/// Action of an Info page (Credits, Updates), listed in display order. The same list serves
/// rendering (highlight, click) and the router (Up/Down, Confirm), so an action reachable
/// with the mouse is also reachable with the keyboard and the gamepad.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InfoAction {
    /// "Version X available: view the download". Opens the channel page.
    ViewDownload(String),
    /// "Ignore this version".
    IgnoreVersion(String),
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
            InfoAction::IgnoreVersion(v) => AppCommand::IgnoreUpdateVersion(v.clone()),
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
                if let Some((version, url)) = &self.update {
                    out.push(InfoAction::ViewDownload(url.clone()));
                    out.push(InfoAction::IgnoreVersion(version.clone()));
                }
                if self.update_status.is_some() {
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
        center_content: false, // reading page: aligned to the top
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
                ui.label(egui::RichText::new(t.update_installed_fmt
                    .replace("{}", env!("CARGO_PKG_VERSION"))).size(16.0));
                ui.add_space(8.0);
                if let Some((version, url)) = &extras.update {
                    // The "version available" line is itself the button (opens the channel page).
                    action_button(ui, &InfoAction::ViewDownload(url.clone()),
                        egui::RichText::new(t.update_view_download_fmt.replace("{}", version))
                            .size(14.0).color(Palette::TEXT_ACCENT));
                    action_button(ui, &InfoAction::IgnoreVersion(version.clone()),
                        egui::RichText::new(t.ignore_this_version).size(12.0).color(Palette::TEXT_DIM));
                    ui.add_space(6.0);
                }
                if let Some(status) = &extras.update_status {
                    dim(ui, status);
                    action_button(ui, &InfoAction::CheckNow,
                        egui::RichText::new(t.update_check_now).size(13.0).color(Palette::TEXT_ACCENT));
                }
                if let Some(enabled) = extras.update_checks {
                    ui.add_space(6.0);
                    let mark = if enabled { "[x]" } else { "[ ]" };
                    action_button(ui, &InfoAction::ToggleUpdateChecks,
                        egui::RichText::new(format!("{mark} {}", t.update_checks_option)).size(13.0));
                }
                if let Some(managed) = extras.managed_info {
                    dim(ui, managed);
                } else if extras.update_status.is_none() {
                    dim(ui, t.update_unavailable);
                }
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
            update: Some(("2.1.0".into(), "https://example.test/releases".into())),
            update_status: Some("Version 2.1.0 available".into()),
            managed_info: Some("managed by"),
            update_checks: Some(true),
        };
        let updates = extras.actions(InfoSection::Updates);
        assert_eq!(updates, vec![
            InfoAction::ViewDownload("https://example.test/releases".into()),
            InfoAction::IgnoreVersion("2.1.0".into()),
            InfoAction::CheckNow,
            InfoAction::ToggleUpdateChecks,
        ]);
        assert!(matches!(updates[0].command(), AppCommand::OpenUrl(u) if u.ends_with("/releases")));
        assert!(matches!(updates[1].command(), AppCommand::IgnoreUpdateVersion(v) if v == "2.1.0"));
        assert!(matches!(updates[2].command(), AppCommand::CheckUpdatesNow));
        assert!(matches!(updates[3].command(), AppCommand::ToggleUpdateChecks));

        let credits = extras.actions(InfoSection::Credits);
        assert_eq!(credits, vec![InfoAction::Contribute("https://example.test/repo".into())]);
        assert!(matches!(credits[0].command(), AppCommand::OpenUrl(u) if u.ends_with("/repo")));
    }
}
