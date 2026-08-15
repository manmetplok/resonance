//! Bottom control strip: Input Gain + Output Gain knobs.
//!
//! Both knobs are built straight from their `FloatParam` (ba todo
//! #1281): range, default, skew and the dB readout come from
//! `params.rs`, which is also where the gain→dB formatter lives. The
//! only caller-supplied arguments left are the two captions, which are
//! facts about this layout cell rather than about the parameter.
//!
//! What that fixed here (ba todo #1283, audit finding F4):
//!
//! * **Output Gain's default.** The call site reset to `1.0` while
//!   `params.rs` declares — and every fresh instance runs — `0.5`, so
//!   double-clicking the knob moved the amp +6 dB off its own default.
//!   The declaration wins; `tests/editor_param_binding.rs` pins it.
//! * **Both arcs.** Each call passed `logarithmic: true`, so neither
//!   declared `FloatRange::Skewed` reached a control. The dial now
//!   follows the parameter's own curve.
//!
//! `tests/editor_param_binding.rs` is the guard: it reads every source
//! file in this directory and fails if a knob appears anywhere else, or
//! if a call site starts restating what `params.rs` declares.

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
