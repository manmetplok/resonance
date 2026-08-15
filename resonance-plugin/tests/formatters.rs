//! Unit tests for the shared value formatters (`src/formatters.rs`).
//!
//! These render every number a user sees on a plugin knob, and the `s2v_*`
//! halves parse what they type back in. A formatter/parser pair that
//! disagrees means typing back the displayed value silently changes it.

use resonance_plugin::{
    s2v_f32_gain_to_db, s2v_f32_hz, s2v_f32_percentage, s2v_f32_ratio, v2s_f32_db,
    v2s_f32_gain_to_db, v2s_f32_hz, v2s_f32_ms, v2s_f32_percent, v2s_f32_percentage, v2s_f32_ratio,
    v2s_f32_rounded,
};

// ---------------------------------------------------------------------------
// Linear gain <-> dB
// ---------------------------------------------------------------------------

#[test]
fn linear_gain_renders_as_decibels() {
    let f = v2s_f32_gain_to_db(1);

    assert_eq!(f(1.0), "0.0 dB");
    assert_eq!(f(2.0), "6.0 dB");
    assert_eq!(f(0.5), "-6.0 dB");
    assert_eq!(f(10.0), "20.0 dB");
    assert_eq!(f(0.1), "-20.0 dB");
}

#[test]
fn gain_decimals_are_configurable() {
    assert_eq!(v2s_f32_gain_to_db(0)(0.5), "-6 dB");
    assert_eq!(v2s_f32_gain_to_db(2)(0.5), "-6.02 dB");
}

#[test]
fn silence_renders_as_minus_infinity_rather_than_a_huge_number() {
    let f = v2s_f32_gain_to_db(1);

    // log10(0) is -inf; the formatter must special-case it instead of
    // printing "-inf dB" via float formatting (or NaN for a negative).
    assert_eq!(f(0.0), "-inf dB");
    assert_eq!(f(1e-9), "-inf dB");
    // The cutoff is 1e-6.
    assert_eq!(f(1e-7), "-inf dB");
    assert_ne!(f(1e-5), "-inf dB");
}

#[test]
fn db_text_parses_back_to_linear_gain() {
    let p = s2v_f32_gain_to_db();

    assert!((p("0.0 dB").unwrap() - 1.0).abs() < 1e-6);
    assert!((p("6.0 dB").unwrap() - 1.9952624).abs() < 1e-5);
    assert!((p("-6.0 dB").unwrap() - 0.5011872).abs() < 1e-6);
    // Whitespace and a missing space before the unit are tolerated.
    assert!((p("  -6.0dB ").unwrap() - 0.5011872).abs() < 1e-6);
    assert!((p("-6.0").unwrap() - 0.5011872).abs() < 1e-6);
    // The silence sentinel round-trips back to zero gain.
    assert_eq!(p("-inf"), Some(0.0));
    // Garbage is rejected rather than defaulting to something audible.
    assert_eq!(p("loud"), None);
    assert_eq!(p(""), None);
}

#[test]
fn gain_formatting_round_trips_through_its_parser() {
    let to_text = v2s_f32_gain_to_db(2);
    let to_value = s2v_f32_gain_to_db();

    for gain in [0.125_f32, 0.25, 0.5, 1.0, 2.0, 4.0] {
        let text = to_text(gain);
        let back = to_value(&text).unwrap_or_else(|| panic!("failed to parse {text:?}"));
        assert!(
            (back - gain).abs() <= gain * 1e-3,
            "{gain} rendered as {text:?} and parsed back as {back}"
        );
    }
}

// ---------------------------------------------------------------------------
// Percentages
// ---------------------------------------------------------------------------

#[test]
fn unit_floats_render_as_percentages() {
    assert_eq!(v2s_f32_percentage(0)(0.0), "0%");
    assert_eq!(v2s_f32_percentage(0)(0.5), "50%");
    assert_eq!(v2s_f32_percentage(0)(1.0), "100%");
    assert_eq!(v2s_f32_percentage(1)(0.333), "33.3%");
    // The mix-flavoured alias behaves identically.
    assert_eq!(v2s_f32_percent(1)(0.333), v2s_f32_percentage(1)(0.333));
    assert_eq!(v2s_f32_percent(0)(0.25), "25%");
}

#[test]
fn percentage_text_parses_back_to_a_unit_float() {
    let p = s2v_f32_percentage();

    assert_eq!(p("50%"), Some(0.5));
    assert_eq!(p("50"), Some(0.5));
    assert_eq!(p(" 100 % "), Some(1.0));
    assert_eq!(p("0%"), Some(0.0));
    assert!((p("33.3%").unwrap() - 0.333).abs() < 1e-6);
    assert_eq!(p("half"), None);
}

#[test]
fn percentage_round_trips_through_its_parser() {
    let to_text = v2s_f32_percentage(2);
    let to_value = s2v_f32_percentage();

    for v in [0.0_f32, 0.125, 0.5, 0.999, 1.0] {
        let text = to_text(v);
        let back = to_value(&text).unwrap_or_else(|| panic!("failed to parse {text:?}"));
        assert!((back - v).abs() < 1e-4, "{v} -> {text:?} -> {back}");
    }
}

// ---------------------------------------------------------------------------
// Direct-unit formatters
// ---------------------------------------------------------------------------

#[test]
fn plain_rounding_honours_the_decimal_count() {
    assert_eq!(v2s_f32_rounded(0)(1.5), "2");
    assert_eq!(v2s_f32_rounded(1)(1.25), "1.2");
    assert_eq!(v2s_f32_rounded(2)(1.23456), "1.23");
    assert_eq!(v2s_f32_rounded(3)(-0.5), "-0.500");
}

#[test]
fn direct_db_values_are_printed_not_converted() {
    // Distinct from v2s_f32_gain_to_db: the input is already in dB.
    assert_eq!(v2s_f32_db(1)(-6.0), "-6.0 dB");
    assert_eq!(v2s_f32_db(1)(0.0), "0.0 dB");
    assert_eq!(v2s_f32_db(0)(12.4), "12 dB");
    assert_eq!(v2s_f32_db(2)(-60.0), "-60.00 dB");
}

#[test]
fn millisecond_values_are_printed_with_their_unit() {
    assert_eq!(v2s_f32_ms(0)(12.0), "12 ms");
    assert_eq!(v2s_f32_ms(1)(0.05), "0.1 ms");
    assert_eq!(v2s_f32_ms(2)(250.0), "250.00 ms");
}

#[test]
fn frequencies_switch_to_kilohertz_above_a_kilohertz() {
    let f = v2s_f32_hz();

    assert_eq!(f(20.0), "20 Hz");
    assert_eq!(f(440.0), "440 Hz");
    assert_eq!(f(999.0), "999 Hz");
    // The switchover is inclusive at 1000.
    assert_eq!(f(1000.0), "1.00 kHz");
    assert_eq!(f(1500.0), "1.50 kHz");
    assert_eq!(f(20_000.0), "20.00 kHz");
}

#[test]
fn frequency_text_parses_back_to_hertz() {
    let f = s2v_f32_hz();

    assert_eq!(f("440"), Some(440.0));
    assert_eq!(f("440 Hz"), Some(440.0));
    assert_eq!(f(" 440hz "), Some(440.0));
    // The kHz form the formatter itself produces above 1 kHz.
    assert_eq!(f("1.50 kHz"), Some(1500.0));
    assert_eq!(f("1.5KHZ"), Some(1500.0));
    // And the shorthand a user reaches for.
    assert_eq!(f("1.2k"), Some(1200.0));
    assert_eq!(f("nope"), None);
    assert_eq!(f(""), None);
    assert_eq!(f("Hz"), None);
}

#[test]
fn frequency_formatting_round_trips_through_its_parser() {
    // The pair has to agree on both sides of the kHz switchover, or a
    // typed-back readout silently moves the value (ba todo #1287).
    let to_text = v2s_f32_hz();
    let to_value = s2v_f32_hz();

    for hz in [20.0_f32, 80.0, 440.0, 999.0, 1000.0, 1500.0, 20_000.0] {
        let back = to_value(&to_text(hz)).expect("the parser must read its own output");
        assert!(
            (back - hz).abs() <= hz * 1e-3,
            "{hz} Hz rendered as {:?} and came back as {back}",
            to_text(hz)
        );
    }
}

#[test]
fn compression_ratios_render_against_one() {
    let f = v2s_f32_ratio();

    assert_eq!(f(1.0), "1.0:1");
    assert_eq!(f(4.0), "4.0:1");
    assert_eq!(f(2.5), "2.5:1");
    assert_eq!(f(20.0), "20.0:1");
}

#[test]
fn ratio_text_parses_in_both_the_written_and_the_bare_form() {
    let f = s2v_f32_ratio();

    assert_eq!(f("4.0:1"), Some(4.0));
    assert_eq!(f(" 2.5 : 1 "), Some(2.5));
    assert_eq!(f("4"), Some(4.0), "a bare number is the ratio");
    // `4:2` is not a form this parameter has a value for — refuse it
    // rather than silently reading it as 4.
    assert_eq!(f("4:2"), None);
    assert_eq!(f("nope"), None);
}
