//! Spectrum helpers shared by the saturation, tape and stereo tests.
//!
//! Every test tone sits exactly on a DFT bin of its measurement window
//! (coherent sampling), so a rectangular window has no leakage: each
//! harmonic lands in one bin and everything else is distortion residue,
//! aliasing or rounding noise.

#![allow(dead_code)]

use rustfft::{num_complex::Complex, FftPlanner};
use std::f64::consts::TAU;

pub const SR: f64 = 48_000.0;

/// `n` samples of `amp · sin(2π·freq·t)` at `sr`, computed in `f64`.
pub fn sine(freq: f64, amp: f64, sr: f64, n: usize) -> Vec<f64> {
    (0..n).map(|i| amp * (TAU * freq * i as f64 / sr).sin()).collect()
}

/// Peak-amplitude spectrum (a full-scale sine reads 1.0 in its bin; DC
/// reads its mean) of `x`, bins `0..=len/2`.
pub fn amplitude_spectrum(x: &[f32]) -> Vec<f64> {
    let n = x.len();
    let mut buf: Vec<Complex<f64>> = x.iter().map(|&s| Complex::new(s as f64, 0.0)).collect();
    FftPlanner::new().plan_fft_forward(n).process(&mut buf);
    (0..=n / 2)
        .map(|k| {
            let scale = if k == 0 || 2 * k == n { 1.0 } else { 2.0 };
            scale * buf[k].norm() / n as f64
        })
        .collect()
}

pub fn db(x: f64) -> f64 {
    20.0 * x.max(1e-30).log10()
}

/// Harmonic levels of a coherent tone at bin `f0_bin`, `h[k]` for
/// `k = 1..=count` in dB relative to the fundamental (`h[1] = 0`).
/// Harmonics above Nyquist read −300.
pub fn harmonics_dbc(spec: &[f64], f0_bin: usize, count: usize) -> Vec<f64> {
    let fund = spec[f0_bin];
    let mut h = vec![0.0; count + 1];
    for (k, slot) in h.iter_mut().enumerate().skip(1) {
        let b = k * f0_bin;
        *slot = if b < spec.len() { db(spec[b] / fund) } else { -300.0 };
    }
    h
}

/// Aliasing floor: the strongest bin that is not DC, not a harmonic of
/// `f0_bin`, and not a neighbour of one, relative to the fundamental.
pub fn alias_floor_dbc(spec: &[f64], f0_bin: usize) -> f64 {
    let fund = spec[f0_bin];
    let mut worst = 0.0f64;
    for (b, &a) in spec.iter().enumerate().skip(2) {
        let r = b % f0_bin;
        if r <= 1 || r + 1 >= f0_bin {
            continue;
        }
        worst = worst.max(a);
    }
    db(worst / fund)
}

/// Least-squares slope (dB per order) of the harmonics in `orders` that
/// sit above `floor_dbc`.
pub fn decay_db_per_order(h: &[f64], orders: impl Iterator<Item = usize>, floor_dbc: f64) -> f64 {
    let pts: Vec<(f64, f64)> = orders
        .filter(|&k| h[k] > floor_dbc)
        .map(|k| (k as f64, h[k]))
        .collect();
    assert!(pts.len() >= 2, "need two harmonics above {floor_dbc} dBc");
    let n = pts.len() as f64;
    let mx = pts.iter().map(|p| p.0).sum::<f64>() / n;
    let my = pts.iter().map(|p| p.1).sum::<f64>() / n;
    let sxy: f64 = pts.iter().map(|p| (p.0 - mx) * (p.1 - my)).sum();
    let sxx: f64 = pts.iter().map(|p| (p.0 - mx) * (p.0 - mx)).sum();
    sxy / sxx
}

/// Pearson correlation of two equal-length signals.
pub fn correlation(a: &[f32], b: &[f32]) -> f64 {
    let (mut ab, mut aa, mut bb) = (0.0f64, 0.0f64, 0.0f64);
    for (&x, &y) in a.iter().zip(b) {
        ab += x as f64 * y as f64;
        aa += x as f64 * x as f64;
        bb += y as f64 * y as f64;
    }
    ab / (aa * bb).sqrt().max(1e-30)
}

/// Deterministic white noise in [-amp, amp).
pub fn noise(n: usize, amp: f32, seed: u64) -> Vec<f32> {
    let mut rng = resonance_dsp::SimpleRng::new(seed);
    (0..n)
        .map(|_| {
            let u = (rng.next_u32() >> 8) as f32 / (1u32 << 24) as f32;
            amp * (2.0 * u - 1.0)
        })
        .collect()
}
