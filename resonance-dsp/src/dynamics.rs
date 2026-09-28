//! Shared building blocks for feed-forward dynamics processors
//! (compressors, expanders, limiters).
//!
//! This module exists because the track compressor, mastering glue
//! compressor, and multiband per-band compressors all share the same
//! log-domain topology: a detector produces a dB level, a static
//! soft-knee curve maps it to gain reduction, and an attack/release
//! ballistic section smooths the GR envelope. Pulling these pieces up
//! keeps the math in one place so a fix in one plugin automatically
//! applies to the others. The wet-return [`Ducker`] the delay and the
//! reverb share is built from the same two pieces.

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


/// Gain reduction a [`Ducker`] makes at `amount = 1.0`, dB.
pub const DUCK_MAX_GR_DB: f32 = 24.0;

/// Knee width of the ducker's gain computer, dB.
const DUCK_KNEE_DB: f32 = 6.0;

/// A wet-signal ducker: pulls a signal down while a detector input is
/// over a threshold, and lets it bloom back in the gaps.
///
/// Shared by `resonance-delay` (keyed off its own dry input) and
/// `resonance-reverb` (keyed off the sidechain, or the dry input). The
/// gain computer is a peak detector — `max(|l|, |r|)` — into a soft knee
/// at `threshold` with an infinite ratio above it (a ducker wants a
/// hand-off, not a compression curve), the reduction capped at
/// `amount × DUCK_MAX_GR_DB`. So with the detector held well over the
/// threshold the signal settles exactly `amount × 24` dB down, which is
/// what makes an amount knob readable as a depth in dB.
#[derive(Debug, Clone)]
pub struct Ducker {
    sample_rate: f32,
    /// Smoothed gain reduction, dB (non-negative).
    gr_db: f32,
    ballistics: Ballistics,
    attack_ms: f32,
    release_ms: f32,
}

impl Ducker {
    pub fn new(sample_rate: f32, attack_ms: f32, release_ms: f32) -> Self {
        let sample_rate = sample_rate.max(1.0);
        Self {
            sample_rate,
            gr_db: 0.0,
            ballistics: Ballistics::from_times(sample_rate, attack_ms, release_ms),
            attack_ms,
            release_ms,
        }
    }

    pub fn clear(&mut self) {
        self.gr_db = 0.0;
    }

    /// Current gain reduction, dB.
    pub fn gain_reduction_db(&self) -> f32 {
        self.gr_db
    }

    /// Refresh the ballistics when a time changed. Per block is plenty
    /// for a control nobody sweeps at audio rate, and recomputing the exp
    /// coefficients per sample would be wasteful.
    pub fn set_times(&mut self, attack_ms: f32, release_ms: f32) {
        if (attack_ms - self.attack_ms).abs() > f32::EPSILON
            || (release_ms - self.release_ms).abs() > f32::EPSILON
        {
            self.attack_ms = attack_ms;
            self.release_ms = release_ms;
            self.ballistics = Ballistics::from_times(self.sample_rate, attack_ms, release_ms);
        }
    }

    /// Advance one sample and return the gain for the ducked signal.
    /// `det_l`/`det_r` are this sample's detector input.
    ///
    /// At `amount = 0` with no reduction left to recover this is exactly
    /// `1.0` without any arithmetic, so a signal that is never ducked
    /// renders bit-identically to one with no ducker. A residual reduction
    /// recovers on the release rather than snapping back.
    #[inline]
    pub fn next_gain(&mut self, det_l: f32, det_r: f32, amount: f32, threshold_db: f32) -> f32 {
        if amount <= 0.0 {
            if self.gr_db == 0.0 {
                return 1.0;
            }
            self.gr_db = self.ballistics.step_envelope(self.gr_db, 0.0);
            if self.gr_db < 1e-4 {
                self.gr_db = 0.0;
            }
            return duck_db_to_linear(-self.gr_db);
        }
        let detector = det_l.abs().max(det_r.abs());
        let detector = if detector.is_finite() { detector } else { 0.0 };
        let detector_db = 20.0 * detector.max(1e-9).log10();
        let raw_gr = soft_knee_gain_reduction_db(
            detector_db,
            threshold_db,
            DUCK_KNEE_DB,
            DUCK_KNEE_DB * 0.5,
            1.0,
        );
        let target = raw_gr.min(amount.min(1.0) * DUCK_MAX_GR_DB);
        self.gr_db = self.ballistics.step_envelope(self.gr_db, target);
        if !self.gr_db.is_finite() {
            self.gr_db = 0.0;
        }
        duck_db_to_linear(-self.gr_db)
    }
}

#[inline]
fn duck_db_to_linear(db: f32) -> f32 {
    10f32.powf(db / 20.0)
}
