//! Unit tests for the parameter system (`src/param.rs`).
//!
//! Every plugin in the family declares its knobs as `FloatParam` /
//! `IntParam` / `BoolParam`, and the CLAP bridge drives them exclusively
//! through the `Param` trait's plain-value API (`get_plain` / `set_plain`).
//! Host automation, preset loads and project loads all funnel through
//! `set_plain`, so its clamping and non-finite guards are the last line of
//! defence between a corrupt value and the DSP.

use std::sync::Arc;

use resonance_plugin::{stable_hash, BoolParam, FloatParam, FloatRange, IntParam, IntRange, Param};

fn linear(min: f32, max: f32) -> FloatRange {
    FloatRange::Linear { min, max }
}

fn skewed(min: f32, max: f32, factor: f32) -> FloatRange {
    FloatRange::Skewed { min, max, factor }
}

// ---------------------------------------------------------------------------
// FloatParam
// ---------------------------------------------------------------------------

#[test]
fn float_param_reports_its_declared_metadata() {
    let p = FloatParam::new("mix", "Mix", 0.25, linear(0.0, 1.0));

    assert_eq!(p.id(), "mix");
    assert_eq!(p.name(), "Mix");
    assert_eq!(p.default_plain(), 0.25);
    assert_eq!(p.min_plain(), 0.0);
    assert_eq!(p.max_plain(), 1.0);
    // A float param is continuous, visible, and never stepped.
    assert!(!p.is_stepped());
    assert!(!p.is_hidden());
    // A freshly constructed param sits at its default.
    assert_eq!(p.value(), 0.25);
    assert_eq!(p.get_plain(), 0.25);
}

#[test]
fn float_param_round_trips_plain_values_across_the_range() {
    let p = FloatParam::new("freq", "Frequency", 1000.0, linear(20.0, 20_000.0));

    // Extremes and interior points survive a set/get round-trip through
    // the atomic f32 storage (within f32 precision).
    for plain in [20.0_f64, 20.5, 440.0, 1000.0, 19_999.0, 20_000.0] {
        p.set_plain(plain);
        let back = p.get_plain();
        assert!(
            (back - plain).abs() <= plain.abs() * 1e-6 + 1e-6,
            "round-trip of {plain} came back as {back}"
        );
    }
}

#[test]
fn float_param_round_trips_on_a_skewed_range() {
    // The skew only affects the normalized (UI) mapping — plain values
    // must still round-trip untouched.
    let p = FloatParam::new("size", "Size", 0.5, skewed(0.1, 10.0, -2.0));

    for plain in [0.1_f64, 0.5, 1.0, 5.0, 10.0] {
        p.set_plain(plain);
        assert!((p.get_plain() - plain).abs() <= plain * 1e-6 + 1e-6);
    }
    assert_eq!(p.min_plain(), 0.1_f32 as f64);
    assert_eq!(p.max_plain(), 10.0);
}

#[test]
fn float_param_set_plain_clamps_out_of_range_values() {
    let p = FloatParam::new("gain", "Gain", 0.0, linear(-24.0, 24.0));

    p.set_plain(1000.0);
    assert_eq!(p.get_plain(), 24.0);

    p.set_plain(-1000.0);
    assert_eq!(p.get_plain(), -24.0);

    // Exactly on the bounds is not clamped away.
    p.set_plain(24.0);
    assert_eq!(p.get_plain(), 24.0);
    p.set_plain(-24.0);
    assert_eq!(p.get_plain(), -24.0);

    // Infinity is *rejected* rather than clamped onto the bound — the
    // non-finite guard runs before the clamp.
    p.set_plain(0.0);
    p.set_plain(f64::INFINITY);
    assert_eq!(p.get_plain(), 0.0, "infinity must be rejected, not clamped");
}

#[test]
fn float_param_ignores_non_finite_values() {
    let p = FloatParam::new("gain", "Gain", 3.0, linear(-24.0, 24.0));

    for bad in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        p.set_plain(bad);
        assert_eq!(
            p.get_plain(),
            3.0,
            "{bad} must leave the param at its previous value"
        );
    }

    // The direct f32 setter guards the same way.
    for bad in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
        p.set_value(bad);
        assert_eq!(p.value(), 3.0);
    }
}

#[test]
fn float_param_default_display_uses_two_decimals_and_the_unit() {
    let bare = FloatParam::new("x", "X", 0.0, linear(0.0, 1.0));
    assert_eq!(bare.display(0.5), "0.50");

    let with_unit = FloatParam::new("f", "F", 0.0, linear(0.0, 20_000.0)).with_unit(" Hz");
    assert_eq!(with_unit.display(440.0), "440.00 Hz");
}

#[test]
fn float_param_custom_formatter_wins_and_the_unit_is_not_doubled() {
    let p = FloatParam::new("g", "G", 0.0, linear(0.0, 2.0))
        .with_unit(" dB")
        .with_value_to_string(Arc::new(|v: f32| format!("{:.1} dB", 20.0 * v.log10())));

    // The formatter already emits the unit, so it must not be appended twice.
    assert_eq!(p.display(1.0), "0.0 dB");
    assert!(!p.display(1.0).ends_with(" dB dB"));

    // A formatter that omits the unit gets it appended.
    let q = FloatParam::new("g", "G", 0.0, linear(0.0, 2.0))
        .with_unit("%")
        .with_value_to_string(Arc::new(|v: f32| format!("{:.0}", v * 100.0)));
    assert_eq!(q.display(0.5), "50%");
}

#[test]
fn float_param_default_parse_strips_the_unit() {
    let p = FloatParam::new("f", "F", 0.0, linear(0.0, 20_000.0)).with_unit(" Hz");

    assert_eq!(p.parse("440.0 Hz"), Some(440.0));
    assert_eq!(p.parse("  440.0 Hz  "), Some(440.0));
    assert_eq!(p.parse("440"), Some(440.0));
    assert_eq!(p.parse("not a number"), None);
    assert_eq!(p.parse(""), None);
}

#[test]
fn float_param_custom_parser_wins() {
    let p = FloatParam::new("g", "G", 0.0, linear(0.0, 2.0)).with_string_to_value(Arc::new(
        |s: &str| if s == "unity" { Some(1.0) } else { None },
    ));

    assert_eq!(p.parse("unity"), Some(1.0));
    // The custom parser replaces the numeric fallback entirely.
    assert_eq!(p.parse("1.0"), None);
}

#[test]
fn float_param_display_parse_round_trip() {
    let p = FloatParam::new("mix", "Mix", 0.0, linear(0.0, 1.0)).with_unit("%");

    p.set_plain(0.5);
    let text = p.display(p.get_plain());
    let parsed = p
        .parse(&text)
        .expect("the param must parse its own display");
    assert!((parsed - 0.5).abs() < 1e-6);
}

#[test]
fn hidden_float_params_report_hidden() {
    let p = FloatParam::new("internal", "Internal", 0.0, linear(0.0, 1.0)).hidden();
    assert!(p.is_hidden());
    // Hiding does not change any other behaviour.
    p.set_plain(0.7);
    assert!((p.get_plain() - 0.7).abs() < 1e-6);
}

// ---------------------------------------------------------------------------
// IntParam
// ---------------------------------------------------------------------------

#[test]
fn int_param_reports_its_declared_metadata() {
    let p = IntParam::new("taps", "Taps", 3, IntRange::Linear { min: 1, max: 8 });

    assert_eq!(p.id(), "taps");
    assert_eq!(p.name(), "Taps");
    assert_eq!(p.default_plain(), 3.0);
    assert_eq!(p.min_plain(), 1.0);
    assert_eq!(p.max_plain(), 8.0);
    assert!(p.is_stepped());
    assert!(!p.is_hidden());
    assert_eq!(p.value(), 3);
}

#[test]
fn int_param_round_trips_every_step() {
    let p = IntParam::new("mode", "Mode", 0, IntRange::Linear { min: -4, max: 4 });

    for step in -4..=4 {
        p.set_plain(step as f64);
        assert_eq!(p.value(), step);
        assert_eq!(p.get_plain(), step as f64);
    }
}

#[test]
fn int_param_set_plain_rounds_to_nearest_and_clamps() {
    let p = IntParam::new("taps", "Taps", 1, IntRange::Linear { min: 1, max: 8 });

    // Rounds, not truncates — a preset written by a float-valued host
    // must land on the nearest step.
    p.set_plain(3.7);
    assert_eq!(p.value(), 4);
    p.set_plain(3.2);
    assert_eq!(p.value(), 3);
    p.set_plain(-0.5);
    assert_eq!(p.value(), 1, "below the minimum clamps to min");
    p.set_plain(1e12);
    assert_eq!(p.value(), 8, "far above the maximum clamps to max");
}

#[test]
fn int_param_ignores_non_finite_values() {
    let p = IntParam::new("taps", "Taps", 5, IntRange::Linear { min: 1, max: 8 });

    for bad in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        p.set_plain(bad);
        assert_eq!(p.value(), 5, "{bad} must leave the param untouched");
    }
}

#[test]
fn int_param_display_and_parse() {
    let p = IntParam::new("taps", "Taps", 1, IntRange::Linear { min: 1, max: 8 });

    assert_eq!(p.display(4.0), "4");
    assert_eq!(p.display(4.6), "5", "display rounds like set_plain does");
    assert_eq!(p.parse("6"), Some(6.0));
    assert_eq!(p.parse(" 6 "), Some(6.0));
    assert_eq!(p.parse("6.5"), None, "only whole numbers parse");
    assert_eq!(p.parse("nope"), None);
}

#[test]
fn hidden_int_params_report_hidden() {
    let p = IntParam::new("x", "X", 0, IntRange::Linear { min: 0, max: 1 }).hidden();
    assert!(p.is_hidden());
}

// ---------------------------------------------------------------------------
// BoolParam
// ---------------------------------------------------------------------------

#[test]
fn bool_param_reports_its_declared_metadata() {
    let p = BoolParam::new("bypass", "Bypass", true);

    assert_eq!(p.id(), "bypass");
    assert_eq!(p.name(), "Bypass");
    assert_eq!(p.default_plain(), 1.0);
    assert_eq!(p.min_plain(), 0.0);
    assert_eq!(p.max_plain(), 1.0);
    assert!(p.is_stepped());
    assert!(p.value());

    let off = BoolParam::new("bypass", "Bypass", false);
    assert_eq!(off.default_plain(), 0.0);
    assert!(!off.value());
}

#[test]
fn bool_param_thresholds_plain_values_at_a_half() {
    let p = BoolParam::new("on", "On", false);

    for (plain, expected) in [
        (0.0, false),
        (0.25, false),
        (0.49999, false),
        (0.5, true),
        (0.75, true),
        (1.0, true),
        // Out-of-range values still resolve sensibly rather than panicking.
        (-3.0, false),
        (7.0, true),
    ] {
        p.set_plain(plain);
        assert_eq!(p.value(), expected, "set_plain({plain})");
        assert_eq!(p.get_plain(), if expected { 1.0 } else { 0.0 });
    }
}

#[test]
fn bool_param_display_and_parse() {
    let p = BoolParam::new("on", "On", false);

    assert_eq!(p.display(1.0), "On");
    assert_eq!(p.display(0.0), "Off");
    assert_eq!(p.display(0.5), "On");

    for on in ["on", "On", "TRUE", "1", "yes", " Yes "] {
        assert_eq!(p.parse(on), Some(1.0), "parse({on:?})");
    }
    for off in ["off", "Off", "false", "0", "no"] {
        assert_eq!(p.parse(off), Some(0.0), "parse({off:?})");
    }
    assert_eq!(p.parse("maybe"), None);
    assert_eq!(p.parse(""), None);
}

#[test]
fn bool_param_display_parse_round_trip() {
    let p = BoolParam::new("on", "On", false);

    for state in [false, true] {
        p.set_value(state);
        let text = p.display(p.get_plain());
        assert_eq!(p.parse(&text), Some(if state { 1.0 } else { 0.0 }));
    }
}

// ---------------------------------------------------------------------------
// CLAP id derivation
// ---------------------------------------------------------------------------

#[test]
fn clap_ids_are_stable_and_derived_from_the_string_id() {
    let p = FloatParam::new("mix", "Mix", 0.0, linear(0.0, 1.0));
    let q = FloatParam::new("mix", "A Different Name", 1.0, linear(-1.0, 5.0));

    // The id follows the *string id* only — renaming a param or changing
    // its range must not move a host's automation lane.
    assert_eq!(p.clap_id(), q.clap_id());
    assert_eq!(p.clap_id(), stable_hash("mix"));

    // Different ids hash apart (these are the ones plugins actually use).
    let ids = [
        "mix", "gain", "drive", "tone", "bypass", "taps", "feedback", "size",
    ];
    for (i, a) in ids.iter().enumerate() {
        for b in &ids[i + 1..] {
            assert_ne!(stable_hash(a), stable_hash(b), "{a} and {b} collide");
        }
    }
}

#[test]
fn stable_hash_matches_the_fnv1a_reference() {
    // Pinned so a refactor of the hash can't silently rewrite every
    // saved automation lane in every user project.
    assert_eq!(stable_hash(""), 2166136261);
    assert_eq!(stable_hash("a"), 0xe40c292c);
    assert_eq!(stable_hash("foobar"), 0xbf9cf968);
}

// ---------------------------------------------------------------------------
// Trait-object use (how the bridge actually sees params)
// ---------------------------------------------------------------------------

#[test]
fn params_are_usable_as_trait_objects() {
    let f = FloatParam::new("mix", "Mix", 0.5, linear(0.0, 1.0));
    let i = IntParam::new("taps", "Taps", 2, IntRange::Linear { min: 1, max: 8 });
    let b = BoolParam::new("on", "On", true);
    let params: Vec<&dyn Param> = vec![&f, &i, &b];

    let ids: Vec<&str> = params.iter().map(|p| p.id()).collect();
    assert_eq!(ids, ["mix", "taps", "on"]);

    let stepped: Vec<bool> = params.iter().map(|p| p.is_stepped()).collect();
    assert_eq!(stepped, [false, true, true]);

    for p in &params {
        // Every param must accept its own default without being clamped.
        let d = p.default_plain();
        p.set_plain(d);
        assert_eq!(p.get_plain(), d, "{} lost its default", p.id());
        assert!(d >= p.min_plain() && d <= p.max_plain());
    }
}
