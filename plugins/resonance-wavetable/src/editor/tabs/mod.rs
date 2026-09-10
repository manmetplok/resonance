//! Tab implementations. Each submodule exports a `draw(ui, app)` function
//! that paints its tab's contents into the central panel.
//!
//! # Every control is built from its parameter (ba todo #1285, findings F4/W4)
//!
//! The helpers below take *only* the parameter and a caption. Travel,
//! default, polarity and readout are read off the `FloatParam` /
//! `IntParam` itself, so a control cannot disagree with the parameter it
//! edits. Before the migration each helper:
//!
//! * mapped the arc **linearly** across `min..max`, which threw away every
//!   declared `FloatRange::Skewed` — finding W4. Filter cutoff swept
//!   20–20000 Hz linearly (everything under 2 kHz in the first 10 % of the
//!   dial) and the sub-100 ms end of the envelope times was unreachable by
//!   mouse;
//! * passed a hardcoded `0.0` as the knob's double-click default, so *every*
//!   knob reset to its **range minimum** instead of the value params.rs
//!   declares — a reset dropped master volume to silence, filter cutoff to
//!   20 Hz and LFO 2's rate to 0.01 Hz;
//! * formatted its own readout from a caller-supplied unit suffix rather
//!   than the parameter's own `Param::display`, so percentages read as raw
//!   fractions and a 5 ms attack printed as `0.01s`.
//!
//! Controls therefore work in **normalized 0..1 travel** and let the
//! parameter's own `FloatRange` apply the declared curve — the same
//! contract `resonance_plugin::editor_widgets` is built on. This editor
//! keeps its own thin bindings only because it draws the *themed* lavender
//! knob (`widgets::knob_themed`) rather than the classic knob that helper
//! wraps.

pub mod env_filter;
pub mod fx;
pub mod lfo;
pub mod mod_matrix;
pub mod osc;

use plugin_gui_core::{egui, widgets};

use resonance_plugin::param::{FloatParam, IntParam, Param};

/// A parameter whose range spans zero is drawn centre-out with a centre
/// tick (pitch, pan, curve, mod amount); everything else fills from the
/// minimum. Derived from the range so no call site has to claim a
/// polarity — `filter_keytrack` was drawn bipolar for exactly that reason
/// until ba todo #1271.
fn is_bipolar(min: f32, max: f32) -> bool {
    min < 0.0 && max > 0.0
}

/// Knob cell bound to a `FloatParam`.
///
/// `label` is the only caller-supplied input: it captions this cell in
/// this layout (`"Reso"`, `"Fb"`, `"Time L"`), which the 52 px dial has
/// room for where the parameter's full name (`"Filter Resonance"`) does
/// not. It is never a fact about the parameter.
pub(crate) fn float_knob(ui: &mut egui::Ui, label: &str, param: &FloatParam) {
    let range = param.range();
    let value_text = param.display(param.value() as f64);
    let knob = widgets::ThemedKnob::new(
        label,
        param.normalized_value(),
        &value_text,
        param.default_normalized(),
    )
    .bipolar(is_bipolar(range.min(), range.max()));
    if let Some(travel) = widgets::knob_themed(ui, &knob) {
        param.set_normalized(travel);
    }
}

/// Where an integer value sits on its control's 0..1 travel. `IntRange`
/// has only a linear variant, so this is the whole mapping.
fn int_travel(param: &IntParam, value: i32) -> f32 {
    param.range().normalize(value) as f32
}

/// The integer a 0..1 control position maps to — the inverse of
/// [`int_travel`].
fn int_at_travel(param: &IntParam, travel: f32) -> i32 {
    let range = param.range();
    let (min, max) = (range.min(), range.max());
    min + (travel.clamp(0.0, 1.0) * (max - min) as f32).round() as i32
}

/// Knob cell bound to an `IntParam`, with the readout produced by `fmt`
/// from the plain value.
///
/// The only remaining caller is the LFO shape knob, whose label table
/// still lives in `dsp::lfo`. ba todo #1292 moves that table onto the
/// parameter via `IntParam::with_choices`, after which every int knob
/// reads its own `Param::display` and this helper goes away.
pub(crate) fn int_knob_fmt(
    ui: &mut egui::Ui,
    label: &str,
    param: &IntParam,
    fmt: impl Fn(i32) -> String,
) {
    let value_text = fmt(param.value());
    int_knob_inner(ui, label, param, &value_text);
}

/// Knob cell bound to an `IntParam`, showing the parameter's own display.
pub(crate) fn int_knob(ui: &mut egui::Ui, label: &str, param: &IntParam) {
    let value_text = param.display(param.get_plain());
    int_knob_inner(ui, label, param, &value_text);
}

fn int_knob_inner(ui: &mut egui::Ui, label: &str, param: &IntParam, value_text: &str) {
    let range = param.range();
    let knob = widgets::ThemedKnob::new(
        label,
        int_travel(param, param.value()),
        value_text,
        int_travel(param, param.default_value()),
    )
    .bipolar(is_bipolar(range.min() as f32, range.max() as f32));
    if let Some(travel) = widgets::knob_themed(ui, &knob) {
        param.set_plain(f64::from(int_at_travel(param, travel)));
    }
}

/// Horizontal slider bound to a `FloatParam`, `width` px wide.
///
/// Same contract as [`float_knob`]: the groove is the parameter's own
/// travel and the polarity comes from its range.
pub(crate) fn float_slider(ui: &mut egui::Ui, width: f32, param: &FloatParam) {
    let range = param.range();
    let bipolar = is_bipolar(range.min(), range.max());
    // The editor's own `slider_unit` was the shared `HSlider` with the
    // `bipolar` flag already curried in (ba todo #1335); this is the same
    // widget, and the "never sees a plain value" rule the fork's doc
    // stated is a property of this binding, not of the drawing code.
    let slider = widgets::HSlider::new(width, param.normalized_value()).bipolar(bipolar);
    if let Some(travel) = widgets::slider(ui, &slider) {
        param.set_normalized(travel);
    }
}

/// The readout a control shows next to a parameter — the parameter's own
/// formatter, never a `format!` at the call site.
pub(crate) fn readout(param: &FloatParam) -> String {
    param.display(param.value() as f64)
}
