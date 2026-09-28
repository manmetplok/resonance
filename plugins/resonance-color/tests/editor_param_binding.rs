//! The dual-surface rule: every parameter is a CLAP parameter **and** a
//! control in the editor, and the control restates nothing the parameter
//! declares.
//!
//! A source guard over `src/editor/controls.rs` (compiled in, so it
//! cannot drift from the file the editor builds): each parameter is drawn
//! by exactly one param-bound call — `float_knob`, `bool_checkbox` or
//! `choice_segmented` — and the number of such calls is the parameter
//! count, so nothing is left without a control and nothing is drawn for
//! a parameter `params.rs` does not declare.

use resonance_color::params::{ColorParams, PARAM_COUNT};

const CONTROLS_SRC: &str = include_str!("../src/editor/controls.rs");

/// Field name → the binding helper that must draw it.
const BINDINGS: [(&str, &str); PARAM_COUNT] = [
    ("mode", "choice_segmented(ui, &p.mode)"),
    ("drive", "float_knob(ui, &p.drive, p.drive.name(),"),
    ("bias", "float_knob(ui, &p.bias, p.bias.name(),"),
    ("response", "float_knob(ui, &p.response, p.response.name(),"),
    ("tone", "float_knob(ui, &p.tone, p.tone.name(),"),
    ("mix", "float_knob(ui, &p.mix, p.mix.name(),"),
    ("auto_gain", "bool_checkbox(ui, &p.auto_gain, p.auto_gain.name())"),
    ("output", "float_knob(ui, &p.output, p.output.name(),"),
    ("oversample", "choice_segmented(ui, &p.oversample)"),
    ("speed", "choice_segmented(ui, &p.speed)"),
    ("flutter", "float_knob(ui, &p.flutter, p.flutter.name(),"),
];

#[test]
fn the_binding_table_is_the_declared_param_list() {
    let p = ColorParams::default();
    let ids: Vec<&str> = (0..PARAM_COUNT).map(|i| p.param_at(i).id()).collect();
    let fields: Vec<&str> = BINDINGS.iter().map(|(f, _)| *f).collect();
    assert_eq!(ids, fields, "each param's field name is its id");
}

#[test]
fn every_param_has_exactly_one_control() {
    for (field, needle) in BINDINGS {
        let hits = CONTROLS_SRC.matches(needle).count();
        assert_eq!(hits, 1, "`{field}` must be drawn by exactly one `{needle}` call, found {hits}");
    }
    let calls = CONTROLS_SRC.matches("float_knob(ui, &p.").count()
        + CONTROLS_SRC.matches("bool_checkbox(ui, &p.").count()
        + CONTROLS_SRC.matches("choice_segmented(ui, &p.").count();
    assert_eq!(calls, PARAM_COUNT, "{calls} param-bound controls for {PARAM_COUNT} params");
}

/// The captions come off the parameter, and the only other string a
/// knob call takes is its sub-label — no range, default or readout.
#[test]
fn knob_calls_restate_nothing() {
    let mut checked = 0;
    for line in CONTROLS_SRC.lines() {
        let Some(rest) = line.trim().strip_prefix("editor_widgets::float_knob(") else {
            continue;
        };
        checked += 1;
        let args: Vec<&str> = rest
            .trim_end_matches(';')
            .trim_end_matches(')')
            .split(',')
            .map(str::trim)
            .collect();
        assert_eq!(args.len(), 4, "float_knob takes ui, the param and two captions: {line}");
        let field = args[1].strip_prefix("&p.").expect("the param itself");
        assert_eq!(args[2], format!("p.{field}.name()"), "label must be the param's name: {line}");
        assert!(args[3].starts_with('"'), "sub-label must be a literal caption: {line}");
    }
    assert_eq!(checked, 7, "seven float params, seven knobs");
    assert!(!CONTROLS_SRC.contains("format!("), "no hand-built readouts in the strip");
}
