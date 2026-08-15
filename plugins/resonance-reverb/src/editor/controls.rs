//! The control strip at the bottom of the reverb editor.
//! Uses rotary knobs in a grid layout for compact, consistent display.
//!
//! Every knob is built straight from its `FloatParam` (ba todo #1281):
//! range, default, skew and the value readout come from `params.rs`, so
//! the four knobs that had drifted from their own declarations cannot
//! drift again.

use resonance_plugin::editor_widgets;
use wayland_plugin_gui::egui;

use crate::params::ReverbParams;

use super::theme;

pub fn draw(ui: &mut egui::Ui, params: &ReverbParams) {
    ui.vertical(|ui| {
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.add_space(12.0);
            ui.label(
                egui::RichText::new("Reverb Controls")
                    .strong()
                    .size(13.0)
                    .color(theme::ACCENT),
            );
        });
        ui.add_space(4.0);

        // Two rows of 6 knobs each.
        ui.horizontal(|ui| {
            ui.add_space(8.0);
            editor_widgets::float_knob(ui, &params.predelay, "Pre-delay", "before tail");
            ui.add_space(4.0);
            editor_widgets::float_knob(ui, &params.er_level, "ER Level", "early refl.");
            ui.add_space(4.0);
            editor_widgets::float_knob(ui, &params.er_time, "ER Time", "tap spread");
            ui.add_space(4.0);
            editor_widgets::float_knob(ui, &params.size, "Size", "");
            ui.add_space(4.0);
            editor_widgets::float_knob(ui, &params.decay, "Decay", "RT60");
            ui.add_space(4.0);
            editor_widgets::float_knob(ui, &params.damping, "Damping", "HF cutoff");
        });

        ui.add_space(2.0);

        ui.horizontal(|ui| {
            ui.add_space(8.0);
            editor_widgets::float_knob(ui, &params.diffusion, "Diffusion", "");
            ui.add_space(4.0);
            editor_widgets::float_knob(ui, &params.mod_rate, "Mod Rate", "chorus");
            ui.add_space(4.0);
            editor_widgets::float_knob(ui, &params.mod_depth, "Mod Depth", "");
            ui.add_space(4.0);
            editor_widgets::float_knob(ui, &params.width, "Width", "stereo");
            ui.add_space(4.0);
            editor_widgets::float_knob(ui, &params.mix, "Mix", "dry/wet");
            ui.add_space(4.0);

            // Freeze is a toggle, not a knob — render as a checkbox.
            ui.vertical(|ui| {
                ui.add_space(16.0);
                editor_widgets::bool_checkbox(ui, &params.freeze, "Freeze");
            });
        });
    });
}
