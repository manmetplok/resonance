//! Top header bar widget: plugin name + master volume slider.

use wayland_plugin_gui::egui;

use crate::params::DrumParams;

use super::theme;

/// Draw the editor header row. Title sits on the left, the master volume
/// slider is right-aligned so it always lands at the edge of the editor
/// regardless of window width — previously the slider grew/shrank with
/// the available middle space which made the title-to-slider distance
/// jitter as the user resized.
pub fn draw(ui: &mut egui::Ui, params: &DrumParams) {
    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new("RESONANCE DRUMS")
                .strong()
                .color(theme::ACCENT)
                .size(theme::TITLE_SIZE),
        );

        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let master = &params.master_volume;
            let mut v = master.value();
            let resp = ui.add(
                egui::Slider::new(&mut v, 0.0..=1.0)
                    .text("Master")
                    .custom_formatter(|x, _| format!("{:.2}", x)),
            );
            if resp.changed() {
                master.set_value(v);
            }
        });
    });
    ui.add_space(4.0);
    ui.separator();
}
