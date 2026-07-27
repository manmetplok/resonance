//! Tests for cubic Hermite interpolation and fractional circular reads.

use resonance_dsp::{hermite4, read_hermite_wrapped};
use std::f32::consts::TAU;

/// Linear 2-point interpolation, the baseline hermite4 must beat.
fn linear2(x0: f32, x1: f32, frac: f32) -> f32 {
    x0 + (x1 - x0) * frac
}

#[test]
fn hermite_reproduces_linear_ramp_exactly() {
    // Samples of f(t) = 3t − 5 at t = -1, 0, 1, 2.
    let f = |t: f32| 3.0 * t - 5.0;
    for i in 0..=100 {
        let frac = i as f32 / 100.0;
        let got = hermite4(f(-1.0), f(0.0), f(1.0), f(2.0), frac);
        let expected = f(frac);
        assert!(
            (got - expected).abs() < 1e-5,
            "frac {frac}: {got} vs {expected}"
        );
    }
}

#[test]
fn hermite_reproduces_quadratics_exactly() {
    // Catmull-Rom has approximation order 3: quadratics sampled on a
    // uniform grid are reconstructed exactly (up to rounding).
    let f = |t: f32| 0.75 * t * t - 1.5 * t + 0.25;
    for i in 0..=100 {
        let frac = i as f32 / 100.0;
        let got = hermite4(f(-1.0), f(0.0), f(1.0), f(2.0), frac);
        let expected = f(frac);
        assert!(
            (got - expected).abs() < 1e-5,
            "frac {frac}: {got} vs {expected}"
        );
    }
}

#[test]
fn hermite_matches_endpoints() {
    let (xm1, x0, x1, x2) = (0.3_f32, -0.7, 0.9, 0.1);
    assert!((hermite4(xm1, x0, x1, x2, 0.0) - x0).abs() < 1e-7);
    assert!((hermite4(xm1, x0, x1, x2, 1.0) - x1).abs() < 1e-6);
}

#[test]
fn hermite_beats_linear_on_resampled_sine() {
    // Resample a 997 Hz sine at rate 1.37 (an upward-transposed grain
    // read) and compare both interpolators against the analytic signal.
    let sr = 48_000.0_f32;
    let freq = 997.0_f32;
    let src: Vec<f32> = (0..4096)
        .map(|i| (TAU * freq * i as f32 / sr).sin())
        .collect();
    let rate = 1.37_f64;
    let mut pos = 1.0_f64;
    let mut err_hermite = 0.0_f64;
    let mut err_linear = 0.0_f64;
    let mut n = 0;
    while pos < (src.len() - 3) as f64 {
        let i0 = pos.floor() as usize;
        let frac = (pos - pos.floor()) as f32;
        let truth = (TAU * freq * pos as f32 / sr).sin();
        let h = hermite4(src[i0 - 1], src[i0], src[i0 + 1], src[i0 + 2], frac);
        let l = linear2(src[i0], src[i0 + 1], frac);
        err_hermite += f64::from((h - truth) * (h - truth));
        err_linear += f64::from((l - truth) * (l - truth));
        n += 1;
        pos += rate;
    }
    let rms_hermite = (err_hermite / n as f64).sqrt();
    let rms_linear = (err_linear / n as f64).sqrt();
    assert!(
        rms_hermite < rms_linear / 10.0,
        "hermite rms {rms_hermite:.3e} not clearly below linear rms {rms_linear:.3e}"
    );
}

#[test]
fn circular_read_wraps_across_the_boundary() {
    // Buffer holds exactly 8 periods of a sine, so the signal is
    // continuous across the wrap point and fractional reads spanning the
    // boundary must match the analytic waveform.
    let len = 1024_usize;
    let periods = 8.0_f32;
    let wave = |t: f32| (TAU * periods * t / len as f32).sin();
    let buf: Vec<f32> = (0..len).map(|i| wave(i as f32)).collect();
    for i in 0..200 {
        // Positions straddling the boundary: len − 5 .. len + 5.
        let index = (len as f64 - 5.0) + i as f64 * 0.05;
        let got = read_hermite_wrapped(&buf, index);
        let expected = wave((index % len as f64) as f32);
        assert!(
            (got - expected).abs() < 1e-3,
            "index {index}: {got} vs {expected}"
        );
    }
}

#[test]
fn circular_read_accepts_negative_indices() {
    let buf: Vec<f32> = (0..8).map(|i| i as f32).collect();
    // index −0.999.. lies between buf[7] (= 7) and buf[0] (= 0); at
    // integer −1 it must be exactly buf[7].
    assert!((read_hermite_wrapped(&buf, -1.0) - 7.0).abs() < 1e-6);
    // Whole negative periods wrap back onto the same samples.
    assert!((read_hermite_wrapped(&buf, -9.0) - 7.0).abs() < 1e-6);
    assert!((read_hermite_wrapped(&buf, -16.0) - 0.0).abs() < 1e-6);
}

#[test]
fn circular_read_matches_integer_samples() {
    let buf: Vec<f32> = (0..16).map(|i| (i as f32 * 0.7).sin()).collect();
    for i in 0..16 {
        let got = read_hermite_wrapped(&buf, i as f64);
        assert!(
            (got - buf[i]).abs() < 1e-6,
            "integer index {i}: {got} vs {}",
            buf[i]
        );
    }
}

#[test]
#[should_panic(expected = "power of two")]
fn circular_read_rejects_non_power_of_two() {
    let buf = vec![0.0_f32; 12];
    read_hermite_wrapped(&buf, 0.5);
}
