//! The control strip at the bottom of the reverb editor.
//! Uses rotary knobs in a grid layout for compact, consistent display.

use wayland_plugin_gui::egui;
use wayland_plugin_gui::widgets;

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
            widgets::float_knob(
                ui,
                &params.predelay,
                0.0..=250.0,
                0.0,
                "Pre-delay",
                "before tail",
                &format!("{:.0} ms", params.predelay.value()),
                false,
            );
            ui.add_space(4.0);
            widgets::float_knob(
                ui,
                &params.er_level,
                0.0..=1.0,
                0.5,
                "ER Level",
                "early refl.",
                &format!("{:.2}", params.er_level.value()),
                false,
            );
            ui.add_space(4.0);
            widgets::float_knob(
                ui,
                &params.er_time,
                0.0..=1.0,
                0.5,
                "ER Time",
                "tap spread",
                &format!("{:.2}", params.er_time.value()),
                false,
            );
            ui.add_space(4.0);
            widgets::float_knob(
                ui,
                &params.size,
                0.0..=1.0,
                0.5,
                "Size",
                "",
                &format!("{:.0}%", params.size.value() * 100.0),
                false,
            );
            ui.add_space(4.0);
            widgets::float_knob(
                ui,
                &params.decay,
                0.1..=30.0,
                2.0,
                "Decay",
                "RT60",
                &format!("{:.2} s", params.decay.value()),
                true,
            );
            ui.add_space(4.0);
            widgets::float_knob(
                ui,
                &params.damping,
                200.0..=20000.0,
                8000.0,
                "Damping",
                "HF cutoff",
                &format!("{:.0} Hz", params.damping.value()),
                true,
            );
        });

        ui.add_space(2.0);

        ui.horizontal(|ui| {
            ui.add_space(8.0);
            widgets::float_knob(
                ui,
                &params.diffusion,
                0.0..=1.0,
                0.7,
                "Diffusion",
                "",
                &format!("{:.2}", params.diffusion.value()),
                false,
            );
            ui.add_space(4.0);
            widgets::float_knob(
                ui,
                &params.mod_rate,
                0.0..=5.0,
                0.5,
                "Mod Rate",
                "chorus",
                &format!("{:.2} Hz", params.mod_rate.value()),
                false,
            );
            ui.add_space(4.0);
            widgets::float_knob(
                ui,
                &params.mod_depth,
                0.0..=1.0,
                0.3,
                "Mod Depth",
                "",
                &format!("{:.2}", params.mod_depth.value()),
                false,
            );
            ui.add_space(4.0);
            widgets::float_knob(
                ui,
                &params.width,
                0.0..=1.0,
                1.0,
                "Width",
                "stereo",
                &format!("{:.0}%", params.width.value() * 100.0),
                false,
            );
            ui.add_space(4.0);
            widgets::float_knob(
                ui,
                &params.mix,
                0.0..=1.0,
                0.3,
                "Mix",
                "dry/wet",
                &format!("{:.0}%", params.mix.value() * 100.0),
                false,
            );
            ui.add_space(4.0);

            // Freeze is a toggle, not a knob — render as a checkbox.
            ui.vertical(|ui| {
                ui.add_space(16.0);
                widgets::bool_checkbox(ui, &params.freeze, "Freeze");
            });
        });
    });
}
