//! The gate editor's knobs against the parameters they edit (ba todo
//! #1286, audit findings F4 and C5).
//!
//! The gate's strip is data-driven: `editor::GROUPS` lists parameter
//! indices and `editor::widgets::param_knob` draws whichever one it is
//! handed, so there is exactly one place where a parameter becomes a
//! control. `tests/editor_layout.rs` already pins the table's coverage;
//! these tests pin the binding.
//!
//! Unlike the compressor (#1282) and the IR loader (#1284), nothing
//! here had drifted in *value*: the old `param_knob` read min, max,
//! default and the readout off `&dyn Param`, so every range, default,
//! unit and caption already agreed with `params.rs`. What it threw away
//! was the *curve* — `&dyn Param` cannot express a `FloatRange`, so the
//! widget got plain `min..=max` travel with `logarithmic: false` while
//! five of the eight parameters are declared `FloatRange::Skewed`. That
//! is finding C5, and it is why a 1 ms attack was undialable: the
//! default sat below 1 % of the dial.
//!
//! Two kinds of test keep it fixed:
//!
//! * a source guard over `editor/widgets.rs`, because the drift class is
//!   a property of the *call site* — no range literal, no `format!`
//!   readout, no unit string, no linear/logarithmic choice of its own,
//!   and no hand-plumbed `min_plain`/`default_plain`;
//! * behavioural tests that the control's 0..1 travel is exactly the
//!   parameter's own `FloatRange`, so the declared skew is what the arc
//!   follows and a reset lands on the declared default verbatim.
//!
//! They need no GUI: `float_knob` adds only egui plumbing on top of
//! `normalized_value` / `default_normalized` / `plain_at_normalized` /
//! `set_normalized`, which is the whole contract under test.

use resonance_gate::params::{GateParams, PARAM_COUNT};
use resonance_plugin::{FloatParam, FloatRange, Param};

/// The source of the file the guard tests read. Compiled in, so it can
/// never drift from the file the editor actually builds.
const KNOB_SRC: &str = include_str!("../src/editor/widgets.rs");

/// Every `FloatParam` in the gate, paired with the field name. The gate
/// declares no bool or int parameters, so this is the complete
/// parameter surface.
fn float_params(p: &GateParams) -> Vec<(&'static str, &FloatParam)> {
    vec![
        ("threshold", &p.threshold),
        ("ratio", &p.ratio),
        ("attack", &p.attack),
        ("hold", &p.hold),
        ("release", &p.release),
        ("range", &p.range),
        ("hysteresis", &p.hysteresis),
        ("key_hpf", &p.key_hpf),
    ]
}

// ---------------------------------------------------------------------------
// Source guard: the one call site restates nothing
// ---------------------------------------------------------------------------

#[test]
fn the_editor_draws_knobs_only_through_the_param_bound_helper() {
    let calls = KNOB_SRC.matches("editor_widgets::float_knob(").count();
    assert_eq!(
        calls, 1,
        "every knob must go through the one param-bound call site, found {calls}"
    );
    assert!(
        !KNOB_SRC.contains("widgets::knob("),
        "the raw widget takes a range and a linear/log flag as arguments — \
         that is the call-site drift finding F4 is about"
    );
}

#[test]
fn the_call_site_passes_the_param_and_nothing_else() {
    let line = KNOB_SRC
        .lines()
        .find(|l| l.contains("editor_widgets::float_knob("))
        .expect("the knob call site must be a single line the guard can read");
    let args: Vec<&str> = line
        .trim()
        .strip_prefix("editor_widgets::float_knob(")
        .and_then(|r| r.trim_end().strip_suffix(';'))
        .and_then(|r| r.trim_end().strip_suffix(')'))
        .expect("unexpected call shape")
        .split(',')
        .map(str::trim)
        .collect();

    assert_eq!(
        args.len(),
        4,
        "float_knob takes ui, the param and two captions — nothing else: {line}"
    );
    assert_eq!(args[0], "ui");
    assert_eq!(
        args[1], "p",
        "the second argument must be the FloatParam itself, so the helper can read \
         its range, default and skew: {line}"
    );
    assert_eq!(
        args[2], "p.name()",
        "the label must come off the param, so renaming it cannot desync: {line}"
    );
    assert_eq!(
        args[3], "\"\"",
        "the group cards carry the captions; a sub-label here would restate the param: {line}"
    );
}

#[test]
fn the_call_site_declares_no_range_default_or_readout_of_its_own() {
    // Skip the module/item docs, which talk *about* these things.
    let body: String = KNOB_SRC
        .lines()
        .filter(|l| !l.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n");

    assert!(
        !body.contains("..="),
        "a range literal means the knob restates params.rs"
    );
    assert!(
        !body.contains("format!"),
        "a hand-rolled readout means the knob restates the param's formatter"
    );
    assert!(
        !body.contains("logarithmic"),
        "the curve is declared by params.rs; the call site must not pick one"
    );
    for hand_plumbed in [
        "min_plain",
        "max_plain",
        "default_plain",
        "get_plain",
        "set_plain",
        "display(",
    ] {
        assert!(
            !body.contains(hand_plumbed),
            "`{hand_plumbed}` means the call site is re-deriving what the helper reads \
             off the param — and it is how the declared skew got lost"
        );
    }
    for unit in [" dB", " ms", " Hz"] {
        assert!(
            !body.contains(unit),
            "unit `{unit}` is declared by params.rs; the call site must not repeat it"
        );
    }
}

/// The layout table addresses parameters by index, so the binding is
/// only as good as the index lookup: `float_at` must reach every
/// declared parameter and agree with the type-erased `param_at` the
/// CLAP bridge uses. A mismatch would mean the GUI and the host edit
/// different parameters.
#[test]
fn every_index_resolves_to_the_same_param_the_host_sees() {
    let p = GateParams::default();
    let mut ids = Vec::new();
    for index in 0..PARAM_COUNT {
        let float = p.float_at(index);
        assert_eq!(
            float.id(),
            p.param_at(index).id(),
            "index {index}: the editor and the host disagree about which param this is"
        );
        ids.push(float.id());
    }
    ids.sort_unstable();
    ids.dedup();
    assert_eq!(
        ids.len(),
        PARAM_COUNT,
        "the index table does not reach every declared parameter"
    );
}

// ---------------------------------------------------------------------------
// Behaviour: the arc is the parameter's own range
// ---------------------------------------------------------------------------

#[test]
fn knob_travel_is_exactly_the_declared_range() {
    let params = GateParams::default();
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
    let params = GateParams::default();
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
    let params = GateParams::default();
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
            // travel must land well below the arithmetic middle.
            // Without this the skew would be declared and thrown away,
            // which is exactly finding C5.
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
    let params = GateParams::default();
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
// Finding C5, pinned
// ---------------------------------------------------------------------------

/// The five parameters whose declared skew the old strip discarded,
/// with what half travel used to read (the arithmetic middle of a
/// linear arc) and what it reads now. Nothing in `params.rs` changed:
/// these are the values the declarations always asked for and never
/// got.
#[test]
fn the_five_skewed_knobs_no_longer_dial_linearly() {
    let p = GateParams::default();
    // (field, param, old linear half travel, new arc half travel)
    let cases: [(&str, &FloatParam, f32, f32); 5] = [
        ("ratio", &p.ratio, 10.5, 5.75),
        ("attack", &p.attack, 50.025, 14.12),
        ("hold", &p.hold, 250.0, 125.0),
        ("release", &p.release, 1002.5, 503.75),
        ("key_hpf", &p.key_hpf, 1000.0, 281.6),
    ];
    for (field, param, old_mid, new_mid) in cases {
        let range = param.range();
        assert!(
            matches!(range, FloatRange::Skewed { .. }),
            "{field} is listed under finding C5 but is no longer Skewed"
        );
        let linear_mid = (range.min() + range.max()) / 2.0;
        assert!(
            (linear_mid - old_mid).abs() < 0.01,
            "{field}: the old linear arc's middle was {linear_mid}, not the recorded {old_mid}"
        );
        let arc_mid = param.plain_at_normalized(0.5);
        assert!(
            (arc_mid - new_mid).abs() < 0.05 * new_mid.max(1.0),
            "{field}: half travel is {arc_mid}, expected about {new_mid}"
        );
    }

    // The other three are Linear, so the migration moves nothing.
    for (field, param) in [
        ("threshold", &p.threshold),
        ("range", &p.range),
        ("hysteresis", &p.hysteresis),
    ] {
        assert!(
            matches!(param.range(), FloatRange::Linear { .. }),
            "{field} became Skewed — params.rs changed under the migration"
        );
    }
}

/// The headline of finding C5. A 1 ms attack is the gate's own default
/// and the useful setting for anything percussive; on the old linear
/// arc it sat at under 1 % of the dial, inside the first pixel of
/// travel.
#[test]
fn a_one_millisecond_attack_is_dialable() {
    let p = GateParams::default();
    assert_eq!(p.attack.default_value(), 1.0, "the default is 1 ms");

    let range = p.attack.range();
    // What the old strip gave it: the plain linear position.
    let linear_travel = (1.0 - range.min()) / (range.max() - range.min());
    assert!(
        linear_travel < 0.01,
        "the old linear arc put 1 ms at {linear_travel} of the dial, which was the finding"
    );

    // What the declared skew gives it.
    let travel = p.attack.default_normalized();
    assert!(
        (0.15..0.25).contains(&travel),
        "1 ms should now sit around a fifth of the way up the dial, got {travel}"
    );

    // And the whole sub-millisecond region is reachable rather than
    // crammed into the first percent of travel.
    let at_five_percent = p.attack.plain_at_normalized(0.05);
    assert!(
        at_five_percent < 0.1,
        "5 % travel should still be down in the fast region, got {at_five_percent} ms"
    );
}

/// `key_hpf` declares `min: 0.0`, which is the "filter off" setting.
/// `widgets::knob`'s logarithmic path clamps a range start to 0.001 and
/// would have quietly moved the parameter's own bound; the param-bound
/// helper maps through a power law instead, so zero survives (ba todo
/// #1281 was required to handle this without a caller-side special
/// case).
#[test]
fn the_key_hpf_zero_lower_bound_survives_the_arc() {
    let p = GateParams::default();
    assert_eq!(p.key_hpf.range().min(), 0.0);
    assert_eq!(
        p.key_hpf.plain_at_normalized(0.0),
        0.0,
        "0 % travel must be exactly 0 Hz — the detector filter off, not 0.001 Hz"
    );
    assert_eq!(
        p.key_hpf.default_normalized(),
        0.0,
        "the default is the filter off, so the knob opens fully counter-clockwise"
    );

    // Dragging to the bottom writes the off value, not a clamped one.
    p.key_hpf.set_value(500.0);
    p.key_hpf.set_normalized(0.0);
    assert_eq!(p.key_hpf.value(), 0.0);
}

#[test]
fn no_readout_doubles_up_the_declared_unit() {
    let params = GateParams::default();
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
