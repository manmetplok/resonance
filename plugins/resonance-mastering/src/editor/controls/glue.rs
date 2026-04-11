//! Glue compressor control panel.

use wayland_plugin_gui::egui;

use crate::params::GlueCompressorParams;

use super::theme;
use super::widgets;

pub fn draw(ui: &mut egui::Ui, params: &GlueCompressorParams) {
    ui.vertical(|ui| {
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.add_space(12.0);
            ui.label(
                egui::RichText::new("Glue Compressor")
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
            widgets::control_column(ui, "Threshold", "", |ui| {
                widgets::float_slider(ui, &params.threshold, -40.0..=0.0, 1, " dB");
            });
            widgets::control_column(ui, "Ratio", "", |ui| {
                widgets::float_slider(ui, &params.ratio, 1.0..=8.0, 1, ":1");
            });
            widgets::control_column(ui, "Attack", "", |ui| {
                widgets::float_slider_log(ui, &params.attack, 1.0..=200.0, 1, " ms");
            });
            widgets::control_column(ui, "Release", "", |ui| {
                widgets::float_slider_log(ui, &params.release, 10.0..=1000.0, 0, " ms");
            });
            widgets::control_column(ui, "Knee", "", |ui| {
                widgets::float_slider(ui, &params.knee, 0.0..=12.0, 1, " dB");
            });
            widgets::control_column(ui, "Makeup", "", |ui| {
                widgets::float_slider(ui, &params.makeup, -6.0..=12.0, 1, " dB");
            });
            widgets::control_column(ui, "Mix", "parallel", |ui| {
                widgets::float_slider(ui, &params.mix, 0.0..=1.0, 2, "");
            });
        });
    });
}
