//! Bottom control strip: Input Gain + Output Gain knobs.

use wayland_plugin_gui::egui;
use wayland_plugin_gui::widgets;

use crate::params::AmpParams;

use super::theme;

pub fn draw(ui: &mut egui::Ui, params: &AmpParams) {
    ui.vertical(|ui| {
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.add_space(12.0);
            ui.label(
                egui::RichText::new("Amp Controls")
                    .strong()
                    .size(13.0)
                    .color(theme::ACCENT),
            );
        });
        ui.add_space(6.0);

        ui.horizontal(|ui| {
            ui.add_space(12.0);
            let in_db = 20.0 * params.input_gain.value().log10();
            widgets::float_knob(
                ui,
                &params.input_gain,
                0.01..=4.0,
                1.0,
                "Input Gain",
                "pre-model",
                &format!("{in_db:+.1} dB"),
                true,
            );
            ui.add_space(8.0);
            let out_db = 20.0 * params.output_gain.value().log10();
            widgets::float_knob(
                ui,
                &params.output_gain,
                0.001..=4.0,
                1.0,
                "Output Gain",
                "post-model",
                &format!("{out_db:+.1} dB"),
                true,
            );
        });
    });
}
