//! Unit tests for the parameter range maths (`src/range.rs`).
//!
//! `normalize` is what maps a plain param value onto the 0..1 travel of a
//! slider/knob, so its endpoint behaviour, clamping and skew curve decide
//! where every control in every editor sits.

use resonance_plugin::{FloatRange, IntRange};

/// Tolerance for the f32 curve maths.
const EPS: f32 = 1e-5;

fn assert_close(actual: f32, expected: f32, what: &str) {
    assert!(
        (actual - expected).abs() <= EPS,
        "{what}: expected {expected}, got {actual}"
    );
}

// ---------------------------------------------------------------------------
// FloatRange::Linear
// ---------------------------------------------------------------------------

#[test]
fn linear_range_reports_its_bounds() {
    let r = FloatRange::Linear {
        min: -24.0,
        max: 12.0,
    };
    assert_eq!(r.min(), -24.0);
    assert_eq!(r.max(), 12.0);
}

#[test]
fn linear_normalize_maps_the_endpoints_and_midpoint() {
    let r = FloatRange::Linear {
        min: 20.0,
        max: 20_000.0,
    };

    assert_close(r.normalize(20.0), 0.0, "min");
    assert_close(r.normalize(20_000.0), 1.0, "max");
    assert_close(r.normalize(10_010.0), 0.5, "midpoint");
}

#[test]
fn linear_normalize_handles_a_negative_range() {
    let r = FloatRange::Linear {
        min: -24.0,
        max: 24.0,
    };

    assert_close(r.normalize(-24.0), 0.0, "min");
    assert_close(r.normalize(0.0), 0.5, "centre");
    assert_close(r.normalize(24.0), 1.0, "max");
}

#[test]
fn linear_normalize_clamps_out_of_range_input() {
    let r = FloatRange::Linear { min: 0.0, max: 1.0 };

    assert_eq!(r.normalize(-100.0), 0.0);
    assert_eq!(r.normalize(100.0), 1.0);
    assert_eq!(r.normalize(f32::NEG_INFINITY), 0.0);
    assert_eq!(r.normalize(f32::INFINITY), 1.0);
}

#[test]
fn linear_normalize_is_monotonic() {
    let r = FloatRange::Linear {
        min: -6.0,
        max: 18.0,
    };

    let mut previous = -1.0_f32;
    for step in 0..=100 {
        let plain = -6.0 + 24.0 * step as f32 / 100.0;
        let n = r.normalize(plain);
        assert!(
            n >= previous - EPS,
            "normalize({plain}) = {n} went backwards from {previous}"
        );
        assert!(
            (0.0..=1.0).contains(&n),
            "normalize({plain}) = {n} escaped 0..1"
        );
        previous = n;
    }
}

#[test]
fn a_degenerate_linear_range_normalizes_to_zero() {
    // Guard against a division by zero when a plugin declares a
    // fixed-value param; every input must map to 0 rather than NaN.
    let r = FloatRange::Linear { min: 5.0, max: 5.0 };

    for probe in [-1.0_f32, 0.0, 5.0, 100.0] {
        let n = r.normalize(probe);
        assert!(n.is_finite(), "normalize({probe}) = {n} is not finite");
        assert_eq!(n, 0.0);
    }
}

// ---------------------------------------------------------------------------
// FloatRange::Skewed
// ---------------------------------------------------------------------------

#[test]
fn skewed_range_reports_its_bounds() {
    let r = FloatRange::Skewed {
        min: 0.1,
        max: 10.0,
        factor: -1.5,
    };
    assert_eq!(r.min(), 0.1);
    assert_eq!(r.max(), 10.0);
}

#[test]
fn skewed_normalize_preserves_the_endpoints() {
    for factor in [-3.0_f32, -1.0, 0.0, 1.0, 3.0] {
        let r = FloatRange::Skewed {
            min: 20.0,
            max: 20_000.0,
            factor,
        };
        assert_close(r.normalize(20.0), 0.0, "min");
        assert_close(r.normalize(20_000.0), 1.0, "max");
    }
}

#[test]
fn a_zero_skew_factor_is_exactly_linear() {
    let skewed = FloatRange::Skewed {
        min: 0.0,
        max: 100.0,
        factor: 0.0,
    };
    let linear = FloatRange::Linear {
        min: 0.0,
        max: 100.0,
    };

    for step in 0..=20 {
        let plain = step as f32 * 5.0;
        assert_close(
            skewed.normalize(plain),
            linear.normalize(plain),
            "zero skew",
        );
    }
}

#[test]
fn the_skew_curve_matches_its_documented_power_law() {
    // normalized = linear^(2^-factor)
    let negative = FloatRange::Skewed {
        min: 0.0,
        max: 1.0,
        factor: -1.0,
    };
    // 2^1 = 2 → linear^2, so the midpoint lands at 0.25.
    assert_close(negative.normalize(0.5), 0.25, "factor -1 at midpoint");
    assert_close(negative.normalize(0.25), 0.0625, "factor -1 at quarter");

    let positive = FloatRange::Skewed {
        min: 0.0,
        max: 1.0,
        factor: 1.0,
    };
    // 2^-1 = 0.5 → sqrt(linear).
    assert_close(
        positive.normalize(0.5),
        0.5_f32.sqrt(),
        "factor 1 at midpoint",
    );
    assert_close(positive.normalize(0.25), 0.5, "factor 1 at quarter");
}

#[test]
fn opposite_skew_factors_bracket_the_linear_mapping() {
    let linear = FloatRange::Linear {
        min: 20.0,
        max: 20_000.0,
    };
    let negative = FloatRange::Skewed {
        min: 20.0,
        max: 20_000.0,
        factor: -2.0,
    };
    let positive = FloatRange::Skewed {
        min: 20.0,
        max: 20_000.0,
        factor: 2.0,
    };

    for step in 1..20 {
        let plain = 20.0 + (20_000.0 - 20.0) * step as f32 / 20.0;
        let l = linear.normalize(plain);
        assert!(
            negative.normalize(plain) < l,
            "negative skew must sit below linear at {plain}"
        );
        assert!(
            positive.normalize(plain) > l,
            "positive skew must sit above linear at {plain}"
        );
    }
}

#[test]
fn skewed_normalize_is_monotonic_and_clamped() {
    for factor in [-3.0_f32, -1.0, 1.0, 3.0] {
        let r = FloatRange::Skewed {
            min: 0.1,
            max: 10.0,
            factor,
        };

        assert_eq!(r.normalize(-5.0), 0.0, "factor {factor} below min");
        assert_eq!(r.normalize(1e9), 1.0, "factor {factor} above max");

        let mut previous = -1.0_f32;
        for step in 0..=100 {
            let plain = 0.1 + (10.0 - 0.1) * step as f32 / 100.0;
            let n = r.normalize(plain);
            assert!(n.is_finite(), "factor {factor}: normalize({plain}) = {n}");
            assert!(
                n >= previous - EPS,
                "factor {factor}: normalize({plain}) = {n} went backwards from {previous}"
            );
            assert!((0.0..=1.0).contains(&n));
            previous = n;
        }
    }
}

#[test]
fn a_degenerate_skewed_range_normalizes_to_zero() {
    let r = FloatRange::Skewed {
        min: 2.0,
        max: 2.0,
        factor: -2.0,
    };

    for probe in [0.0_f32, 2.0, 50.0] {
        assert_eq!(r.normalize(probe), 0.0);
    }
}

// ---------------------------------------------------------------------------
// Skew-factor helpers
// ---------------------------------------------------------------------------

#[test]
fn skew_factor_is_the_identity() {
    for f in [-2.5_f32, 0.0, 1.75] {
        assert_eq!(FloatRange::skew_factor(f), f);
    }
}

#[test]
fn gain_skew_factor_bunches_toward_the_low_end() {
    // -60..+12 dB, the usual mixer-fader span.
    let f = FloatRange::gain_skew_factor(-60.0, 12.0);
    assert_close(f, -2.0 * 60.0 / 72.0, "gain skew");
    assert!(f < 0.0, "a gain fader must skew negative");

    // A symmetric trim range skews less than a fader with a long tail.
    let trim = FloatRange::gain_skew_factor(-12.0, 12.0);
    assert!(trim > f, "the shorter tail must skew less");
}

#[test]
fn gain_skew_factor_survives_a_degenerate_span() {
    assert_eq!(FloatRange::gain_skew_factor(0.0, 0.0), 0.0);
    assert_eq!(FloatRange::gain_skew_factor(-6.0, -6.0), 0.0);
}

// ---------------------------------------------------------------------------
// IntRange
// ---------------------------------------------------------------------------

#[test]
fn int_range_reports_its_bounds() {
    let r = IntRange::Linear { min: 1, max: 8 };
    assert_eq!(r.min(), 1);
    assert_eq!(r.max(), 8);
}

#[test]
fn int_normalize_maps_every_step_evenly() {
    let r = IntRange::Linear { min: 0, max: 4 };

    assert_eq!(r.normalize(0), 0.0);
    assert_eq!(r.normalize(1), 0.25);
    assert_eq!(r.normalize(2), 0.5);
    assert_eq!(r.normalize(3), 0.75);
    assert_eq!(r.normalize(4), 1.0);
}

#[test]
fn int_normalize_handles_negative_ranges_and_clamps() {
    let r = IntRange::Linear { min: -2, max: 2 };

    assert_eq!(r.normalize(-2), 0.0);
    assert_eq!(r.normalize(0), 0.5);
    assert_eq!(r.normalize(2), 1.0);
    assert_eq!(r.normalize(-100), 0.0);
    assert_eq!(r.normalize(100), 1.0);
}

#[test]
fn a_degenerate_int_range_normalizes_to_zero() {
    let r = IntRange::Linear { min: 3, max: 3 };
    assert_eq!(r.normalize(3), 0.0);
    assert_eq!(r.normalize(0), 0.0);
}
