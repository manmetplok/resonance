//! Bottom control strip: Dry/Wet mix and Output Gain knobs.

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
            editor_widgets::float_knob(
                ui,
                &params.dry_wet,
                0.0..=1.0,
                0.5,
                "Dry / Wet",
                "convolution mix",
                &format!("{:.0}%", params.dry_wet.value() * 100.0),
                false,
            );
            ui.add_space(8.0);
            let out_db = 20.0 * params.output_gain.value().log10();
            editor_widgets::float_knob(
                ui,
                &params.output_gain,
                0.1..=10.0,
                1.0,
                "Output Gain",
                "post-convolver",
                &format!("{out_db:+.1} dB"),
                true,
            );
        });
    });
}
