//! The two auxiliary sources mixed in before the filter: a sub oscillator
//! one or two octaves under osc1, and a noise generator.
//!
//! Both are per voice, mono and centred (they are the body and the air of a
//! patch, not part of its stereo image), and both are skipped outright at a
//! level of zero, so a patch that does not use them renders bit-identically
//! and — for the noise — draws nothing from the engine's shared RNG.
//!
//! Neither calls a transcendental per sample: the sine is a polynomial and
//! the square is a polyBLEP-corrected naive square.

use resonance_dsp::SimpleRng;

/// Sub oscillator waveform. Values are the `sub_waveform` parameter's
/// integers.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
#[repr(u8)]
pub enum SubWave {
    #[default]
    Sine = 0,
    Square = 1,
}

impl SubWave {
    pub const LABELS: [&'static str; 2] = ["Sine", "Square"];

    pub fn from_int(v: i32) -> Self {
        match v {
            1 => Self::Square,
            _ => Self::Sine,
        }
    }
}

/// Sub oscillator octave below osc1. Values are the `sub_octave`
/// parameter's integers.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
#[repr(u8)]
pub enum SubOctave {
    #[default]
    Down1 = 0,
    Down2 = 1,
}

impl SubOctave {
    pub const LABELS: [&'static str; 2] = ["-1 Oct", "-2 Oct"];

    pub fn from_int(v: i32) -> Self {
        match v {
            1 => Self::Down2,
            _ => Self::Down1,
        }
    }

    /// Offset from osc1's pitch.
    pub fn semitones(self) -> f32 {
        match self {
            Self::Down1 => -12.0,
            Self::Down2 => -24.0,
        }
    }
}

/// Noise colour. Values are the `noise_type` parameter's integers.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
#[repr(u8)]
pub enum NoiseType {
    #[default]
    White = 0,
    Pink = 1,
}

impl NoiseType {
    pub const LABELS: [&'static str; 2] = ["White", "Pink"];

    pub fn from_int(v: i32) -> Self {
        match v {
            1 => Self::Pink,
            _ => Self::White,
        }
    }
}

/// Corner of the one-pole split the noise `Color` tilt works around.
pub const NOISE_TILT_HZ: f32 = 1_000.0;

/// `sin(2π p)` for `p` in 0..1, from a 9th-order Taylor polynomial on the
/// folded quarter-wave. Worst-case error is about 4e-6 (-108 dB), so the
/// sub's harmonics sit far below anything audible, at five multiplies.
#[inline]
pub fn sine_approx(p: f64) -> f32 {
    // sin(2πp) = sin(π s), s = 2p in 0..2; fold into -0.5..=0.5.
    let mut s = 2.0 * p as f32;
    if s > 1.0 {
        s -= 2.0;
    }
    if s > 0.5 {
        s = 1.0 - s;
    } else if s < -0.5 {
        s = -1.0 - s;
    }
    let z = std::f32::consts::PI * s;
    let z2 = z * z;
    z * (1.0 + z2 * (-1.0 / 6.0 + z2 * (1.0 / 120.0 + z2 * (-1.0 / 5040.0 + z2 / 362_880.0))))
}

/// The standard two-sample polyBLEP residual for a unit-per-side step at
/// phase 0 of an oscillator advancing `dt` per sample.
#[inline]
fn poly_blep(t: f64, dt: f64) -> f32 {
    if t < dt {
        let t = t / dt;
        (t + t - t * t - 1.0) as f32
    } else if t > 1.0 - dt {
        let t = (t - 1.0) / dt;
        (t * t + t + t + 1.0) as f32
    } else {
        0.0
    }
}

/// Per-voice sub oscillator state.
#[derive(Clone, Copy, Default)]
pub struct SubOsc {
    pub phase: f64,
}

impl SubOsc {
    /// One sample at increment `inc`, then advance.
    #[inline]
    pub fn next(&mut self, wave: SubWave, inc: f64) -> f32 {
        let p = self.phase;
        let out = match wave {
            SubWave::Sine => sine_approx(p),
            SubWave::Square => {
                let naive = if p < 0.5 { 1.0 } else { -1.0 };
                let mut half = p + 0.5;
                if half >= 1.0 {
                    half -= 1.0;
                }
                naive + poly_blep(p, inc) - poly_blep(half, inc)
            }
        };
        self.phase += inc;
        self.phase -= self.phase.floor();
        out
    }
}

/// Per-voice noise state: Paul Kellet's three-pole "economy" pink filter
/// and the one-pole low band the tilt is built on.
#[derive(Clone, Copy, Default)]
pub struct NoiseGen {
    b0: f32,
    b1: f32,
    b2: f32,
    low: f32,
}

impl NoiseGen {
    /// One sample. `tilt_coeff` is the one-pole coefficient for
    /// [`NOISE_TILT_HZ`] at the running rate; `color` in -1..=1 tilts the
    /// spectrum from the low band alone (-1) through flat (0) to the input
    /// with its high band doubled (+1).
    #[inline]
    pub fn next(&mut self, rng: &mut SimpleRng, kind: NoiseType, tilt_coeff: f32, color: f32) -> f32 {
        // Uniform in -1..1 from the top 24 bits: exact in an f32.
        let white = (rng.next_u32() >> 8) as f32 * (2.0 / 16_777_216.0) - 1.0;
        let raw = match kind {
            NoiseType::White => white,
            NoiseType::Pink => {
                self.b0 = 0.997_65 * self.b0 + white * 0.099_046;
                self.b1 = 0.963 * self.b1 + white * 0.296_516_4;
                self.b2 = 0.57 * self.b2 + white * 1.052_691_3;
                // The filter's gain sums to about 3.5 at DC; 0.25 brings
                // its level back near the white noise it is made from.
                (self.b0 + self.b1 + self.b2 + white * 0.184_8) * 0.25
            }
        };
        if color == 0.0 {
            return raw;
        }
        self.low += tilt_coeff * (raw - self.low);
        self.low + (1.0 + color) * (raw - self.low)
    }
}
