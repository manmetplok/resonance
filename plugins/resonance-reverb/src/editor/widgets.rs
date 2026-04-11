//! Shared helpers for the reverb control strip. Same pattern the
//! mastering and compressor editors use — each helper takes the atomic
//! param directly, reads its value, pushes the widget, and commits the
//! new value back if it changed.

use resonance_plugin::{BoolParam, FloatParam};
use wayland_plugin_gui::egui;

use super::theme;

/// Standard per-control column width.
pub const COL_WIDTH: f32 = 104.0;

/// A thin frame around a column with a bold label + optional sub-label.
pub fn control_column<R>(
    ui: &mut egui::Ui,
    label: &str,
    sub: &str,
    contents: impl FnOnce(&mut egui::Ui) -> R,
) -> R {
    let mut out = None;
    egui::Frame::group(ui.style())
        .fill(theme::PANEL_LIGHT)
        .stroke(egui::Stroke::new(1.0, theme::BORDER))
        .inner_margin(egui::Margin::same(6))
        .show(ui, |ui| {
            ui.vertical(|ui| {
                ui.set_min_width(COL_WIDTH);
                ui.set_max_width(COL_WIDTH);
                ui.spacing_mut().slider_width = COL_WIDTH - 14.0;
                ui.label(
                    egui::RichText::new(label)
                        .strong()
                        .size(11.0)
                        .color(theme::TEXT),
                );
                if !sub.is_empty() {
                    ui.label(
                        egui::RichText::new(sub)
                            .size(9.0)
                            .color(theme::TEXT_DIM),
                    );
                }
                ui.add_space(2.0);
                out = Some(contents(ui));
            });
        });
    ui.add_space(4.0);
    out.expect("control_column content closure always runs")
}

pub fn float_slider(
    ui: &mut egui::Ui,
    param: &FloatParam,
    range: std::ops::RangeInclusive<f32>,
    decimals: usize,
    suffix: &str,
) {
    let mut v = param.value();
    let mut slider = egui::Slider::new(&mut v, range)
        .fixed_decimals(decimals)
        .show_value(true);
    if !suffix.is_empty() {
        slider = slider.suffix(suffix.to_string());
    }
    if ui.add(slider).changed() {
        param.set_value(v);
    }
}

pub fn float_slider_log(
    ui: &mut egui::Ui,
    param: &FloatParam,
    range: std::ops::RangeInclusive<f32>,
    decimals: usize,
    suffix: &str,
) {
    let mut v = param.value();
    let mut slider = egui::Slider::new(&mut v, range)
        .logarithmic(true)
        .fixed_decimals(decimals)
        .show_value(true);
    if !suffix.is_empty() {
        slider = slider.suffix(suffix.to_string());
    }
    if ui.add(slider).changed() {
        param.set_value(v);
    }
}

pub fn bool_checkbox(ui: &mut egui::Ui, param: &BoolParam, label: &str) {
    let mut v = param.value();
    if ui.checkbox(&mut v, label).changed() {
        param.set_value(v);
    }
}
