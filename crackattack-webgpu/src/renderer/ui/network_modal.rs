//! LAN discovery screen egui UI — rendering only, no session mutations.
//! Custom keyboard navigation. Uses the global arcade theme from egui_theme.

use crate::app::manual_connect::{ManualConnectState, ManualField, ManualFocus};
use crate::app::meta_state::LanScreenState;
use crate::renderer::ui::egui_theme::Palette;
use crackattack_net::discovery::PeerStatus;

/// Result of rendering the LAN screen for one frame.
pub enum LanScreenOutcome {
    StillOpen,
    /// Paste button (manual screen): asks the shell to read the clipboard. The UI never
    /// accesses the clipboard directly. `main.rs` then splits the text into
    /// `LanScreenState.manual` (through `LanPasteRequest`).
    RequestPaste,
    PeerClicked {
        addr: std::net::IpAddr,
        port: u16,
    },
    ManualConnect {
        addr: std::net::IpAddr,
        port: u16,
    },
    /// Cancel an invite **we sent** (`Inviting` phase) and go back to the list.
    CancelInvite,
    /// Reconnect screen: switch to the manual connection screen, prefilled.
    ReconnectManual,
    /// Version mismatch error: "View download" button (the URL comes with the error).
    OpenUrl(String),
    Cancelled,
}

pub fn render_lan_screen(
    ctx: &egui::Context,
    state: &mut LanScreenState,
) -> LanScreenOutcome {
    use crate::app::meta_state::LanScreenPhase;
    // Theme applied globally in renderer_runtime.rs — no local set_style
    let local_port = state.local_port;

    // Selection and the manual dialog live in the session state
    // (`state.selected` / `state.manual_open`), not in egui temp data.
    let mut outcome = LanScreenOutcome::StillOpen;

    // The manual screen is a **sibling** screen (not an overlay). Input state and focus live
    // in the session (`state.manual`), keyboard and gamepad navigation is handled by the
    // router (`route_lan_intent`), and this code only renders, handles click/touch and maps
    // hover to focus. Back returns to the menu.
    if state.manual_open {
        // Local network detection runs once per opening of the screen (the field is reset to
        // `None` on each opening), outside any egui lock.
        let net_info = state.manual_net_info
            .get_or_insert_with(crackattack_net::discovery::detect_local_network)
            .clone();
        let error = state.manual_error.clone();
        render_manual_screen(ctx, local_port, &mut state.manual, error.as_deref(),
            net_info.as_ref(), &mut outcome);
        return outcome;
    }

    // --- Auto screen (scan + peer list) ---
    // Keyboard and gamepad navigation both go through `route_lan_intent` (see
    // `keyboard_goes_to_egui`). This code only renders: click/touch and focus highlight
    // (`state.selected`). It never reads `ctx.input()`.
    let selected = state.selected;

    egui::Window::new("VS Network — LAN")
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .resizable(false)
        .collapsible(false)
        .title_bar(false)
        .show(ctx, |ui| {
            ui.set_min_width(380.0);

            egui::Frame::NONE
                .inner_margin(egui::Margin::same(16))
                .show(ui, |ui| {
                    ui.label(egui::RichText::new(crate::app::locale::t().vs_network)
                        .size(super::egui_theme::TextSize::TITLE).strong().color(Palette::TEXT));
                    ui.add_space(2.0);
                    ui.add(egui::Separator::default().spacing(4.0));
                    ui.add_space(4.0);

                    match &state.phase {
                        LanScreenPhase::Scanning => {
                            render_peer_list(ui, &state.peers, &mut state.selected, true, &mut outcome);
                        }
                        // Waiting for the invite answer: the list stays in the background
                        // (inactive) and the waiting modal opens on top (see below).
                        LanScreenPhase::Inviting { .. } => {
                            render_peer_list(ui, &state.peers, &mut state.selected, false, &mut outcome);
                        }
                        LanScreenPhase::Error { error, hint } => {
                            let t = crate::app::locale::t();
                            ui.add_space(5.0);
                            // Translated every frame, so a language change applies even
                            // while the error screen is open.
                            ui.label(egui::RichText::new(error.message(t))
                                .color(Palette::DANGER));
                            ui.add_space(10.0);
                            // Guidance: a navigable action first in the error focus ring
                            // (index 0), or a plain notice.
                            match hint {
                                Some(crate::app::meta_state::LanErrorHint::Download(url)) => {
                                    let resp = super::helpers::selectable_row(
                                        ui, selected == 0, true, super::helpers::RowGauge::ModalThin, 280.0,
                                        |ui, text_color| {
                                            ui.label(egui::RichText::new(t.view_download).color(text_color)
                                                .size(super::egui_theme::TextSize::BODY));
                                        });
                                    if resp.clicked() {
                                        outcome = LanScreenOutcome::OpenUrl(url.clone());
                                    }
                                    if super::helpers::hover_select_transition(
                                        ui.ctx(), "lan_error_action_hover", resp.hovered().then_some(0))
                                        .is_some()
                                    {
                                        state.selected = 0;
                                    }
                                    ui.add_space(6.0);
                                }
                                Some(crate::app::meta_state::LanErrorHint::ManagedBy(m)) => {
                                    use crate::platform::distribution::ManagedBy;
                                    let text = match m {
                                        ManagedBy::Store => t.update_managed_by_store,
                                        ManagedBy::Distro => t.update_managed_by_distro,
                                    };
                                    ui.label(egui::RichText::new(text).color(Palette::TEXT_DIM));
                                    ui.add_space(6.0);
                                }
                                None => {}
                            }
                        }
                        // Reconnect screen: instructions for the peer, repeated attempts
                        // while this screen stays open, and a switch to the prefilled
                        // manual connection screen.
                        LanScreenPhase::Reconnecting { peer_name } => {
                            let t = crate::app::locale::t();
                            ui.add_space(8.0);
                            ui.label(egui::RichText::new(t.reconnecting_to_fmt.replace("{}", peer_name))
                                .size(16.0).strong().color(Palette::TEXT));
                            ui.add_space(6.0);
                            ui.label(egui::RichText::new(t.reconnect_instructions.replace("{}", peer_name))
                                .size(13.0).color(Palette::TEXT_DIM));
                            searching_indicator(ui, egui::RichText::new(t.reconnect_trying)
                                .size(12.0).color(Palette::TEXT_DIM));
                            ui.add_space(8.0);
                            // Two navigable buttons: [Manual connection] / [Back].
                            let mut hovered_btn: Option<usize> = None;
                            for (idx, label) in [(0usize, t.manual_connection), (1usize, t.back)] {
                                let is_sel = state.selected == idx;
                                let resp = super::helpers::selectable_row(
                                    ui, is_sel, true, super::helpers::RowGauge::ModalThin, 280.0,
                                    |ui, text_color| {
                                        ui.label(egui::RichText::new(label).color(text_color)
                                            .size(super::egui_theme::TextSize::BODY));
                                    });
                                if resp.clicked() {
                                    outcome = if idx == 0 {
                                        LanScreenOutcome::ReconnectManual
                                    } else {
                                        LanScreenOutcome::CancelInvite
                                    };
                                } else if resp.hovered() {
                                    hovered_btn = Some(idx);
                                }
                                ui.add_space(2.0);
                            }
                            if let Some(i) = super::helpers::hover_select_transition(
                                ui.ctx(), "reconnect_btn_hover", hovered_btn)
                            {
                                state.selected = i;
                            }
                        }
                    }

                    // Standard footer: Back (to the menu). While scanning or on error it is the
                    // last navigable element (index `peers.len()`). Hidden while waiting for an
                    // invite answer (the modal handles the actions).
                    if !matches!(state.phase,
                        LanScreenPhase::Inviting { .. } | LanScreenPhase::Reconnecting { .. })
                    {
                        let in_footer = matches!(
                            state.phase,
                            LanScreenPhase::Scanning | LanScreenPhase::Error { .. }
                        );
                        let retour_focused = in_footer && selected == state.retour_index();
                        let (clicked, hovered) = lan_footer(ui, retour_focused);
                        if clicked {
                            outcome = LanScreenOutcome::Cancelled;
                        }
                        // Hover transition on Back focuses it (index = `retour_index`).
                        if in_footer
                            && super::helpers::hover_select_transition(
                                ui.ctx(), "lan_footer_hover", hovered.then_some(0)).is_some()
                        {
                            state.selected = state.retour_index();
                        }
                    }
                });
        });

    // Waiting modal, centered on top of the list (foreground): "Waiting for {peer}" and a
    // Cancel button that returns to the list. (The incoming invite is a separate overlay.)
    if let LanScreenPhase::Inviting { peer_name } = &state.phase {
        render_waiting_modal(ctx, peer_name, &mut outcome);
    }

    outcome
}

/// Waiting modal (we sent an invite and wait for the answer): "Waiting for {peer}...", a
/// spinner and a **Cancel** button that returns to the list. Drawn on the foreground, with
/// the list visible behind.
fn render_waiting_modal(ctx: &egui::Context, peer_name: &str, outcome: &mut LanScreenOutcome) {
    egui::Window::new("lan_waiting_modal")
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .resizable(false)
        .collapsible(false)
        .title_bar(false)
        .order(egui::Order::Foreground)
        .show(ctx, |ui| {
            egui::Frame::NONE
                .inner_margin(egui::Margin::same(20))
                .show(ui, |ui| {
                    ui.vertical_centered(|ui| {
                        ui.spinner();
                        ui.add_space(8.0);
                        ui.label(egui::RichText::new(
                            crate::app::locale::t().waiting_for_fmt.replace("{}", peer_name))
                            .color(Palette::TEXT_ACCENT).size(super::egui_theme::TextSize::BODY));
                        ui.add_space(16.0);
                        if ui.add(egui::Button::new(
                            egui::RichText::new(format!("  {}  ", crate::app::locale::t().cancel))
                                .color(Palette::TEXT_DIM).size(15.0))
                            .min_size(egui::vec2(150.0, 36.0)))
                            .on_hover_cursor(egui::CursorIcon::PointingHand)
                            .clicked()
                        {
                            *outcome = LanScreenOutcome::CancelInvite;
                        }
                    });
                });
        });
}

/// Outcome of the invite overlay (mouse click; keyboard and gamepad go through
/// `route_invite_intent`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InviteOverlayOutcome {
    StillOpen,
    Accept,
    Decline,
}

/// Incoming invite **overlay**, drawn on the foreground above whatever screen is showing
/// (menu, network rank, recent opponents, LAN screen...).
///
/// Two states: *request* (Accept / Decline) and *connecting* (spinner "Connecting to ..."
/// during the handshake). Renders and handles mouse clicks; the focus (`selection`) is shared
/// with the keyboard and gamepad router.
pub fn render_invite_overlay(
    ctx: &egui::Context,
    info: &crate::app::meta_state::IncomingInvite,
) -> InviteOverlayOutcome {
    let mut outcome = InviteOverlayOutcome::StillOpen;
    let t = crate::app::locale::t();
    // Full-screen dark scrim: separates the overlay from the screen below.
    egui::Area::new(egui::Id::new("invite_overlay_scrim"))
        .order(egui::Order::Middle)
        .fixed_pos(ctx.screen_rect().min)
        .show(ctx, |ui| {
            ui.painter().rect_filled(ctx.screen_rect(), 0.0,
                egui::Color32::from_black_alpha(150));
        });
    let mut hovered_now: Option<usize> = None;
    egui::Window::new("invite_overlay")
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .resizable(false).collapsible(false).title_bar(false)
        .order(egui::Order::Foreground)
        .show(ctx, |ui| {
            egui::Frame::new()
                .fill(Palette::BG_PANEL)
                .stroke(egui::Stroke::new(1.0, Palette::BORDER_ACTIVE))
                .corner_radius(10.0)
                .inner_margin(egui::Margin::same(22))
                .show(ui, |ui| {
                    ui.vertical_centered(|ui| {
                        if info.connecting {
                            ui.horizontal(|ui| {
                                ui.add(egui::Spinner::new().size(16.0).color(Palette::TEXT_ACCENT));
                                ui.label(egui::RichText::new(
                                    t.connecting_to_fmt.replace("{}", &info.from_name))
                                    .color(Palette::TEXT_ACCENT).size(16.0).strong());
                            });
                            return;
                        }
                        ui.label(egui::RichText::new(
                            t.wants_to_play_fmt.replace("{}", &info.from_name))
                            .color(Palette::TEXT).size(18.0).strong());
                        if let Some(rt) = info.from_rating {
                            ui.add_space(4.0);
                            let cfg = crackattack_rating::RatingConfig::default();
                            let lg = crate::app::ladder_view::league_of_value(rt as f64, &cfg);
                            super::rank_page::league_chip(ui, lg, Some(rt), 14.0);
                        }
                        ui.add_space(16.0);
                        let sel = info.selection.get().min(1);
                        ui.horizontal(|ui| {
                            let accept = ui.add(egui::Button::new(
                                egui::RichText::new(format!("  {}  ", crate::app::locale::t().accept))
                                    .color(Palette::TEXT).size(15.0))
                                .min_size(egui::vec2(150.0, 36.0)))
                                .on_hover_cursor(egui::CursorIcon::PointingHand);
                            if sel == 0 { accept.clone().highlight(); }
                            if accept.clicked() { outcome = InviteOverlayOutcome::Accept; }
                            else if accept.hovered() { hovered_now = Some(0); }
                            ui.add_space(8.0);
                            let decline = ui.add(egui::Button::new(
                                egui::RichText::new(format!("  {}  ", crate::app::locale::t().decline))
                                    .color(Palette::TEXT_DIM).size(15.0))
                                .min_size(egui::vec2(150.0, 36.0)))
                                .on_hover_cursor(egui::CursorIcon::PointingHand);
                            if sel == 1 { decline.clone().highlight(); }
                            if decline.clicked() { outcome = InviteOverlayOutcome::Decline; }
                            else if decline.hovered() { hovered_now = Some(1); }
                        });
                    });
                });
        });
    if let Some(i) = super::helpers::hover_select_transition(ctx, "invite_overlay_hover", hovered_now) {
        info.selection.set(i);
    }
    outcome
}

/// Standard footer of the LAN screens: a Back item (arrow icon and word) that returns to
/// the menu.
///
/// `retour_focused` draws the item in the accent color (keyboard/gamepad focus). Returns
/// `(clicked, hovered)`; the caller uses the hover to move the focus to Back.
fn lan_footer(ui: &mut egui::Ui, retour_focused: bool) -> (bool, bool) {
    ui.add_space(6.0);
    ui.add(egui::Separator::default().spacing(4.0));
    ui.add_space(4.0);
    let mut retour = false;
    let mut hovered = false;
    ui.horizontal(|ui| {
        let rcol = if retour_focused { Palette::TEXT_ACCENT } else { Palette::TEXT_DIM };
        let i = ui
            .add(egui::Label::new(super::icons::icon(super::icons::BACK).size(18.0).color(rcol)).sense(egui::Sense::click()))
            .on_hover_cursor(egui::CursorIcon::PointingHand);
        let l = ui
            .add(egui::Label::new(egui::RichText::new(crate::app::locale::t().menu_back).size(15.0).color(rcol)).sense(egui::Sense::click()))
            .on_hover_cursor(egui::CursorIcon::PointingHand);
        retour = i.clicked() || l.clicked();
        hovered = i.hovered() || l.hovered();
    });
    (retour, hovered)
}

/// Manual connection screen: pure rendering plus click/touch (see the focus contract in
/// `helpers.rs`).
///
/// The state (input and focus) lives in the session (`LanScreenState.manual`), and keyboard
/// and gamepad navigation goes through the router (`route_lan_intent` and
/// `AppCommand::Manual*`). This code draws, maps hover transitions to focus, and turns clicks
/// into an outcome or a direct mutation (way (a) of the contract).
fn render_manual_screen(
    ctx: &egui::Context,
    local_port: u16,
    manual: &mut ManualConnectState,
    error: Option<&str>,
    net_info: Option<&crackattack_net::discovery::LocalNetInfo>,
    outcome: &mut LanScreenOutcome,
) {
    // Element hovered this frame (any element of the focus ring); a transition moves the focus.
    let mut hovered: Option<ManualFocus> = None;
    let editing_open = manual.editing.is_some();
    egui::Window::new("manual_connect_dialog")
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .resizable(false)
        .collapsible(false)
        .title_bar(false)
        .show(ctx, |ui| {
            ui.set_min_width(370.0);

            egui::Frame::NONE
                .inner_margin(egui::Margin::same(16))
                .show(ui, |ui| {
                    ui.label(egui::RichText::new(crate::app::locale::t().manual_connection)
                        .size(super::egui_theme::TextSize::TITLE).strong().color(Palette::TEXT));
                    ui.add_space(2.0);
                    ui.add(egui::Separator::default().spacing(4.0));
                    ui.add_space(6.0);

                    // Copy / Set IPv4 / IPv6 / Port / Paste / Connect buttons.
                    render_manual_connect(ui, local_port, manual, error, net_info,
                        outcome, &mut hovered);

                    // Standard footer: Back (to the menu, last element of the focus ring).
                    // Escape/B = Back through the router (`Cancel` -> `CloseLanScreen`). With
                    // the on-screen keyboard open, Escape closes the keyboard instead.
                    let back_focused = manual.nav_focus == ManualFocus::Back;
                    let (retour, retour_hovered) = lan_footer(ui, back_focused);
                    if retour && !editing_open {
                        *outcome = LanScreenOutcome::Cancelled;
                    }
                    if retour_hovered {
                        hovered = Some(ManualFocus::Back);
                    }
                });
        });

    // A hover transition moves the focus shared by keyboard, gamepad and mouse, so a still
    // mouse does not block navigation (rule 2 of the contract). Indexed on the current ring.
    if !editing_open {
        let ring = ManualFocus::ring(net_info.is_some());
        let hovered_idx = hovered.and_then(|f| ring.iter().position(|r| *r == f));
        if let Some(i) = super::helpers::hover_select_transition(ctx, "manual_hover", hovered_idx) {
            manual.nav_focus = ring[i];
        }
    }

    // On-screen keyboard matching the field, drawn **above** the window (at `ctx` level,
    // after the window, like the name editor). It edits the field in the session state.
    render_manual_osk(ctx, manual);
}

/// Draws the on-screen keyboard of the manual screen when it is open (`manual.editing`),
/// above everything.
///
/// Edits the target field **in the session state**, and finalizes it (trim, default) when
/// closed. Physical keys arrive through egui (`keyboard_goes_to_egui`) and the gamepad through
/// the router (`ManualKb*`): one grid, two converging inputs.
fn render_manual_osk(ctx: &egui::Context, manual: &mut ManualConnectState) {
    let Some(field) = manual.editing else { return };
    let format = field.format();
    let mut focus = manual.kb_focus;
    let mut goal = manual.kb_goal_x;
    let frame = {
        let text = field.value_mut(manual);
        super::onscreen_keyboard::render_osk_grid(ctx, text, format, &mut focus, &mut goal)
    };
    manual.kb_focus = focus;
    manual.kb_goal_x = goal;
    if frame.closed {
        manual.kb_close(); // finalizes the field and closes
    }
}

/// Body of the manual screen: your address (with Copy), the 3 fields (IPv4, IPv6, port),
/// Paste, validation messages and the Connect button.
///
/// Rendering and click/touch only. The focus is read from `manual.nav_focus` (highlight);
/// keyboard and gamepad activation goes through the router (`ManualNavActivate`). Hovers are
/// reported in `hovered` and turned into focus by the caller.
fn render_manual_connect(
    ui: &mut egui::Ui,
    local_port: u16,
    manual: &mut ManualConnectState,
    error: Option<&str>,
    net_info: Option<&crackattack_net::discovery::LocalNetInfo>,
    outcome: &mut LanScreenOutcome,
    hovered: &mut Option<ManualFocus>,
) {
    let t = crate::app::locale::t();
    let focus = manual.nav_focus;
    let editing_open = manual.editing.is_some();

    // --- Your address (to copy and send to the opponent) ---
    ui.label(egui::RichText::new(t.your_address).color(Palette::TEXT_DIM).size(13.0));
    ui.add_space(2.0);
    match net_info.as_ref() {
        Some(info) => {
            let ip = info.ip.to_string();
            let port = local_port.to_string();
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(&ip).color(Palette::TEXT_ACCENT).strong().size(super::egui_theme::TextSize::BODY));
                ui.add_space(8.0);
                ui.label(egui::RichText::new(":").color(Palette::TEXT_DIM).size(super::egui_theme::TextSize::BODY));
                ui.add_space(8.0);
                ui.label(egui::RichText::new(&port).color(Palette::TEXT_ACCENT).strong().size(super::egui_theme::TextSize::BODY));
            });
            ui.add_space(6.0);
            let copy = egui::Button::new(super::icons::icon_text(
                super::icons::COPY, t.copy_address, 15.0, Palette::TEXT));
            let copy_resp = ui.add(copy).on_hover_cursor(egui::CursorIcon::PointingHand);
            if focus == ManualFocus::Copy { copy_resp.clone().highlight(); }
            if copy_resp.clicked() {
                // Mouse: egui clipboard output. Keyboard and gamepad go through
                // `ManualNavActivate` and `CopyToClipboard` (handled by the platform shell).
                ui.ctx().copy_text(format!("{}:{}", ip, port));
            } else if copy_resp.hovered() {
                *hovered = Some(ManualFocus::Copy);
            }
        }
        None => {
            ui.label(egui::RichText::new(
                format!("{} ({} {})", t.no_network_detected, t.port_field, local_port))
                .color(Palette::TEXT_DIM).size(13.0));
        }
    }

    ui.add_space(8.0);
    ui.add(egui::Separator::default().spacing(4.0));
    ui.add_space(8.0);

    // --- Opponent address: 3 buttons (IPv4 / IPv6 / port), each opening a matching
    //     on-screen keyboard. The values live in the session state (`manual`). ---
    ui.label(egui::RichText::new(t.opponent_address).color(Palette::TEXT_DIM).size(13.0));
    ui.add_space(4.0);

    // One row per field: [Set X] (fixed width) | current value (dash if empty; no example
    // address, which would be confusing). `fvar` is the matching focus element (highlight;
    // keyboard and gamepad activation goes through the router).
    let field_row = |ui: &mut egui::Ui, manual: &mut ManualConnectState,
                     hovered: &mut Option<ManualFocus>,
                     field: ManualField, name: &str, fvar: ManualFocus| {
        ui.horizontal(|ui| {
            let label = format!("  {} {}  ", t.define_field, name);
            let btn = egui::Button::new(egui::RichText::new(label).size(15.0).color(Palette::TEXT))
                .min_size(egui::vec2(140.0, 0.0));
            let resp = ui.add_enabled(!editing_open, btn).on_hover_cursor(egui::CursorIcon::PointingHand);
            if focus == fvar { resp.clone().highlight(); }
            if resp.clicked() {
                manual.open_editor(field);
            } else if resp.hovered() {
                *hovered = Some(fvar);
            }
            ui.add_space(8.0);
            let val = field.value(manual);
            if val.is_empty() {
                ui.label(egui::RichText::new("—").color(Palette::TEXT_DIM).size(15.0));
            } else {
                // Malformed value (e.g. 999.999.999.999): warning color. This is structural
                // validation, not just the character filter.
                let color = if field.is_valid(val) { Palette::TEXT_ACCENT } else { Palette::DANGER };
                ui.label(egui::RichText::new(val).color(color).strong().size(15.0));
            }
        });
    };
    field_row(ui, manual, hovered, ManualField::Ipv4, "IPv4", ManualFocus::Ipv4);
    ui.add_space(4.0);
    field_row(ui, manual, hovered, ManualField::Ipv6, "IPv6", ManualFocus::Ipv6);
    ui.add_space(4.0);
    field_row(ui, manual, hovered, ManualField::Port, t.port_field, ManualFocus::Port);

    ui.add_space(6.0);
    // **Paste** button: asks the shell to read the clipboard. `main.rs` (`LanPasteRequest`)
    // splits it into IP (v4/v6) and port directly in the session state.
    let paste = egui::Button::new(super::icons::icon_text(
        super::icons::PASTE, t.paste_address, 15.0, Palette::TEXT));
    let paste_resp = ui.add_enabled(!editing_open, paste).on_hover_cursor(egui::CursorIcon::PointingHand);
    if focus == ManualFocus::Paste { paste_resp.clone().highlight(); }
    if paste_resp.clicked() {
        *outcome = LanScreenOutcome::RequestPaste;
    } else if paste_resp.hovered() {
        *hovered = Some(ManualFocus::Paste);
    }

    // --- Validation and connection target (single source: `manual_connect`) ---
    // Format feedback: malformed address (v4 or v6), then port out of range. `is_valid`
    // accepts empty input, so only values that are filled in **and** invalid are reported.
    if !ManualField::Ipv4.is_valid(&manual.ipv4) || !ManualField::Ipv6.is_valid(&manual.ipv6) {
        ui.label(egui::RichText::new(t.invalid_address).color(Palette::DANGER).size(12.0));
    }
    if !manual.port.trim().is_empty() && manual.effective_port().is_none() {
        ui.label(egui::RichText::new(t.invalid_port).color(Palette::DANGER).size(12.0));
    }
    // Connection attempt error (e.g. "your own listening address"). Shown HERE, in the dialog:
    // an `Error` phase would be hidden behind it.
    if let Some(err) = error {
        ui.label(egui::RichText::new(err).color(Palette::DANGER).size(12.0));
    }

    ui.add_space(8.0);
    ui.add(egui::Separator::default().spacing(4.0));
    ui.add_space(8.0);

    // Connect button plus a permanent listening indicator. The TCP listener stays bound while
    // the network screen is open (see `OpenLanScreen`), so a spinner and a "Listening" label
    // remind the user that an incoming connection can arrive at any time.
    let target = manual.connect_target();
    let can_connect = target.is_some() && !editing_open;
    let mut connect_clicked = false;
    ui.horizontal(|ui| {
        let connect_resp = ui.add_enabled(
            can_connect,
            egui::Button::new(egui::RichText::new(format!("  {}  ", t.connect)).color(Palette::TEXT).size(15.0)),
        ).on_hover_cursor(egui::CursorIcon::PointingHand);
        if focus == ManualFocus::Connect { connect_resp.clone().highlight(); }
        connect_clicked = connect_resp.clicked();
        if connect_resp.hovered() { *hovered = Some(ManualFocus::Connect); }
        ui.add_space(12.0);
        ui.spinner();
        ui.add_space(4.0);
        ui.label(egui::RichText::new(t.listening_incoming).color(Palette::TEXT_DIM).size(12.0));
    });
    if connect_clicked {
        if let Some((addr, port)) = target {
            *outcome = LanScreenOutcome::ManualConnect { addr, port };
        }
    }
}

fn render_peer_list(
    ui: &mut egui::Ui,
    peers: &[crate::app::meta_state::LanPeer],
    selected: &mut usize,
    active: bool,
    outcome: &mut LanScreenOutcome,
) {
    ui.label(egui::RichText::new(crate::app::locale::t().players_on_lan)
        .size(13.0).color(Palette::TEXT_DIM));
    ui.add_space(4.0);

    // Keyboard and gamepad navigation goes through `route_lan_intent`. Here only click/touch
    // (select and activate) and focus rendering. Index `peers.len()` (Back) is an element of
    // the **footer**: the selection is clamped to that range without forcing it onto a peer,
    // otherwise the footer would be unreachable (notably with an empty list). While the list
    // is in the background of an invite (inactive), the selection is left alone (it is the
    // modal's focus).
    if active {
        *selected = (*selected).min(peers.len());
    }

    if peers.is_empty() {
        searching_indicator(ui, egui::RichText::new(crate::app::locale::t().looking_for_players)
            .color(Palette::TEXT_DIM));
    } else {
        // Mouse: peer index hovered this frame (moves the shared selection after the loop).
        let mut hovered_now: Option<usize> = None;
        // Render rows. When inactive (list behind an invite), no highlight and no click.
        for (i, peer) in peers.iter().enumerate() {
            let is_selected = active && i == *selected;
            let available = peer.status == PeerStatus::Available;
            // Unavailable suffix: "busy" (negotiating) vs "in game" (playing).
            let suffix = match peer.status {
                PeerStatus::Available => "",
                PeerStatus::Busy => crate::app::locale::t().busy_suffix,
                PeerStatus::InGame => crate::app::locale::t().in_game_suffix,
            };

            // Clickable (hand cursor, click, hover to focus) only if the list is active AND
            // the peer is **available**; otherwise the row is informational.
            let resp = peer_row(ui, &peer.name, peer.announced_rating, is_selected,
                active && available, available, suffix);
            if active && available {
                if resp.clicked() {
                    *selected = i;
                    *outcome = LanScreenOutcome::PeerClicked {
                        addr: peer.addr,
                        port: peer.port,
                    };
                } else if resp.hovered() {
                    hovered_now = Some(i);
                }
            }
        }

        // A hover transition moves the selection shared by keyboard and mouse.
        if active {
            if let Some(i) = super::helpers::hover_select_transition(ui.ctx(), "lan_peer_hover", hovered_now) {
                *selected = i;
            }
        }

        searching_indicator(ui, egui::RichText::new(crate::app::locale::t().scanning)
            .size(12.0).color(Palette::TEXT_DIM));
    }
}

/// Search indicator: a **centered** spinner with its label **below**, with a small margin
/// above and below. The color comes from the caller's `label`.
fn searching_indicator(ui: &mut egui::Ui, label: egui::RichText) {
    ui.add_space(8.0);
    ui.vertical_centered(|ui| {
        ui.spinner();
        ui.add_space(4.0);
        ui.label(label);
    });
    ui.add_space(8.0);
}

/// Row for one peer, in the `ListFlush` style (2 px, border invisible at rest: stable row
/// height, no jitter, same as the menu and options; see `RowGauge`).
///
/// `clickable` = list active AND peer available (click and hand cursor). `available` only
/// drives the text color (a busy peer stays readable in the background).
fn peer_row(
    ui: &mut egui::Ui,
    name: &str,
    announced_rating: Option<i32>,
    selected: bool,
    clickable: bool,
    available: bool,
    suffix: &str,
) -> egui::Response {
    let label = if suffix.is_empty() {
        name.to_string()
    } else {
        format!("{} {}", name, suffix)
    };

    super::helpers::selectable_row(
        ui, selected, clickable, super::helpers::RowGauge::ListFlush, 340.0,
        |ui, _conventional| {
            // Own color rule (not the 3-case convention): an unfocused peer stays in full
            // `TEXT` for list readability, and only an unavailable peer is dimmed.
            let text_color = if available { Palette::TEXT } else { Palette::TEXT_UNAVAILABLE };
            ui.label(egui::RichText::new(&label).color(text_color).size(15.0));
            // Self-announced rank: league chip, shown the same way as on the network rank and
            // opponents pages.
            if let Some(rt) = announced_rating {
                let cfg = crackattack_rating::RatingConfig::default();
                let lg = crate::app::ladder_view::league_of_value(rt as f64, &cfg);
                super::rank_page::league_chip(ui, lg, Some(rt), 12.0);
            }
        })
}

