//! Real-time streaming monophonic pitch tracker.
//!
//! Audio-thread counterpart of the offline [`YinDetector`](crate::pitch):
//! a SIFT-style front end (fixed ~1 kHz lowpass + 4–8× decimation,
//! Markel 1972) feeding an MPM tracker (McLeod & Wyvill, "A Smarter Way
//! to Find Pitch") on the decimated band. It powers pitch-synchronous
//! granulation (research doc #252 §4): besides the running f0 estimate it
//! emits *period markers* — absolute input-sample positions spaced one
//! period apart — that a granulator can align grain onsets to.
//!
//! Why MPM rather than YIN here: the NSDF key-maximum rule (pick among
//! peaks after the first zero crossing, take the first within 90% of the
//! highest) gives strong octave-error immunity without a material-tuned
//! threshold, and its peak value is a natural `[0, 1]` clarity measure
//! for the voiced/unvoiced gate. On the decimated band the O(W·τ_max)
//! NSDF costs a few tens of kFLOPs per 16 ms hop — negligible. The
//! offline YIN detector stays for clip-length vocal analysis.
//!
//! [`PitchTracker::new`] pre-allocates every buffer; [`feed`] performs no
//! allocation and takes no locks, so it is safe on the audio thread.
//!
//! [`feed`]: PitchTracker::feed

use crate::biquad::Biquad;

/// Detection range lower bound, Hz.
const F_MIN_HZ: f32 = 60.0;
/// Detection range upper bound, Hz.
const F_MAX_HZ: f32 = 800.0;
/// Anti-alias / band-limit lowpass cutoff ahead of the decimator, Hz.
const LOWPASS_HZ: f32 = 1000.0;
/// Target decimated sample rate, Hz (factor clamped to 4–8×).
const TARGET_DECIMATED_SR: f32 = 8000.0;
/// MPM key-maximum tolerance: first peak within this fraction of the
/// tallest peak wins (favours the longest — lowest — strong candidate).
const KEY_MAX_TOLERANCE: f32 = 0.9;
/// Clarity needed to switch to voiced …
const CLARITY_ON: f32 = 0.65;
/// … and clarity below which a voiced track releases (hysteresis).
const CLARITY_OFF: f32 = 0.5;
/// Decimated-frame RMS below this is treated as silence.
const SILENCE_RMS: f32 = 1.0e-4;
/// Relative period change treated as continuous (larger = a jump).
const CONTINUITY_TOLERANCE: f32 = 0.25;
/// Consecutive analyses a jumped period must persist before it is
/// accepted (suppresses one-frame octave glitches).
const JUMP_CONFIRM_FRAMES: u32 = 3;
/// Capacity of the per-`feed` marker buffer. At the shortest period
/// (800 Hz) this covers blocks of several hundred milliseconds; markers
/// beyond it are dropped rather than allocated.
const MARKER_CAPACITY: usize = 4096;

/// Latest pitch estimate reported by [`PitchTracker::feed`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PitchEstimate {
    /// Estimated fundamental in Hz. Meaningful only when
    /// [`voiced`](Self::voiced) is `true`; `0.0` otherwise.
    pub f0_hz: f32,
    /// Fundamental period in *full-rate* samples (`sample_rate / f0_hz`);
    /// `0.0` when unvoiced.
    pub period_samples: f32,
    /// NSDF peak clarity in `[0, 1]` of the most recent analysis frame.
    pub clarity: f32,
    /// Whether a reliable pitch is currently being tracked.
    pub voiced: bool,
}

impl PitchEstimate {
    const UNVOICED: Self = Self {
        f0_hz: 0.0,
        period_samples: 0.0,
        clarity: 0.0,
        voiced: false,
    };
}

/// Streaming monophonic pitch tracker for audio-thread use.
///
/// Construct once with [`new`](Self::new) (allocates everything), then
/// call [`feed`](Self::feed) with successive input blocks of any length.
/// The tracker analyses the low-passed, decimated signal every ~16 ms
/// and keeps the estimate from the most recent analysis in between.
#[derive(Debug, Clone)]
pub struct PitchTracker {
    sample_rate: f32,
    /// Two cascaded RBJ lowpass sections ≈ 4th-order Butterworth at
    /// [`LOWPASS_HZ`]: −48 dB at the decimated Nyquist keeps aliases out
    /// of the tracking band.
    lp1: Biquad,
    lp2: Biquad,
    /// Decimation factor (4–8), full rate → decimated rate.
    factor: usize,
    /// Phase of the decimation counter, `0..factor`.
    decim_phase: usize,

    /// Ring of decimated samples, `ring.len()` is a power of two.
    ring: Vec<f32>,
    ring_mask: usize,
    ring_write: usize,
    /// Total decimated samples written (saturating at usize::MAX is a
    /// non-issue: u64-scale counts).
    ring_filled: usize,
    /// Analysis frame length in decimated samples (power of two).
    frame_len: usize,
    /// Decimated samples between analyses.
    hop: usize,
    /// Decimated samples accumulated since the last analysis.
    since_analysis: usize,

    /// Lag search range in decimated samples.
    tau_min: usize,
    tau_max: usize,
    /// Scratch: ordered analysis frame (oldest → newest).
    frame: Vec<f32>,
    /// Scratch: NSDF values for lags `0..=tau_max`.
    nsdf: Vec<f32>,
    /// Scratch: candidate peaks `(refined_lag, refined_value)`.
    candidates: Vec<(f32, f32)>,

    /// Current estimate (held between analyses).
    estimate: PitchEstimate,
    /// Accepted period in decimated samples (only valid while voiced).
    period_dec: f32,
    /// Pending post-jump period and its confirmation count.
    jump_period_dec: f32,
    jump_count: u32,

    /// Absolute count of full-rate input samples consumed.
    total_samples: u64,
    /// Absolute full-rate position of the next period marker.
    next_marker: f64,
    /// Markers emitted during the most recent [`feed`](Self::feed) call.
    markers: Vec<f64>,
}

impl PitchTracker {
    /// Create a tracker for `sample_rate`, pre-allocating all buffers.
    ///
    /// # Panics
    /// Panics if `sample_rate` is not finite or is below 8 kHz (the
    /// front end needs headroom above the 1 kHz tracking band).
    pub fn new(sample_rate: f32) -> Self {
        assert!(
            sample_rate.is_finite() && sample_rate >= 8000.0,
            "sample_rate must be finite and >= 8 kHz"
        );
        let factor = ((sample_rate / TARGET_DECIMATED_SR).round() as usize).clamp(4, 8);
        let decimated_sr = sample_rate / factor as f32;

        let tau_max = (decimated_sr / F_MIN_HZ).ceil() as usize;
        let tau_min = ((decimated_sr / F_MAX_HZ).floor() as usize).max(2);
        let frame_len = (2 * tau_max).next_power_of_two();
        let hop = frame_len / 4;

        let mut lp1 = Biquad::identity();
        let mut lp2 = Biquad::identity();
        // Butterworth section Qs for a 4th-order lowpass.
        lp1.set_low_pass(sample_rate, LOWPASS_HZ, 0.541_196);
        lp2.set_low_pass(sample_rate, LOWPASS_HZ, 1.306_563);

        Self {
            sample_rate,
            lp1,
            lp2,
            factor,
            decim_phase: 0,
            ring: vec![0.0; frame_len],
            ring_mask: frame_len - 1,
            ring_write: 0,
            ring_filled: 0,
            frame_len,
            hop,
            since_analysis: 0,
            tau_min,
            tau_max,
            frame: vec![0.0; frame_len],
            nsdf: vec![0.0; tau_max + 1],
            candidates: Vec::with_capacity(tau_max / 2 + 2),
            estimate: PitchEstimate::UNVOICED,
            period_dec: 0.0,
            jump_period_dec: 0.0,
            jump_count: 0,
            total_samples: 0,
            next_marker: 0.0,
            markers: Vec::with_capacity(MARKER_CAPACITY),
        }
    }

    /// The sample rate this tracker was built for.
    pub fn sample_rate(&self) -> f32 {
        self.sample_rate
    }

    /// Feed one block of full-rate samples and return the latest
    /// estimate. Allocation-free and lock-free; blocks may be any length
    /// (including empty).
    ///
    /// Period markers emitted while consuming this block are available
    /// from [`markers`](Self::markers) until the next `feed` call.
    pub fn feed(&mut self, block: &[f32]) -> PitchEstimate {
        self.markers.clear();
        for &x in block {
            let filtered = self.lp2.process(self.lp1.process(x));
            self.decim_phase += 1;
            if self.decim_phase == self.factor {
                self.decim_phase = 0;
                self.ring[self.ring_write] = filtered;
                self.ring_write = (self.ring_write + 1) & self.ring_mask;
                self.ring_filled = self.ring_filled.saturating_add(1);
                self.since_analysis += 1;
                if self.ring_filled >= self.frame_len && self.since_analysis >= self.hop {
                    self.since_analysis = 0;
                    self.analyze();
                }
            }
            self.total_samples += 1;
            if self.estimate.voiced {
                let pos = self.total_samples as f64;
                let period = f64::from(self.estimate.period_samples);
                while self.next_marker <= pos {
                    if self.markers.len() < self.markers.capacity() {
                        self.markers.push(self.next_marker);
                    }
                    self.next_marker += period;
                }
            }
        }
        self.estimate
    }

    /// The most recent estimate without feeding new audio.
    pub fn latest(&self) -> PitchEstimate {
        self.estimate
    }

    /// Period markers (absolute full-rate sample positions of pitch
    /// epochs) emitted during the most recent [`feed`](Self::feed) call,
    /// in ascending order. Empty while unvoiced.
    pub fn markers(&self) -> &[f64] {
        &self.markers
    }

    /// Reset all state (filters, ring, estimate, marker train) without
    /// touching the pre-allocated buffers.
    pub fn reset(&mut self) {
        self.lp1.reset();
        self.lp2.reset();
        self.decim_phase = 0;
        self.ring.fill(0.0);
        self.ring_write = 0;
        self.ring_filled = 0;
        self.since_analysis = 0;
        self.estimate = PitchEstimate::UNVOICED;
        self.period_dec = 0.0;
        self.jump_period_dec = 0.0;
        self.jump_count = 0;
        self.total_samples = 0;
        self.next_marker = 0.0;
        self.markers.clear();
    }

    /// Analyse the newest `frame_len` decimated samples and update the
    /// running estimate.
    fn analyze(&mut self) {
        // Copy the ring into an ordered frame (oldest → newest) and
        // remove the mean so residual DC cannot fake periodicity.
        let start = self.ring_write; // oldest sample in a full ring
        for i in 0..self.frame_len {
            self.frame[i] = self.ring[(start + i) & self.ring_mask];
        }
        let mean = self.frame.iter().sum::<f32>() / self.frame_len as f32;
        let mut energy = 0.0;
        for s in self.frame.iter_mut() {
            *s -= mean;
            energy += *s * *s;
        }
        let rms = (energy / self.frame_len as f32).sqrt();
        if rms < SILENCE_RMS {
            self.set_unvoiced(0.0);
            return;
        }

        self.compute_nsdf(energy);
        self.pick_candidates();
        if self.candidates.is_empty() {
            self.set_unvoiced(0.0);
            return;
        }

        // McLeod key-maximum rule: the tallest peak sets the bar; the
        // first candidate (ascending lag) within tolerance of it wins.
        // This selects the fundamental period over its sub-octave peaks
        // (longer lags, similar height) while harmonics (shorter lags,
        // clearly lower NSDF) stay under the bar.
        let best_val = self
            .candidates
            .iter()
            .map(|&(_, v)| v)
            .fold(f32::MIN, f32::max);
        let bar = KEY_MAX_TOLERANCE * best_val;
        let chosen = self
            .candidates
            .iter()
            .copied()
            .find(|&(_, v)| v >= bar)
            .unwrap_or(self.candidates[0]);

        let clarity = chosen.1.clamp(0.0, 1.0);
        let voiced_gate = if self.estimate.voiced {
            CLARITY_OFF
        } else {
            CLARITY_ON
        };
        let decimated_sr = self.sample_rate / self.factor as f32;
        let f0 = decimated_sr / chosen.0;
        if clarity < voiced_gate || !(F_MIN_HZ..=F_MAX_HZ).contains(&f0) {
            self.set_unvoiced(clarity);
            return;
        }

        // Octave-jump hysteresis: a period step beyond the continuity
        // tolerance must persist for JUMP_CONFIRM_FRAMES analyses before
        // it replaces the tracked period.
        let was_voiced = self.estimate.voiced;
        let new_period = if !was_voiced {
            self.jump_count = 0;
            chosen.0
        } else {
            let rel = (chosen.0 - self.period_dec).abs() / self.period_dec;
            if rel <= CONTINUITY_TOLERANCE {
                self.jump_count = 0;
                chosen.0
            } else {
                let near_pending = self.jump_period_dec > 0.0
                    && (chosen.0 - self.jump_period_dec).abs() / self.jump_period_dec <= 0.1;
                if near_pending {
                    self.jump_count += 1;
                } else {
                    self.jump_period_dec = chosen.0;
                    self.jump_count = 1;
                }
                if self.jump_count >= JUMP_CONFIRM_FRAMES {
                    self.jump_count = 0;
                    chosen.0
                } else {
                    // Hold the previous period until the jump confirms.
                    self.period_dec
                }
            }
        };

        self.period_dec = new_period;
        let period_full = new_period * self.factor as f32;
        self.estimate = PitchEstimate {
            f0_hz: self.sample_rate / period_full,
            period_samples: period_full,
            clarity,
            voiced: true,
        };
        if !was_voiced {
            // Start the marker train at the current stream position.
            self.next_marker = self.total_samples as f64;
        }
    }

    /// Drop to unvoiced, ending the marker train.
    fn set_unvoiced(&mut self, clarity: f32) {
        self.estimate = PitchEstimate {
            clarity,
            ..PitchEstimate::UNVOICED
        };
        self.jump_count = 0;
    }

    /// Normalised square difference function over the analysis frame:
    /// `n(τ) = 2·Σ x_j·x_{j+τ} / Σ (x_j² + x_{j+τ}²)`, `j < W − τ`.
    /// `energy` is `Σ_{j<W} x_j²`, reused for the incremental
    /// denominator (McLeod's m'(τ) recurrence).
    fn compute_nsdf(&mut self, energy: f32) {
        let w = self.frame_len;
        self.nsdf[0] = 1.0;
        let mut m = 2.0 * energy;
        for tau in 1..=self.tau_max {
            // Remove the two samples that leave the correlation window.
            m -= self.frame[w - tau] * self.frame[w - tau];
            m -= self.frame[tau - 1] * self.frame[tau - 1];
            let mut r = 0.0;
            for j in 0..w - tau {
                r += self.frame[j] * self.frame[j + tau];
            }
            self.nsdf[tau] = if m > f32::EPSILON { 2.0 * r / m } else { 0.0 };
        }
    }

    /// Collect NSDF key maxima: after the first negative-going zero
    /// crossing, the highest point of each positive region becomes a
    /// candidate (parabolically refined). Restricting candidates to
    /// after the first crossing rejects the trivial lag-0 lobe that
    /// low-passed noise correlates within.
    fn pick_candidates(&mut self) {
        self.candidates.clear();
        let mut tau = 1;
        // Skip the initial positive lobe around lag 0.
        while tau <= self.tau_max && self.nsdf[tau] > 0.0 {
            tau += 1;
        }
        while tau <= self.tau_max {
            // Skip the non-positive region.
            while tau <= self.tau_max && self.nsdf[tau] <= 0.0 {
                tau += 1;
            }
            if tau > self.tau_max {
                break;
            }
            // Track the maximum of this positive region.
            let mut peak_tau = tau;
            while tau <= self.tau_max && self.nsdf[tau] > 0.0 {
                if self.nsdf[tau] > self.nsdf[peak_tau] {
                    peak_tau = tau;
                }
                tau += 1;
            }
            if peak_tau >= self.tau_min
                && peak_tau <= self.tau_max
                && self.candidates.len() < self.candidates.capacity()
            {
                self.candidates.push(self.refine_peak(peak_tau));
            }
        }
    }

    /// Parabolic interpolation of the NSDF around `tau` for sub-sample
    /// lag (and peak-value) precision.
    fn refine_peak(&self, tau: usize) -> (f32, f32) {
        if tau == 0 || tau >= self.tau_max {
            return (tau as f32, self.nsdf[tau]);
        }
        let s0 = self.nsdf[tau - 1];
        let s1 = self.nsdf[tau];
        let s2 = self.nsdf[tau + 1];
        let denom = s0 + s2 - 2.0 * s1;
        if denom.abs() < f32::EPSILON {
            return (tau as f32, s1);
        }
        let delta = (0.5 * (s0 - s2) / denom).clamp(-1.0, 1.0);
        let value = s1 - 0.25 * (s0 - s2) * delta;
        (tau as f32 + delta, value)
    }
}
