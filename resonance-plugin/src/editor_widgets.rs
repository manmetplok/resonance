//! Param-typed egui widget helpers for plugin editors.
//!
//! Binds the pure egui widgets from `wayland_plugin_gui::widgets` (and
//! plain egui controls) to this crate's parameter types: each helper
//! reads the param, draws the widget, and writes the value back if it
//! changed. Feature-gated behind `editor-widgets` so DSP-only consumers
//! don't pull in the GUI stack.
//!
//! # Everything comes from the parameter (ba todo #1281, finding F4)
//!
//! These helpers take *only* the `FloatParam` and the two caption
//! strings. Range, default, skew, unit and formatter are read off the
//! param itself, so a control cannot disagree with the parameter it
//! edits. The previous signature took range, default and the value text
//! as arguments, and four editors had silently drifted from their own
//! `params.rs` (reverb 4 knobs, compressor 4, amp 1, IR 1); the same
//! call sites also hardcoded `logarithmic: false`, which threw away
//! every declared `FloatRange::Skewed`.
//!
//! The controls therefore work in **normalized 0..1 travel** and let
//! [`FloatParam::plain_at_normalized`] apply the declared curve, rather
//! than asking the widget for a logarithmic drag. Two consequences worth
//! knowing:
//!
//! * the arc/groove follows the param's own skew, so 50 % travel is
//!   whatever the parameter says 50 % is;
//! * a range whose `min` is exactly `0.0` (the gate's `key_hpf`) works
//!   without a special case — the power-law mapping has no logarithm to
//!   clamp away from zero, unlike `widgets::knob`'s `logarithmic: true`
//!   path, which silently raises such a minimum to 0.001.

use crate::param::{BoolParam, FloatParam};
use wayland_plugin_gui::egui;

/// Horizontal slider bound to a `FloatParam`.
///
/// Moves in normalized travel and formats through the param's own
/// display, which also makes egui's click-to-type entry parse through
/// [`crate::param::Param::parse`].
pub fn float_slider(ui: &mut egui::Ui, param: &FloatParam) {
    use crate::param::Param;

    let mut normalized = param.normalized_value();
    let slider = egui::Slider::new(&mut normalized, 0.0..=1.0)
        .custom_formatter(|n, _| param.display(param.plain_at_normalized(n as f32) as f64))
        .custom_parser(|text| {
            param
                .parse(text)
                .map(|plain| param.range().normalize(plain as f32) as f64)
        })
        .show_value(true);
    if ui.add(slider).changed() {
        param.set_normalized(normalized);
    }
}

/// Rotary knob bound to a `FloatParam`.
///
/// `label` and `sub_label` are the only caller-supplied inputs: they are
/// captions for a specific cell in a specific layout (`"SC HPF"`,
/// `"early refl."`), not facts about the parameter. Pass
/// `param.name()` when the full name fits.
pub fn float_knob(ui: &mut egui::Ui, param: &FloatParam, label: &str, sub_label: &str) {
    use crate::param::Param;

    let mut normalized = param.normalized_value();
    let value_text = param.display(param.value() as f64);
    if wayland_plugin_gui::widgets::knob(
        ui,
        &mut normalized,
        0.0..=1.0,
        param.default_normalized(),
        label,
        sub_label,
        &value_text,
        // The skew already lives in the normalized mapping; asking the
        // widget for a second, logarithmic curve would apply it twice.
        false,
    ) {
        param.set_normalized(normalized);
    }
}

/// Checkbox bound to a `BoolParam`.
pub fn bool_checkbox(ui: &mut egui::Ui, param: &BoolParam, label: &str) {
    let mut v = param.value();
    if ui.checkbox(&mut v, label).changed() {
        param.set_value(v);
    }
}
