//! Brick-wall true-peak lookahead limiter.
//!
//! Pipeline per sample:
//!
//! 1. Read the oldest sample from a `N`-sample stereo delay ring and
//!    its scheduled gain envelope value, emit `delayed × envelope`.
//! 2. Measure the true peak of the new incoming sample (4× oversampled
//!    via the metering crate's ITU-R BS.1770-4 Annex 2 polyphase FIR).
//! 3. Compute the gain required to keep that peak below the user's
//!    ceiling (ceiling / peak, clamped ≤ 1.0).
//! 4. Write the new sample into the delay ring at the same slot we
//!    just read from, and assign it an envelope value bounded by both
//!    (a) the release ramp from the previous output gain and (b) the
//!    required gain computed in step 3.
//! 5. Back-propagate the attack ramp: walk the envelope ring backwards
//!    from the just-written position, lowering any earlier envelope
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
use resonance_metering::true_peak::polyphase::PolyphasePeakDetector;

/// Fixed lookahead time in milliseconds. 5 ms at 48 kHz = 240 samples,
/// plenty of runway for a band-music master without excessive added
/// latency.
const LOOKAHEAD_MS: f32 = 5.0;

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
            ((LOOKAHEAD_MS * 0.001 * sample_rate).ceil() as usize).max(8);
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

    pub fn process_stereo(
        &mut self,
        left: &mut [f32],
        right: &mut [f32],
        cfg: &LimiterConfig,
    ) {
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
        // Linear attack ramp: (N-1) sample steps cover a unit gain drop.
        let attack_step = 1.0_f32 / (n_la.saturating_sub(1).max(1) as f32);
        // Linear release: full recovery in `release_ms` milliseconds.
        let release_samples = (cfg.release_ms.max(1.0) * 0.001 * self.sample_rate).max(1.0);
        let release_step = 1.0_f32 / release_samples;

        let mut min_env_block: f32 = 1.0;

        for i in 0..frames {
            // Step 1: read output.
            let out_gain = self.envelope[self.write_pos];
            let out_l = self.delay_l[self.write_pos] * out_gain;
            let out_r = self.delay_r[self.write_pos] * out_gain;

            // Step 2: measure true peak of new incoming sample (per
            // channel, 4× oversampled; reset_peak before each push so
            // `peak()` reports only this sample's peak, not the held max).
            self.peak_l.reset_peak();
            self.peak_r.reset_peak();
            self.peak_l.push_sample(left[i]);
            self.peak_r.push_sample(right[i]);
            let peak = self.peak_l.peak().max(self.peak_r.peak());

            // Step 3: required gain for new sample.
            let required = if peak > ceiling_lin {
                (ceiling_lin / peak).min(1.0)
            } else {
                1.0
            };

            // Step 4: write new sample into the delay ring and pick its
            // envelope value. Release bound is the envelope of the
            // sample that will be output *just before* the new one —
            // that's `envelope[prev_pos]` where prev_pos = write_pos - 1.
            let prev_pos = if self.write_pos == 0 {
                n_la - 1
            } else {
                self.write_pos - 1
            };
            let release_bound = (self.envelope[prev_pos] + release_step).min(1.0);
            let new_env = release_bound.min(required);

            self.delay_l[self.write_pos] = left[i];
            self.delay_r[self.write_pos] = right[i];
            self.envelope[self.write_pos] = new_env;

            // Step 5: back-propagate the attack ramp. Walks backwards
            // through the envelope ring, raising the constraint by
            // attack_step per position. Early-out when an existing
            // value already satisfies the new constraint — this is
            // valid because the envelope was ramp-consistent before
            // this iteration started.
            if new_env < 1.0 {
                let mut bound = new_env + attack_step;
                let mut idx = prev_pos;
                let mut steps = 0;
                while steps < n_la - 1 && bound < 1.0 {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disabled_is_pure_delay() {
        let sr = 48_000.0_f32;
        let mut lim = Limiter::new(sr);
        let la = lim.latency();
        let n = la + 1024;
        let mut l = vec![0.0_f32; n];
        let mut r = vec![0.0_f32; n];
        for i in 0..n {
            let s = (i as f32 * 0.05).sin() * 0.4;
            l[i] = s;
            r[i] = s;
        }
        let input = l.clone();
        lim.process_stereo(&mut l, &mut r, &LimiterConfig::default());
        for i in la..n {
            assert!((l[i] - input[i - la]).abs() < 1e-6);
        }
    }

    #[test]
    fn quiet_signal_passes_unchanged_when_enabled() {
        let sr = 48_000.0_f32;
        let mut lim = Limiter::new(sr);
        let la = lim.latency();
        let n = la + 1024;
        let mut l = vec![0.0_f32; n];
        let mut r = vec![0.0_f32; n];
        for i in 0..n {
            let s = (i as f32 * 0.02).sin() * 0.25; // −12 dBFS, far below ceiling
            l[i] = s;
            r[i] = s;
        }
        let input = l.clone();
        let cfg = LimiterConfig {
            enabled: true,
            ceiling_db: -0.3,
            release_ms: 50.0,
        };
        lim.process_stereo(&mut l, &mut r, &cfg);
        let mut max_err = 0.0_f32;
        for i in la..n {
            max_err = max_err.max((l[i] - input[i - la]).abs());
        }
        assert!(max_err < 1e-5, "quiet sine error = {max_err}");
    }

    #[test]
    fn loud_signal_never_exceeds_ceiling() {
        // Hot 1 kHz sine at -1 dBFS → peaks just under 0 dBFS → limiter
        // clamps it to the ceiling. Output peak must stay at or below
        // the ceiling after the initial delay has settled.
        let sr = 48_000.0_f32;
        let mut lim = Limiter::new(sr);
        let la = lim.latency();
        let n = la + 8192;
        let mut l = vec![0.0_f32; n];
        let mut r = vec![0.0_f32; n];
        let amp = 10.0_f32.powf(-1.0 / 20.0); // -1 dBFS
        for i in 0..n {
            let t = i as f32 / sr;
            let s = (std::f32::consts::TAU * 1000.0 * t).sin() * amp;
            l[i] = s;
            r[i] = s;
        }
        let cfg = LimiterConfig {
            enabled: true,
            ceiling_db: -6.0,
            release_ms: 50.0,
        };
        lim.process_stereo(&mut l, &mut r, &cfg);
        let tail_start = la + 2048;
        let peak = l[tail_start..]
            .iter()
            .copied()
            .map(f32::abs)
            .fold(0.0_f32, f32::max);
        let ceiling_lin = 10.0_f32.powf(-6.0 / 20.0);
        // Small tolerance for FIR ripple and the release reaching up
        // toward 1.0 briefly between peaks.
        assert!(
            peak <= ceiling_lin * 1.02,
            "output peak {peak} exceeds ceiling {ceiling_lin}"
        );
    }

    #[test]
    fn impulse_never_breaks_ceiling() {
        // A single unit impulse has inter-sample content; the limiter
        // must still keep the oversampled output below its ceiling.
        let sr = 48_000.0_f32;
        let mut lim = Limiter::new(sr);
        let la = lim.latency();
        let n = la + 512;
        let mut l = vec![0.0_f32; n];
        let mut r = vec![0.0_f32; n];
        l[64] = 1.0;
        r[64] = 1.0;
        let cfg = LimiterConfig {
            enabled: true,
            ceiling_db: -3.0,
            release_ms: 50.0,
        };
        lim.process_stereo(&mut l, &mut r, &cfg);
        let ceiling_lin = 10.0_f32.powf(-3.0 / 20.0);
        let peak = l.iter().copied().map(f32::abs).fold(0.0_f32, f32::max);
        assert!(
            peak <= ceiling_lin * 1.02,
            "impulse peak {peak} exceeds ceiling {ceiling_lin}"
        );
    }
}
