//! Typing an exact value into a parameter (ba todo #1287, finding F5).
//!
//! `Param::apply_typed_entry` is what a knob's readout field and the
//! slider's value box hand their text to: parse through the parameter's
//! own `parse`, clamp through its own `set_plain`, refuse anything it
//! cannot read. Everything above it in `editor_widgets` is egui plumbing
//! — focus, Enter, Esc — so this is where the behaviour lives and where
//! it is tested.
//!
//! The finding: `Param::parse` and the bridge's `text_to_value` were
//! fully implemented *and* unit-tested, and nothing in the fleet called
//! them. There was no way to set a compressor to exactly -18.0 dB.

use std::sync::Arc;

use resonance_plugin::{formatters, BoolParam, FloatParam, FloatRange, IntParam, IntRange, Param};

/// The compressor's threshold: a dB param with a declared unit.
fn threshold() -> FloatParam {
    FloatParam::new(
        "threshold",
        "Threshold",
        -20.0,
        FloatRange::Linear {
            min: -60.0,
            max: 0.0,
        },
    )
    .with_unit(" dB")
    .with_value_to_string(formatters::v2s_f32_db(1))
}

/// A sidechain HPF: skewed, with a unit, and a formatter that switches to
/// kHz — the shape the DoD asks for.
fn sc_hpf() -> FloatParam {
    FloatParam::new(
        "sc_hpf_freq",
        "SC HPF",
        80.0,
        FloatRange::Skewed {
            min: 20.0,
            max: 20_000.0,
            factor: FloatRange::skew_factor(-2.0),
        },
    )
    .with_unit(" Hz")
    .with_value_to_string(formatters::v2s_f32_hz())
    .with_string_to_value(formatters::s2v_f32_hz())
}

#[test]
fn a_typed_value_with_a_unit_lands_exactly() {
    let p = threshold();

    assert!(p.apply_typed_entry("-18 dB"));
    assert_eq!(p.value(), -18.0);

    // The unit is optional on the way in, and surrounding space is not
    // the user's problem.
    assert!(p.apply_typed_entry("  -7.5  "));
    assert_eq!(p.value(), -7.5);
    assert!(p.apply_typed_entry("-30.0 dB"));
    assert_eq!(p.value(), -30.0);
}

#[test]
fn a_typed_value_on_a_skewed_param_lands_exactly() {
    // The point of typing: the value is whatever was asked for, not
    // wherever the arc's resolution happened to land.
    let p = sc_hpf();

    assert!(p.apply_typed_entry("1 kHz"));
    assert_eq!(p.value(), 1000.0);
    assert!(p.apply_typed_entry("440 Hz"));
    assert_eq!(p.value(), 440.0);
    assert!(p.apply_typed_entry("2.5k"));
    assert_eq!(p.value(), 2500.0);
}

#[test]
fn a_param_reads_back_what_it_displays() {
    let p = sc_hpf();

    for hz in [20.0_f32, 80.0, 440.0, 1000.0, 12_500.0, 20_000.0] {
        p.set_value(hz);
        let shown = p.display(p.value() as f64);
        assert!(
            p.apply_typed_entry(&shown),
            "the param must accept its own readout {shown:?}"
        );
        assert!(
            (p.value() - hz).abs() <= hz * 1e-3,
            "{hz} displayed as {shown:?} and came back as {}",
            p.value()
        );
    }
}

#[test]
fn an_unreadable_entry_is_refused_and_changes_nothing() {
    let p = threshold();
    p.set_value(-12.0);

    for text in ["", "   ", "loud", "-18 dBFS", "1/2"] {
        assert!(!p.apply_typed_entry(text), "{text:?} must be refused");
        assert_eq!(p.value(), -12.0, "{text:?} must leave the value alone");
    }
}

#[test]
fn an_out_of_range_entry_clamps_to_the_declared_bounds() {
    // Clamping rather than refusing: a user typing 200 on a knob that
    // stops at 0 dB means "as high as it goes".
    let p = threshold();

    assert!(p.apply_typed_entry("200 dB"));
    assert_eq!(p.value(), 0.0);
    assert!(p.apply_typed_entry("-500 dB"));
    assert_eq!(p.value(), -60.0);
}

#[test]
fn a_non_finite_entry_is_refused() {
    // A parser can legitimately produce infinity ("inf" parses as a
    // float); storing it would poison the DSP.
    let p = threshold();
    p.set_value(-12.0);

    for text in ["inf", "-inf", "NaN"] {
        assert!(!p.apply_typed_entry(text), "{text:?} must be refused");
        assert_eq!(p.value(), -12.0);
    }
}

#[test]
fn a_custom_parser_is_what_decides() {
    // The amp's gain knobs declare a dB<->linear pair; typing dB has to
    // reach the param as linear gain, not as the number typed.
    let p = FloatParam::new(
        "input_gain",
        "Input Gain",
        1.0,
        FloatRange::Skewed {
            min: 0.01,
            max: 4.0,
            factor: FloatRange::gain_skew_factor(-40.0, 12.0),
        },
    )
    .with_unit(" dB")
    .with_value_to_string(formatters::v2s_f32_gain_to_db(2))
    .with_string_to_value(formatters::s2v_f32_gain_to_db());

    assert!(p.apply_typed_entry("0 dB"));
    assert!((p.value() - 1.0).abs() < 1e-6, "0 dB is unity gain");

    assert!(p.apply_typed_entry("6 dB"));
    assert!((p.value() - 1.9953).abs() < 1e-3, "got {}", p.value());

    // "-inf" is the formatter's own word for silence, and it is finite
    // on the way back (a gain of zero), so it clamps into range.
    assert!(p.apply_typed_entry("-inf dB"));
    assert_eq!(p.value(), 0.01, "silence clamps to the declared minimum");
}

#[test]
fn typed_entry_works_for_stepped_params_too() {
    let taps = IntParam::new("taps", "Taps", 3, IntRange::Linear { min: 1, max: 8 });
    assert!(taps.apply_typed_entry("6"));
    assert_eq!(taps.value(), 6);
    assert!(taps.apply_typed_entry("99"));
    assert_eq!(taps.value(), 8, "out of range clamps");
    assert!(!taps.apply_typed_entry("six"));
    assert_eq!(taps.value(), 8);

    const SHAPES: &[&str] = &["Sine", "Square"];
    let shape = IntParam::new("shape", "Shape", 0, IntRange::Linear { min: 0, max: 1 })
        .with_choices(SHAPES);
    assert!(shape.apply_typed_entry("Square"));
    assert_eq!(shape.value(), 1);

    let bypass = BoolParam::new("bypass", "Bypass", false);
    assert!(bypass.apply_typed_entry("On"));
    assert!(bypass.value());
    assert!(!bypass.apply_typed_entry("perhaps"));
    assert!(bypass.value());
}

#[test]
fn a_param_without_a_formatter_still_accepts_plain_numbers() {
    let p = FloatParam::new(
        "size",
        "Size",
        0.5,
        FloatRange::Linear { min: 0.0, max: 1.0 },
    );

    assert!(p.apply_typed_entry("0.25"));
    assert_eq!(p.value(), 0.25);
}

#[test]
fn a_custom_parser_replaces_the_numeric_fallback() {
    // Same rule the display side follows: if a param declares how its
    // text is read, that is the only way in.
    let p = FloatParam::new(
        "mode",
        "Mode",
        0.0,
        FloatRange::Linear { min: 0.0, max: 1.0 },
    )
    .with_string_to_value(Arc::new(|s: &str| (s.trim() == "max").then_some(1.0)));

    assert!(p.apply_typed_entry("max"));
    assert_eq!(p.value(), 1.0);
    assert!(!p.apply_typed_entry("0.5"));
    assert_eq!(p.value(), 1.0);
}
