//! Glue compressor control panel.

use wayland_plugin_gui::egui;

use crate::params::GlueCompressorParams;

use super::theme;
use super::widgets;

pub fn draw(ui: &mut egui::Ui, params: &GlueCompressorParams) {
    ui.vertical(|ui| {
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.add_space(12.0);
            ui.label(
                egui::RichText::new("Glue Compressor")
                    .strong()
                    .size(14.0)
                    .color(theme::ACCENT),
            );
        });
        ui.add_space(4.0);

        ui.horizontal(|ui| {
            ui.add_space(8.0);
            widgets::bool_checkbox(ui, &params.on, "On");
            ui.add_space(8.0);

            let v = params.threshold.value();
            widgets::float_knob(
                ui,
                &params.threshold,
                -40.0..=0.0,
                -18.0,
                "Threshold",
                "",
                &format!("{:.1} dB", v),
                false,
            );

            let v = params.ratio.value();
            widgets::float_knob(
                ui,
                &params.ratio,
                1.0..=8.0,
                2.0,
                "Ratio",
                "",
                &format!("{:.1}:1", v),
                false,
            );

            let v = params.attack.value();
            widgets::float_knob(
                ui,
                &params.attack,
                1.0..=200.0,
                30.0,
                "Attack",
                "",
                &format!("{:.1} ms", v),
                true,
            );

            let v = params.release.value();
            widgets::float_knob(
                ui,
                &params.release,
                10.0..=1000.0,
                150.0,
                "Release",
                "",
                &format!("{:.0} ms", v),
                true,
            );

            let v = params.knee.value();
            widgets::float_knob(
                ui,
                &params.knee,
                0.0..=12.0,
                6.0,
                "Knee",
                "",
                &format!("{:.1} dB", v),
                false,
            );

            let v = params.makeup.value();
            widgets::float_knob(
                ui,
                &params.makeup,
                -6.0..=12.0,
                0.0,
                "Makeup",
                "",
                &format!("{:.1} dB", v),
                false,
            );

            let v = params.mix.value();
            widgets::float_knob(
                ui,
                &params.mix,
                0.0..=1.0,
                1.0,
                "Mix",
                "parallel",
                &format!("{:.0}%", v * 100.0),
                false,
            );
        });
    });
}
