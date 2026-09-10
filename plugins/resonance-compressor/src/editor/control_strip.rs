//! Bottom control strip with knob cluster + toggles for the compressor.
//!
//! Every control here is built straight from its parameter (ba todos
//! #1281/#1282): range, default, skew, unit and the value readout all
//! come off the `FloatParam`, and the captions come off `Param::name()`.
//! Nothing in this file restates a fact that `params.rs` declares, which
//! is the point — the previous call sites passed their own range,
//! default and `format!` readout, and four of them had drifted from the
//! parameter they edited (see `tests/editor_param_binding.rs`).
//!
//! The only caller-supplied strings left are the *sub*-labels, which are
//! captions for a cell in this layout ("dry/wet", "Peak/RMS") rather
//! than facts about the parameter.

use plugin_gui_core::egui;

use super::app::CompressorEditorApp;

pub(crate) fn draw_control_strip(ui: &mut egui::Ui, app: &mut CompressorEditorApp) {
    use resonance_plugin::editor_widgets;
    use resonance_plugin::Param;

    let p = &app.params;

    ui.add_space(4.0);
    // Row 1: core dynamics controls.
    ui.horizontal(|ui| {
        ui.add_space(8.0);
        editor_widgets::float_knob(ui, &p.threshold, p.threshold.name(), "");
        ui.add_space(4.0);
        editor_widgets::float_knob(ui, &p.ratio, p.ratio.name(), "");
        ui.add_space(4.0);
        editor_widgets::float_knob(ui, &p.attack, p.attack.name(), "");
        ui.add_space(4.0);
        editor_widgets::float_knob(ui, &p.release, p.release.name(), "");
        ui.add_space(4.0);
        editor_widgets::float_knob(ui, &p.knee, p.knee.name(), "");
        ui.add_space(4.0);
        editor_widgets::float_knob(ui, &p.makeup, p.makeup.name(), "");
        ui.add_space(4.0);
        editor_widgets::float_knob(ui, &p.mix, p.mix.name(), "dry/wet");
        ui.add_space(4.0);
        editor_widgets::float_knob(ui, &p.detector_mix, p.detector_mix.name(), "Peak/RMS");
        ui.add_space(4.0);
        editor_widgets::float_knob(ui, &p.sc_hpf_freq, p.sc_hpf_freq.name(), "");
        ui.add_space(8.0);
        // Toggles.
        ui.vertical(|ui| {
            ui.add_space(16.0);
            editor_widgets::bool_checkbox(ui, &p.auto_makeup, p.auto_makeup.name());
            editor_widgets::bool_checkbox(ui, &p.sc_hpf_on, p.sc_hpf_on.name());
        });
    });
}
