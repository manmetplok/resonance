//! The one place the gate turns a parameter into a control (ba todos
//! #1281/#1286, audit findings F4 and C5).
//!
//! Every knob in the editor is drawn through [`param_knob`], and
//! [`param_knob`] hands the whole `FloatParam` to the shared helper:
//! range, default, skew, unit, readout and caption all come off the
//! parameter. Nothing in this file restates a fact that `params.rs`
//! declares — `tests/editor_param_binding.rs` fails the build if it
//! creeps back.
//!
//! Unlike the compressor and the IR loader, the gate's old strip had no
//! drifted *value*: it read min, max, default and the readout off
//! `&dyn Param`, so those already agreed. What it threw away was the
//! *curve*. `&dyn Param` exposes `min_plain`/`max_plain` but not the
//! `FloatRange`, so the only arc it could ask for was `min..=max` with
//! `logarithmic: false` — and five of the gate's eight parameters
//! (ratio, attack, hold, release, key HPF) are declared
//! `FloatRange::Skewed`. That is finding C5, and it is why a 1 ms
//! attack was undialable: the gate's own default sat below 1 % of the
//! dial. The declared curve now reaches the arc, so the feel of those
//! five knobs changes substantially.

use egui::Ui;
use plugin_gui_core::egui;

use crate::params::GateParams;

/// Draw the knob for the parameter at `index` — the index the layout
/// table in [`super::GROUPS`] carries.
///
/// The caption is the parameter's own name, so renaming a parameter
/// cannot desync the GUI from the host's automation lane. The sub-label
/// is empty: the group cards are already captioned (DETECTION, TIMING,
/// DEPTH), and anything else here would be a fact about the parameter
/// restated at the call site.
pub fn param_knob(ui: &mut Ui, params: &GateParams, index: usize) {
    use resonance_plugin::editor_widgets;
    use resonance_plugin::Param;

    let p = params.float_at(index);
    editor_widgets::float_knob(ui, p, p.name(), "");
}
