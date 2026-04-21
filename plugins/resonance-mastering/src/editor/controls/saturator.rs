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

            let v = params.drive.value();
            widgets::float_knob(
                ui, &params.drive, 0.0..=18.0, 3.0,
                "Drive", "", &format!("{:.1} dB", v), false,
            );

            let v = params.character.value();
            widgets::float_knob(
                ui, &params.character, 0.0..=1.0, 0.3,
                "Character", "tube \u{2194} tape", &format!("{:.0}%", v * 100.0), false,
            );

            let v = params.mix.value();
            widgets::float_knob(
                ui, &params.mix, 0.0..=1.0, 1.0,
                "Mix", "", &format!("{:.0}%", v * 100.0), false,
            );
        });
    });
}
