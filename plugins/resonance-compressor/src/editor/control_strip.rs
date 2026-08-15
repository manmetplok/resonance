//! Bottom control strip with knob cluster + toggles for the compressor.
//!
//! Every knob is built straight from its `FloatParam` (ba todo #1281):
//! range, default, skew and the value readout come from `params.rs`.

use wayland_plugin_gui::egui;

use super::app::CompressorEditorApp;

pub(crate) fn draw_control_strip(ui: &mut egui::Ui, app: &mut CompressorEditorApp) {
    use resonance_plugin::editor_widgets;

    ui.add_space(4.0);
    // Row 1: core dynamics controls.
    ui.horizontal(|ui| {
        ui.add_space(8.0);
        editor_widgets::float_knob(ui, &app.params.threshold, "Threshold", "");
        ui.add_space(4.0);
        editor_widgets::float_knob(ui, &app.params.ratio, "Ratio", "");
        ui.add_space(4.0);
        editor_widgets::float_knob(ui, &app.params.attack, "Attack", "");
        ui.add_space(4.0);
        editor_widgets::float_knob(ui, &app.params.release, "Release", "");
        ui.add_space(4.0);
        editor_widgets::float_knob(ui, &app.params.knee, "Knee", "");
        ui.add_space(4.0);
        editor_widgets::float_knob(ui, &app.params.makeup, "Makeup", "");
        ui.add_space(4.0);
        editor_widgets::float_knob(ui, &app.params.mix, "Mix", "dry/wet");
        ui.add_space(4.0);
        editor_widgets::float_knob(ui, &app.params.detector_mix, "Detector", "Peak/RMS");
        ui.add_space(4.0);
        editor_widgets::float_knob(ui, &app.params.sc_hpf_freq, "SC HPF", "");
        ui.add_space(8.0);
        // Toggles.
        ui.vertical(|ui| {
            ui.add_space(16.0);
            editor_widgets::bool_checkbox(ui, &app.params.auto_makeup, "Auto Gain");
            editor_widgets::bool_checkbox(ui, &app.params.sc_hpf_on, "SC HPF On");
        });
    });
}
