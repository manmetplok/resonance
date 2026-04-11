//! Parametric EQ control panel — used for both the corrective and
//! tonal EQ stages. Lays out the four bands as four compact columns
//! across the panel, each with enable / type / freq / Q / gain.

use wayland_plugin_gui::egui;

use crate::params::EqStageParams;
use crate::stages::linear_phase_eq::NUM_BANDS;

use super::theme;
use super::widgets;

const TYPE_LABELS: &[&str] = &["Bell", "Low Shelf", "High Shelf", "HPF", "LPF"];

pub fn draw(ui: &mut egui::Ui, params: &EqStageParams, title: &str) {
    ui.vertical(|ui| {
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.add_space(12.0);
            ui.label(
                egui::RichText::new(title)
                    .strong()
                    .size(14.0)
                    .color(theme::ACCENT),
            );
        });
        ui.add_space(4.0);

        ui.horizontal(|ui| {
            ui.add_space(8.0);
            for i in 0..NUM_BANDS {
                let band = &params.bands[i];
                let col_title = format!("Band {}", i + 1);
                widgets::control_column(ui, &col_title, "", |ui| {
                    widgets::bool_checkbox(ui, &band.on, "On");
                    widgets::int_combo(
                        ui,
                        &band.band_type,
                        &format!("{}_type_{i}", title),
                        TYPE_LABELS,
                    );
                    ui.label(egui::RichText::new("Freq").size(9.0).color(theme::TEXT_DIM));
                    widgets::float_slider_log(ui, &band.freq, 20.0..=20_000.0, 0, " Hz");
                    ui.label(egui::RichText::new("Q").size(9.0).color(theme::TEXT_DIM));
                    widgets::float_slider_log(ui, &band.q, 0.1..=24.0, 2, "");
                    ui.label(egui::RichText::new("Gain").size(9.0).color(theme::TEXT_DIM));
                    widgets::float_slider(ui, &band.gain, -24.0..=24.0, 1, " dB");
                });
            }
        });
    });
}
