//! Shared helpers for stage control panels. Each helper takes the
//! atomic param directly, reads its value, pushes the widget, and
//! commits the new value back if it changed. That keeps the control
//! panel code short and free of repetitive glue.

use resonance_plugin::{BoolParam, FloatParam, IntParam, Param};
use wayland_plugin_gui::egui;

use super::theme;

/// Standard per-control column width in the stage panels.
pub const COL_WIDTH: f32 = 108.0;

/// A thin frame around a column with a bold label + optional sub-label.
/// Matches the compressor editor's `control_column` look so the plugin
/// family feels consistent.
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
                ui.spacing_mut().slider_width = COL_WIDTH - 12.0;
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

/// Linear float slider bound to a `FloatParam`.
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

/// Logarithmic float slider bound to a `FloatParam`. Use for freq /
/// attack / release type params.
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

/// Simple on/off checkbox bound to a `BoolParam`.
pub fn bool_checkbox(ui: &mut egui::Ui, param: &BoolParam, label: &str) {
    let mut v = param.value();
    if ui.checkbox(&mut v, label).changed() {
        param.set_value(v);
    }
}

/// Labeled combo box bound to an `IntParam`. `labels` is indexed by the
/// integer value (offset from range min).
pub fn int_combo(
    ui: &mut egui::Ui,
    param: &IntParam,
    id: &str,
    labels: &[&str],
) {
    let current = param.value();
    let min = param.min_plain() as i32;
    let idx = (current - min).clamp(0, labels.len() as i32 - 1) as usize;
    let current_label = labels.get(idx).copied().unwrap_or("?");
    egui::ComboBox::from_id_salt(id)
        .width(COL_WIDTH - 16.0)
        .selected_text(current_label)
        .show_ui(ui, |ui| {
            for (i, label) in labels.iter().enumerate() {
                if ui
                    .selectable_label(i == idx, *label)
                    .clicked()
                {
                    param.set_value(min + i as i32);
                }
            }
        });
}
