//! Tests for cubic Hermite interpolation, fractional circular reads,
//! and the quality-tier kernels (ba todo #1083): the 6-point, 5th-order
//! B-spline HQ kernel and the linear Lo-fi circular read.

use resonance_dsp::{
    bspline6, hermite4, read_bspline6_wrapped, read_hermite_wrapped, read_linear_wrapped,
};
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

// --- 6-point B-spline (HQ tier, ba todo #1083). -----------------------

#[test]
fn bspline6_is_a_partition_of_unity() {
    // Constant input must be reproduced exactly at every fraction (the
    // six blending weights sum to 1).
    for i in 0..=100 {
        let frac = i as f32 / 100.0;
        let got = bspline6(0.6, 0.6, 0.6, 0.6, 0.6, 0.6, frac);
        assert!((got - 0.6).abs() < 1e-6, "frac {frac}: {got} vs 0.6");
    }
}

#[test]
fn bspline6_reproduces_linear_ramp_exactly() {
    // Quintic B-splines reproduce degree-1 polynomials: samples of
    // f(t) = 3t − 5 at t = -2..3 reconstruct f on [0, 1].
    let f = |t: f32| 3.0 * t - 5.0;
    for i in 0..=100 {
        let frac = i as f32 / 100.0;
        let got = bspline6(f(-2.0), f(-1.0), f(0.0), f(1.0), f(2.0), f(3.0), frac);
        let expected = f(frac);
        assert!(
            (got - expected).abs() < 1e-4,
            "frac {frac}: {got} vs {expected}"
        );
    }
}

#[test]
fn bspline6_stays_inside_the_input_hull() {
    // All six blending weights are non-negative on [0, 1], so the
    // output is a convex combination of the inputs — it can never
    // overshoot, unlike Catmull-Rom.
    let xs = [0.9_f32, -0.7, 0.3, -0.2, 0.8, -0.5];
    let lo = xs.iter().copied().fold(f32::INFINITY, f32::min);
    let hi = xs.iter().copied().fold(f32::NEG_INFINITY, f32::max);
    for i in 0..=100 {
        let frac = i as f32 / 100.0;
        let got = bspline6(xs[0], xs[1], xs[2], xs[3], xs[4], xs[5], frac);
        assert!(
            (lo - 1e-6..=hi + 1e-6).contains(&got),
            "frac {frac}: {got} outside [{lo}, {hi}]"
        );
    }
}

#[test]
fn bspline6_beats_hermite_on_image_rejection() {
    // The property the HQ tier buys (doc #252 §3, Niemitalo deip.pdf):
    // the B-spline's stopband crushes the spectral images that
    // fractional-position reads let through. Reconstruct a 3 kHz sine
    // at 10× oversampling (index step 0.1) with both kernels; in the
    // oversampled-domain spectrum the first image sits at
    // SR − 3 kHz = 45 kHz. The fundamental must survive both kernels,
    // the image must be far lower with the B-spline.
    let sr = 48_000.0_f32;
    let freq = 3_000.0_f32;
    let src: Vec<f32> = (0..4096)
        .map(|i| (TAU * freq * i as f32 / sr).sin())
        .collect();
    let step = 0.1_f64; // 10x oversampled reconstruction
    let total = 30_000_usize;
    let mut out_h = vec![0.0_f32; total];
    let mut out_b = vec![0.0_f32; total];
    let mut pos = 8.0_f64;
    for i in 0..total {
        let i0 = pos.floor() as usize;
        let frac = (pos - pos.floor()) as f32;
        out_h[i] = hermite4(src[i0 - 1], src[i0], src[i0 + 1], src[i0 + 2], frac);
        out_b[i] = bspline6(
            src[i0 - 2],
            src[i0 - 1],
            src[i0],
            src[i0 + 1],
            src[i0 + 2],
            src[i0 + 3],
            frac,
        );
        pos += step;
    }
    // Single-bin Hann DFT at `f` in the oversampled domain (fs = 10·SR).
    let os_sr = sr as f64 / step;
    let mag = |x: &[f32], f: f32| -> f64 {
        let w = TAU as f64 * f as f64 / os_sr;
        let n_max = (x.len() - 1) as f64;
        let (mut re, mut im) = (0.0_f64, 0.0_f64);
        for (n, &s) in x.iter().enumerate() {
            let win = 0.5 - 0.5 * (TAU as f64 * n as f64 / n_max).cos();
            let phase = w * n as f64;
            re += s as f64 * win * phase.cos();
            im -= s as f64 * win * phase.sin();
        }
        (re * re + im * im).sqrt() / x.len() as f64
    };
    let image = sr - freq; // 45 kHz, representable at fs = 480 kHz
    let (fund_h, fund_b) = (mag(&out_h, freq), mag(&out_b, freq));
    let (img_h, img_b) = (mag(&out_h, image), mag(&out_b, image));
    // Both kernels keep the fundamental (B-spline droop at 3 kHz is
    // well under 1 dB).
    assert!(
        fund_b > 0.8 * fund_h && fund_h > 0.1,
        "fundamental lost: hermite {fund_h:.3e}, bspline {fund_b:.3e}"
    );
    // Hermite lets a measurable image through; the B-spline rejects at
    // least 10x (20 dB) more of it.
    assert!(
        img_h > 1e-6,
        "no measurable hermite image at {image} Hz ({img_h:.3e}) — test misconfigured"
    );
    assert!(
        img_b < img_h / 10.0,
        "bspline image {img_b:.3e} not clearly below hermite image {img_h:.3e}"
    );
}

#[test]
fn bspline6_circular_read_wraps_seamlessly() {
    // Same seamless-wrap setup as the Hermite read: 8 whole periods in
    // the ring, reads straddling the boundary must match reads of the
    // same waveform away from it (the kernel smooths, so compare
    // against a non-wrapping evaluation of the same kernel, not the
    // analytic sine).
    let len = 1024_usize;
    let periods = 8.0_f32;
    let wave = |t: f64| (TAU as f64 * periods as f64 * t / len as f64).sin() as f32;
    let buf: Vec<f32> = (0..len).map(|i| wave(i as f64)).collect();
    for i in 0..200 {
        let index = (len as f64 - 5.0) + i as f64 * 0.05;
        let got = read_bspline6_wrapped(&buf, index);
        // One whole period earlier the kernel sees identical samples.
        let reference = read_bspline6_wrapped(&buf, index - 128.0);
        assert!(
            (got - reference).abs() < 1e-5,
            "index {index}: {got} vs {reference}"
        );
    }
}

#[test]
#[should_panic(expected = "power of two")]
fn bspline6_circular_read_rejects_short_buffers() {
    let buf = vec![0.0_f32; 4]; // power of two but below the 6-pt span
    read_bspline6_wrapped(&buf, 0.5);
}

// --- Linear circular read (Lo-fi tier, ba todo #1083). ----------------

#[test]
fn linear_circular_read_matches_integer_samples_and_wraps() {
    let buf: Vec<f32> = (0..16).map(|i| (i as f32 * 0.7).sin()).collect();
    for i in 0..16 {
        let got = read_linear_wrapped(&buf, i as f64);
        assert!((got - buf[i]).abs() < 1e-6, "integer index {i}");
    }
    // Halfway between the last and first sample (across the wrap).
    let got = read_linear_wrapped(&buf, 15.5);
    let expected = 0.5 * (buf[15] + buf[0]);
    assert!((got - expected).abs() < 1e-6, "{got} vs {expected}");
    // Negative indices wrap like the other readers.
    assert!((read_linear_wrapped(&buf, -1.0) - buf[15]).abs() < 1e-6);
}
