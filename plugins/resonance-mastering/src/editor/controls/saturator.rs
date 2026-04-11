//! Saturator control panel.

use wayland_plugin_gui::egui;

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
            widgets::control_column(ui, "On", "enable", |ui| {
                widgets::bool_checkbox(ui, &params.on, "");
            });
            widgets::control_column(ui, "Shaper", "smooth ↔ gritty", |ui| {
                widgets::int_combo(
                    ui,
                    &params.shaper,
                    "sat_shaper_combo",
                    &["Smooth", "Gritty"],
                );
            });
            widgets::control_column(ui, "Drive", "", |ui| {
                widgets::float_slider(ui, &params.drive, 0.0..=18.0, 1, " dB");
            });
            widgets::control_column(ui, "Character", "tube ↔ tape", |ui| {
                widgets::float_slider(ui, &params.character, 0.0..=1.0, 2, "");
            });
            widgets::control_column(ui, "Mix", "", |ui| {
                widgets::float_slider(ui, &params.mix, 0.0..=1.0, 2, "");
            });
        });
    });
}
