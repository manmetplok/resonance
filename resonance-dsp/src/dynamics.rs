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
    /// milliseconds at the given sample rate. Both times and the sample
    /// rate are clamped to sensible minimums to avoid division by zero;
    /// a zero/negative/NaN sample rate degrades to instant ballistics
    /// instead of producing non-finite coefficients.
    pub fn from_times(sample_rate: f32, attack_ms: f32, release_ms: f32) -> Self {
        let sample_rate = sample_rate.max(1.0);
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

