//! Gate / downward-expander DSP.
//!
//! The topology mirrors the compressor's — detector → static curve →
//! ballistics — but runs the curve *downward*: signal **below** the
//! threshold is attenuated, rather than signal above it. Everything is in
//! the log domain so the gain reduction is a subtraction and the range
//! limit is a clamp.
//!
//! Three things separate a usable gate from a naive one, and all three
//! are here:
//!
//! * **Hysteresis.** Opening at the threshold and closing at the same
//!   level makes a signal hovering there chatter the gate open and shut.
//!   The close threshold sits `hysteresis` dB *below* the open threshold,
//!   so a signal has to actually fall away before the gate shuts.
//! * **Hold.** After the signal drops below the close threshold the gate
//!   stays open for `hold` ms. Without it, the gaps between syllables or
//!   between snare hits chop the tail.
//! * **Range.** A gate that closes to silence is rarely what you want on
//!   a drum bus; `range` caps the attenuation so the closed state ducks
//!   rather than mutes.
//!
//! The detector runs a peak envelope with instant attack and a short
//! release ([`DETECTOR_RELEASE_MS`]) rather than reading instantaneous
//! `|x|`. Without it the threshold comparison sees the waveform itself,
//! which crosses zero twice per cycle, and the gate chatters open and
//! shut at the signal's own frequency — audible as a buzz on anything
//! sustained, and it defeats hold and hysteresis alike because the state
//! machine is re-triggered constantly.

use resonance_dsp::dynamics::Ballistics;

/// Release time of the detector's peak envelope, in milliseconds. Long
/// enough to ride over the zero crossings of anything down to ~60 Hz,
/// short enough not to smear a real decay. Separate from the gate's own
/// `release` on purpose: a slow musical release must not make the
/// detector sluggish about noticing the next transient.
pub const DETECTOR_RELEASE_MS: f32 = 15.0;

/// Per-block gate settings, resolved once per `process` call.
pub struct GateSettings {
    pub threshold_db: f32,
    /// `1.0` is a hard gate; higher ratios expand more gently.
    pub ratio: f32,
    pub attack_ms: f32,
    pub hold_ms: f32,
    pub release_ms: f32,
    /// Maximum attenuation while closed, in dB (positive).
    pub range_db: f32,
    /// How far below `threshold_db` the gate closes.
    pub hysteresis_db: f32,
    /// Detector high-pass cutoff in Hz; `0` disables it. Keeps kick
    /// bleed from holding a snare gate open.
    pub key_hpf_hz: f32,
}

/// Target gain reduction in dB for a detector level, before ballistics.
///
/// Below the (already hysteresis-adjusted) threshold the expander slope
/// `ratio − 1` scales how far under the signal is; the result is clamped
/// to `range_db`. Above the threshold it is zero. Pure, so the static
/// curve can be checked without running a signal through it.
pub fn expander_gain_reduction_db(
    detector_db: f32,
    threshold_db: f32,
    ratio: f32,
    range_db: f32,
) -> f32 {
    let under = threshold_db - detector_db;
    if under <= 0.0 {
        return 0.0;
    }
    // ratio 1.0 => slope 0 => a gate that never attenuates. Clamp the
    // ratio at its minimum so the control cannot silently disable itself.
    let slope = (ratio.max(1.0) - 1.0).max(0.0);
    (under * slope).min(range_db.max(0.0))
}

/// One-pole high-pass on the detector path. Deliberately not the shared
/// `Biquad`: the key filter only needs a gentle 6 dB/oct tilt to stop
/// low-frequency bleed from holding the gate open, and a one-pole cannot
/// ring or overshoot the way a resonant biquad can when the key is
/// transient-heavy.
#[derive(Default, Clone, Copy)]
struct KeyHighPass {
    prev_in: f32,
    prev_out: f32,
    coef: f32,
}

impl KeyHighPass {
    fn set_cutoff(&mut self, hz: f32, sample_rate: f32) {
        if hz <= 0.0 || sample_rate <= 0.0 {
            self.coef = 0.0;
            return;
        }
        let rc = 1.0 / (std::f32::consts::TAU * hz.max(1.0));
        let dt = 1.0 / sample_rate;
        self.coef = rc / (rc + dt);
    }

    #[inline]
    fn process(&mut self, x: f32) -> f32 {
        if self.coef <= 0.0 {
            return x;
        }
        let y = self.coef * (self.prev_out + x - self.prev_in);
        self.prev_in = x;
        self.prev_out = y;
        y
    }

    fn reset(&mut self) {
        self.prev_in = 0.0;
        self.prev_out = 0.0;
    }
}

/// Whether the gate is letting signal through, and why.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GateState {
    /// Signal is above the open threshold.
    Open,
    /// Signal has fallen away but the hold timer is still running.
    Holding,
    /// Fully closed (attenuating by up to `range_db`).
    Closed,
}

pub struct GateDsp {
    sample_rate: f32,
    /// Smoothed gain reduction in dB (non-negative).
    gr_db: f32,
    state: GateState,
    /// Samples left on the hold timer.
    hold_remaining: u32,
    ballistics: Ballistics,
    cached_attack: f32,
    cached_release: f32,
    hpf_l: KeyHighPass,
    hpf_r: KeyHighPass,
    cached_hpf_hz: f32,
    /// Peak envelope of the detector signal, linear.
    det_env: f32,
    /// Per-sample decay applied to `det_env`.
    det_release_coef: f32,
    /// Peak gain reduction across the last block, for metering.
    pub last_gr_db: f32,
    /// Whether the gate was open at the end of the last block.
    pub last_open: bool,
}

impl GateDsp {
    pub fn new(sample_rate: f32) -> Self {
        let sample_rate = sample_rate.max(1.0);
        Self {
            sample_rate,
            gr_db: 0.0,
            state: GateState::Closed,
            hold_remaining: 0,
            ballistics: Ballistics::from_times(sample_rate, 1.0, 100.0),
            cached_attack: 1.0,
            cached_release: 100.0,
            hpf_l: KeyHighPass::default(),
            hpf_r: KeyHighPass::default(),
            cached_hpf_hz: -1.0,
            det_env: 0.0,
            det_release_coef: (-1.0 / (DETECTOR_RELEASE_MS * 0.001 * sample_rate)).exp(),
            last_gr_db: 0.0,
            last_open: false,
        }
    }

    pub fn reset(&mut self) {
        self.gr_db = 0.0;
        self.state = GateState::Closed;
        self.hold_remaining = 0;
        self.hpf_l.reset();
        self.hpf_r.reset();
        self.det_env = 0.0;
        self.last_gr_db = 0.0;
        self.last_open = false;
    }

    /// Refresh the coefficients that depend on time/frequency controls.
    /// Recomputed per block rather than per sample — exp and reciprocal
    /// are expensive, and neither control is audible at block rate.
    fn prepare_block(&mut self, s: &GateSettings) {
        if (s.attack_ms - self.cached_attack).abs() > f32::EPSILON
            || (s.release_ms - self.cached_release).abs() > f32::EPSILON
        {
            self.cached_attack = s.attack_ms;
            self.cached_release = s.release_ms;
            self.ballistics = Ballistics::from_times(self.sample_rate, s.attack_ms, s.release_ms);
        }
        if (s.key_hpf_hz - self.cached_hpf_hz).abs() > f32::EPSILON {
            self.cached_hpf_hz = s.key_hpf_hz;
            self.hpf_l.set_cutoff(s.key_hpf_hz, self.sample_rate);
            self.hpf_r.set_cutoff(s.key_hpf_hz, self.sample_rate);
        }
    }

    /// Process one block in place.
    ///
    /// `key` is the external sidechain signal when the host has connected
    /// one; `None` means the gate keys off its own input, which is the
    /// ordinary noise-gate case. That single substitution is the whole
    /// difference between "gate this track" and "gate this track from
    /// that one".
    pub fn process_block(
        &mut self,
        left: &mut [f32],
        right: &mut [f32],
        key: Option<(&[f32], &[f32])>,
        frames: usize,
        s: &GateSettings,
    ) {
        self.prepare_block(s);

        let hold_samples = (s.hold_ms * 0.001 * self.sample_rate).max(0.0) as u32;
        let close_threshold = s.threshold_db - s.hysteresis_db.max(0.0);
        let mut peak_gr = 0.0f32;

        for i in 0..frames {
            // Detector source: the external key when connected, else the
            // signal being gated.
            let (kl, kr) = match key {
                Some((l, r)) => (
                    l.get(i).copied().unwrap_or(0.0),
                    r.get(i).copied().unwrap_or(0.0),
                ),
                None => (left[i], right[i]),
            };
            let rectified = self
                .hpf_l
                .process(kl)
                .abs()
                .max(self.hpf_r.process(kr).abs());
            // Peak envelope: instant attack, exponential release. This is
            // what the threshold actually compares against.
            self.det_env = rectified.max(self.det_env * self.det_release_coef);
            let det_db = 20.0 * self.det_env.max(1e-9).log10();

            // Hysteresis + hold decide which threshold applies this
            // sample, so the static curve below sees a stable decision.
            self.state = match self.state {
                GateState::Open | GateState::Holding => {
                    if det_db >= s.threshold_db {
                        self.hold_remaining = hold_samples;
                        GateState::Open
                    } else if det_db >= close_threshold {
                        // Between the two thresholds: stay as we are.
                        self.state
                    } else if self.hold_remaining > 0 {
                        self.hold_remaining -= 1;
                        GateState::Holding
                    } else {
                        GateState::Closed
                    }
                }
                GateState::Closed => {
                    if det_db >= s.threshold_db {
                        self.hold_remaining = hold_samples;
                        GateState::Open
                    } else {
                        GateState::Closed
                    }
                }
            };

            let target = match self.state {
                GateState::Open | GateState::Holding => 0.0,
                GateState::Closed => {
                    expander_gain_reduction_db(det_db, s.threshold_db, s.ratio, s.range_db)
                }
            };
            self.gr_db = self.ballistics.step_envelope(self.gr_db, target);
            peak_gr = peak_gr.max(self.gr_db);

            let gain = 10f32.powf(-self.gr_db / 20.0);
            left[i] *= gain;
            right[i] *= gain;
        }

        self.last_gr_db = peak_gr;
        self.last_open = self.state != GateState::Closed;
    }
}
