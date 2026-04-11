//! Multiband compressor control panel.
//!
//! Top row: master on + three crossover frequencies.
//! Bottom row: four per-band columns, each with enable, threshold,
//! ratio, and gain.

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
            ui.label(egui::RichText::new("Crossovers:").size(11.0).color(theme::TEXT_DIM));
            ui.add_space(4.0);
            crossover_slider(ui, &params.xo1, "LO/LM", 40.0..=400.0);
            crossover_slider(ui, &params.xo2, "LM/HM", 250.0..=2500.0);
            crossover_slider(ui, &params.xo3, "HM/HI", 1500.0..=10_000.0);
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
                widgets::control_column(ui, title, "", |ui| {
                    widgets::bool_checkbox(ui, &band.on, "On");
                    ui.label(egui::RichText::new("Threshold").size(9.0).color(theme::TEXT_DIM));
                    widgets::float_slider(ui, &band.threshold, -40.0..=0.0, 1, " dB");
                    ui.label(egui::RichText::new("Ratio").size(9.0).color(theme::TEXT_DIM));
                    widgets::float_slider(ui, &band.ratio, 1.0..=8.0, 1, ":1");
                    ui.label(egui::RichText::new("Gain").size(9.0).color(theme::TEXT_DIM));
                    widgets::float_slider(ui, &band.gain, -12.0..=12.0, 1, " dB");
                });
            }
        });
    });
}

fn crossover_slider(
    ui: &mut egui::Ui,
    param: &resonance_plugin::FloatParam,
    label: &str,
    range: std::ops::RangeInclusive<f32>,
) {
    ui.vertical(|ui| {
        ui.label(egui::RichText::new(label).size(9.0).color(theme::TEXT_DIM));
        ui.spacing_mut().slider_width = 120.0;
        widgets::float_slider_log(ui, param, range, 0, " Hz");
    });
    ui.add_space(6.0);
}
