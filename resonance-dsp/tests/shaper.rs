//! Accuracy contract for [`tanh_fast`].
//!
//! These bounds are what justify substituting the approximation for
//! `f32::tanh` on audio paths; if a future change to the coefficients
//! loosens them, that is an audible-quality decision and this test should
//! fail loudly rather than let it through.

use resonance_dsp::tanh_fast;

/// Max absolute error over the range audio realistically occupies.
/// 2.0 is +6 dBFS into the shaper.
const MAX_ERR_AUDIO_RANGE: f32 = 1.0e-6;
/// Max absolute error anywhere, including the deeply saturated tail.
const MAX_ERR_GLOBAL: f32 = 1.0e-4;

fn sweep(lo: f64, hi: f64, step: f64) -> f32 {
    let mut worst = 0.0f32;
    let mut x = lo;
    while x <= hi {
        let xf = x as f32;
        let e = (tanh_fast(xf) - xf.tanh()).abs();
        if e > worst {
            worst = e;
        }
        x += step;
    }
    worst
}

#[test]
fn matches_tanh_to_ulp_over_audio_range() {
    let worst = sweep(-2.0, 2.0, 1e-5);
    assert!(
        worst < MAX_ERR_AUDIO_RANGE,
        "|x|<=2 error {worst:.3e} exceeds {MAX_ERR_AUDIO_RANGE:.3e}"
    );
}

#[test]
fn bounded_error_everywhere() {
    let worst = sweep(-25.0, 25.0, 1e-4);
    assert!(
        worst < MAX_ERR_GLOBAL,
        "global error {worst:.3e} exceeds {MAX_ERR_GLOBAL:.3e}"
    );
}

#[test]
fn odd_exact_at_zero_and_bounded() {
    assert_eq!(tanh_fast(0.0), 0.0);
    let mut x = 0.0f64;
    while x <= 25.0 {
        let xf = x as f32;
        assert_eq!(tanh_fast(-xf), -tanh_fast(xf), "not odd at {xf}");
        assert!(tanh_fast(xf).abs() <= 1.0, "|tanh_fast({xf})| > 1");
        x += 1e-3;
    }
}

#[test]
fn monotonic_non_decreasing() {
    // A waveshaper that folds back would add spurious harmonics, so the
    // curve must never turn around. The tolerance absorbs rounding in the
    // `num / den` evaluation itself, which can make two adjacent outputs
    // differ by an ULP in the wrong direction near saturation — that is
    // f32 noise, not a fold-back in the underlying function.
    const ULP_NOISE: f32 = 4.0 * f32::EPSILON;
    let mut prev = f32::NEG_INFINITY;
    // Step in exact f32 increments so the input sequence is genuinely
    // increasing (an f64 accumulator rounded to f32 is not).
    let mut xi = -250_000i32;
    while xi <= 250_000 {
        let x = xi as f32 * 1e-4;
        let v = tanh_fast(x);
        assert!(v >= prev - ULP_NOISE, "fold-back at x={x}: {v} < {prev}");
        prev = prev.max(v);
        xi += 1;
    }
}

#[test]
fn handles_extremes() {
    assert!(tanh_fast(f32::MAX).abs() <= 1.0);
    assert!(tanh_fast(-f32::MAX).abs() <= 1.0);
    assert!(tanh_fast(f32::INFINITY).is_finite());
    assert!(tanh_fast(f32::NEG_INFINITY).is_finite());
}
