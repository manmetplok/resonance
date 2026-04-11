//! True-peak limiter control panel.

use wayland_plugin_gui::egui;

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
            widgets::control_column(ui, "On", "enable", |ui| {
                widgets::bool_checkbox(ui, &params.on, "");
            });
            widgets::control_column(ui, "Ceiling", "dBTP", |ui| {
                widgets::float_slider(ui, &params.ceiling, -6.0..=0.0, 1, " dB");
            });
            widgets::control_column(ui, "Release", "", |ui| {
                widgets::float_slider_log(ui, &params.release, 5.0..=500.0, 0, " ms");
            });
        });

        ui.add_space(8.0);
        ui.horizontal(|ui| {
            ui.add_space(16.0);
            ui.label(
                egui::RichText::new(
                    "Lookahead: 5 ms · 4× oversampled ITU-R BS.1770-4 true-peak detection",
                )
                .size(10.0)
                .color(theme::TEXT_DIM),
            );
        });
    });
}
