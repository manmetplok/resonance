//! The contract between a `FloatParam` and the control that edits it
//! (ba todo #1281, finding F4).
//!
//! `editor_widgets::float_knob` / `float_slider` work in normalized
//! 0..1 travel and map it through the param's own range with
//! `FloatParam::normalized_value` / `plain_at_normalized` /
//! `set_normalized` / `default_normalized`. Those four are exactly what
//! the widgets call, so testing them here tests the binding without a
//! GUI — the widget bodies add nothing but the egui plumbing.
//!
//! What this file is guarding against: before #1281 the helper took
//! range, default and the value text as *arguments*, and four editors
//! had drifted from their own `params.rs` (an IR mix knob defaulting to
//! 50 % against a param declaring 100 %, an amp output knob to 1.0
//! against 0.5). It also hardcoded a linear arc, so every declared
//! `FloatRange::Skewed` was thrown away.

use resonance_plugin::{FloatParam, FloatRange, Param};

const EPS: f32 = 1e-4;

fn assert_close(actual: f32, expected: f32, what: &str) {
    let tolerance = EPS * expected.abs().max(1.0);
    assert!(
        (actual - expected).abs() <= tolerance,
        "{what}: expected {expected}, got {actual}"
    );
}

/// A filter cutoff of the shape wavetable and the gate declare: a wide
/// span bunched toward the low end.
fn cutoff() -> FloatParam {
    FloatParam::new(
        "cutoff",
        "Cutoff",
        1_000.0,
        FloatRange::Skewed {
            min: 20.0,
            max: 20_000.0,
            factor: FloatRange::skew_factor(-3.0),
        },
    )
}

#[test]
fn a_skewed_knob_reaches_the_params_own_values_at_0_50_and_100_percent() {
    let p = cutoff();

    // 0 % and 100 % of the arc are the parameter's declared bounds — a
    // control can never reach past them, nor stop short of them.
    p.set_normalized(0.0);
    assert_close(p.value(), 20.0, "0% of the arc");
    p.set_normalized(1.0);
    assert_close(p.value(), 20_000.0, "100% of the arc");

    // 50 % is whatever the declared skew says it is, not the arithmetic
    // midpoint (factor -3 => plain = min + (max-min) * 0.5^8). The point
    // of the assertion is that it is the *param's* answer, so ask the
    // range itself and check the two agree.
    p.set_normalized(0.5);
    assert_close(p.value(), p.range().denormalize(0.5), "50% of the arc");

    // And that it is genuinely bunched: half-way up a 20 Hz..20 kHz
    // cutoff must land in the low hundreds, not at 10 kHz. This is the
    // W4 complaint — a linear arc put everything under 2 kHz in the
    // first 10 % of the travel.
    let mid = p.value();
    assert!(
        (20.0..2_000.0).contains(&mid),
        "50% of a -3-skewed 20..20k arc should sit low, got {mid} Hz"
    );
}

#[test]
fn the_arc_position_and_the_value_agree_in_both_directions() {
    // A reverb-decay-shaped param: a moderate skew, where the curve is
    // invertible in f32 across the whole travel. (A very steep skew —
    // `cutoff`'s factor -3 is an 8th power — collapses the bottom of the
    // dial onto the minimum in f32, so a travel round-trip there is a
    // statement about float precision, not about the binding. The plain
    // round-trip below is the part that matters for those.)
    let p = FloatParam::new(
        "decay",
        "Decay",
        2.0,
        FloatRange::Skewed {
            min: 0.1,
            max: 30.0,
            factor: FloatRange::skew_factor(-1.5),
        },
    );

    for step in 0..=20 {
        let travel = step as f32 / 20.0;
        p.set_normalized(travel);
        assert!(
            (p.normalized_value() - travel).abs() <= 1e-3,
            "travel {travel} read back as {}",
            p.normalized_value()
        );
        assert_close(p.value(), p.plain_at_normalized(travel), "plain at travel");
    }

    // The steep case: the value the widget writes is always the value
    // the range says that travel means.
    let steep = cutoff();
    for step in 0..=20 {
        let travel = step as f32 / 20.0;
        steep.set_normalized(travel);
        assert_close(
            steep.value(),
            steep.plain_at_normalized(travel),
            "steep skew: plain at travel",
        );
    }
}

#[test]
fn the_arc_is_monotonic_and_never_leaves_the_declared_bounds() {
    let p = cutoff();

    let mut previous = f32::MIN;
    for step in 0..=100 {
        p.set_normalized(step as f32 / 100.0);
        let v = p.value();
        assert!(v.is_finite(), "travel {step}% produced {v}");
        assert!(
            (20.0..=20_000.0).contains(&v),
            "travel {step}% produced {v}, outside the declared range"
        );
        assert!(v >= previous, "travel {step}% went backwards to {v}");
        previous = v;
    }
}

#[test]
fn resetting_to_the_default_position_writes_the_declared_default_exactly() {
    // Double-clicking a knob hands back `default_normalized`; the round
    // trip through a power curve must not leave 1.9999998 where the
    // parameter declares 2.0.
    let decay = FloatParam::new(
        "decay",
        "Decay",
        2.0,
        FloatRange::Skewed {
            min: 0.1,
            max: 30.0,
            factor: FloatRange::skew_factor(-1.5),
        },
    );

    decay.set_normalized(1.0);
    assert_ne!(decay.value(), 2.0);

    decay.set_normalized(decay.default_normalized());
    assert_eq!(
        decay.value(),
        2.0,
        "a reset must land on the declared default bit for bit"
    );
}

#[test]
fn a_knob_on_a_zero_bounded_param_can_reach_zero() {
    // The gate's key_hpf: min 0.0 on a skewed range. The old
    // `logarithmic: true` widget path clamped the low end to 0.001 and
    // could never produce the parameter's own minimum.
    let key_hpf = FloatParam::new(
        "key_hpf",
        "Key HPF",
        100.0,
        FloatRange::Skewed {
            min: 0.0,
            max: 2_000.0,
            factor: FloatRange::skew_factor(-2.0),
        },
    );

    key_hpf.set_normalized(0.0);
    assert_eq!(key_hpf.value(), 0.0, "0% of the arc must reach 0 Hz");
    key_hpf.set_normalized(1.0);
    assert_close(key_hpf.value(), 2_000.0, "100% of the arc");
}

#[test]
fn the_control_cannot_be_pushed_past_the_declared_bounds() {
    let p = FloatParam::new("mix", "Mix", 1.0, FloatRange::Linear { min: 0.0, max: 1.0 });

    p.set_normalized(-3.0);
    assert_eq!(p.value(), 0.0);
    p.set_normalized(7.0);
    assert_eq!(p.value(), 1.0);
    p.set_normalized(f32::NAN);
    assert_eq!(p.value(), 0.0);
}

#[test]
fn the_readout_comes_from_the_params_own_formatter() {
    // What the knob prints under the dial is `Param::display` of the
    // current value — so the editor cannot invent a unit the host does
    // not see. (An amp knob used to compute its own dB readout while
    // the param already declared a gain-to-dB formatter.)
    let p = FloatParam::new(
        "mix",
        "Mix",
        0.25,
        FloatRange::Linear { min: 0.0, max: 1.0 },
    )
    .with_unit("%")
    .with_value_to_string(resonance_plugin::formatters::v2s_f32_percentage(0));

    p.set_normalized(0.5);
    assert_eq!(p.display(p.value() as f64), "50%");
}

#[test]
fn an_int_choice_without_a_choice_table_lists_its_whole_range() {
    use resonance_plugin::editor_widgets::int_choice_options;
    use resonance_plugin::{IntParam, IntRange};

    // A plain IntParam — no `with_choices` — used to draw an empty
    // dropdown: the combo iterated the (missing) label table.
    let plain = IntParam::new("voices", "Voices", 2, IntRange::Linear { min: 1, max: 4 });
    let got: Vec<i32> = int_choice_options(&plain).iter().map(|(v, _)| *v).collect();
    assert_eq!(got, vec![1, 2, 3, 4]);
    for (v, label) in int_choice_options(&plain) {
        assert_eq!(label, plain.display(v as f64), "labels come from the param");
    }

    // With a table, the table's labels, from the range's minimum.
    let table: &'static [&'static str] = &["Off", "Soft", "Hard"];
    let choice =
        IntParam::new("mode", "Mode", 0, IntRange::Linear { min: 0, max: 2 }).with_choices(table);
    let got = int_choice_options(&choice);
    assert_eq!(
        got,
        vec![(0, "Off".to_string()), (1, "Soft".to_string()), (2, "Hard".to_string())]
    );
}
