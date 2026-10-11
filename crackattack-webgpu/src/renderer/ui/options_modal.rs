//! Options modal egui UI — rendering only, no session mutations.

use crate::app::meta_state::{NameInputState, OptionsTab};
use crate::platform::persist::BindingAction;
use crate::renderer::ui::onscreen_keyboard::render_onscreen_keyboard;

/// Result of rendering the Options modal for one frame.
pub enum ModalOutcome {
    /// Modal still open. Preview name may have changed.
    StillOpen {
        name_preview: Option<String>,
        music_volume_preview: Option<f32>,
        music_enabled_preview: Option<bool>,
        /// Widget to focus after a mouse hover transition (absolute focus via `OptionsNavFocus`).
        /// The persistent modal state lives in the session and rendering gets a copy, so focus
        /// changes go through a command, like keyboard navigation.
        nav_focus_request: Option<crate::app::options_nav::OptWidget>,
    },
    /// Leaving the options (live model): apply the settings, save and close.
    Confirmed {
        player_name: String,
        sfx_enabled: bool,
        music_enabled: bool,
        sfx_volume: f32,
        music_volume: f32,
        language: Option<String>,
        fullscreen: bool,
        key_bindings: crate::platform::persist::KeyBindings,
    },
}

/// Human-readable display name for a key name string.
/// Delegates to the keycodes module via a function pointer set at init.
/// Falls back to the raw name if no display function is registered.
static KEY_DISPLAY_FN: std::sync::OnceLock<fn(&str) -> &str> = std::sync::OnceLock::new();

/// Register the display name function (called once from main.rs).
pub fn set_key_display_fn(f: fn(&str) -> &str) {
    let _ = KEY_DISPLAY_FN.set(f);
}

fn display_key(name: &str) -> &str {
    KEY_DISPLAY_FN.get().map_or(name, |f| f(name))
}

/// Tints the handle of the focused slider with the accent color, leaving the rail alone.
/// `Response::highlight()` is not visible on an egui slider, and the handle and rail share
/// the theme `bg_fill`. So the handle outline (`fg_stroke`, unused by the rail) gets the
/// accent. This does not affect layout.
fn with_focus_tint(ui: &mut egui::Ui, focused: bool, add: impl FnOnce(&mut egui::Ui)) {
    if !focused {
        add(ui);
        return;
    }
    ui.scope(|ui| {
        use crate::renderer::ui::egui_theme::Palette;
        let accent = egui::Stroke::new(2.0, Palette::TEXT_ACCENT);
        let w = &mut ui.visuals_mut().widgets;
        w.inactive.fg_stroke = accent; // idle handle (slider not hovered)
        w.hovered.fg_stroke = accent;
        w.active.fg_stroke = accent;
        add(ui);
    });
}

/// Render the Options modal inside an egui context.
pub fn render_options_modal(
    ctx: &egui::Context,
    modal: &mut NameInputState,
) -> ModalOutcome {
    let mut should_close = false;
    let mut name_changed = false;
    let mut confirmed_name = String::new();
    let mut music_vol_changed = false;
    let mut music_enabled_changed = false;
    // Mouse hover on a footer button (transition): absolute focus, sent up as a command.
    let mut nav_focus_request: Option<crate::app::options_nav::OptWidget> = None;

    // Keyboard and gamepad navigation both go through `dispatch_input` ->
    // `route_options_intent` (a single pipeline); egui does not read navigation here.
    // The name text goes through the on-screen keyboard (`render_onscreen_keyboard`), which
    // collects egui `Event::Text` when `keyboard_goes_to_egui()` routes the keyboard to egui.
    // There is no egui `TextEdit`. Selection and the keyboard are changed through the
    // `AppCommand`s returned by `route_options_intent`.

    // Exit request (Back / Escape / B / Backspace), set by the input routing and consumed
    // here so it shares the Back button logic. Live model: apply, save and close.
    let force_ok = std::mem::take(&mut modal.confirm_requested);

    let t = crate::app::locale::t();

    // The section is chosen from the Options submenu of the main menu (Player / Sound /
    // Controls). The title shows the section.
    let section_title = match modal.tab {
        OptionsTab::Player => t.player_tab,
        OptionsTab::Sound => t.sound_tab,
        OptionsTab::Keyboard => t.keyboard_tab,
        OptionsTab::Gamepad => t.gamepad_tab,
    };

    egui::Window::new(section_title)
        .id(egui::Id::new("options_modal"))
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .resizable(false).collapsible(false)
        .show(ctx, |ui| {
            ui.set_min_width(320.0);

            match modal.tab {
                OptionsTab::Player => {
                    use crate::app::options_nav::OptWidget;
                    use super::egui_theme::Palette;
                    let fw = modal.focused_widget();
                    // Two-column grid (label | control) keeps labels and controls aligned.
                    egui::Grid::new("general_grid")
                        .num_columns(2)
                        .spacing([16.0, 12.0])
                        .show(ui, |ui| {
                            // Player: <name> [Edit]. The name is a displayed value, not a
                            // field; editing goes through the focusable button.
                            ui.label(t.player_name);
                            ui.horizontal(|ui| {
                                let name = if modal.text.trim().is_empty() { "—" } else { modal.text.as_str() };
                                ui.label(egui::RichText::new(name).strong().color(Palette::TEXT_ACCENT));
                                ui.add_space(8.0);
                                let edit = ui.button(t.edit_name).on_hover_cursor(egui::CursorIcon::PointingHand);
                                if edit.clicked() { modal.kb_open(); }
                                if fw == Some(OptWidget::EditName) { edit.highlight(); }
                            });
                            ui.end_row();

                            // Language: [combo], built from `Language::ALL` (Auto + languages).
                            ui.label(t.language_label);
                            let current_label = match modal.language_choice {
                                None => t.language_auto,
                                Some(l) => l.display_name(),
                            };
                            let combo = egui::ComboBox::from_id_salt("lang_combo")
                                .selected_text(current_label)
                                .show_ui(ui, |ui| {
                                    ui.selectable_value(&mut modal.language_choice, None, t.language_auto);
                                    for &lang in crate::app::locale::Language::ALL {
                                        ui.selectable_value(&mut modal.language_choice, Some(lang), lang.display_name());
                                    }
                                });
                            if fw == Some(OptWidget::Language) { combo.response.highlight(); }
                            ui.end_row();

                            // Fullscreen: [checkbox] (label in the left column).
                            ui.label(t.fullscreen);
                            let cb = ui.checkbox(&mut modal.fullscreen, "");
                            if fw == Some(OptWidget::Fullscreen) { cb.highlight(); }
                            ui.end_row();
                        });
                }
                OptionsTab::Sound => {
                    use crate::app::options_nav::OptWidget;
                    let fw = modal.focused_widget();

                    // Sound effects block: toggle (header) + indented volume.
                    let cb_sfx = ui.checkbox(&mut modal.sfx_enabled, t.sound_effects);
                    if fw == Some(OptWidget::SfxEnabled) { cb_sfx.highlight(); }
                    // `highlight()` is not visible on an egui slider, so tint the outline.
                    let sfx_pct = format!("{}%", (modal.sfx_volume * 100.0).round() as i32);
                    ui.indent("sfx_block", |ui| {
                        with_focus_tint(ui, fw == Some(OptWidget::SfxVolume), |ui| {
                            ui.add_enabled(
                                modal.sfx_enabled,
                                egui::Slider::new(&mut modal.sfx_volume, 0.0..=1.0)
                                    .show_value(false)
                                    .text(sfx_pct),
                            );
                        });
                    });

                    // Music block: toggle (header) + indented volume. Hidden when no music
                    // track is shipped.
                    if modal.music_available {
                        // Clear gap between the two blocks.
                        ui.add_space(16.0);

                        let prev_music_enabled = modal.music_enabled;
                        let cb_mus = ui.checkbox(&mut modal.music_enabled, t.music);
                        if fw == Some(OptWidget::MusicEnabled) { cb_mus.highlight(); }
                        if modal.music_enabled != prev_music_enabled {
                            music_enabled_changed = true;
                        }
                        let prev_music_vol = modal.music_volume;
                        let music_pct = format!("{}%", (modal.music_volume * 100.0).round() as i32);
                        ui.indent("music_block", |ui| {
                            with_focus_tint(ui, fw == Some(OptWidget::MusicVolume), |ui| {
                                ui.add_enabled(
                                    modal.music_enabled,
                                    egui::Slider::new(&mut modal.music_volume, 0.0..=1.0)
                                        .show_value(false)
                                        .text(music_pct),
                                );
                            });
                        });
                        if (modal.music_volume - prev_music_vol).abs() > f32::EPSILON {
                            music_vol_changed = true;
                        }
                    }
                }
                OptionsTab::Keyboard => render_keyboard_tab(ui, modal),
                OptionsTab::Gamepad => render_gamepad_tab(ui, modal),
            }

            // Footer (live model): Back (icon + word) applies, saves and closes. On the
            // Keyboard and Gamepad screens, Reset is on the same row, on the right.
            // There is no global confirm or cancel.
            ui.add_space(8.0);
            ui.separator();
            ui.add_space(6.0);
            {
                use crate::app::options_nav::OptWidget;
                use super::egui_theme::Palette;
                let fw = modal.focused_widget();
                let show_reset = matches!(modal.tab, OptionsTab::Keyboard | OptionsTab::Gamepad);
                ui.horizontal(|ui| {
                    // Back (arrow-left icon + word), on the left.
                    let back_focus = fw == Some(OptWidget::Back);
                    let col = if back_focus { Palette::TEXT_ACCENT } else { Palette::TEXT_DIM };
                    let i = ui
                        .add(egui::Label::new(super::icons::icon(super::icons::BACK).size(18.0).color(col)).sense(egui::Sense::click()))
                        .on_hover_cursor(egui::CursorIcon::PointingHand);
                    let l = ui
                        .add(egui::Label::new(egui::RichText::new(t.menu_back).size(15.0).color(col)).sense(egui::Sense::click()))
                        .on_hover_cursor(egui::CursorIcon::PointingHand);
                    let back_hovered = i.hovered() || l.hovered();
                    if i.clicked() || l.clicked() || force_ok {
                        modal.capture_binding = None;
                        // Trim the name; `finalize` falls back to the default name if empty.
                        confirmed_name = modal.format.finalize(&modal.text);
                        name_changed = true;
                        should_close = true;
                    }
                    // Hover transition on Back focuses it (same as the main menu). Sent up as a
                    // command, because the persistent state lives in the session, not in this copy.
                    if super::helpers::hover_select_transition(ctx, "options_back_hover", back_hovered.then_some(0)).is_some() {
                        nav_focus_request = Some(OptWidget::Back);
                    }
                    // Reset, on the right, Keyboard and Gamepad screens only.
                    if show_reset {
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            let reset_focus = fw == Some(OptWidget::ResetBindings);
                            let rc = if reset_focus { Palette::TEXT_ACCENT } else { Palette::TEXT_DIM };
                            // Layout is right-to-left: add the word first (rightmost), then the
                            // icon. Visual result: [rotate-ccw icon] [Reset].
                            let l = ui
                                .add(egui::Label::new(egui::RichText::new(t.reset_defaults).size(15.0).color(rc)).sense(egui::Sense::click()))
                                .on_hover_cursor(egui::CursorIcon::PointingHand);
                            let i = ui
                                .add(egui::Label::new(super::icons::icon(super::icons::ROTATE_CCW).size(18.0).color(rc)).sense(egui::Sense::click()))
                                .on_hover_cursor(egui::CursorIcon::PointingHand);
                            let reset_hovered = i.hovered() || l.hovered();
                            if i.clicked() || l.clicked() {
                                modal.key_bindings = crate::platform::persist::KeyBindings::default();
                                modal.capture_binding = None;
                            }
                            // Hover transition focuses Reset (sent up as a command).
                            if super::helpers::hover_select_transition(ctx, "options_reset_hover", reset_hovered.then_some(0)).is_some() {
                                nav_focus_request = Some(OptWidget::ResetBindings);
                            }
                        });
                    }
                });
            }
        });

    // On-screen keyboard, drawn over the modal when open.
    if modal.keyboard_open {
        if render_onscreen_keyboard(ctx, modal) {
            name_changed = true;
        }
    }

    if should_close {
        // Live model: leaving always applies the settings (no cancel). `confirmed_name`
        // falls back to the default name, so it is never empty.
        let language = modal.language_choice.map(|l| l.code().to_string());
        let mut bindings = modal.key_bindings.clone();
        bindings.normalize();
        ModalOutcome::Confirmed {
            player_name: confirmed_name,
            sfx_enabled: modal.sfx_enabled,
            music_enabled: modal.music_enabled,
            sfx_volume: modal.sfx_volume,
            music_volume: modal.music_volume,
            language,
            fullscreen: modal.fullscreen,
            key_bindings: bindings,
        }
    } else {
        let music_preview = if music_vol_changed { Some(modal.music_volume) } else { None };
        let enabled_preview = if music_enabled_changed { Some(modal.music_enabled) } else { None };
        let name_preview = if name_changed { Some(modal.text.clone()) } else { None };
        ModalOutcome::StillOpen { name_preview, music_volume_preview: music_preview, music_enabled_preview: enabled_preview, nav_focus_request }
    }
}

/// Capture button for one binding slot (keyboard or gamepad).
/// Shows the current key name and arms the capture on click.
fn binding_slot_button(
    ui: &mut egui::Ui,
    modal: &mut NameInputState,
    action: BindingAction,
    slot: usize,
    is_capturing: bool,
    focused: bool,
) {
    let is_this_capturing = modal.capture_binding == Some((action, slot));
    let label = if is_this_capturing {
        crate::app::locale::t().press_key_or_button.to_string()
    } else if let Some(name) = modal.key_bindings.get(action).get(slot) {
        display_key(name).to_string()
    } else {
        "—".to_string()
    };
    let btn = egui::Button::new(&label).min_size(egui::vec2(70.0, 0.0));
    // While a capture is in progress, only the slot being captured stays enabled.
    let enabled = !is_capturing || is_this_capturing;
    let mut resp = ui.add_enabled(enabled, btn).on_hover_cursor(egui::CursorIcon::PointingHand);
    if focused {
        resp = resp.highlight();
    }
    if resp.clicked() && !is_this_capturing {
        modal.capture_binding = Some((action, slot));
    }
}

/// Subgroup header: strong label + separator line.
fn subgroup(ui: &mut egui::Ui, txt: &str) {
    ui.label(egui::RichText::new(txt).strong());
    ui.separator();
}

/// Small "press a key" hint shown while a binding is being captured.
fn capture_hint(ui: &mut egui::Ui, is_capturing: bool) {
    if is_capturing {
        ui.add_space(4.0);
        ui.label(egui::RichText::new(crate::app::prompts::cancel_capture_hint()).small().weak());
    }
}

/// Keyboard screen: per-key bindings in two subgroups, Movement and Actions.
/// The Reset control is in the footer.
fn render_keyboard_tab(ui: &mut egui::Ui, modal: &mut NameInputState) {
    use crate::app::options_nav::OptWidget;
    let is_capturing = modal.capture_binding.is_some();
    let focused = modal.focused_widget();
    let slot_focused = |a: BindingAction, s: usize| focused == Some(OptWidget::BindingSlot(a, s));

    subgroup(ui, crate::app::locale::t().ctrl_movement);
    egui::Grid::new("kbd_dirs")
        .num_columns(3).min_col_width(80.0).spacing([8.0, 4.0])
        .show(ui, |ui| {
            for action in [BindingAction::Up, BindingAction::Down, BindingAction::Left, BindingAction::Right] {
                ui.label(crate::app::locale::binding_action_label(crate::app::locale::t(), action));
                binding_slot_button(ui, modal, action, 0, is_capturing, slot_focused(action, 0));
                ui.label("");
                ui.end_row();
            }
        });
    ui.add_space(8.0);
    subgroup(ui, crate::app::locale::t().ctrl_actions);
    egui::Grid::new("kbd_actions")
        .num_columns(3).min_col_width(80.0).spacing([8.0, 4.0])
        .show(ui, |ui| {
            for action in [BindingAction::Swap, BindingAction::Raise, BindingAction::Pause] {
                ui.label(crate::app::locale::binding_action_label(crate::app::locale::t(), action));
                binding_slot_button(ui, modal, action, 0, is_capturing, slot_focused(action, 0));
                ui.label("");
                ui.end_row();
            }
        });
    capture_hint(ui, is_capturing);
}

/// Gamepad screen: direction mode + Swap/Raise (gamepad slot).
fn render_gamepad_tab(ui: &mut egui::Ui, modal: &mut NameInputState) {
    use crate::app::options_nav::OptWidget;
    use crate::platform::persist::DirMode;
    let is_capturing = modal.capture_binding.is_some();
    let focused = modal.focused_widget();
    let slot_focused = |a: BindingAction, s: usize| focused == Some(OptWidget::BindingSlot(a, s));

    subgroup(ui, crate::app::locale::t().ctrl_directions);
    let dm_focus = focused == Some(OptWidget::DirMode);
    ui.horizontal(|ui| {
        for (mode, label) in [
            (DirMode::DPad, crate::app::locale::t().dir_dpad),
            (DirMode::Stick, crate::app::locale::t().dir_stick),
            (DirMode::Both, crate::app::locale::t().dir_both),
        ] {
            let selected = modal.key_bindings.dir_mode == mode;
            let mut r = ui.selectable_value(&mut modal.key_bindings.dir_mode, mode, label);
            // Focus uses the same `.highlight()` as the binding buttons, so the color matches.
            if dm_focus && selected {
                r = r.highlight();
            }
        }
    });
    ui.add_space(8.0);
    subgroup(ui, crate::app::locale::t().ctrl_actions);
    egui::Grid::new("pad_actions")
        .num_columns(2).min_col_width(80.0).spacing([8.0, 4.0])
        .show(ui, |ui| {
            for action in [BindingAction::Swap, BindingAction::Raise] {
                ui.label(crate::app::locale::binding_action_label(crate::app::locale::t(), action));
                binding_slot_button(ui, modal, action, 1, is_capturing, slot_focused(action, 1));
                ui.end_row();
            }
        });
    capture_hint(ui, is_capturing);
}
