//! Clipper control panel.

use plugin_gui_core::egui;

use crate::params::ClipperParams;

use super::theme;
use super::widgets;

pub fn draw(ui: &mut egui::Ui, params: &ClipperParams) {
    ui.vertical(|ui| {
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.add_space(12.0);
            ui.label(
                egui::RichText::new("Clipper")
                    .strong()
                    .size(14.0)
                    .color(theme::ACCENT),
            );
        });
        ui.add_space(4.0);

        ui.horizontal(|ui| {
            ui.add_space(8.0);
            widgets::bool_checkbox(ui, &params.on, "On");
            ui.add_space(8.0);

            widgets::float_knob(ui, &params.drive, "Drive", "dB shaved off peaks");
            widgets::float_knob(ui, &params.shape, "Shape", "hard \u{2194} soft");
        });

        ui.add_space(8.0);
        ui.horizontal(|ui| {
            ui.add_space(16.0);
            ui.label(
                egui::RichText::new(
                    "Before the limiter \u{00b7} 8\u{00d7} IIR oversampled, no added latency \u{00b7} level-matched below the knee",
                )
                .size(10.0)
                .color(theme::TEXT_DIM),
            );
        });
    });
}
