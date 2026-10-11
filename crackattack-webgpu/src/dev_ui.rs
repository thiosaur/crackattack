//! Dev panel UI rendering (egui). Only compiled under `--features dev`.

#[cfg(feature = "dev")]
pub(crate) fn render_dev_tuning_window(
    ctx: &egui::Context,
    panel: &mut crate::dev_tuning::DevPanelState,
) {
    egui::Window::new("Dev (`)")
        .default_pos([10.0, 10.0])
        .resizable(true)
        .collapsible(true)
        .movable(true)
        .default_width(280.0)
        .show(ctx, |ui| {
            let t = &mut panel.tuning;

            ui.checkbox(&mut t.freeze_simulation, "Freeze simulation");
            ui.separator();

            ui.label(egui::RichText::new("Drop garbage").strong());
            ui.horizontal(|ui| {
                ui.label("Type:");
                ui.radio_value(&mut t.debug_drop_garbage_flavor, 0, "Rouge");
                ui.radio_value(&mut t.debug_drop_garbage_flavor, 1, "Gris");
            });
            ui.horizontal(|ui| {
                if ui.button("1").clicked() { t.debug_drop_garbage_height = 1; }
                if ui.button("2").clicked() { t.debug_drop_garbage_height = 2; }
                if ui.button("3").clicked() { t.debug_drop_garbage_height = 3; }
                if ui.button("4").clicked() { t.debug_drop_garbage_height = 4; }
                if ui.button("6").clicked() { t.debug_drop_garbage_height = 6; }
            });

            ui.separator();
            ui.label(egui::RichText::new("Press ` (backquote) to toggle.").small().weak());
        });
}
