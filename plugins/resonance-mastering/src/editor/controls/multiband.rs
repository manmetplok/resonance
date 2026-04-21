//! Multiband compressor control panel.
//!
//! Top row: master on + three crossover frequency knobs.
//! Bottom row: four per-band groups, each with enable, threshold,
//! ratio, and gain knobs.

use resonance_plugin::Param;
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

            let v = params.xo1.value();
            let def = params.xo1.default_plain() as f32;
            widgets::float_knob(
                ui,
                &params.xo1,
                40.0..=400.0,
                def,
                "LO/LM",
                "",
                &format!("{:.0} Hz", v),
                true,
            );

            let v = params.xo2.value();
            let def = params.xo2.default_plain() as f32;
            widgets::float_knob(
                ui,
                &params.xo2,
                250.0..=2500.0,
                def,
                "LM/HM",
                "",
                &format!("{:.0} Hz", v),
                true,
            );

            let v = params.xo3.value();
            let def = params.xo3.default_plain() as f32;
            widgets::float_knob(
                ui,
                &params.xo3,
                1500.0..=10_000.0,
                def,
                "HM/HI",
                "",
                &format!("{:.0} Hz", v),
                true,
            );
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
                        let v = band.threshold.value();
                        let def = band.threshold.default_plain() as f32;
                        widgets::float_knob(
                            ui,
                            &band.threshold,
                            -40.0..=0.0,
                            def,
                            "Threshold",
                            "",
                            &format!("{:.1} dB", v),
                            false,
                        );

                        let v = band.ratio.value();
                        let def = band.ratio.default_plain() as f32;
                        widgets::float_knob(
                            ui,
                            &band.ratio,
                            1.0..=8.0,
                            def,
                            "Ratio",
                            "",
                            &format!("{:.1}:1", v),
                            false,
                        );

                        let v = band.gain.value();
                        let def = band.gain.default_plain() as f32;
                        widgets::float_knob(
                            ui,
                            &band.gain,
                            -12.0..=12.0,
                            def,
                            "Gain",
                            "",
                            &format!("{:.1} dB", v),
                            false,
                        );
                    });
                });
                if i + 1 < NUM_BANDS {
                    ui.add_space(8.0);
                }
            }
        });
    });
}
