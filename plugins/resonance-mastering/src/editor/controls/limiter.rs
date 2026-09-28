//! True-peak limiter control panel.

use plugin_gui_core::egui;

use crate::params::LimiterParams;

use super::theme;
use super::widgets;

pub fn draw(ui: &mut egui::Ui, params: &LimiterParams) {
    ui.vertical(|ui| {
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.add_space(12.0);
            ui.label(
                egui::RichText::new("True-Peak Limiter")
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

            widgets::float_knob(ui, &params.gain, "Gain", "push into the limiter");
            widgets::float_knob(ui, &params.ceiling, "Ceiling", "dBTP");
            widgets::float_knob(ui, &params.release, "Release", "");
        });

        ui.add_space(8.0);
        ui.horizontal(|ui| {
            ui.add_space(16.0);
            ui.label(
                egui::RichText::new(
                    "Lookahead: 5 ms \u{00b7} 4\u{00d7} oversampled ITU-R BS.1770-4 true-peak detection",
                )
                .size(10.0)
                .color(theme::TEXT_DIM),
            );
        });
    });
}
