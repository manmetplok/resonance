//! Ducking of the reverb's wet return.
//!
//! The classic ducked vocal reverb (warmth-width-depth.md §3.3): the wet
//! return is pulled down while the lead sings and blooms in the gaps, so
//! the vocal stays upfront without the reverb being turned down overall.
//!
//! The detector is the external sidechain **key** when the host has
//! connected one — the usual setup is a shared reverb on a return bus,
//! keyed from the dry lead — and otherwise this plugin's own dry input
//! (self-ducking, the same behaviour as `resonance-delay`'s `duck_*`).
//! Only the wet signal is ducked; the dry passes untouched.
//!
//! The gain computer is the delay's: a peak detector, a soft knee at
//! `threshold` with an infinite ratio above it (a ducker wants a hand-off,
//! not a compression curve), and the reduction capped at
//! `amount × DUCK_MAX_GR_DB`. So with the key held well over the
//! threshold the wet settles exactly `amount × 24` dB down, which is what
//! makes the amount knob readable as a depth in dB.

use resonance_dsp::dynamics::{soft_knee_gain_reduction_db, Ballistics};

/// Gain reduction at `duck_amount = 1.0`, in dB.
pub const DUCK_MAX_GR_DB: f32 = 24.0;

/// Knee width of the ducker's gain computer, in dB.
const DUCK_KNEE_DB: f32 = 6.0;

/// The wet-return ducker's envelope, carried across blocks.
pub struct Ducker {
    sample_rate: f32,
    /// Smoothed gain reduction, dB (non-negative).
    gr_db: f32,
    ballistics: Ballistics,
    attack_ms: f32,
    release_ms: f32,
}

impl Ducker {
    pub fn new(sample_rate: f32) -> Self {
        let sample_rate = sample_rate.max(1.0);
        Self {
            sample_rate,
            gr_db: 0.0,
            ballistics: Ballistics::from_times(sample_rate, 15.0, 200.0),
            attack_ms: 15.0,
            release_ms: 200.0,
        }
    }

    pub fn clear(&mut self) {
        self.gr_db = 0.0;
    }

    /// Current gain reduction in dB, for the editor's meter.
    pub fn gain_reduction_db(&self) -> f32 {
        self.gr_db
    }

    /// Refresh the ballistics when a time changed. Per block is plenty for
    /// a control nobody sweeps at audio rate.
    pub fn prepare_block(&mut self, attack_ms: f32, release_ms: f32) {
        if (attack_ms - self.attack_ms).abs() > f32::EPSILON
            || (release_ms - self.release_ms).abs() > f32::EPSILON
        {
            self.attack_ms = attack_ms;
            self.release_ms = release_ms;
            self.ballistics = Ballistics::from_times(self.sample_rate, attack_ms, release_ms);
        }
    }

    /// Advance one sample and return the gain for the wet signal.
    /// `det_l`/`det_r` are this sample's detector input: the key, or the
    /// dry input when no key is connected.
    ///
    /// At `amount = 0` with no reduction left to recover this is exactly
    /// `1.0` without any arithmetic, so a reverb that never ducks renders
    /// bit-identically to one that has no ducker.
    #[inline]
    pub fn next_gain(&mut self, det_l: f32, det_r: f32, amount: f32, threshold_db: f32) -> f32 {
        if amount <= 0.0 {
            if self.gr_db == 0.0 {
                return 1.0;
            }
            // Let a residual reduction recover rather than snapping the
            // wet back to full the moment the amount reaches zero.
            self.gr_db = self.ballistics.step_envelope(self.gr_db, 0.0);
            if self.gr_db < 1e-4 {
                self.gr_db = 0.0;
            }
            return db_to_linear(-self.gr_db);
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
        db_to_linear(-self.gr_db)
    }
}

#[inline]
fn db_to_linear(db: f32) -> f32 {
    10f32.powf(db / 20.0)
}
