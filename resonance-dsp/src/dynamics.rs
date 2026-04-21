//! Shared building blocks for feed-forward dynamics processors
//! (compressors, expanders, limiters).
//!
//! This module exists because the track compressor, mastering glue
//! compressor, and multiband per-band compressors all share the same
//! log-domain topology: a detector produces a dB level, a static
//! soft-knee curve maps it to gain reduction, and an attack/release
//! ballistic section smooths the GR envelope. Pulling these pieces up
//! keeps the math in one place so a fix in one plugin automatically
//! applies to the others.

/// Soft-knee log-domain gain computer.
///
/// Given the detector level in dB, the threshold in dB, the knee width
/// in dB, and the compression slope (`1 − 1/ratio`), returns the gain
/// reduction to apply in dB. The return value is always non-negative
/// (a compressor can only attenuate).
///
/// The curve is continuous and C¹-smooth: below `threshold − knee/2`
/// it's zero, above `threshold + knee/2` it's linear with the given
/// slope, and in between it's a quadratic interpolation.
///
/// `half_knee` is passed in explicitly so hot-loop callers can hoist
/// the `knee * 0.5` multiply out of the inner loop.
#[inline]
pub fn soft_knee_gain_reduction_db(
    detector_db: f32,
    threshold_db: f32,
    knee_db: f32,
    half_knee_db: f32,
    slope: f32,
) -> f32 {
    let over = detector_db - threshold_db;
    if knee_db > 0.0 && over > -half_knee_db && over < half_knee_db {
        let x = over + half_knee_db;
        slope * (x * x) / (2.0 * knee_db)
    } else if over > 0.0 {
        slope * over
    } else {
        0.0
    }
}

/// Attack/release coefficients for a one-pole GR-envelope smoother.
///
/// The caller converts attack/release times in milliseconds into
/// sample-rate-dependent exp coefficients once per block, then calls
/// [`step_envelope`] per sample to smooth a target gain-reduction value
/// toward the current envelope.
#[derive(Debug, Clone, Copy)]
pub struct Ballistics {
    pub attack_coef: f32,
    pub release_coef: f32,
}

impl Ballistics {
    /// Build a `Ballistics` pair from attack / release times in
    /// milliseconds at the given sample rate. Both times are clamped
    /// to a sensible minimum to avoid division by zero.
    pub fn from_times(sample_rate: f32, attack_ms: f32, release_ms: f32) -> Self {
        let attack_samples = (attack_ms.max(0.1) * 0.001 * sample_rate).max(1.0);
        let release_samples = (release_ms.max(1.0) * 0.001 * sample_rate).max(1.0);
        Self {
            attack_coef: (-1.0_f32 / attack_samples).exp(),
            release_coef: (-1.0_f32 / release_samples).exp(),
        }
    }

    /// Advance the GR envelope one sample toward `target_db`. Returns
    /// the new envelope value. When the target exceeds the current
    /// envelope the attack coefficient applies; otherwise the release
    /// coefficient.
    #[inline]
    pub fn step_envelope(&self, current_db: f32, target_db: f32) -> f32 {
        let coef = if target_db > current_db {
            self.attack_coef
        } else {
            self.release_coef
        };
        target_db + (current_db - target_db) * coef
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn below_threshold_is_zero_gr() {
        // Detector well below threshold — hard knee, any slope.
        let gr = soft_knee_gain_reduction_db(-30.0, -20.0, 0.0, 0.0, 0.5);
        assert_eq!(gr, 0.0);
    }

    #[test]
    fn above_threshold_applies_slope() {
        // 10 dB over threshold, 4:1 ratio → slope 0.75 → 7.5 dB GR.
        let slope = 1.0 - 1.0 / 4.0;
        let gr = soft_knee_gain_reduction_db(-10.0, -20.0, 0.0, 0.0, slope);
        assert!((gr - 7.5).abs() < 1e-4);
    }

    #[test]
    fn soft_knee_is_continuous_at_edges() {
        let knee = 6.0;
        let half_knee = knee * 0.5;
        let threshold = -20.0;
        let slope = 0.75;
        // Just below lower knee edge = 0 GR.
        let lower = soft_knee_gain_reduction_db(
            threshold - half_knee - 0.01,
            threshold,
            knee,
            half_knee,
            slope,
        );
        assert!(lower.abs() < 1e-2);
        // At upper knee edge the knee formula should match the linear
        // formula with a tight tolerance.
        let at_edge =
            soft_knee_gain_reduction_db(threshold + half_knee, threshold, knee, half_knee, slope);
        let linear = slope * half_knee;
        assert!(
            (at_edge - linear).abs() < 1e-4,
            "knee {at_edge} vs linear {linear}"
        );
    }

    #[test]
    fn attack_is_faster_than_release() {
        // Attack 1 ms, release 100 ms at 48 kHz.
        let b = Ballistics::from_times(48_000.0, 1.0, 100.0);
        assert!(b.attack_coef < b.release_coef);
    }

    #[test]
    fn envelope_converges_to_target() {
        // Step from 0 dB current to 6 dB target; envelope should climb.
        let b = Ballistics::from_times(48_000.0, 1.0, 100.0);
        let mut cur = 0.0_f32;
        for _ in 0..1000 {
            cur = b.step_envelope(cur, 6.0);
        }
        assert!(cur > 5.9, "cur = {cur}");
    }
}
