//! Bottom control strip: Dry/Wet mix and Output Gain knobs.
//!
//! Both knobs are built straight from their `FloatParam` (ba todo
//! #1281): range, default, skew and the readout come from `params.rs`.

use resonance_plugin::editor_widgets;
use wayland_plugin_gui::egui;

use crate::params::IrParams;

use super::theme;

pub fn draw(ui: &mut egui::Ui, params: &IrParams) {
    ui.vertical(|ui| {
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.add_space(12.0);
            ui.label(
                egui::RichText::new("IR Controls")
                    .strong()
                    .size(13.0)
                    .color(theme::ACCENT),
            );
        });
        ui.add_space(6.0);

        ui.horizontal(|ui| {
            ui.add_space(12.0);
            editor_widgets::float_knob(ui, &params.dry_wet, "Dry / Wet", "convolution mix");
            ui.add_space(8.0);
            editor_widgets::float_knob(ui, &params.output_gain, "Output Gain", "post-convolver");
        });
    });
}
