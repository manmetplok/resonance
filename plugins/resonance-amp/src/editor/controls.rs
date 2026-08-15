//! Bottom control strip: Input Gain + Output Gain knobs.
//!
//! Both knobs are built straight from their `FloatParam` (ba todo
//! #1281): range, default, skew and the dB readout come from
//! `params.rs`, which is also where the gain→dB formatter lives.

use resonance_plugin::editor_widgets;
use wayland_plugin_gui::egui;

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
            editor_widgets::float_knob(ui, &params.input_gain, "Input Gain", "pre-model");
            ui.add_space(8.0);
            editor_widgets::float_knob(ui, &params.output_gain, "Output Gain", "post-model");
        });
    });
}
