//! Saturator control panel.

use plugin_gui_core::egui;

use crate::params::SaturatorParams;

use super::theme;
use super::widgets;

pub fn draw(ui: &mut egui::Ui, params: &SaturatorParams) {
    ui.vertical(|ui| {
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.add_space(12.0);
            ui.label(
                egui::RichText::new("Saturator")
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

            ui.vertical(|ui| {
                ui.label(
                    egui::RichText::new("Shaper")
                        .size(10.0)
                        .color(theme::TEXT_DIM),
                );
                widgets::int_combo(
                    ui,
                    &params.shaper,
                    "sat_shaper_combo",
                    &["Smooth", "Gritty"],
                );
            });
            ui.add_space(8.0);

            widgets::float_knob(ui, &params.drive, "Drive", "");
            widgets::float_knob(ui, &params.character, "Character", "tube \u{2194} tape");
            widgets::float_knob(ui, &params.mix, "Mix", "");
        });
    });
}
