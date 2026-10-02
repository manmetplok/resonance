//! RBJ cookbook biquad filter.
//!
//! Transposed Direct Form II topology. One `Biquad` filters a single audio
//! channel; use one per channel for stereo. Coefficients are updated via
//! the `set_*` methods (cheap per-block); `process` is per-sample.
//!
//! Reference: "Cookbook formulae for audio EQ biquad filter coefficients"
//! by Robert Bristow-Johnson. All formulas are bilinear-transformed
//! analog prototypes normalized by a0.
//!
//! [`Biquad`] designs in f32 (kept bit-stable: every IIR golden depends on
//! it). [`BiquadCoeffs`] carries the same formulas in f64 for offline
//! consumers that need the *designed* response rather than that of the
//! f32 filter — the mastering FIR designers: at low
//! frequencies and high sample rates `a1 ≈ -2`, `a2 ≈ 1` and the f32
//! rounding of the coefficients alone moves the response by up to ~1 dB
//! (20 Hz high-pass, 5 Hz, 192 kHz).

use std::f32::consts::PI;
use std::f64::consts::PI as PI_F64;

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
    // State: transposed Direct Form II (two z^-1 registers).
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

    /// Assign already-normalized coefficients directly (`a0` is the
    /// implicit 1.0). For callers deriving coefficients outside the RBJ
    /// cookbook set — e.g. the BS.1770 K-weighting filters in
    /// `resonance-metering`. Like the `set_*` methods, this leaves the
    /// delay-line state untouched.
    pub fn assign_raw(&mut self, b0: f32, b1: f32, b2: f32, a1: f32, a2: f32) {
        self.b0 = b0;
        self.b1 = b1;
        self.b2 = b2;
        self.a1 = a1;
        self.a2 = a2;
    }

    /// Round f64 design coefficients into this filter. Leaves the
    /// delay-line state untouched.
    pub fn assign_coeffs(&mut self, c: BiquadCoeffs) {
        self.b0 = c.b0 as f32;
        self.b1 = c.b1 as f32;
        self.b2 = c.b2 as f32;
        self.a1 = c.a1 as f32;
        self.a2 = c.a2 as f32;
    }

    /// This filter's (f32) coefficients, widened to f64.
    pub fn coeffs(&self) -> BiquadCoeffs {
        BiquadCoeffs {
            b0: self.b0 as f64,
            b1: self.b1 as f64,
            b2: self.b2 as f64,
            a1: self.a1 as f64,
            a2: self.a2 as f64,
        }
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

    /// Second-order all-pass: unity magnitude, phase falling through −π
    /// at `freq`; `q` sets how fast (higher = narrower transition).
    pub fn set_all_pass(&mut self, sr: f32, freq: f32, q: f32) {
        let (freq, q) = clamp_params(sr, freq, q);
        let w0 = 2.0 * PI * freq / sr;
        let (sin_w0, cos_w0) = w0.sin_cos();
        let alpha = sin_w0 / (2.0 * q);

        let b0 = 1.0 - alpha;
        let b1 = -2.0 * cos_w0;
        let b2 = 1.0 + alpha;
        let a0 = 1.0 + alpha;
        let a1 = -2.0 * cos_w0;
        let a2 = 1.0 - alpha;
        self.assign_normalized(b0, b1, b2, a0, a1, a2);
    }

    /// Band-pass with a constant 0 dB peak at `freq` (the RBJ
    /// "constant 0 dB peak gain" form). `q` sets the bandwidth.
    pub fn set_band_pass(&mut self, sr: f32, freq: f32, q: f32) {
        let (freq, q) = clamp_params(sr, freq, q);
        let w0 = 2.0 * PI * freq / sr;
        let (sin_w0, cos_w0) = w0.sin_cos();
        let alpha = sin_w0 / (2.0 * q);
        self.assign_normalized(alpha, 0.0, -alpha, 1.0 + alpha, -2.0 * cos_w0, 1.0 - alpha);
    }

    /// 6 dB/oct (1st order) low-pass, as a biquad with `b2 = a2 = 0`.
    /// Paired with a 12 dB/oct Butterworth section of Q 1.0 it gives an
    /// 18 dB/oct (3rd-order) Butterworth response.
    pub fn set_first_order_low_pass(&mut self, sr: f32, freq: f32) {
        let (freq, _) = clamp_params(sr, freq, 1.0);
        self.set_first_order_analog(sr, 0.0, 1.0, 1.0 / (2.0 * PI * freq), 1.0, freq);
    }

    /// 6 dB/oct (1st order) high-pass. See [`Biquad::set_first_order_low_pass`].
    pub fn set_first_order_high_pass(&mut self, sr: f32, freq: f32) {
        let (freq, _) = clamp_params(sr, freq, 1.0);
        let tau = 1.0 / (2.0 * PI * freq);
        self.set_first_order_analog(sr, tau, 0.0, tau, 1.0, freq);
    }

    /// Bilinear transform of the first-order analog section
    /// `H(s) = (b1·s + b0) / (a1·s + a0)`, with `s` in rad/s, prewarped so
    /// the digital response equals the analog one exactly at
    /// `prewarp_hz` (clamped below Nyquist).
    ///
    /// This is the one first-order design every other first-order shape
    /// here reduces to, and the route for sections whose analog zero or
    /// pole lies *above* Nyquist (an "air" shelf with a 40 kHz corner):
    /// the bilinear map sends every left-half-plane pole inside the unit
    /// circle whatever its frequency, so such a section is always stable,
    /// and the prewarp point decides where in the audible band the digital
    /// curve tracks the analog one. With `a0 / a1 > 0` (a left-half-plane
    /// pole) the result is stable for any sample rate.
    pub fn set_first_order_analog(
        &mut self,
        sr: f32,
        b1: f32,
        b0: f32,
        a1: f32,
        a0: f32,
        prewarp_hz: f32,
    ) {
        let nyquist = (sr * 0.5).max(20.0);
        let fw = prewarp_hz.clamp(1.0, nyquist * 0.95);
        let w = 2.0 * PI * fw;
        // s = K (1 - z^-1) / (1 + z^-1), K chosen so ω_analog(fw) maps to fw.
        let k = w / (PI * fw / sr).tan();
        let nb0 = b1 * k + b0;
        let nb1 = b0 - b1 * k;
        let na0 = a1 * k + a0;
        let na1 = a0 - a1 * k;
        self.assign_normalized(nb0, nb1, 0.0, na0, na1, 0.0);
    }

    /// Evaluate |H(e^{jω})| of this (f32) filter at a given frequency for
    /// offline analysis (e.g. rendering the response curve in the editor).
    /// Pure function of the current coefficients; does not touch state.
    ///
    /// Evaluated in f64 in the sin²(ω/2) form, so it is the true response
    /// of the filter as it runs. For the response of the *design* (free of
    /// f32 coefficient rounding), use [`BiquadCoeffs::magnitude`].
    pub fn magnitude(&self, freq: f32, sr: f32) -> f32 {
        self.coeffs().magnitude(freq as f64, sr as f64) as f32
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

/// Normalized biquad coefficients in f64 (`a0` is the implicit 1.0): the
/// RBJ cookbook designs, and an accurate magnitude evaluation.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BiquadCoeffs {
    pub b0: f64,
    pub b1: f64,
    pub b2: f64,
    pub a1: f64,
    pub a2: f64,
}

impl Default for BiquadCoeffs {
    fn default() -> Self {
        Self::IDENTITY
    }
}

impl BiquadCoeffs {
    /// The unity transfer function.
    pub const IDENTITY: Self = Self {
        b0: 1.0,
        b1: 0.0,
        b2: 0.0,
        a1: 0.0,
        a2: 0.0,
    };

    /// Peaking bell EQ. `gain_db` positive = boost, negative = cut.
    pub fn bell(sr: f64, freq: f64, q: f64, gain_db: f64) -> Self {
        let (freq, q) = clamp_params_f64(sr, freq, q);
        let a = 10.0_f64.powf(gain_db / 40.0);
        let w0 = 2.0 * PI_F64 * freq / sr;
        let (sin_w0, cos_w0) = w0.sin_cos();
        let alpha = sin_w0 / (2.0 * q);

        let b0 = 1.0 + alpha * a;
        let b1 = -2.0 * cos_w0;
        let b2 = 1.0 - alpha * a;
        let a0 = 1.0 + alpha / a;
        let a1 = -2.0 * cos_w0;
        let a2 = 1.0 - alpha / a;
        Self::normalized(b0, b1, b2, a0, a1, a2)
    }

    /// Low shelf. `gain_db` boost/cut in the low band; `freq` is the shelf
    /// midpoint; `q` shapes the transition (0.707 = maximally flat).
    pub fn low_shelf(sr: f64, freq: f64, q: f64, gain_db: f64) -> Self {
        let (freq, q) = clamp_params_f64(sr, freq, q);
        let a = 10.0_f64.powf(gain_db / 40.0);
        let w0 = 2.0 * PI_F64 * freq / sr;
        let (sin_w0, cos_w0) = w0.sin_cos();
        let alpha = sin_w0 / (2.0 * q);
        let two_sqrt_a_alpha = 2.0 * a.sqrt() * alpha;

        let b0 = a * ((a + 1.0) - (a - 1.0) * cos_w0 + two_sqrt_a_alpha);
        let b1 = 2.0 * a * ((a - 1.0) - (a + 1.0) * cos_w0);
        let b2 = a * ((a + 1.0) - (a - 1.0) * cos_w0 - two_sqrt_a_alpha);
        let a0 = (a + 1.0) + (a - 1.0) * cos_w0 + two_sqrt_a_alpha;
        let a1 = -2.0 * ((a - 1.0) + (a + 1.0) * cos_w0);
        let a2 = (a + 1.0) + (a - 1.0) * cos_w0 - two_sqrt_a_alpha;
        Self::normalized(b0, b1, b2, a0, a1, a2)
    }

    /// High shelf. Mirror of [`BiquadCoeffs::low_shelf`].
    pub fn high_shelf(sr: f64, freq: f64, q: f64, gain_db: f64) -> Self {
        let (freq, q) = clamp_params_f64(sr, freq, q);
        let a = 10.0_f64.powf(gain_db / 40.0);
        let w0 = 2.0 * PI_F64 * freq / sr;
        let (sin_w0, cos_w0) = w0.sin_cos();
        let alpha = sin_w0 / (2.0 * q);
        let two_sqrt_a_alpha = 2.0 * a.sqrt() * alpha;

        let b0 = a * ((a + 1.0) + (a - 1.0) * cos_w0 + two_sqrt_a_alpha);
        let b1 = -2.0 * a * ((a - 1.0) + (a + 1.0) * cos_w0);
        let b2 = a * ((a + 1.0) + (a - 1.0) * cos_w0 - two_sqrt_a_alpha);
        let a0 = (a + 1.0) - (a - 1.0) * cos_w0 + two_sqrt_a_alpha;
        let a1 = 2.0 * ((a - 1.0) - (a + 1.0) * cos_w0);
        let a2 = (a + 1.0) - (a - 1.0) * cos_w0 - two_sqrt_a_alpha;
        Self::normalized(b0, b1, b2, a0, a1, a2)
    }

    /// 12 dB/oct (2nd order) high-pass.
    pub fn high_pass(sr: f64, freq: f64, q: f64) -> Self {
        let (freq, q) = clamp_params_f64(sr, freq, q);
        let w0 = 2.0 * PI_F64 * freq / sr;
        let (sin_w0, cos_w0) = w0.sin_cos();
        let alpha = sin_w0 / (2.0 * q);

        let b0 = (1.0 + cos_w0) * 0.5;
        let b1 = -(1.0 + cos_w0);
        let b2 = (1.0 + cos_w0) * 0.5;
        let a0 = 1.0 + alpha;
        let a1 = -2.0 * cos_w0;
        let a2 = 1.0 - alpha;
        Self::normalized(b0, b1, b2, a0, a1, a2)
    }

    /// 12 dB/oct (2nd order) low-pass.
    pub fn low_pass(sr: f64, freq: f64, q: f64) -> Self {
        let (freq, q) = clamp_params_f64(sr, freq, q);
        let w0 = 2.0 * PI_F64 * freq / sr;
        let (sin_w0, cos_w0) = w0.sin_cos();
        let alpha = sin_w0 / (2.0 * q);

        let b0 = (1.0 - cos_w0) * 0.5;
        let b1 = 1.0 - cos_w0;
        let b2 = (1.0 - cos_w0) * 0.5;
        let a0 = 1.0 + alpha;
        let a1 = -2.0 * cos_w0;
        let a2 = 1.0 - alpha;
        Self::normalized(b0, b1, b2, a0, a1, a2)
    }

    /// Second-order all-pass: unity magnitude, phase falling through −π
    /// at `freq`; `q` sets how fast (higher = narrower transition).
    pub fn all_pass(sr: f64, freq: f64, q: f64) -> Self {
        let (freq, q) = clamp_params_f64(sr, freq, q);
        let w0 = 2.0 * PI_F64 * freq / sr;
        let (sin_w0, cos_w0) = w0.sin_cos();
        let alpha = sin_w0 / (2.0 * q);

        let b0 = 1.0 - alpha;
        let b1 = -2.0 * cos_w0;
        let b2 = 1.0 + alpha;
        let a0 = 1.0 + alpha;
        let a1 = -2.0 * cos_w0;
        let a2 = 1.0 - alpha;
        Self::normalized(b0, b1, b2, a0, a1, a2)
    }

    /// Band-pass with a constant 0 dB peak at `freq`.
    pub fn band_pass(sr: f64, freq: f64, q: f64) -> Self {
        let (freq, q) = clamp_params_f64(sr, freq, q);
        let w0 = 2.0 * PI_F64 * freq / sr;
        let (sin_w0, cos_w0) = w0.sin_cos();
        let alpha = sin_w0 / (2.0 * q);
        Self::normalized(alpha, 0.0, -alpha, 1.0 + alpha, -2.0 * cos_w0, 1.0 - alpha)
    }

    /// 6 dB/oct (1st order) low-pass, as a biquad with `b2 = a2 = 0`.
    pub fn first_order_low_pass(sr: f64, freq: f64) -> Self {
        let (freq, _) = clamp_params_f64(sr, freq, 1.0);
        Self::first_order_analog(sr, 0.0, 1.0, 1.0 / (2.0 * PI_F64 * freq), 1.0, freq)
    }

    /// 6 dB/oct (1st order) high-pass.
    pub fn first_order_high_pass(sr: f64, freq: f64) -> Self {
        let (freq, _) = clamp_params_f64(sr, freq, 1.0);
        let tau = 1.0 / (2.0 * PI_F64 * freq);
        Self::first_order_analog(sr, tau, 0.0, tau, 1.0, freq)
    }

    /// Bilinear transform of the first-order analog section
    /// `H(s) = (b1·s + b0) / (a1·s + a0)`, with `s` in rad/s, prewarped so
    /// the digital response equals the analog one exactly at
    /// `prewarp_hz` (clamped below Nyquist).
    ///
    /// This is the one first-order design every other first-order shape
    /// here reduces to, and the route for sections whose analog zero or
    /// pole lies *above* Nyquist (an "air" shelf with a 40 kHz corner):
    /// the bilinear map sends every left-half-plane pole inside the unit
    /// circle whatever its frequency, so such a section is always stable,
    /// and the prewarp point decides where in the audible band the digital
    /// curve tracks the analog one. With `a0 / a1 > 0` (a left-half-plane
    /// pole) the result is stable for any sample rate.
    pub fn first_order_analog(
        sr: f64,
        b1: f64,
        b0: f64,
        a1: f64,
        a0: f64,
        prewarp_hz: f64,
    ) -> Self {
        let nyquist = (sr * 0.5).max(20.0);
        let fw = prewarp_hz.clamp(1.0, nyquist * 0.95);
        let w = 2.0 * PI_F64 * fw;
        // s = K (1 - z^-1) / (1 + z^-1), K chosen so ω_analog(fw) maps to fw.
        let k = w / (PI_F64 * fw / sr).tan();
        let nb0 = b1 * k + b0;
        let nb1 = b0 - b1 * k;
        let na0 = a1 * k + a0;
        let na1 = a0 - a1 * k;
        Self::normalized(nb0, nb1, 0.0, na0, na1, 0.0)
    }

    /// |H(e^{jω})| at `freq`.
    pub fn magnitude(&self, freq: f64, sr: f64) -> f64 {
        let half_w = PI_F64 * freq / sr;
        let s = half_w.sin();
        self.magnitude_at_sin2(s * s)
    }

    /// |H(e^{jω})| given `phi = sin²(ω/2)`, so a caller evaluating many
    /// sections at one frequency computes the sine once.
    ///
    /// Uses `|P(e^{jω})|² = (p0+p1+p2)² − 4(p0·p1 + 4·p0·p2 + p1·p2)·φ +
    /// 16·p0·p2·φ²`. The `cos ω` form instead subtracts nearly equal terms
    /// (`1 + a1 cos ω + a2 cos 2ω` with `a1 ≈ −2`, `a2 ≈ 1`) at low
    /// frequencies; here the small DC term `p0+p1+p2` is formed directly.
    #[inline]
    pub fn magnitude_at_sin2(&self, phi: f64) -> f64 {
        #[inline]
        fn power(p0: f64, p1: f64, p2: f64, phi: f64) -> f64 {
            let dc = p0 + p1 + p2;
            dc * dc - 4.0 * (p0 * p1 + 4.0 * p0 * p2 + p1 * p2) * phi + 16.0 * p0 * p2 * phi * phi
        }
        let num = power(self.b0, self.b1, self.b2, phi).max(0.0);
        let den = power(1.0, self.a1, self.a2, phi).max(1e-300);
        (num / den).sqrt()
    }

    fn normalized(b0: f64, b1: f64, b2: f64, a0: f64, a1: f64, a2: f64) -> Self {
        let inv_a0 = 1.0 / a0;
        Self {
            b0: b0 * inv_a0,
            b1: b1 * inv_a0,
            b2: b2 * inv_a0,
            a1: a1 * inv_a0,
            a2: a2 * inv_a0,
        }
    }
}

/// Clamp frequency away from DC and Nyquist, and Q away from zero, to keep
/// the bilinear transform well-conditioned.
fn clamp_params(sr: f32, freq: f32, q: f32) -> (f32, f32) {
    // Floor the Nyquist estimate so a zero/negative/NaN sample rate
    // can't produce an inverted clamp range — `clamp(10.0, x)` panics
    // when `x < 10.0` (and on NaN bounds).
    let nyquist = (sr * 0.5).max(20.0);
    let f = freq.clamp(10.0, nyquist * 0.995);
    let q = q.max(0.05);
    (f, q)
}

/// f64 twin of [`clamp_params`].
fn clamp_params_f64(sr: f64, freq: f64, q: f64) -> (f64, f64) {
    // Floor the Nyquist estimate so a zero/negative/NaN sample rate
    // can't produce an inverted clamp range — `clamp(10.0, x)` panics
    // when `x < 10.0` (and on NaN bounds).
    let nyquist = (sr * 0.5).max(20.0);
    let f = freq.clamp(10.0, nyquist * 0.995);
    let q = q.max(0.05);
    (f, q)
}
