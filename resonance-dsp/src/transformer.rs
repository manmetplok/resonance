//! Frequency-weighted drive: pre-/de-emphasis around a shaper, and a
//! transformer-style stage built from it.
//!
//! A transformer's core flux is proportional to V/f, so bass saturates
//! first. The cheap model of that is a low-shelf boost *into* the shaper
//! and the exact inverse shelf *after* it: at small signal the pair
//! cancels (the chain stays flat), but loud low frequencies reach the
//! curve hotter than highs and distort first. The same pair with a
//! negative gain weights the drive toward the highs, which is the
//! "response" tilt of a character plugin.
//!
//! All types are fixed-size and allocation-free. Coefficient setters do a
//! few transcendentals and are block-rate; `process` is per sample. When
//! the owner oversamples, pass the *oversampled* rate to the setters and
//! run `process` inside the high-rate loop.

use crate::saturate::{Adaa1, Curve};
use crate::Biquad;

/// Shelf slope used by the emphasis pair (0.707 = maximally flat).
const EMPHASIS_Q: f32 = std::f32::consts::FRAC_1_SQRT_2;

/// A low-shelf pre-emphasis and its exact inverse. With the RBJ shelf,
/// `H(−g) = 1/H(+g)` holds exactly for the same corner and slope, so
/// `post(pre(x))` equals `x` up to float rounding (the f32 biquads leave
/// ≈ −85 dB of rounding noise at a 300 Hz corner). Gain 0 is an exact
/// passthrough.
#[derive(Clone, Copy, Debug, Default)]
pub struct EmphasisPair {
    pre: Biquad,
    post: Biquad,
    active: bool,
}

impl EmphasisPair {
    pub fn new() -> Self {
        Self::default()
    }

    /// `gain_db` > 0 drives frequencies below `corner_hz` harder;
    /// `gain_db` < 0 weights the drive toward the highs.
    pub fn set(&mut self, sample_rate: f32, corner_hz: f32, gain_db: f32) {
        self.active = gain_db != 0.0;
        self.pre.set_low_shelf(sample_rate, corner_hz, EMPHASIS_Q, gain_db);
        self.post.set_low_shelf(sample_rate, corner_hz, EMPHASIS_Q, -gain_db);
    }

    pub fn reset(&mut self) {
        self.pre.reset();
        self.post.reset();
    }

    /// Emphasis, before the nonlinearity.
    #[inline]
    pub fn pre(&mut self, x: f32) -> f32 {
        if self.active {
            self.pre.process(x)
        } else {
            x
        }
    }

    /// De-emphasis, after the nonlinearity.
    #[inline]
    pub fn post(&mut self, x: f32) -> f32 {
        if self.active {
            self.post.process(x)
        } else {
            x
        }
    }
}

/// LF-weighted drive, the transformer behaviour:
///
/// `x → emphasis → ADAA curve(drive·x)/drive → de-emphasis → sub-sonic
/// HPF → HF resonance`
///
/// - The output is divided by `drive`, so at small signal the stage has
///   the curve's small-signal gain (1 for the default [`Curve::Tanh`])
///   at any drive.
/// - The sub-sonic high-pass is second order with a little resonance
///   (Q ≈ 0.9 gives a ≈ 0.7 dB bump just above the corner, as a real
///   transformer's coupling does). It also removes the DC an asymmetric
///   curve adds. Off by default.
/// - The HF resonance is a broad bell near the top of the band. Off
///   (0 dB) by default.
#[derive(Clone, Copy, Debug)]
pub struct LfWeightedDrive {
    emphasis: EmphasisPair,
    adaa: Adaa1,
    curve: Curve,
    drive: f32,
    subsonic: Biquad,
    subsonic_on: bool,
    hf_res: Biquad,
    hf_res_on: bool,
}

impl Default for LfWeightedDrive {
    fn default() -> Self {
        Self {
            emphasis: EmphasisPair::new(),
            adaa: Adaa1::new(),
            curve: Curve::Tanh,
            drive: 1.0,
            subsonic: Biquad::identity(),
            subsonic_on: false,
            hf_res: Biquad::identity(),
            hf_res_on: false,
        }
    }
}

impl LfWeightedDrive {
    /// Default Q of the sub-sonic high-pass.
    pub const SUBSONIC_Q: f32 = 0.9;

    pub fn new() -> Self {
        Self::default()
    }

    /// The emphasis corner and boost (see [`EmphasisPair::set`]).
    pub fn set_emphasis(&mut self, sample_rate: f32, corner_hz: f32, gain_db: f32) {
        self.emphasis.set(sample_rate, corner_hz, gain_db);
    }

    /// Linear drive into the curve (floored at 1e-3).
    pub fn set_drive(&mut self, drive: f32) {
        self.drive = if drive > 1.0e-3 { drive } else { 1.0e-3 };
    }

    pub fn set_curve(&mut self, curve: Curve) {
        self.curve = curve;
    }

    /// Sub-sonic high-pass corner and Q; `freq_hz` ≤ 0 turns it off.
    pub fn set_subsonic(&mut self, sample_rate: f32, freq_hz: f32, q: f32) {
        self.subsonic_on = freq_hz > 0.0;
        if self.subsonic_on {
            self.subsonic.set_high_pass(sample_rate, freq_hz, q);
        }
    }

    /// HF resonance bell; `gain_db` 0 turns it off.
    pub fn set_hf_resonance(&mut self, sample_rate: f32, freq_hz: f32, gain_db: f32) {
        self.hf_res_on = gain_db != 0.0;
        self.hf_res.set_bell(sample_rate, freq_hz, 0.8, gain_db);
    }

    pub fn reset(&mut self) {
        self.emphasis.reset();
        self.adaa.reset();
        self.subsonic.reset();
        self.hf_res.reset();
    }

    #[inline]
    pub fn process(&mut self, x: f32) -> f32 {
        let e = self.emphasis.pre(x);
        let y = self.adaa.process(&self.curve, self.drive * e) / self.drive;
        let mut y = self.emphasis.post(y);
        if self.subsonic_on {
            y = self.subsonic.process(y);
        }
        if self.hf_res_on {
            y = self.hf_res.process(y);
        }
        y
    }
}
