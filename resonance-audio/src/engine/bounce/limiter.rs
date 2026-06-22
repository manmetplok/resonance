//! True-peak brick-wall lookahead limiter for the loudness-normalized
//! export path (doc #196, ba todo #652).
//!
//! This is a port of the mastering plugin's brick-wall limiter
//! (`plugins/resonance-mastering/src/stages/limiter.rs`) specialized for
//! offline export: it folds the normalization gain trim into the same
//! pass, consumes/produces interleaved stereo `f32` frames (the bounce
//! render loop's format), and is a frame-count-preserving streaming stage
//! — the `lookahead_samples` of latency it introduces is absorbed
//! internally (leading silence dropped, the tail drained by [`flush`]) so
//! the encoded file has exactly as many frames as were rendered.
//!
//! The algorithm is unchanged from the plugin: a lookahead delay ring
//! whose per-sample gain envelope is constrained by the 4× oversampled
//! true peak (ITU-R BS.1770-4 Annex 2 polyphase FIR from
//! `resonance-metering`). Back-propagating an attack ramp and
//! forward-propagating a release ramp guarantees the gain has ramped down
//! before a loud sample reaches the output — a true brick wall, so no
//! inter-sample peak escapes the ceiling. See the plugin module for the
//! step-by-step derivation.
//!
//! [`flush`]: TruePeakLimiter::flush

use resonance_dsp::db_to_linear;
use resonance_metering::true_peak::coefficients::{PHASES, TAPS};
use resonance_metering::true_peak::polyphase::PolyphasePeakDetector;

/// Lookahead time in milliseconds. 5 ms matches the mastering limiter —
/// ample runway for the attack ramp without bloating export latency.
const LOOKAHEAD_MS: f32 = 5.0;

/// Release time in milliseconds. Linear recovery from the gain floor back
/// to unity. 50 ms is the mastering limiter's default: short enough to
/// chase transients, long enough to stay transparent on sustained
/// material.
const RELEASE_MS: f32 = 50.0;

/// Group delay of the true-peak upsampler in input samples (see the
/// mastering limiter): peak readings describe the sample pushed this many
/// `step` calls ago.
const TP_GROUP_DELAY: usize = (TAPS * PHASES).div_ceil(2 * PHASES);

/// Gain-trim + true-peak brick-wall limiter as a streaming interleaved
/// stereo stage. Construct with the normalization gain and the ceiling,
/// feed rendered frames through [`process`](Self::process), then drain
/// the lookahead tail with [`flush`](Self::flush).
pub(super) struct TruePeakLimiter {
    lookahead_samples: usize,
    /// Linear normalization gain applied to every input sample *before*
    /// limiting, so the limiter constrains the already-trimmed signal.
    gain_lin: f32,
    ceiling_lin: f32,
    attack_run: usize,
    attack_step: f32,
    release_step: f32,

    delay_l: Vec<f32>,
    delay_r: Vec<f32>,
    envelope: Vec<f32>,
    write_pos: usize,

    peak_l: PolyphasePeakDetector,
    peak_r: PolyphasePeakDetector,

    /// Leading output frames still to discard. Equal to `lookahead_samples`
    /// at construction (the delay ring's initial silence); once drained the
    /// stage emits one output frame per input frame.
    skip: usize,
}

impl TruePeakLimiter {
    pub(super) fn new(sample_rate: f32, gain_db: f32, ceiling_dbtp: f32) -> Self {
        let sample_rate = sample_rate.max(1.0);
        let lookahead_samples =
            ((LOOKAHEAD_MS * 0.001 * sample_rate).ceil() as usize).max(TP_GROUP_DELAY + 2);
        // The constraint lands TP_GROUP_DELAY positions behind the write
        // head; the attack ramp must cover a full unit gain drop within the
        // runway that remains before the constrained sample reaches output.
        let attack_run = lookahead_samples.saturating_sub(1 + TP_GROUP_DELAY).max(1);
        let release_samples = (RELEASE_MS * 0.001 * sample_rate).max(1.0);
        Self {
            lookahead_samples,
            gain_lin: db_to_linear(gain_db),
            ceiling_lin: db_to_linear(ceiling_dbtp),
            attack_run,
            attack_step: 1.0 / attack_run as f32,
            release_step: 1.0 / release_samples,
            delay_l: vec![0.0; lookahead_samples],
            delay_r: vec![0.0; lookahead_samples],
            envelope: vec![1.0; lookahead_samples],
            write_pos: 0,
            peak_l: PolyphasePeakDetector::new(),
            peak_r: PolyphasePeakDetector::new(),
            skip: lookahead_samples,
        }
    }

    /// Apply the gain trim to `mix` (interleaved stereo), limit true peaks
    /// to the ceiling, and append the aligned output frames to `out`.
    /// Leading lookahead-latency frames are dropped here so the stage is
    /// transparent in frame count and timing.
    pub(super) fn process(&mut self, mix: &[f32], out: &mut Vec<f32>) {
        for f in mix.chunks_exact(2) {
            let (ol, or) = self.step(f[0] * self.gain_lin, f[1] * self.gain_lin);
            if self.skip > 0 {
                self.skip -= 1;
            } else {
                out.push(ol);
                out.push(or);
            }
        }
    }

    /// Drain the `lookahead_samples` frames still held in the delay ring
    /// (the project's tail) by feeding silence, appending them to `out`.
    /// Call once after the final [`process`](Self::process).
    pub(super) fn flush(&mut self, out: &mut Vec<f32>) {
        for _ in 0..self.lookahead_samples {
            let (ol, or) = self.step(0.0, 0.0);
            if self.skip > 0 {
                self.skip -= 1;
            } else {
                out.push(ol);
                out.push(or);
            }
        }
    }

    /// One ring iteration: emit the oldest (gain-corrected) frame, measure
    /// the incoming frame's true peak, and constrain the gain envelope so
    /// the measured sample lands at or below the ceiling by the time it
    /// reaches the output. Returns the output (delayed) frame.
    #[inline]
    fn step(&mut self, in_l: f32, in_r: f32) -> (f32, f32) {
        let n_la = self.lookahead_samples;

        // Step 1: read the output (oldest ring slot × its envelope).
        let out_gain = self.envelope[self.write_pos];
        let out_l = self.delay_l[self.write_pos] * out_gain;
        let out_r = self.delay_r[self.write_pos] * out_gain;

        // Step 2: measure the incoming sample's true peak (per channel, 4×
        // oversampled). reset_peak before each push so `peak()` reports
        // this push only. The FIR group delay means it describes the sample
        // pushed TP_GROUP_DELAY iterations ago.
        self.peak_l.reset_peak();
        self.peak_r.reset_peak();
        self.peak_l.push_sample(in_l);
        self.peak_r.push_sample(in_r);
        let peak = self.peak_l.peak().max(self.peak_r.peak());

        // Step 3: gain required to hold that peak at the ceiling.
        let required = if peak > self.ceiling_lin {
            (self.ceiling_lin / peak).min(1.0)
        } else {
            1.0
        };

        // Step 4: write the new sample with a release-bounded envelope.
        let prev_pos = if self.write_pos == 0 {
            n_la - 1
        } else {
            self.write_pos - 1
        };
        let release_bound = (self.envelope[prev_pos] + self.release_step).min(1.0);
        self.delay_l[self.write_pos] = in_l;
        self.delay_r[self.write_pos] = in_r;
        self.envelope[self.write_pos] = release_bound;

        // Apply the required gain at the slot the detector measured, then
        // keep the ring ramp-consistent on both sides.
        let peak_pos = (self.write_pos + n_la - TP_GROUP_DELAY) % n_la;
        if required < self.envelope[peak_pos] {
            self.envelope[peak_pos] = required;

            // Forward-propagate the release ramp over the newer samples.
            let mut bound = required + self.release_step;
            let mut idx = (peak_pos + 1) % n_la;
            for _ in 0..TP_GROUP_DELAY {
                if self.envelope[idx] <= bound {
                    break;
                }
                self.envelope[idx] = bound;
                bound += self.release_step;
                idx = (idx + 1) % n_la;
            }

            // Step 5: back-propagate the attack ramp.
            let mut bound = required + self.attack_step;
            let mut idx = if peak_pos == 0 { n_la - 1 } else { peak_pos - 1 };
            let mut steps = 0;
            while steps < self.attack_run && bound < 1.0 {
                if self.envelope[idx] <= bound {
                    break;
                }
                self.envelope[idx] = bound;
                bound += self.attack_step;
                idx = if idx == 0 { n_la - 1 } else { idx - 1 };
                steps += 1;
            }
        }

        self.write_pos = (self.write_pos + 1) % n_la;
        (out_l, out_r)
    }
}
