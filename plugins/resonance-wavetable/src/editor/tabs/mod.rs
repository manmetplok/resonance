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

use resonance_plugin::editor_widgets::{self, ParamKnob, ParamSlider};
use resonance_plugin::param::{FloatParam, IntParam, Param};

/// Knob cell bound to a `FloatParam`.
///
/// `label` is the only caller-supplied input: it captions this cell in
/// this layout (`"Reso"`, `"Fb"`, `"Time L"`), which the 52 px dial has
/// room for where the parameter's full name (`"Filter Resonance"`) does
/// not. It is never a fact about the parameter. The fleet's binding
/// (`editor_widgets::param_knob`) reads travel, skew, default, polarity
/// and readout off the param, takes typed entry on the readout and
/// announces each gesture to the host (code review PUX-01/-06/-11).
pub(crate) fn float_knob(ui: &mut egui::Ui, label: &str, param: &FloatParam) {
    editor_widgets::param_knob(ui, ParamKnob::new(param, label));
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
    editor_widgets::param_knob(ui, ParamKnob::new(param, label).value_text(value_text));
}

/// Knob cell bound to an `IntParam`, showing the parameter's own display.
/// The drag accumulates through the gesture, so it steps on an ordinary
/// drag (code review PUX-02: dist Mode, OS, Coarse, Voices, the LFO and
/// S&H divisions only answered fast flicks).
pub(crate) fn int_knob(ui: &mut egui::Ui, label: &str, param: &IntParam) {
    editor_widgets::param_knob(ui, ParamKnob::new(param, label));
}

/// Horizontal slider bound to a `FloatParam`, `width` px wide.
///
/// Same contract as [`float_knob`]: the groove is the parameter's own
/// travel, the polarity comes from its range, a double-click resets to
/// the declared default (PUX-06) and the drag is one announced edit.
pub(crate) fn float_slider(ui: &mut egui::Ui, width: f32, param: &FloatParam) {
    editor_widgets::param_slider(ui, ParamSlider::new(param, width));
}

/// Segmented selector bound to a choice `IntParam` (one declared with
/// `IntParam::with_choices`): one segment per declared label, so the
/// control can offer exactly the values the parameter holds and nothing
/// else.
pub(crate) fn choice_segmented(ui: &mut egui::Ui, param: &IntParam) {
    editor_widgets::choice_segmented(ui, param, &widgets::SegmentedStyle::LAVENDER);
}

/// A single chip bound to a choice `IntParam` that shows the current
/// label and steps to the next one on a click, wrapping. For the two-way
/// switches (sub waveform, sub octave, noise type) where a full segmented
/// row would not fit the card.
pub(crate) fn choice_cycle(ui: &mut egui::Ui, param: &IntParam) {
    let value = param.value();
    let label = param.display(param.get_plain());
    if widgets::chip_button(ui, &label, true) {
        let range = param.range();
        let next = if value >= range.max() {
            range.min()
        } else {
            value + 1
        };
        editor_widgets::commit_plain(ui.ctx(), param, f64::from(next));
    }
}

/// The readout a control shows next to a parameter — the parameter's own
/// formatter, never a `format!` at the call site.
pub(crate) fn readout(param: &FloatParam) -> String {
    param.display(param.value() as f64)
}
