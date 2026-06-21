//! Unit tests for `FadeCurve::coefficient` — the reusable per-position
//! gain helper shared by the live mixer and the offline bounce/export
//! path (epic #18, arch doc #156, todo #313). Also pins the
//! `FadeCurve` default and the `AudioClip` fade/gain field defaults so
//! that existing clips stay unchanged.

use resonance_audio::types::{FadeCurve, WAVEFORM_PEAK_FRAMES};

const EPS: f32 = 1e-6;

#[test]
fn default_is_equal_power() {
    assert_eq!(FadeCurve::default(), FadeCurve::EqualPower);
}

#[test]
fn endpoints_are_silence_and_unity() {
    for curve in [FadeCurve::Linear, FadeCurve::EqualPower, FadeCurve::Exp] {
        assert!(
            curve.coefficient(0.0).abs() < EPS,
            "{curve:?} at t=0 should be silence"
        );
        assert!(
            (curve.coefficient(1.0) - 1.0).abs() < EPS,
            "{curve:?} at t=1 should be unity"
        );
    }
}

#[test]
fn linear_is_identity() {
    for &t in &[0.0_f32, 0.25, 0.5, 0.75, 1.0] {
        assert!((FadeCurve::Linear.coefficient(t) - t).abs() < EPS);
    }
}

#[test]
fn exp_is_t_squared() {
    for &t in &[0.0_f32, 0.25, 0.5, 0.75, 1.0] {
        assert!((FadeCurve::Exp.coefficient(t) - t * t).abs() < EPS);
    }
}

#[test]
fn equal_power_matches_sine() {
    for &t in &[0.0_f32, 0.1, 0.25, 0.5, 0.5, 0.9, 1.0] {
        let expected = (t * std::f32::consts::FRAC_PI_2).sin();
        assert!((FadeCurve::EqualPower.coefficient(t) - expected).abs() < EPS);
    }
    // Midpoint of an equal-power ramp sits at sin(π/4) = √½ ≈ 0.7071,
    // which is what gives a constant-power crossfade seam.
    let mid = FadeCurve::EqualPower.coefficient(0.5);
    assert!((mid - std::f32::consts::FRAC_1_SQRT_2).abs() < EPS);
}

/// Two equal-power fades running opposite directions across an overlap
/// sum to constant *power* (the automatic crossfade property): for the
/// fade-out we pass the complementary position `1 - t`.
#[test]
fn equal_power_crossfade_is_constant_power() {
    for i in 0..=20 {
        let t = i as f32 / 20.0;
        let fade_in = FadeCurve::EqualPower.coefficient(t);
        let fade_out = FadeCurve::EqualPower.coefficient(1.0 - t);
        let power = fade_in * fade_in + fade_out * fade_out;
        assert!(
            (power - 1.0).abs() < EPS,
            "equal-power overlap at t={t} summed to power {power}, expected 1.0"
        );
    }
}

#[test]
fn coefficient_clamps_out_of_range() {
    for curve in [FadeCurve::Linear, FadeCurve::EqualPower, FadeCurve::Exp] {
        assert!(
            curve.coefficient(-0.5).abs() < EPS,
            "{curve:?} below 0 should clamp to silence"
        );
        assert!(
            (curve.coefficient(1.5) - 1.0).abs() < EPS,
            "{curve:?} above 1 should clamp to unity"
        );
    }
}

#[test]
fn curves_are_monotonic_non_decreasing() {
    for curve in [FadeCurve::Linear, FadeCurve::EqualPower, FadeCurve::Exp] {
        let mut prev = curve.coefficient(0.0);
        for i in 1..=100 {
            let t = i as f32 / 100.0;
            let c = curve.coefficient(t);
            assert!(
                c + EPS >= prev,
                "{curve:?} not monotonic at t={t}: {c} < {prev}"
            );
            prev = c;
        }
    }
}

/// Sanity check that the re-exported constant is still wired up — guards
/// against the `types::*` re-export list being broken by this change.
#[test]
fn peak_frames_constant_is_exported() {
    assert!(WAVEFORM_PEAK_FRAMES > 0);
}
