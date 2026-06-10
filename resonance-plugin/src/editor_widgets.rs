//! Param-typed egui widget helpers for plugin editors.
//!
//! Binds the pure egui widgets from `wayland_plugin_gui::widgets` (and
//! plain egui controls) to this crate's parameter types: each helper
//! reads the param, draws the widget, and writes the value back if it
//! changed. Feature-gated behind `editor-widgets` so DSP-only consumers
//! don't pull in the GUI stack.

use crate::param::{BoolParam, FloatParam};
use wayland_plugin_gui::egui;

/// Horizontal slider bound to a `FloatParam`.
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

/// Logarithmic horizontal slider bound to a `FloatParam`.
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

/// Rotary knob bound to a `FloatParam`.
///
/// 8 arguments because the knob exposes every visual + interaction knob
/// the plugin editors set per-call; a config struct would add boilerplate
/// without any readability win.
#[allow(clippy::too_many_arguments)]
pub fn float_knob(
    ui: &mut egui::Ui,
    param: &FloatParam,
    range: std::ops::RangeInclusive<f32>,
    default: f32,
    label: &str,
    sub_label: &str,
    value_text: &str,
    logarithmic: bool,
) {
    let mut v = param.value();
    if wayland_plugin_gui::widgets::knob(
        ui,
        &mut v,
        range,
        default,
        label,
        sub_label,
        value_text,
        logarithmic,
    ) {
        param.set_value(v);
    }
}

/// Checkbox bound to a `BoolParam`.
pub fn bool_checkbox(ui: &mut egui::Ui, param: &BoolParam, label: &str) {
    let mut v = param.value();
    if ui.checkbox(&mut v, label).changed() {
        param.set_value(v);
    }
}
