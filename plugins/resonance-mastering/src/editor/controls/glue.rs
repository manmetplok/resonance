//! Glue compressor control panel.

use plugin_gui_core::egui;

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
            widgets::bool_checkbox(ui, &params.on, "On");
            ui.add_space(8.0);

            widgets::float_knob(ui, &params.threshold, "Threshold", "");
            widgets::float_knob(ui, &params.ratio, "Ratio", "");
            widgets::float_knob(ui, &params.attack, "Attack", "");
            widgets::float_knob(ui, &params.release, "Release", "");
            widgets::float_knob(ui, &params.knee, "Knee", "");
            widgets::float_knob(ui, &params.makeup, "Makeup", "");
            widgets::float_knob(ui, &params.mix, "Mix", "parallel");
        });
    });
}
