//! Bottom control strip: Dry/Wet mix and Output Gain knobs.
//!
//! Both knobs are built straight from their `FloatParam` (ba todo
//! #1281/#1284): range, default, skew, unit, readout *and* caption all
//! come off `params.rs`. Nothing here restates a fact about a parameter
//! — the only caller-supplied strings are the sub-captions, which
//! describe this layout cell ("post-convolver"), not the parameter.
//!
//! Before the migration this file passed its own `0.0..=1.0` / `0.5`
//! for the mix (the param declares a default of 1.0, fully wet), its
//! own `0.1..=10.0` / `1.0` plus a hardcoded logarithmic arc for the
//! output gain (the param declares `FloatRange::Skewed` with
//! `gain_skew_factor(-20, 20)`), and hand-rolled `format!` readouts
//! that bypassed the params' own formatters. `tests/editor_bindings.rs`
//! pins the result so a control cannot drift from its parameter again.

use resonance_plugin::{editor_widgets, Param};
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
                params.dry_wet.name(),
                "convolution mix",
            );
            ui.add_space(8.0);
            editor_widgets::float_knob(
                ui,
                &params.output_gain,
                params.output_gain.name(),
                "post-convolver",
            );
        });
    });
}
