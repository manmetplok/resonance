//! Bottom control strip with knob cluster + toggles for the compressor.

use wayland_plugin_gui::egui;

use super::app::CompressorEditorApp;

pub(crate) fn draw_control_strip(ui: &mut egui::Ui, app: &mut CompressorEditorApp) {
    use resonance_plugin::editor_widgets;

    ui.add_space(4.0);
    // Row 1: core dynamics controls.
    ui.horizontal(|ui| {
        ui.add_space(8.0);
        editor_widgets::float_knob(
            ui,
            &app.params.threshold,
            -60.0..=0.0,
            -20.0,
            "Threshold",
            "",
            &format!("{:.1} dB", app.params.threshold.value()),
            false,
        );
        ui.add_space(4.0);
        editor_widgets::float_knob(
            ui,
            &app.params.ratio,
            1.0..=20.0,
            4.0,
            "Ratio",
            "",
            &format!("{:.1}:1", app.params.ratio.value()),
            true,
        );
        ui.add_space(4.0);
        editor_widgets::float_knob(
            ui,
            &app.params.attack,
            0.1..=200.0,
            10.0,
            "Attack",
            "",
            &format!("{:.1} ms", app.params.attack.value()),
            true,
        );
        ui.add_space(4.0);
        editor_widgets::float_knob(
            ui,
            &app.params.release,
            5.0..=2000.0,
            100.0,
            "Release",
            "",
            &format!("{:.0} ms", app.params.release.value()),
            true,
        );
        ui.add_space(4.0);
        editor_widgets::float_knob(
            ui,
            &app.params.knee,
            0.0..=12.0,
            3.0,
            "Knee",
            "",
            &format!("{:.1} dB", app.params.knee.value()),
            false,
        );
        ui.add_space(4.0);
        editor_widgets::float_knob(
            ui,
            &app.params.makeup,
            -12.0..=24.0,
            0.0,
            "Makeup",
            "",
            &format!("{:.1} dB", app.params.makeup.value()),
            false,
        );
        ui.add_space(4.0);
        editor_widgets::float_knob(
            ui,
            &app.params.mix,
            0.0..=1.0,
            1.0,
            "Mix",
            "dry/wet",
            &format!("{:.0}%", app.params.mix.value() * 100.0),
            false,
        );
        ui.add_space(4.0);
        editor_widgets::float_knob(
            ui,
            &app.params.detector_mix,
            0.0..=1.0,
            0.0,
            "Detector",
            "Peak/RMS",
            &format!("{:.2}", app.params.detector_mix.value()),
            false,
        );
        ui.add_space(4.0);
        editor_widgets::float_knob(
            ui,
            &app.params.sc_hpf_freq,
            20.0..=500.0,
            80.0,
            "SC HPF",
            "",
            &format!("{:.0} Hz", app.params.sc_hpf_freq.value()),
            true,
        );
        ui.add_space(8.0);
        // Toggles.
        ui.vertical(|ui| {
            ui.add_space(16.0);
            editor_widgets::bool_checkbox(ui, &app.params.auto_makeup, "Auto Gain");
            editor_widgets::bool_checkbox(ui, &app.params.sc_hpf_on, "SC HPF On");
        });
    });
}
