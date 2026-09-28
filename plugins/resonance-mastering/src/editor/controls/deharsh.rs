//! De-harsh (resonance suppressor) control panel.

use plugin_gui_core::egui;

use crate::params::deharsh::MODE_LABELS;
use crate::params::DeharshParams;

use super::theme;
use super::widgets;

pub fn draw(ui: &mut egui::Ui, params: &DeharshParams) {
    ui.vertical(|ui| {
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.add_space(12.0);
            ui.label(
                egui::RichText::new("De-harsh")
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
                widgets::int_combo(ui, &params.mode, "dh_mode_combo", MODE_LABELS);
                ui.add_space(4.0);
                widgets::bool_checkbox(ui, &params.delta, "Delta");
            });
            ui.add_space(8.0);

            widgets::float_knob(ui, &params.depth, "Depth", "max cut");
            widgets::float_knob(ui, &params.selectivity, "Selectivity", "dB over ref");
            widgets::float_knob(ui, &params.sharpness, "Sharpness", "cut Q");
            widgets::float_knob(ui, &params.attack, "Attack", "");
            widgets::float_knob(ui, &params.release, "Release", "");
            widgets::float_knob(ui, &params.low, "Low", "band");
            widgets::float_knob(ui, &params.high, "High", "band");
            widgets::float_knob(ui, &params.mix, "Mix", "");
        });

        ui.add_space(8.0);
        ui.horizontal(|ui| {
            ui.add_space(16.0);
            ui.label(
                egui::RichText::new(
                    "Cuts peaks that stand Selectivity dB above the smoothed spectrum, by at most Depth \u{00b7} Delta plays only what is removed \u{00b7} 43 ms latency, on or off",
                )
                .size(10.0)
                .color(theme::TEXT_DIM),
            );
        });
    });
}
