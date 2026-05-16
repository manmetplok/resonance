//! Tab implementations. Each submodule exports a `draw(ui, app)` function
//! that paints its tab's contents into the central panel.

pub mod env_filter;
pub mod fx;
pub mod lfo;
pub mod mod_matrix;
pub mod osc;

use wayland_plugin_gui::egui;

use crate::editor::widgets;
use resonance_plugin::param::{FloatParam, IntParam, Param};

/// Knob driven by a unipolar FloatParam (range mapped to 0..1).
pub(crate) fn float_knob(
    ui: &mut egui::Ui,
    label: &str,
    param: &FloatParam,
    unit: Option<&str>,
) {
    let min = param.min_plain() as f32;
    let max = param.max_plain() as f32;
    let value = param.value();
    let unit_val = if max > min {
        ((value - min) / (max - min)).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let formatted = match unit {
        Some(u) => format!("{:.2}{}", value, u),
        None => format!("{:.2}", value),
    };
    if let Some(new_unit) = widgets::knob_unipolar(ui, label, unit_val, &formatted, 0.0) {
        let new_plain = (min + new_unit * (max - min)) as f64;
        param.set_plain(new_plain);
    }
}

/// Bipolar knob — assumes the param's range is symmetric around 0.
pub(crate) fn float_knob_bipolar(
    ui: &mut egui::Ui,
    label: &str,
    param: &FloatParam,
    unit: Option<&str>,
) {
    let min = param.min_plain() as f32;
    let max = param.max_plain() as f32;
    let value = param.value();
    let half = (max - min) * 0.5;
    let signed = if half > 0.0 {
        ((value - (min + half)) / half).clamp(-1.0, 1.0)
    } else {
        0.0
    };
    let formatted = match unit {
        Some(u) => format!("{:+.2}{}", value, u),
        None => format!("{:+.2}", value),
    };
    if let Some(new_signed) = widgets::knob_bipolar(ui, label, signed, &formatted, 0.0) {
        let new_plain = (min + half + new_signed * half) as f64;
        param.set_plain(new_plain);
    }
}

/// Integer knob.
pub(crate) fn int_knob(ui: &mut egui::Ui, label: &str, param: &IntParam) {
    let min = param.min_plain() as f32;
    let max = param.max_plain() as f32;
    let value = param.value() as f32;
    let unit_val = if max > min {
        ((value - min) / (max - min)).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let formatted = format!("{}", param.value());
    if let Some(new_unit) = widgets::knob_unipolar(ui, label, unit_val, &formatted, 0.0) {
        let new_plain = (min + new_unit * (max - min)).round() as f64;
        param.set_plain(new_plain);
    }
}

