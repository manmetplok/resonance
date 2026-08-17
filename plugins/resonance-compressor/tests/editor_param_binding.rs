//! The editor's controls against the parameters they edit (ba todo
//! #1282, audit finding F4).
//!
//! Before the migration, `editor_widgets::float_knob` took the range,
//! the default, the value text and a linear/log flag as *arguments*, so
//! `control_strip.rs` restated by hand what `params.rs` already
//! declares. Four of the compressor's nine knobs had drifted: the
//! double-click-to-reset default a user got was not the default the
//! host, the presets or a fresh instance used
//! (`old_hardcoded_defaults_that_disagreed_with_the_param` below pins
//! all four).
//!
//! Two kinds of test keep it fixed:
//!
//! * a source guard over `control_strip.rs`, because the drift was a
//!   property of the *call site* — no range literal, no `format!`
//!   readout, no caption that isn't `Param::name()`;
//! * behavioural tests that the control's 0..1 travel is exactly the
//!   parameter's own `FloatRange`, so the declared skew is what the arc
//!   follows and a reset lands on the declared default verbatim.
//!
//! They need no GUI: `float_knob` adds only egui plumbing on top of
//! `normalized_value` / `default_normalized` / `plain_at_normalized` /
//! `set_normalized`, which is the whole contract under test.

use resonance_compressor::params::{CompressorParams, PARAM_COUNT};
use resonance_plugin::{FloatParam, FloatRange, Param};

/// The source of the strip the guard tests read. Compiled in, so it can
/// never drift from the file the editor actually builds.
const CONTROL_STRIP_SRC: &str = include_str!("../src/editor/control_strip.rs");

/// Every `FloatParam` in the compressor, paired with the field name the
/// control strip must reference.
fn float_params(p: &CompressorParams) -> Vec<(&'static str, &FloatParam)> {
    vec![
        ("threshold", &p.threshold),
        ("ratio", &p.ratio),
        ("attack", &p.attack),
        ("release", &p.release),
        ("knee", &p.knee),
        ("makeup", &p.makeup),
        ("mix", &p.mix),
        ("detector_mix", &p.detector_mix),
        ("sc_hpf_freq", &p.sc_hpf_freq),
    ]
}

const BOOL_PARAM_FIELDS: [&str; 2] = ["sc_hpf_on", "auto_makeup"];

/// Split a single-line call's argument list on commas. Safe here because
/// the guard also rejects any argument that isn't a plain path or a
/// string literal, neither of which can contain a comma.
fn call_args<'a>(line: &'a str, callee: &str) -> Option<Vec<&'a str>> {
    let rest = line.trim().strip_prefix(callee)?;
    let inner = rest
        .trim_end()
        .strip_suffix(';')?
        .trim_end()
        .strip_suffix(')')?;
    Some(inner.split(',').map(str::trim).collect())
}

// ---------------------------------------------------------------------------
// Source guard: the call sites restate nothing
// ---------------------------------------------------------------------------

#[test]
fn every_param_has_exactly_one_control_in_the_strip() {
    let params = CompressorParams::default();
    for (field, _) in float_params(&params) {
        let needle = format!("float_knob(ui, &p.{field},");
        let hits = CONTROL_STRIP_SRC.matches(&needle).count();
        assert_eq!(
            hits, 1,
            "`{field}` must be drawn by exactly one param-bound float_knob call, found {hits}"
        );
    }
    for field in BOOL_PARAM_FIELDS {
        let needle = format!("bool_checkbox(ui, &p.{field},");
        let hits = CONTROL_STRIP_SRC.matches(&needle).count();
        assert_eq!(
            hits, 1,
            "`{field}` must be drawn by exactly one param-bound bool_checkbox call, found {hits}"
        );
    }

    // Completeness in the other direction: no parameter is left without
    // a control, and no control is drawn for something params.rs does
    // not declare.
    let controls = CONTROL_STRIP_SRC.matches("float_knob(ui, &p.").count()
        + CONTROL_STRIP_SRC.matches("bool_checkbox(ui, &p.").count();
    assert_eq!(
        controls, PARAM_COUNT,
        "the strip draws {controls} controls for {PARAM_COUNT} parameters"
    );
}

#[test]
fn no_knob_call_site_restates_a_param_fact() {
    let mut checked = 0;
    for line in CONTROL_STRIP_SRC.lines() {
        let Some(args) = call_args(line, "editor_widgets::float_knob(") else {
            continue;
        };
        checked += 1;
        assert_eq!(
            args.len(),
            4,
            "float_knob takes ui, the param and two captions — nothing else: {line}"
        );
        assert_eq!(args[0], "ui");
        let field = args[1]
            .strip_prefix("&p.")
            .unwrap_or_else(|| panic!("second argument must be the FloatParam itself: {line}"));
        // The caption is the parameter's own name, not a copy of it.
        assert_eq!(
            args[2],
            format!("p.{field}.name()"),
            "the label must come off the param, so renaming it cannot desync: {line}"
        );
        // Only the sub-label is a free string: it captions a cell in
        // this layout ("dry/wet"), not the parameter.
        assert!(
            args[3].starts_with('"') && args[3].ends_with('"'),
            "the sub-label must be a literal caption: {line}"
        );
    }
    assert_eq!(checked, 9, "expected all nine knobs to be checked");
}

#[test]
fn the_strip_declares_no_range_default_or_readout_of_its_own() {
    // Skip the module doc header, which talks *about* these things.
    let body: String = CONTROL_STRIP_SRC
        .lines()
        .filter(|l| !l.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n");

    assert!(
        !body.contains("..="),
        "a range literal in the control strip means the knob restates params.rs"
    );
    assert!(
        !body.contains("format!"),
        "a hand-rolled readout means the knob restates the param's formatter"
    );
    for unit in [" dB", " ms", " Hz", ":1", "%"] {
        assert!(
            !body.contains(unit),
            "unit `{unit}` is declared by params.rs; the strip must not repeat it"
        );
    }
}

// ---------------------------------------------------------------------------
// Behaviour: the arc is the parameter's own range
// ---------------------------------------------------------------------------

#[test]
fn knob_travel_is_exactly_the_declared_range() {
    let params = CompressorParams::default();
    for (field, param) in float_params(&params) {
        let range = param.range();
        assert_eq!(
            param.plain_at_normalized(0.0),
            range.min(),
            "{field}: 0 % travel must be the declared minimum"
        );
        assert_eq!(
            param.plain_at_normalized(1.0),
            range.max(),
            "{field}: 100 % travel must be the declared maximum"
        );

        let mut previous = f32::NEG_INFINITY;
        for step in 0..=100 {
            let t = step as f32 / 100.0;
            let plain = param.plain_at_normalized(t);
            assert_eq!(
                plain,
                range.denormalize(t),
                "{field}: the knob must sweep the param's own denormalize"
            );
            assert!(
                plain >= previous,
                "{field}: travel must move the value monotonically (t = {t})"
            );
            assert!(
                plain >= range.min() && plain <= range.max(),
                "{field}: {plain} at t = {t} escapes the declared range"
            );
            previous = plain;
        }
    }
}

#[test]
fn the_knob_and_the_parameter_agree_in_both_directions() {
    let params = CompressorParams::default();
    for (field, param) in float_params(&params) {
        let range = param.range();
        for step in 0..=20 {
            // Where the host puts the value, the knob must point.
            let plain = range.denormalize(step as f32 / 20.0);
            param.set_value(plain);
            let travel = param.normalized_value();
            let round_tripped = param.plain_at_normalized(travel);
            let tolerance = (range.max() - range.min()).abs() * 1e-4;
            assert!(
                (round_tripped - plain).abs() <= tolerance,
                "{field}: {plain} reads back as {round_tripped} through the arc"
            );

            // And where the user drags the knob, the parameter follows.
            param.set_normalized(travel);
            assert!(
                (param.value() - plain).abs() <= tolerance,
                "{field}: dragging to {travel} left the param at {}",
                param.value()
            );
        }
    }
}

#[test]
fn the_declared_skew_reaches_the_arc() {
    let params = CompressorParams::default();
    for (field, param) in float_params(&params) {
        let range = param.range();
        let linear_mid = (range.min() + range.max()) / 2.0;
        let arc_mid = param.plain_at_normalized(0.5);
        match range {
            FloatRange::Linear { .. } => assert!(
                (arc_mid - linear_mid).abs() < 1e-5,
                "{field} is Linear, so half travel must be the arithmetic middle"
            ),
            // Every skewed param here declares a negative factor, which
            // buys the low end of the range more of the dial: half
            // travel must land well below the arithmetic middle. Without
            // this the skew would be declared and thrown away, which is
            // exactly finding F4.
            FloatRange::Skewed { .. } => assert!(
                arc_mid < linear_mid * 0.8,
                "{field}: half travel is {arc_mid}, barely below the linear middle {linear_mid} — \
                 the declared skew is not reaching the control"
            ),
        }
    }
}

#[test]
fn a_reset_lands_on_the_declared_default_verbatim() {
    let params = CompressorParams::default();
    for (field, param) in float_params(&params) {
        param.set_value(param.range().max());
        // Double-click-to-reset drops the knob on default_normalized.
        param.set_normalized(param.default_normalized());
        assert_eq!(
            param.value(),
            param.default_value(),
            "{field}: reset must write the declared default exactly, not the curve's round trip"
        );
    }
}

// ---------------------------------------------------------------------------
// The drift itself, pinned
// ---------------------------------------------------------------------------

/// The four knobs whose hardcoded editor argument disagreed with
/// `params.rs`. The parameter was always the truth — a fresh plugin
/// instance, every preset and the host's own "reset to default" already
/// used these values; only the editor's reset gesture and arc origin
/// used the stale ones. Fixing the call sites is therefore a visible
/// change for anyone who double-clicked a knob.
#[test]
fn old_hardcoded_defaults_that_disagreed_with_the_param() {
    let p = CompressorParams::default();
    // (param, what the editor used to hardcode, what params.rs declares)
    let drifted: [(&str, f32, f32); 4] = [
        ("threshold", -20.0, p.threshold.default_value()),
        ("release", 100.0, p.release.default_value()),
        ("knee", 3.0, p.knee.default_value()),
        ("detector_mix", 0.0, p.detector_mix.default_value()),
    ];
    let expected = [-18.0, 120.0, 6.0, 0.3];
    for ((field, old, declared), want) in drifted.iter().zip(expected) {
        assert_eq!(
            *declared, want,
            "{field}: params.rs no longer declares {want}; re-check the migration"
        );
        assert_ne!(
            *old, *declared,
            "{field} is listed as drifted but the old editor value now matches"
        );
    }

    // The knobs that were already in agreement stay that way.
    assert_eq!(p.ratio.default_value(), 4.0);
    assert_eq!(p.attack.default_value(), 10.0);
    assert_eq!(p.makeup.default_value(), 0.0);
    assert_eq!(p.mix.default_value(), 1.0);
    assert_eq!(p.sc_hpf_freq.default_value(), 80.0);
}

#[test]
fn the_readout_is_the_params_own_formatter() {
    let p = CompressorParams::default();
    // At the declared default, which is what the knob shows on open.
    let expected = [
        ("threshold", p.threshold.display(-18.0), "-18.0 dB"),
        // format_ratio, not the old plain "{:.1}:1" — it collapses a
        // limiting ratio to the infinity glyph.
        ("ratio", p.ratio.display(4.0), "4.0:1"),
        ("ratio at the top", p.ratio.display(20.0), "∞:1"),
        ("attack", p.attack.display(10.0), "10.00 ms"),
        ("release", p.release.display(120.0), "120 ms"),
        ("knee", p.knee.display(6.0), "6.0 dB"),
        ("makeup", p.makeup.display(0.0), "0.0 dB"),
        ("mix", p.mix.display(1.0), "100%"),
        // The old editor printed a bare "0.30" here.
        ("detector_mix", p.detector_mix.display(0.3), "30% RMS"),
        ("detector at peak", p.detector_mix.display(0.0), "Peak"),
        ("sc_hpf_freq", p.sc_hpf_freq.display(80.0), "80 Hz"),
    ];
    for (what, got, want) in expected {
        assert_eq!(got, want, "{what} reads back as `{got}`, expected `{want}`");
    }
}

#[test]
fn no_readout_doubles_up_the_declared_unit() {
    let params = CompressorParams::default();
    for (field, param) in float_params(&params) {
        let unit = param.unit();
        if unit.is_empty() {
            continue;
        }
        for step in 0..=10 {
            let value = param.plain_at_normalized(step as f32 / 10.0);
            let text = param.display(value as f64);
            assert_eq!(
                text.matches(unit.trim()).count(),
                1,
                "{field}: `{text}` repeats the unit `{unit}`"
            );
        }
    }
}
