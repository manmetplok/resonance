//! Shared synthetic signal generators for the integration tests.
//!
//! Each integration test compiles `common` as its own module, so helpers
//! used by only a subset of tests trip `dead_code` warnings in the others.
#![allow(dead_code)]

use std::f32::consts::TAU;

/// Generate a mono 1 kHz sine at the given dBFS amplitude for `secs`
/// seconds. Returns (left, right). For a "mono" test, both channels are
/// identical.
pub fn sine_mono(sr: f32, freq: f32, dbfs: f32, secs: f32) -> (Vec<f32>, Vec<f32>) {
    let amp = 10.0_f32.powf(dbfs / 20.0);
    let n = (sr * secs) as usize;
    let mut l = vec![0.0_f32; n];
    let mut r = vec![0.0_f32; n];
    for i in 0..n {
        let s = (i as f32 / sr * freq * TAU).sin() * amp;
        l[i] = s;
        r[i] = s;
    }
    (l, r)
}

/// Deterministic xorshift64 in `[-1, 1)`.
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Self(seed | 1)
    }

    pub fn next(&mut self) -> f64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 >> 11) as f64 / (1u64 << 52) as f64 - 1.0
    }
}

/// Noise with an exact `1/f^exponent` power spectrum (0 = white, 1 =
/// pink), synthesised in the frequency domain: every bin gets magnitude
/// `k^(-exponent/2)` and a random phase, then one inverse FFT. `len` must
/// be a power of two; the result is periodic in `len` and scaled to the
/// given RMS in dBFS. DC is zero.
pub fn coloured_noise(len: usize, exponent: f64, rms_dbfs: f64, seed: u64) -> Vec<f32> {
    use rustfft::num_complex::Complex;
    let mut rng = Rng::new(seed);
    let mut spec = vec![Complex::new(0.0f64, 0.0); len];
    for k in 1..len / 2 {
        let mag = (k as f64).powf(-exponent / 2.0);
        let phase = std::f64::consts::PI * rng.next();
        let c = Complex::from_polar(mag, phase);
        spec[k] = c;
        spec[len - k] = c.conj();
    }
    let mut planner = rustfft::FftPlanner::<f64>::new();
    planner.plan_fft_inverse(len).process(&mut spec);
    let rms = (spec.iter().map(|c| c.re * c.re).sum::<f64>() / len as f64).sqrt();
    let gain = 10f64.powf(rms_dbfs / 20.0) / rms;
    spec.iter().map(|c| (c.re * gain) as f32).collect()
}

/// Concatenate two stereo buffers.
pub fn concat(a: (Vec<f32>, Vec<f32>), b: (Vec<f32>, Vec<f32>)) -> (Vec<f32>, Vec<f32>) {
    let mut l = a.0;
    let mut r = a.1;
    l.extend(b.0);
    r.extend(b.1);
    (l, r)
}
