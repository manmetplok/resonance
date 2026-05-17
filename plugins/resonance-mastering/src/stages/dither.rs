//! Final-stage dither.
//!
//! Adds triangular-PDF dither to the stereo output, sized for a
//! user-selected target bit depth. The plugin output stays in 32-bit
//! float — the dither is in the signal by the time the DAW runs its
//! own export-time quantization, which is exactly what you want for
//! mastering delivery.
//!
//! Optional first-order high-pass noise shaper pushes the dither noise
//! energy up the spectrum so the audible noise floor is lower at the
//! same LSB-relative level. Useful for aggressive 16-bit delivery.
//!
//! Per-sample cost: two RNG calls per channel plus (if enabled) one
//! multiply-add of feedback. Zero added latency.

use resonance_dsp::SimpleRng;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DitherConfig {
    pub enabled: bool,
    /// Target bit depth for the dither magnitude. 16 = CD, 20 = broadcast,
    /// 24 = high-res delivery. Dither amplitude scales as `2^-(bits-1)`.
    pub target_bits: i32,
    /// Apply first-order high-pass shaping to the dither noise.
    pub noise_shape: bool,
}

impl Default for DitherConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            target_bits: 16,
            noise_shape: false,
        }
    }
}

/// Feedback coefficient for the first-order noise shaper. 0.5 gives a
/// gentle high-pass tilt, enough to pull low-frequency noise down ~6 dB
/// without making the top octave objectionable.
const NS_FEEDBACK: f32 = 0.5;

pub struct Dither {
    rng_l: SimpleRng,
    rng_r: SimpleRng,
    prev_l: f32,
    prev_r: f32,
}

impl Dither {
    pub fn new() -> Self {
        Self {
            // Distinct seeds so L and R dither sequences are uncorrelated.
            rng_l: SimpleRng::new(0x9E37_79B1_7F4A_7C15),
            rng_r: SimpleRng::new(0x61C8_8646_80B5_83EB),
            prev_l: 0.0,
            prev_r: 0.0,
        }
    }

    pub fn reset(&mut self) {
        self.prev_l = 0.0;
        self.prev_r = 0.0;
    }

    pub fn process_stereo(&mut self, left: &mut [f32], right: &mut [f32], cfg: &DitherConfig) {
        if !cfg.enabled {
            return;
        }
        // LSB at the target bit depth for signed-symmetric audio.
        let lsb = 2.0_f32.powi(-(cfg.target_bits.clamp(8, 32) - 1));
        let frames = left.len().min(right.len());
        for i in 0..frames {
            let mut d_l = tpdf_sample(&mut self.rng_l, lsb);
            let mut d_r = tpdf_sample(&mut self.rng_r, lsb);
            if cfg.noise_shape {
                let shaped_l = d_l - NS_FEEDBACK * self.prev_l;
                let shaped_r = d_r - NS_FEEDBACK * self.prev_r;
                self.prev_l = shaped_l;
                self.prev_r = shaped_r;
                d_l = shaped_l;
                d_r = shaped_r;
            }
            left[i] += d_l;
            right[i] += d_r;
        }
    }
}

impl Default for Dither {
    fn default() -> Self {
        Self::new()
    }
}

/// Generate one TPDF sample scaled to `lsb`: the sum of two independent
/// uniform samples in `[-lsb/2, lsb/2]`, giving a triangular distribution
/// on `[-lsb, lsb]` with peak density at zero.
#[inline]
#[doc(hidden)]
pub fn tpdf_sample(rng: &mut SimpleRng, lsb: f32) -> f32 {
    let u1 = u32_to_symmetric_unit(rng.next_u32());
    let u2 = u32_to_symmetric_unit(rng.next_u32());
    (u1 + u2) * 0.5 * lsb
}

/// Map a `u32` to a float in `[-1.0, 1.0]` (inclusive of 0, exclusive
/// of exact ±1.0). Uses 24 bits of entropy which is sufficient for f32
/// precision.
#[inline]
fn u32_to_symmetric_unit(x: u32) -> f32 {
    // Take the top 24 bits, scale to [0, 1), remap to [-1, 1).
    let v = (x >> 8) as f32 * (1.0 / ((1u32 << 24) as f32));
    v * 2.0 - 1.0
}

