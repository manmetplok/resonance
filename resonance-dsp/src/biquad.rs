//! RBJ cookbook biquad filter.
//!
//! Direct Form I transposed topology. One `Biquad` filters a single audio
//! channel; use one per channel for stereo. Coefficients are updated via
//! the `set_*` methods (cheap per-block); `process` is per-sample.
//!
//! Reference: "Cookbook formulae for audio EQ biquad filter coefficients"
//! by Robert Bristow-Johnson. All formulas are bilinear-transformed
//! analog prototypes normalized by a0.

use std::f32::consts::PI;

/// A single second-order IIR section. Stores both the normalized
/// coefficients and the delay line state.
#[derive(Clone, Copy, Debug)]
pub struct Biquad {
    // Normalized feedforward coefficients.
    pub b0: f32,
    pub b1: f32,
    pub b2: f32,
    // Normalized feedback coefficients (a0 is implicit 1.0 after normalization).
    pub a1: f32,
    pub a2: f32,
    // State: Direct Form I transposed (two z^-1 registers).
    z1: f32,
    z2: f32,
}

impl Default for Biquad {
    fn default() -> Self {
        Self::identity()
    }
}

impl Biquad {
    /// An all-pass-through (unity) biquad. Useful for unused cascade slots.
    pub const fn identity() -> Self {
        Self {
            b0: 1.0,
            b1: 0.0,
            b2: 0.0,
            a1: 0.0,
            a2: 0.0,
            z1: 0.0,
            z2: 0.0,
        }
    }

    /// Clear the delay line without touching coefficients.
    pub fn reset(&mut self) {
        self.z1 = 0.0;
        self.z2 = 0.0;
    }

    /// Replace coefficients with the unity transfer function.
    pub fn set_identity(&mut self) {
        self.b0 = 1.0;
        self.b1 = 0.0;
        self.b2 = 0.0;
        self.a1 = 0.0;
        self.a2 = 0.0;
    }

    /// Process one sample through the biquad (DF1 transposed).
    #[inline]
    pub fn process(&mut self, x: f32) -> f32 {
        let y = self.b0 * x + self.z1;
        self.z1 = self.b1 * x - self.a1 * y + self.z2;
        self.z2 = self.b2 * x - self.a2 * y;
        y
    }

    /// Peaking bell EQ. `gain_db` positive = boost, negative = cut.
    pub fn set_bell(&mut self, sr: f32, freq: f32, q: f32, gain_db: f32) {
        let (freq, q) = clamp_params(sr, freq, q);
        let a = 10.0_f32.powf(gain_db / 40.0);
        let w0 = 2.0 * PI * freq / sr;
        let (sin_w0, cos_w0) = w0.sin_cos();
        let alpha = sin_w0 / (2.0 * q);

        let b0 = 1.0 + alpha * a;
        let b1 = -2.0 * cos_w0;
        let b2 = 1.0 - alpha * a;
        let a0 = 1.0 + alpha / a;
        let a1 = -2.0 * cos_w0;
        let a2 = 1.0 - alpha / a;
        self.assign_normalized(b0, b1, b2, a0, a1, a2);
    }

    /// Low shelf. `gain_db` boost/cut in the low band; `freq` is the shelf
    /// midpoint; `q` shapes the transition (0.707 = maximally flat).
    pub fn set_low_shelf(&mut self, sr: f32, freq: f32, q: f32, gain_db: f32) {
        let (freq, q) = clamp_params(sr, freq, q);
        let a = 10.0_f32.powf(gain_db / 40.0);
        let w0 = 2.0 * PI * freq / sr;
        let (sin_w0, cos_w0) = w0.sin_cos();
        let alpha = sin_w0 / (2.0 * q);
        let two_sqrt_a_alpha = 2.0 * a.sqrt() * alpha;

        let b0 = a * ((a + 1.0) - (a - 1.0) * cos_w0 + two_sqrt_a_alpha);
        let b1 = 2.0 * a * ((a - 1.0) - (a + 1.0) * cos_w0);
        let b2 = a * ((a + 1.0) - (a - 1.0) * cos_w0 - two_sqrt_a_alpha);
        let a0 = (a + 1.0) + (a - 1.0) * cos_w0 + two_sqrt_a_alpha;
        let a1 = -2.0 * ((a - 1.0) + (a + 1.0) * cos_w0);
        let a2 = (a + 1.0) + (a - 1.0) * cos_w0 - two_sqrt_a_alpha;
        self.assign_normalized(b0, b1, b2, a0, a1, a2);
    }

    /// High shelf. Mirror of `set_low_shelf`.
    pub fn set_high_shelf(&mut self, sr: f32, freq: f32, q: f32, gain_db: f32) {
        let (freq, q) = clamp_params(sr, freq, q);
        let a = 10.0_f32.powf(gain_db / 40.0);
        let w0 = 2.0 * PI * freq / sr;
        let (sin_w0, cos_w0) = w0.sin_cos();
        let alpha = sin_w0 / (2.0 * q);
        let two_sqrt_a_alpha = 2.0 * a.sqrt() * alpha;

        let b0 = a * ((a + 1.0) + (a - 1.0) * cos_w0 + two_sqrt_a_alpha);
        let b1 = -2.0 * a * ((a - 1.0) + (a + 1.0) * cos_w0);
        let b2 = a * ((a + 1.0) + (a - 1.0) * cos_w0 - two_sqrt_a_alpha);
        let a0 = (a + 1.0) - (a - 1.0) * cos_w0 + two_sqrt_a_alpha;
        let a1 = 2.0 * ((a - 1.0) - (a + 1.0) * cos_w0);
        let a2 = (a + 1.0) - (a - 1.0) * cos_w0 - two_sqrt_a_alpha;
        self.assign_normalized(b0, b1, b2, a0, a1, a2);
    }

    /// 12 dB/oct (2nd order) high-pass. Cascade N of these for N*12 dB/oct.
    pub fn set_high_pass(&mut self, sr: f32, freq: f32, q: f32) {
        let (freq, q) = clamp_params(sr, freq, q);
        let w0 = 2.0 * PI * freq / sr;
        let (sin_w0, cos_w0) = w0.sin_cos();
        let alpha = sin_w0 / (2.0 * q);

        let b0 = (1.0 + cos_w0) * 0.5;
        let b1 = -(1.0 + cos_w0);
        let b2 = (1.0 + cos_w0) * 0.5;
        let a0 = 1.0 + alpha;
        let a1 = -2.0 * cos_w0;
        let a2 = 1.0 - alpha;
        self.assign_normalized(b0, b1, b2, a0, a1, a2);
    }

    /// 12 dB/oct (2nd order) low-pass. Cascade N of these for N*12 dB/oct.
    pub fn set_low_pass(&mut self, sr: f32, freq: f32, q: f32) {
        let (freq, q) = clamp_params(sr, freq, q);
        let w0 = 2.0 * PI * freq / sr;
        let (sin_w0, cos_w0) = w0.sin_cos();
        let alpha = sin_w0 / (2.0 * q);

        let b0 = (1.0 - cos_w0) * 0.5;
        let b1 = 1.0 - cos_w0;
        let b2 = (1.0 - cos_w0) * 0.5;
        let a0 = 1.0 + alpha;
        let a1 = -2.0 * cos_w0;
        let a2 = 1.0 - alpha;
        self.assign_normalized(b0, b1, b2, a0, a1, a2);
    }

    /// Evaluate |H(e^{jω})| at a given frequency for offline analysis
    /// (e.g. rendering the response curve in the editor). Pure function of
    /// the current coefficients; does not touch state.
    pub fn magnitude(&self, freq: f32, sr: f32) -> f32 {
        // Transfer function: H(z) = (b0 + b1 z^-1 + b2 z^-2) / (1 + a1 z^-1 + a2 z^-2)
        // Evaluate at z = e^{jω} where ω = 2π f / sr.
        let w = 2.0 * PI * freq / sr;
        let (s1, c1) = w.sin_cos();
        let (s2, c2) = (2.0 * w).sin_cos();
        let num_re = self.b0 + self.b1 * c1 + self.b2 * c2;
        let num_im = -self.b1 * s1 - self.b2 * s2;
        let den_re = 1.0 + self.a1 * c1 + self.a2 * c2;
        let den_im = -self.a1 * s1 - self.a2 * s2;
        let num_mag_sq = num_re * num_re + num_im * num_im;
        let den_mag_sq = den_re * den_re + den_im * den_im;
        (num_mag_sq / den_mag_sq.max(1e-30)).sqrt()
    }

    fn assign_normalized(&mut self, b0: f32, b1: f32, b2: f32, a0: f32, a1: f32, a2: f32) {
        let inv_a0 = 1.0 / a0;
        self.b0 = b0 * inv_a0;
        self.b1 = b1 * inv_a0;
        self.b2 = b2 * inv_a0;
        self.a1 = a1 * inv_a0;
        self.a2 = a2 * inv_a0;
    }
}

/// Clamp frequency away from DC and Nyquist, and Q away from zero, to keep
/// the bilinear transform well-conditioned.
fn clamp_params(sr: f32, freq: f32, q: f32) -> (f32, f32) {
    let nyquist = sr * 0.5;
    let f = freq.clamp(10.0, nyquist * 0.995);
    let q = q.max(0.05);
    (f, q)
}

