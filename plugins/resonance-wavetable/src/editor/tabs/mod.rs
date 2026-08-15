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

/// Knob driven by a unipolar FloatParam (range mapped to 0..1), with the
/// readout produced by `fmt` from the param's plain value.
///
/// Use this whenever `{:.2}` plus a unit suffix would misreport the
/// parameter — e.g. glide, which is stored in milliseconds over a 0..2000
/// range and used to be drawn as "0.00s".
pub(crate) fn float_knob_fmt(
    ui: &mut egui::Ui,
    label: &str,
    param: &FloatParam,
    fmt: impl Fn(f32) -> String,
) {
    let min = param.min_plain() as f32;
    let max = param.max_plain() as f32;
    let value = param.value();
    let unit_val = if max > min {
        ((value - min) / (max - min)).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let formatted = fmt(value);
    if let Some(new_unit) = widgets::knob_unipolar(ui, label, unit_val, &formatted, 0.0) {
        let new_plain = (min + new_unit * (max - min)) as f64;
        param.set_plain(new_plain);
    }
}

/// Knob driven by a unipolar FloatParam, displayed as `{:.2}` plus an
/// optional unit suffix.
pub(crate) fn float_knob(
    ui: &mut egui::Ui,
    label: &str,
    param: &FloatParam,
    unit: Option<&str>,
) {
    let unit = unit.unwrap_or("");
    float_knob_fmt(ui, label, param, |v| format!("{:.2}{}", v, unit));
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

/// Integer knob, with the readout produced by `fmt` from the plain value.
pub(crate) fn int_knob_fmt(
    ui: &mut egui::Ui,
    label: &str,
    param: &IntParam,
    fmt: impl Fn(i32) -> String,
) {
    let min = param.min_plain() as f32;
    let max = param.max_plain() as f32;
    let value = param.value() as f32;
    let unit_val = if max > min {
        ((value - min) / (max - min)).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let formatted = fmt(param.value());
    if let Some(new_unit) = widgets::knob_unipolar(ui, label, unit_val, &formatted, 0.0) {
        let new_plain = (min + new_unit * (max - min)).round() as f64;
        param.set_plain(new_plain);
    }
}

/// Integer knob showing the raw count. Only for params whose number *is*
/// the meaning (voice counts, semitones) — a choice param must use
/// [`int_knob_fmt`] with its label table.
pub(crate) fn int_knob(ui: &mut egui::Ui, label: &str, param: &IntParam) {
    int_knob_fmt(ui, label, param, |v| format!("{}", v));
}

