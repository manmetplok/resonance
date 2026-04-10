//! Tab implementations. Each submodule exports a `draw(ui, app)` function
//! that paints its tab's contents into the central panel.

pub mod env_filter;
pub mod fx;
pub mod lfo;
pub mod mod_matrix;
pub mod osc;

use wayland_plugin_gui::egui;

use crate::editor::theme;
use resonance_plugin::param::{FloatParam, IntParam, Param};

/// Shared helper: labelled horizontal slider over a FloatParam.
pub(crate) fn float_slider(
    ui: &mut egui::Ui,
    label: &str,
    param: &FloatParam,
    display_unit: Option<&str>,
) {
    let min = param.min_plain() as f32;
    let max = param.max_plain() as f32;
    let mut value = param.value();
    let unit = display_unit.map(|s| s.to_string());
    let response = ui.add(
        egui::Slider::new(&mut value, min..=max)
            .text(label)
            .custom_formatter(move |v, _| match &unit {
                Some(unit) => format!("{:.2}{}", v, unit),
                None => format!("{:.3}", v),
            }),
    );
    if response.changed() {
        param.set_value(value);
    }
}

pub(crate) fn int_slider(ui: &mut egui::Ui, label: &str, param: &IntParam) {
    let min = param.min_plain() as i32;
    let max = param.max_plain() as i32;
    let mut value = param.value();
    let response = ui.add(egui::Slider::new(&mut value, min..=max).text(label));
    if response.changed() {
        param.set_plain(value as f64);
    }
}

pub(crate) fn bool_checkbox(
    ui: &mut egui::Ui,
    label: &str,
    param: &resonance_plugin::param::BoolParam,
) {
    let mut value = param.value();
    if ui.checkbox(&mut value, label).changed() {
        param.set_plain(if value { 1.0 } else { 0.0 });
    }
}

/// Helper to draw a labelled section header.
pub(crate) fn section_header(ui: &mut egui::Ui, title: &str) {
    ui.label(
        egui::RichText::new(title)
            .size(10.0)
            .strong()
            .color(theme::TEXT_DIM),
    );
    ui.separator();
}
