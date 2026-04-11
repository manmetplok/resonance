//! Bottom control strip: Dry/Wet mix and Output Gain. Matches the
//! amp / reverb control-column aesthetic so all plugins look alike.

use resonance_plugin::FloatParam;
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
            control_column(ui, "Dry / Wet", "convolution mix", |ui| {
                percent_slider(ui, &params.dry_wet);
            });
            control_column(ui, "Output Gain", "post-convolver", |ui| {
                gain_slider(ui, &params.output_gain, 0.1..=10.0);
            });
        });
    });
}

const COL_WIDTH: f32 = 220.0;

fn control_column<R>(
    ui: &mut egui::Ui,
    label: &str,
    sub: &str,
    contents: impl FnOnce(&mut egui::Ui) -> R,
) -> R {
    let mut out = None;
    egui::Frame::group(ui.style())
        .fill(theme::PANEL_LIGHT)
        .stroke(egui::Stroke::new(1.0, theme::BORDER))
        .inner_margin(egui::Margin::same(8))
        .show(ui, |ui| {
            ui.vertical(|ui| {
                ui.set_min_width(COL_WIDTH);
                ui.set_max_width(COL_WIDTH);
                ui.spacing_mut().slider_width = COL_WIDTH - 20.0;
                ui.label(
                    egui::RichText::new(label)
                        .strong()
                        .size(12.0)
                        .color(theme::TEXT),
                );
                if !sub.is_empty() {
                    ui.label(
                        egui::RichText::new(sub)
                            .size(10.0)
                            .color(theme::TEXT_DIM),
                    );
                }
                ui.add_space(4.0);
                out = Some(contents(ui));
            });
        });
    ui.add_space(8.0);
    out.expect("control_column content closure always runs")
}

fn percent_slider(ui: &mut egui::Ui, param: &FloatParam) {
    let mut v = param.value();
    let slider = egui::Slider::new(&mut v, 0.0..=1.0)
        .show_value(true)
        .custom_formatter(|x, _| format!("{:.0}%", x * 100.0));
    if ui.add(slider).changed() {
        param.set_value(v);
    }
}

fn gain_slider(
    ui: &mut egui::Ui,
    param: &FloatParam,
    range: std::ops::RangeInclusive<f32>,
) {
    let mut v = param.value();
    let slider = egui::Slider::new(&mut v, range)
        .logarithmic(true)
        .show_value(true)
        .custom_formatter(|x, _| {
            let db = 20.0 * (x as f32).log10();
            format!("{db:+.1} dB")
        });
    if ui.add(slider).changed() {
        param.set_value(v);
    }
}
