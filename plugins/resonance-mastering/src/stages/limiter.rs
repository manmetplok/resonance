//! Brick-wall true-peak lookahead limiter.
//!
//! Pipeline per sample:
//!
//! 1. Read the oldest sample from a `N`-sample stereo delay ring and
//!    its scheduled gain envelope value, emit `delayed × envelope`.
//! 2. Measure the true peak of the new incoming sample (4× oversampled
//!    via the metering crate's ITU-R BS.1770-4 Annex 2 polyphase FIR).
//!    The FIR has ~6 input samples of group delay, so this reading
//!    describes the sample written `TP_GROUP_DELAY` iterations ago.
//! 3. Compute the gain required to keep that peak below the user's
//!    ceiling (ceiling / peak, clamped ≤ 1.0).
//! 4. Write the new sample into the delay ring at the same slot we
//!    just read from, with an envelope bounded by the release ramp
//!    from the previous position. Apply the required gain from step 3
//!    at the ring position `TP_GROUP_DELAY` slots earlier — the sample
//!    the detector actually measured — then forward-propagate the
//!    release ramp from there up to the just-written position.
//! 5. Back-propagate the attack ramp: walk the envelope ring backwards
//!    from the constrained position, lowering any earlier envelope
//!    values that would violate the linear attack slope. Because the
//!    envelope is always ramp-consistent on entry to each iteration,
//!    the walk short-circuits as soon as an existing value already
//!    satisfies the new constraint.
//!
//! The back-propagation guarantees that by the time the loud sample
//! reaches the output position (after `N` iterations) the gain has
//! already ramped down to the target. This is a true brick-wall — no
//! inter-sample peak can exceed the ceiling.

use resonance_dsp::db_to_linear;
use resonance_metering::true_peak::coefficients::{PHASES, TAPS};
use resonance_metering::true_peak::polyphase::PolyphasePeakDetector;

/// Fixed lookahead time in milliseconds. 5 ms at 48 kHz = 240 samples,
/// plenty of runway for a band-music master without excessive added
/// latency.
const LOOKAHEAD_MS: f32 = 5.0;

/// Group delay of the true-peak upsampler in input samples: the
/// `TAPS * PHASES`-tap prototype FIR delays by `(TAPS * PHASES - 1) / 2`
/// output samples, i.e. just under `TAPS / 2` input samples (rounded up
/// to 6). Peak readings describe the sample pushed this many calls ago.
const TP_GROUP_DELAY: usize = (TAPS * PHASES).div_ceil(2 * PHASES);

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LimiterConfig {
    pub enabled: bool,
    /// Output ceiling in dBTP. Typical choice: -0.3 dBTP for streaming
    /// delivery, -1.0 dBTP for extra safety with lossy downstream codecs.
    pub ceiling_db: f32,
    /// Release time in milliseconds. Linear release from the gain floor
    /// back to unity — short values chase transients, long values
    /// preserve perceived loudness and dynamics.
    pub release_ms: f32,
}

impl Default for LimiterConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            ceiling_db: -0.3,
            release_ms: 50.0,
        }
    }
}

pub struct Limiter {
    sample_rate: f32,
    lookahead_samples: usize,

    delay_l: Vec<f32>,
    delay_r: Vec<f32>,
    envelope: Vec<f32>,
    write_pos: usize,

    peak_l: PolyphasePeakDetector,
    peak_r: PolyphasePeakDetector,

    /// Held GR for the UI meter, linear.
    meter_gr_lin: f32,
    meter_decay: f32,
}

impl Limiter {
    pub fn new(sample_rate: f32) -> Self {
        let lookahead_samples =
            ((LOOKAHEAD_MS * 0.001 * sample_rate).ceil() as usize).max(TP_GROUP_DELAY + 2);
        Self {
            sample_rate,
            lookahead_samples,
            delay_l: vec![0.0; lookahead_samples],
            delay_r: vec![0.0; lookahead_samples],
            envelope: vec![1.0; lookahead_samples],
            write_pos: 0,
            peak_l: PolyphasePeakDetector::new(),
            peak_r: PolyphasePeakDetector::new(),
            meter_gr_lin: 1.0,
            meter_decay: (-1.0_f32 / (0.25 * sample_rate)).exp(),
        }
    }

    pub fn reset(&mut self) {
        self.delay_l.fill(0.0);
        self.delay_r.fill(0.0);
        self.envelope.fill(1.0);
        self.write_pos = 0;
        self.peak_l.reset();
        self.peak_r.reset();
        self.meter_gr_lin = 1.0;
    }

    /// Reported latency. Matches the lookahead length because the
    /// limiter outputs the oldest sample in the delay ring and uses
    /// the rest of the ring for peak detection.
    pub fn latency(&self) -> usize {
        self.lookahead_samples
    }

    /// Current held gain reduction in dB (positive = reduction).
    pub fn meter_gr_db(&self) -> f32 {
        let lin = self.meter_gr_lin.max(1e-12);
        -20.0 * lin.log10()
    }

    pub fn process_stereo(&mut self, left: &mut [f32], right: &mut [f32], cfg: &LimiterConfig) {
        let frames = left.len().min(right.len());
        if frames == 0 {
            return;
        }

        // Bypass path: still run the delay line so plugin latency
        // stays constant when the user toggles the limiter on/off.
        if !cfg.enabled {
            for i in 0..frames {
                let out_l = self.delay_l[self.write_pos];
                let out_r = self.delay_r[self.write_pos];
                self.delay_l[self.write_pos] = left[i];
                self.delay_r[self.write_pos] = right[i];
                left[i] = out_l;
                right[i] = out_r;
                self.write_pos = (self.write_pos + 1) % self.lookahead_samples;
            }
            // Decay the GR meter toward 0 dB (= 1.0 linear).
            self.meter_gr_lin = 1.0 - (1.0 - self.meter_gr_lin) * self.meter_decay;
            return;
        }

        let ceiling_lin = db_to_linear(cfg.ceiling_db);
        let n_la = self.lookahead_samples;
        // The constraint lands TP_GROUP_DELAY positions behind the write
        // head, so only that much runway remains before the constrained
        // sample reaches the output. The linear attack ramp must cover a
        // unit gain drop within it.
        let attack_run = n_la.saturating_sub(1 + TP_GROUP_DELAY).max(1);
        let attack_step = 1.0_f32 / attack_run as f32;
        // Linear release: full recovery in `release_ms` milliseconds.
        let release_samples = (cfg.release_ms.max(1.0) * 0.001 * self.sample_rate).max(1.0);
        let release_step = 1.0_f32 / release_samples;

        let mut min_env_block: f32 = 1.0;

        for i in 0..frames {
            // Step 1: read output.
            let out_gain = self.envelope[self.write_pos];
            let out_l = self.delay_l[self.write_pos] * out_gain;
            let out_r = self.delay_r[self.write_pos] * out_gain;

            // Step 2: measure true peak (per channel, 4× oversampled;
            // reset_peak before each push so `peak()` reports only this
            // push's peak, not the held max). Because of the FIR's group
            // delay, this reading describes the sample written
            // TP_GROUP_DELAY iterations ago, not `left[i]`.
            self.peak_l.reset_peak();
            self.peak_r.reset_peak();
            self.peak_l.push_sample(left[i]);
            self.peak_r.push_sample(right[i]);
            let peak = self.peak_l.peak().max(self.peak_r.peak());

            // Step 3: required gain for the measured sample.
            let required = if peak > ceiling_lin {
                (ceiling_lin / peak).min(1.0)
            } else {
                1.0
            };

            // Step 4: write the new sample into the delay ring with the
            // release-ramp envelope. Release bound is the envelope of
            // the sample output *just before* the new one — that's
            // `envelope[prev_pos]` where prev_pos = write_pos - 1.
            let prev_pos = if self.write_pos == 0 {
                n_la - 1
            } else {
                self.write_pos - 1
            };
            let release_bound = (self.envelope[prev_pos] + release_step).min(1.0);

            self.delay_l[self.write_pos] = left[i];
            self.delay_r[self.write_pos] = right[i];
            self.envelope[self.write_pos] = release_bound;

            // Apply the required gain at the ring position the detector
            // actually measured — TP_GROUP_DELAY slots behind the write
            // head — then keep the ring ramp-consistent on both sides.
            let peak_pos = (self.write_pos + n_la - TP_GROUP_DELAY) % n_la;
            if required < self.envelope[peak_pos] {
                self.envelope[peak_pos] = required;

                // Forward-propagate the release ramp over the newer
                // samples (peak_pos+1 ..= write_pos) so the gain
                // recovers at release_step per sample after the peak.
                let mut bound = required + release_step;
                let mut idx = (peak_pos + 1) % n_la;
                for _ in 0..TP_GROUP_DELAY {
                    if self.envelope[idx] <= bound {
                        break;
                    }
                    self.envelope[idx] = bound;
                    bound += release_step;
                    idx = (idx + 1) % n_la;
                }

                // Step 5: back-propagate the attack ramp. Walks
                // backwards through the envelope ring, raising the
                // constraint by attack_step per position. Early-out
                // when an existing value already satisfies the new
                // constraint — this is valid because the envelope was
                // ramp-consistent before this iteration started.
                let mut bound = required + attack_step;
                let mut idx = if peak_pos == 0 { n_la - 1 } else { peak_pos - 1 };
                let mut steps = 0;
                while steps < attack_run && bound < 1.0 {
                    if self.envelope[idx] <= bound {
                        break;
                    }
                    self.envelope[idx] = bound;
                    bound += attack_step;
                    idx = if idx == 0 { n_la - 1 } else { idx - 1 };
                    steps += 1;
                }
            }

            // Emit the output sample.
            left[i] = out_l;
            right[i] = out_r;

            if out_gain < min_env_block {
                min_env_block = out_gain;
            }

            self.write_pos = (self.write_pos + 1) % n_la;
        }

        // GR meter: track the lowest gain seen this block with slow decay.
        self.meter_gr_lin = if min_env_block < self.meter_gr_lin {
            min_env_block
        } else {
            1.0 - (1.0 - self.meter_gr_lin) * self.meter_decay
        };
    }
}

