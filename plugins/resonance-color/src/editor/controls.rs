//! Bottom control strip: one control per parameter, each built from the
//! parameter itself.
//!
//! Range, default, skew, unit and readout come off the `FloatParam`;
//! captions come off `Param::name()`; the choice rows list the
//! parameter's own choice table. `tests/editor_param_binding.rs` reads
//! this file and fails if a parameter has no control, has two, or has a
//! call site that restates something `params.rs` declares.
//!
//! Controls a mode does not use stay visible but are greyed out
//! (`add_enabled_ui`): Bias in Console; Speed, Flutter and Tape Quality
//! outside Tape; Solver outside Tape HQ. In Tape HQ the Bias knob is tape
//! bias (the hysteresis loop's reversible fraction), not asymmetry.

use plugin_gui_core::{egui, widgets};
use resonance_plugin::{editor_widgets, IntParam, Param};

use crate::params::{ColorParams, Mode, TapeQuality};

use super::theme;

/// Segmented selector bound to a choice `IntParam`: one segment per
/// declared label, so it offers exactly the values the parameter holds.
fn choice_segmented(ui: &mut egui::Ui, param: &IntParam) {
    ui.vertical(|ui| {
        ui.label(
            egui::RichText::new(param.name())
                .size(10.0)
                .color(theme::TEXT_DIM),
        );
        editor_widgets::choice_segmented(ui, param, &widgets::SegmentedStyle::LAVENDER);
    });
}

pub(crate) fn draw_control_strip(ui: &mut egui::Ui, p: &ColorParams) {
    let mode = p.mode();

    ui.add_space(4.0);
    // Row 1: the switches.
    ui.horizontal(|ui| {
        ui.add_space(8.0);
        choice_segmented(ui, &p.mode);
        ui.add_space(12.0);
        choice_segmented(ui, &p.oversample);
        ui.add_space(12.0);
        ui.add_enabled_ui(mode == Mode::Tape, |ui| {
            choice_segmented(ui, &p.speed);
        });
        ui.add_space(12.0);
        ui.vertical(|ui| {
            ui.add_space(14.0);
            editor_widgets::bool_checkbox(ui, &p.auto_gain, p.auto_gain.name());
        });
    });

    ui.add_space(2.0);
    // Row 2: the knobs, in signal order.
    ui.horizontal(|ui| {
        ui.add_space(8.0);
        editor_widgets::float_knob(ui, &p.drive, p.drive.name(), "");
        ui.add_space(4.0);
        ui.add_enabled_ui(mode.uses_bias(), |ui| {
            editor_widgets::float_knob(ui, &p.bias, p.bias.name(), "asymmetry / HQ bias");
        });
        ui.add_space(4.0);
        editor_widgets::float_knob(ui, &p.response, p.response.name(), "−highs / +lows");
        ui.add_space(4.0);
        editor_widgets::float_knob(ui, &p.tone, p.tone.name(), "tilt");
        ui.add_space(4.0);
        editor_widgets::float_knob(ui, &p.mix, p.mix.name(), "dry/wet");
        ui.add_space(4.0);
        editor_widgets::float_knob(ui, &p.output, p.output.name(), "");
        ui.add_space(16.0);
        ui.add_enabled_ui(mode == Mode::Tape, |ui| {
            editor_widgets::float_knob(ui, &p.flutter, p.flutter.name(), "wow/flutter");
        });
        ui.add_space(12.0);
        // The tape quality pair, beside the other Tape-only controls:
        // HQ swaps the curve for the hysteresis stage, and the solver
        // only means something there.
        ui.vertical(|ui| {
            ui.add_enabled_ui(mode == Mode::Tape, |ui| {
                choice_segmented(ui, &p.tape_quality);
            });
            ui.add_space(4.0);
            let hq = mode == Mode::Tape && p.tape_quality() == TapeQuality::Hq;
            ui.add_enabled_ui(hq, |ui| {
                choice_segmented(ui, &p.tape_solver);
            });
        });
    });
}
