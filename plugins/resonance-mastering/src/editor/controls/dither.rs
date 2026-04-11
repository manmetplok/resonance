//! Dither control panel.

use wayland_plugin_gui::egui;

use crate::params::DitherParams;

use super::theme;
use super::widgets;

const BIT_LABELS: &[&str] = &[
    "16-bit", "17-bit", "18-bit", "19-bit", "20-bit", "21-bit", "22-bit", "23-bit", "24-bit",
];

pub fn draw(ui: &mut egui::Ui, params: &DitherParams) {
    ui.vertical(|ui| {
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.add_space(12.0);
            ui.label(
                egui::RichText::new("Dither")
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
            widgets::control_column(ui, "Target", "bit depth", |ui| {
                widgets::int_combo(ui, &params.target_bits, "dith_bits_combo", BIT_LABELS);
            });
            widgets::control_column(ui, "Noise Shape", "HF tilt", |ui| {
                widgets::bool_checkbox(ui, &params.noise_shape, "");
            });
        });

        ui.add_space(8.0);
        ui.horizontal(|ui| {
            ui.add_space(16.0);
            ui.label(
                egui::RichText::new(
                    "TPDF dither with optional 1st-order high-pass shaping. \
                     Plugin output stays 32-bit float; the dither sits ahead of \
                     the host's export-time quantization.",
                )
                .size(10.0)
                .color(theme::TEXT_DIM),
            );
        });
    });
}
