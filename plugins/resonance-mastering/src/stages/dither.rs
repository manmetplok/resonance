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

    pub fn process_stereo(
        &mut self,
        left: &mut [f32],
        right: &mut [f32],
        cfg: &DitherConfig,
    ) {
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
fn tpdf_sample(rng: &mut SimpleRng, lsb: f32) -> f32 {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disabled_passes_audio_unchanged() {
        let mut d = Dither::new();
        let mut l = vec![0.1, 0.2, -0.3, 0.4];
        let mut r = vec![-0.1, 0.2, 0.3, -0.4];
        let el = l.clone();
        let er = r.clone();
        d.process_stereo(&mut l, &mut r, &DitherConfig::default());
        assert_eq!(l, el);
        assert_eq!(r, er);
    }

    #[test]
    fn enabled_dither_stays_within_two_lsb() {
        // TPDF on [-lsb, lsb] means any added noise has magnitude ≤ lsb.
        // Feed silence and verify the output never exceeds ±lsb.
        let mut d = Dither::new();
        let cfg = DitherConfig {
            enabled: true,
            target_bits: 16,
            noise_shape: false,
        };
        let lsb = 2.0_f32.powi(-15);
        let n = 8192;
        let mut l = vec![0.0_f32; n];
        let mut r = vec![0.0_f32; n];
        d.process_stereo(&mut l, &mut r, &cfg);
        let peak = l
            .iter()
            .chain(r.iter())
            .copied()
            .map(f32::abs)
            .fold(0.0_f32, f32::max);
        assert!(peak <= lsb * 1.01, "TPDF peak = {peak} vs lsb {lsb}");
    }

    #[test]
    fn dither_magnitude_scales_with_bit_depth() {
        // 24-bit dither should be much quieter than 16-bit dither.
        let mut d16 = Dither::new();
        let mut d24 = Dither::new();
        let n = 4096;
        let mut l16 = vec![0.0_f32; n];
        let mut r16 = vec![0.0_f32; n];
        let mut l24 = vec![0.0_f32; n];
        let mut r24 = vec![0.0_f32; n];
        d16.process_stereo(
            &mut l16,
            &mut r16,
            &DitherConfig {
                enabled: true,
                target_bits: 16,
                noise_shape: false,
            },
        );
        d24.process_stereo(
            &mut l24,
            &mut r24,
            &DitherConfig {
                enabled: true,
                target_bits: 24,
                noise_shape: false,
            },
        );
        let rms16 = (l16.iter().map(|x| (*x as f64).powi(2)).sum::<f64>() / n as f64).sqrt();
        let rms24 = (l24.iter().map(|x| (*x as f64).powi(2)).sum::<f64>() / n as f64).sqrt();
        // 24-bit LSB is 256× smaller than 16-bit.
        let ratio = rms16 / rms24;
        assert!(
            ratio > 200.0 && ratio < 320.0,
            "16/24 dither RMS ratio = {ratio} (expected ≈ 256)"
        );
    }

    #[test]
    fn tpdf_sample_is_triangular() {
        // Generate many samples and verify they form a rough triangular
        // distribution: most mass near zero, bounded by ±lsb.
        let mut rng = SimpleRng::new(42);
        let lsb = 1.0_f32;
        let n = 100_000usize;
        let mut near_zero = 0usize;
        let mut near_edge = 0usize;
        let mut peak = 0.0_f32;
        for _ in 0..n {
            let v = tpdf_sample(&mut rng, lsb);
            if v.abs() < lsb * 0.2 {
                near_zero += 1;
            }
            if v.abs() > lsb * 0.8 {
                near_edge += 1;
            }
            peak = peak.max(v.abs());
        }
        assert!(peak <= lsb * 1.01, "peak = {peak}");
        // Triangular distribution: density near zero is ~5× density near
        // the edge (for |v| < 0.2*lsb vs |v| > 0.8*lsb windows).
        assert!(
            near_zero > near_edge * 3,
            "near_zero = {near_zero}, near_edge = {near_edge}"
        );
    }
}
