//! Multiband compressor control panel.
//!
//! Top row: master on + three crossover frequency knobs.
//! Bottom row: four per-band groups, each with enable, threshold,
//! ratio, and gain knobs.

use wayland_plugin_gui::egui;

use crate::params::MultibandParams;
use crate::stages::multiband::NUM_BANDS;

use super::theme;
use super::widgets;

pub fn draw(ui: &mut egui::Ui, params: &MultibandParams) {
    ui.vertical(|ui| {
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.add_space(12.0);
            ui.label(
                egui::RichText::new("Multiband")
                    .strong()
                    .size(14.0)
                    .color(theme::ACCENT),
            );
            ui.add_space(16.0);
            widgets::bool_checkbox(ui, &params.on, "Enabled");
            ui.add_space(16.0);
            ui.label(
                egui::RichText::new("Crossovers:")
                    .size(11.0)
                    .color(theme::TEXT_DIM),
            );
            ui.add_space(4.0);

            widgets::float_knob(ui, &params.xo1, "LO/LM", "");
            widgets::float_knob(ui, &params.xo2, "LM/HM", "");
            widgets::float_knob(ui, &params.xo3, "HM/HI", "");
        });
        ui.add_space(6.0);

        ui.horizontal(|ui| {
            ui.add_space(8.0);
            for i in 0..NUM_BANDS {
                let band = &params.bands[i];
                let title = match i {
                    0 => "Low",
                    1 => "Low-Mid",
                    2 => "High-Mid",
                    _ => "High",
                };

                ui.vertical(|ui| {
                    ui.horizontal(|ui| {
                        ui.label(
                            egui::RichText::new(title)
                                .strong()
                                .size(11.0)
                                .color(theme::TEXT),
                        );
                        widgets::bool_checkbox(ui, &band.on, "On");
                    });
                    ui.horizontal(|ui| {
                        widgets::float_knob(ui, &band.threshold, "Threshold", "");
                        widgets::float_knob(ui, &band.ratio, "Ratio", "");
                        widgets::float_knob(ui, &band.gain, "Gain", "");
                    });
                });
                if i + 1 < NUM_BANDS {
                    ui.add_space(8.0);
                }
            }
        });
    });
}
