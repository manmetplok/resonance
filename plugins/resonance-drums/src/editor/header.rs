//! Top header bar widget: plugin name + master volume slider.

use wayland_plugin_gui::egui;

use crate::params::DrumParams;

use super::theme;

/// Draw the editor header row.
pub fn draw(ui: &mut egui::Ui, params: &DrumParams) {
    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new("RESONANCE DRUMS")
                .strong()
                .color(theme::ACCENT),
        );
        ui.add_space(12.0);
        ui.separator();
        ui.add_space(8.0);
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
    ui.separator();
}
