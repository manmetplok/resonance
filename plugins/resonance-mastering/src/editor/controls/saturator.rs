//! Saturator control panel.
//!
//! Character and Shaper belong to the Blend mode and Curve to the
//! Inflator; each is greyed out in the modes it does nothing in, so the
//! panel never offers a knob that silently has no effect.

use plugin_gui_core::egui;

use crate::params::SaturatorParams;
use crate::stages::saturator::SatMode;

use super::theme;
use super::widgets;

pub fn draw(ui: &mut egui::Ui, params: &SaturatorParams) {
    let mode = SatMode::from_index(params.mode.value());
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
                    egui::RichText::new("Mode")
                        .size(10.0)
                        .color(theme::TEXT_DIM),
                );
                widgets::int_combo(ui, &params.mode, "sat_mode_combo", SatMode::LABELS);
                ui.label(
                    egui::RichText::new("Shaper")
                        .size(10.0)
                        .color(theme::TEXT_DIM),
                );
                ui.add_enabled_ui(mode == SatMode::Blend, |ui| {
                    widgets::int_combo(
                        ui,
                        &params.shaper,
                        "sat_shaper_combo",
                        &["Smooth", "Gritty"],
                    );
                });
            });
            ui.add_space(8.0);

            widgets::float_knob(ui, &params.drive, "Drive", "");
            ui.add_enabled_ui(mode == SatMode::Blend, |ui| {
                widgets::float_knob(ui, &params.character, "Character", "tube \u{2194} tape");
            });
            ui.add_enabled_ui(mode == SatMode::Inflator, |ui| {
                widgets::float_knob(ui, &params.curve, "Curve", "inflator");
            });
            widgets::float_knob(ui, &params.mix, "Mix", "");
            ui.add_space(8.0);
            widgets::bool_checkbox(ui, &params.auto_gain, "Auto Gain");
        });
    });
}
