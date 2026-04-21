//! Parametric EQ control panel — used for both the corrective and
//! tonal EQ stages. Lays out the four bands as horizontal rows of
//! knobs, each with enable / type / freq / Q / gain.

use resonance_plugin::Param;
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

        for i in 0..NUM_BANDS {
            let band = &params.bands[i];
            ui.horizontal(|ui| {
                ui.add_space(8.0);

                ui.vertical(|ui| {
                    ui.add_space(8.0);
                    ui.label(
                        egui::RichText::new(format!("Band {}", i + 1))
                            .strong()
                            .size(11.0)
                            .color(theme::TEXT),
                    );
                    widgets::bool_checkbox(ui, &band.on, "On");
                    widgets::int_combo(
                        ui,
                        &band.band_type,
                        &format!("{}_type_{i}", title),
                        TYPE_LABELS,
                    );
                });
                ui.add_space(8.0);

                let v = band.freq.value();
                let def = band.freq.default_plain() as f32;
                widgets::float_knob(
                    ui, &band.freq, 20.0..=20_000.0, def,
                    "Freq", "", &format!("{:.0} Hz", v), true,
                );

                let v = band.q.value();
                let def = band.q.default_plain() as f32;
                widgets::float_knob(
                    ui, &band.q, 0.1..=24.0, def,
                    "Q", "", &format!("{:.2}", v), true,
                );

                let v = band.gain.value();
                let def = band.gain.default_plain() as f32;
                widgets::float_knob(
                    ui, &band.gain, -24.0..=24.0, def,
                    "Gain", "", &format!("{:.1} dB", v), false,
                );
            });
            if i + 1 < NUM_BANDS {
                ui.add_space(2.0);
            }
        }
    });
}
