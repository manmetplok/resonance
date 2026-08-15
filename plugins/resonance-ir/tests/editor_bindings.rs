//! Every control in the IR editor is its parameter (ba todo #1284,
//! audit finding F4).
//!
//! The editor's two knobs are built by `editor_widgets::float_knob`,
//! which since ba todo #1281 derives range, default, skew, unit and
//! readout from the `FloatParam` it is handed. This file guards both
//! halves of that:
//!
//! * the *behaviour* half — sweeping a knob from 0 % to 100 % of its
//!   travel produces exactly what `params.rs` declares, including the
//!   skew, and a reset lands on the declared default. Those are the
//!   four calls the widget makes (`normalized_value`,
//!   `plain_at_normalized`, `set_normalized`, `default_normalized`)
//!   plus `display`, so they can be tested without standing up a GUI;
//! * the *source* half — `controls.rs` must not restate any of it.
//!   Before the migration it passed `0.5` as the mix default against a
//!   param declaring `1.0`, a hardcoded logarithmic arc over
//!   `0.1..=10.0` against a declared `FloatRange::Skewed`, and its own
//!   `format!` readouts. A scan of the call sites is what stops those
//!   arguments from creeping back in a future edit.

use resonance_ir::params::IrParams;
use resonance_plugin::{FloatParam, Param};

const CONTROLS_SRC: &str = include_str!("../src/editor/controls.rs");
const PARAMS_SRC: &str = include_str!("../src/params.rs");

const EPS: f32 = 1e-4;

fn assert_close(actual: f32, expected: f32, what: &str) {
    let tolerance = EPS * expected.abs().max(1.0);
    assert!(
        (actual - expected).abs() <= tolerance,
        "{what}: expected {expected}, got {actual}"
    );
}

/// Sweep a knob across its whole travel and check every position is the
/// parameter's own answer, not the widget's idea of one.
fn assert_arc_follows_the_param(param: &FloatParam, what: &str) {
    let restore = param.value();

    // The ends of the arc are the declared bounds — a control can
    // neither overshoot them nor stop short of them.
    param.set_normalized(0.0);
    assert_close(param.value(), param.range().min(), &format!("{what} at 0%"));
    param.set_normalized(1.0);
    assert_close(
        param.value(),
        param.range().max(),
        &format!("{what} at 100%"),
    );

    // Everything in between is `denormalize`, and the travel is
    // monotonic — the DoD's "swept from 0% to 100% matches the param's
    // own denormalize".
    let mut previous = f32::NEG_INFINITY;
    for step in 0..=20 {
        let travel = step as f32 / 20.0;
        param.set_normalized(travel);
        assert_close(
            param.value(),
            param.range().denormalize(travel),
            &format!("{what} at {:.0}% of the arc", travel * 100.0),
        );
        assert!(
            param.value() >= previous,
            "{what}: the arc must rise monotonically, {} followed {previous}",
            param.value()
        );
        previous = param.value();

        // And the knob's readout is the param's own display, unit
        // included — the hand-rolled `format!("{:.0}%")` /
        // `format!("{:+.1} dB")` the editor used to build are gone.
        let readout = param.display(param.value() as f64);
        assert!(
            readout.contains(param.unit().trim()),
            "{what}: readout {readout} must carry the param's declared unit {:?}",
            param.unit()
        );
    }

    param.set_value(restore);
}

#[test]
fn the_mix_knob_defaults_to_the_declared_fully_wet_value() {
    let params = IrParams::default();

    // The drift this todo exists to fix: the knob passed 0.5 as its
    // default while `params.rs` declares 1.0, so a double-click reset
    // parked a convolution plugin at 50 % wet and the arc's rest
    // position lied about the plugin's own default state.
    assert_close(params.dry_wet.default_value(), 1.0, "declared mix default");
    assert_close(
        params.dry_wet.default_normalized(),
        1.0,
        "the mix default sits at the top of the arc, not the middle",
    );

    // A reset writes the declared default verbatim.
    params.dry_wet.set_normalized(0.25);
    params
        .dry_wet
        .set_normalized(params.dry_wet.default_normalized());
    assert_eq!(
        params.dry_wet.value(),
        1.0,
        "reset must land exactly on the declared default"
    );
}

#[test]
fn the_mix_arc_and_readout_are_the_mix_param() {
    let params = IrParams::default();
    assert_arc_follows_the_param(&params.dry_wet, "dry_wet");

    // Linear 0..1 rendered as whole percent, per the param's unit and
    // formatter.
    params.dry_wet.set_normalized(0.0);
    assert_eq!(params.dry_wet.display(params.dry_wet.value() as f64), "0%");
    params.dry_wet.set_normalized(1.0);
    assert_eq!(
        params.dry_wet.display(params.dry_wet.value() as f64),
        "100%"
    );
}

#[test]
fn the_output_gain_arc_follows_the_declared_skew() {
    let params = IrParams::default();
    assert_arc_follows_the_param(&params.output_gain, "output_gain");

    // The editor used to draw this knob as a logarithmic sweep of
    // 0.1..=10.0, which threw the declared `gain_skew_factor(-20, 20)`
    // away. Half travel is now whatever the parameter says it is, and
    // that is emphatically not the arithmetic midpoint of the range.
    params.output_gain.set_normalized(0.5);
    let mid = params.output_gain.value();
    assert_close(
        mid,
        params.output_gain.range().denormalize(0.5),
        "output gain at 50% of the arc",
    );
    let arithmetic_mid =
        0.5 * (params.output_gain.range().min() + params.output_gain.range().max());
    assert!(
        (mid - arithmetic_mid).abs() > 1.0,
        "a skewed gain range must not behave linearly, got {mid} vs {arithmetic_mid}"
    );

    // The declared skew bunches the low end, so unity gain — the
    // default — sits below the middle of the travel rather than at it.
    let unity_travel = params.output_gain.default_normalized();
    assert!(
        (0.0..0.5).contains(&unity_travel),
        "unity gain should sit in the lower half of a low-bunched arc, got {unity_travel}"
    );
    assert_close(
        params.output_gain.plain_at_normalized(unity_travel),
        1.0,
        "the default travel position maps back to unity gain",
    );
    params.output_gain.set_normalized(unity_travel);
    assert_eq!(
        params
            .output_gain
            .display(params.output_gain.value() as f64),
        "0.00 dB",
        "the readout at the default is the param's dB formatter"
    );
}

/// `controls.rs` with its comments stripped — the prose there *quotes*
/// the arguments the migration deleted, and a scan for them must look
/// at the code only.
fn controls_code() -> String {
    CONTROLS_SRC
        .lines()
        .filter(|line| !line.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// The `float_knob` call sites, as they appear in the editor source.
fn knob_call_arguments() -> Vec<Vec<String>> {
    let code = controls_code();
    let mut calls = Vec::new();
    let mut rest = code.as_str();
    while let Some(at) = rest.find("float_knob(") {
        let after = &rest[at + "float_knob(".len()..];
        let mut depth = 1usize;
        let mut end = after.len();
        for (i, c) in after.char_indices() {
            match c {
                '(' => depth += 1,
                ')' => {
                    depth -= 1;
                    if depth == 0 {
                        end = i;
                        break;
                    }
                }
                _ => {}
            }
        }
        let body = &after[..end];
        // Split on the commas that are not nested inside a call.
        let mut args = Vec::new();
        let mut depth = 0usize;
        let mut current = String::new();
        for c in body.chars() {
            match c {
                '(' | '[' => depth += 1,
                ')' | ']' => depth -= 1,
                ',' if depth == 0 => {
                    args.push(std::mem::take(&mut current));
                    continue;
                }
                _ => {}
            }
            current.push(c);
        }
        if !current.trim().is_empty() {
            args.push(current);
        }
        calls.push(args.into_iter().map(|a| a.trim().to_string()).collect());
        rest = &after[end..];
    }
    calls
}

#[test]
fn no_knob_call_site_restates_a_range_default_unit_or_skew() {
    let calls = knob_call_arguments();
    assert!(!calls.is_empty(), "expected knob call sites in controls.rs");

    for args in &calls {
        assert_eq!(
            args.len(),
            4,
            "float_knob takes (ui, param, label, sub_label); \
             extra arguments are param facts restated at the call site: {args:?}"
        );
        assert_eq!(args[0], "ui");
        assert!(
            args[1].starts_with("&params."),
            "the knob must be handed the parameter itself, got {}",
            args[1]
        );
        // The caption is either the param's own name or a layout
        // caption; the sub-caption describes the cell. Neither may
        // smuggle in a number.
        for arg in &args[2..] {
            assert!(
                !arg.chars().any(|c| c.is_ascii_digit()),
                "caption {arg} looks like a restated parameter fact"
            );
        }
    }

    // Ranges, hand-rolled readouts and dB conversions are what the
    // deleted arguments were made of.
    let code = controls_code();
    for banned in ["..=", "format!", "log10", "powf"] {
        assert!(
            !code.contains(banned),
            "controls.rs still contains `{banned}` — a parameter fact restated in the editor"
        );
    }
}

#[test]
fn every_float_param_is_reachable_through_a_bound_knob() {
    // Read the float params straight out of the struct definition, so
    // adding one without giving it a bound control fails here rather
    // than shipping an unreachable parameter.
    let declared: Vec<&str> = PARAMS_SRC
        .lines()
        .filter_map(|line| line.trim().strip_suffix(": FloatParam,"))
        .filter_map(|name| name.strip_prefix("pub "))
        .collect();
    assert_eq!(
        declared,
        vec!["dry_wet", "output_gain"],
        "IrParams' float parameters changed; update the editor to match"
    );

    let calls = knob_call_arguments();
    for name in declared {
        assert!(
            calls
                .iter()
                .any(|args| args[1] == format!("&params.{name}")),
            "{name} has no param-bound knob in the editor"
        );
    }
    assert_eq!(
        calls.len(),
        2,
        "one knob per float param, and no knob without one"
    );
}
