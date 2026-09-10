//! Reverb orchestrator: wires the pre-delay, diffusion cascade, early
//! reflections, FDN feedback loop and final stereo mix together.
//!
//! Signal flow (Signalsmith / Geraint Luff style):
//!   Input -> Pre-delay -> 4-step Diffusion Network -> FDN Feedback Loop -> Stereo Output
//!                                     `--> Parallel Early Reflections ----^
//!
//! The diffusion network blurs the fresh input into dense reflections
//! using Hadamard mixing. The FDN provides the decaying tail with
//! Householder feedback and frequency-dependent damping. Early
//! reflections are an independent multi-tap delay summed into the wet
//! bus before the width/mix stage.

use resonance_dsp::DelayLine;

use super::diffusion::DiffusionStep;
use super::er::{EarlyReflections, ER_TAPS};
use super::fdn::FdnBank;
use super::{CHANNELS, DIFFUSION_STEPS};

/// Maximum pre-delay in seconds. The delay lines (and the tap clamp)
/// are sized from this at the *actual* sample rate at construction —
/// this used to be a hardcoded 48 000 samples, which was only "1
/// second" at 48 kHz and silently halved the maximum at 96 kHz.
const MAX_PREDELAY_SECONDS: f32 = 1.0;
/// Pre-delay tap-move crossfade length, ms (see `set_predelay`).
const PREDELAY_FADE_MS: f32 = 20.0;
/// Lower bound of the `size` parameter expressed in ms. Maps to the
/// shortest "room" feel the user can dial in.
const MIN_SIZE_MS: f32 = 10.0;
/// Upper bound of the `size` parameter expressed in ms. Also used to
/// pre-size every internal delay buffer so `set_size` can slide the
/// tap positions freely without the read index wrapping past the end
/// of a too-small buffer.
const MAX_SIZE_MS: f32 = 200.0;

/// The complete reverb processor.
pub struct ReverbDsp {
    sample_rate: f32,

    // Pre-delay. The read tap never relocates mid-signal: a change
    // crossfades from the old tap to the new one (see `set_predelay`).
    predelay_l: DelayLine,
    predelay_r: DelayLine,
    /// Settled tap length — and the fade *destination* while a fade runs.
    predelay_samples: usize,
    /// Tap being faded out while a fade runs.
    predelay_from: usize,
    /// Remaining crossfade samples; 0 means no fade is running.
    predelay_fade_left: u32,
    /// Total crossfade length in samples at this sample rate.
    predelay_fade_total: u32,
    /// Newest tap requested while a fade was already running; started
    /// as the next fade the moment the running one completes.
    predelay_pending: Option<usize>,
    /// Last tap length requested — dedupes per-block `set_predelay`.
    predelay_requested: usize,
    /// False until the first processed sample. While false a pre-delay
    /// change snaps (configuring a fresh/reset instance must not fade
    /// in from a stale tap).
    predelay_primed: bool,
    /// `MAX_PREDELAY_SECONDS` at the actual sample rate.
    max_predelay_samples: usize,

    // Diffusion network (4 cascaded steps)
    diffusion: [DiffusionStep; DIFFUSION_STEPS],
    /// Normalized tap positions (0..1) for each diffusion step + channel,
    /// computed once at creation to avoid re-randomization on size changes.
    diffusion_ratios: [[f32; CHANNELS]; DIFFUSION_STEPS],

    /// FDN feedback loop: delay bank, damping, modulation, recirculation.
    fdn: FdnBank,

    /// Early reflections (parallel multi-tap delay).
    er: EarlyReflections,

    // Current parameters (for per-sample smoothing)
    decay_gain: f32,
    mod_depth_samples: f32,
    room_size_ms: f32,
    frozen: bool,

    /// Running sum-of-squares for wet RMS (reset by `take_wet_rms`).
    wet_sumsq: f64,
    wet_count: u32,
}

impl ReverbDsp {
    pub fn new(sample_rate: f32) -> Self {
        // Diffusion steps. The delay *ranges* are halving multiples of
        // the current room size, and `set_size` slides the actual tap
        // positions in real-time — so every diffusion buffer is sized
        // for the widest range it will ever need to serve (at
        // `size=1.0`, i.e. `MAX_SIZE_MS`). This is what keeps the tap
        // reads inside the buffer no matter what `set_size` does.
        let mut max_diff_ms = MAX_SIZE_MS;
        let mut diffusion_ratios = [[0.0f32; CHANNELS]; DIFFUSION_STEPS];
        let diffusion = std::array::from_fn(|step| {
            max_diff_ms *= 0.5;
            let max_range_samples = (max_diff_ms * 0.001 * sample_rate) as usize;
            let ds = DiffusionStep::new(
                max_range_samples.max(64),
                (step as u64 + 1).wrapping_mul(0x517cc1b727220a95),
            );
            // Store normalized tap positions (against the MAX range)
            // so set_size() can rescale without re-randomizing and the
            // resulting tap positions always stay ≤ max_range_samples.
            let rs = max_range_samples.max(64);
            for (c, r) in diffusion_ratios[step].iter_mut().enumerate() {
                *r = ds.delay_samples[c] as f32 / rs as f32;
            }
            ds
        });

        let max_predelay_samples = (MAX_PREDELAY_SECONDS * sample_rate) as usize;

        Self {
            sample_rate,
            predelay_l: DelayLine::new(max_predelay_samples),
            predelay_r: DelayLine::new(max_predelay_samples),
            predelay_samples: 0,
            predelay_from: 0,
            predelay_fade_left: 0,
            predelay_fade_total: ((PREDELAY_FADE_MS * 0.001 * sample_rate) as u32).max(1),
            predelay_pending: None,
            predelay_requested: 0,
            predelay_primed: false,
            max_predelay_samples,
            diffusion,
            diffusion_ratios,
            fdn: FdnBank::new(sample_rate, MAX_SIZE_MS),
            er: EarlyReflections::new(sample_rate),
            decay_gain: 0.85,
            mod_depth_samples: 0.0,
            room_size_ms: 150.0,
            frozen: false,
            wet_sumsq: 0.0,
            wet_count: 0,
        }
    }

    /// Set early-reflections level (0..1, normalized).
    pub fn set_er_level(&mut self, norm: f32) {
        self.er.set_level(norm);
    }

    /// Set early-reflections time scaling (0..1, normalized).
    pub fn set_er_time(&mut self, norm: f32) {
        self.er.set_time(norm);
    }

    /// Snapshot the current scaled ER tap times (ms) for the editor.
    pub fn er_tap_times_ms(&self) -> [(f32, f32); ER_TAPS] {
        self.er.scaled_ms
    }

    /// Snapshot the ER tap gains (incl. polarity) for the editor.
    pub fn er_tap_gains(&self) -> [(f32, f32); ER_TAPS] {
        self.er.gains
    }

    /// Snapshot the smoothed per-FDN-channel energies for the tank view.
    pub fn channel_energies(&self) -> [f32; CHANNELS] {
        self.fdn.channel_energies()
    }

    /// Current FDN delay lengths in ms (affected by `size`).
    pub fn fdn_delay_ms(&self) -> [f32; CHANNELS] {
        self.fdn.delay_ms(self.sample_rate)
    }

    /// Take the RMS of the wet output since the last call and reset the accumulator.
    /// Returns 0.0 on the first call after construction / clear.
    pub fn take_wet_rms(&mut self) -> f32 {
        if self.wet_count == 0 {
            return 0.0;
        }
        let mean = self.wet_sumsq / self.wet_count as f64;
        self.wet_sumsq = 0.0;
        self.wet_count = 0;
        (mean as f32).sqrt()
    }

    /// Reconfigure for a new sample rate. Clears all state.
    pub fn set_sample_rate(&mut self, sample_rate: f32) {
        *self = Self::new(sample_rate);
    }

    /// Update room size. Recalculates FDN delay times and diffusion.
    pub fn set_size(&mut self, size_normalized: f32) {
        // Logarithmic map 0..1 → MIN_SIZE_MS..MAX_SIZE_MS so the middle
        // of the knob lands on a "medium room" (~ 90 ms) instead of a
        // linear mapping's half-cathedral.
        let t = size_normalized.clamp(0.0, 1.0);
        let size_ms = MIN_SIZE_MS * (MAX_SIZE_MS / MIN_SIZE_MS).powf(t);
        self.room_size_ms = size_ms;

        let base_samples = size_ms * 0.001 * self.sample_rate;
        self.fdn.set_size(base_samples);

        // Update diffusion delay ranges using stored ratios (no re-randomization)
        let mut diff_ms = size_ms;
        for (step_idx, step) in self.diffusion.iter_mut().enumerate() {
            diff_ms *= 0.5;
            let range_samples = ((diff_ms * 0.001 * self.sample_rate) as usize).max(8);
            for c in 0..CHANNELS {
                step.delay_samples[c] =
                    (self.diffusion_ratios[step_idx][c] * range_samples as f32) as usize;
                step.delay_samples[c] = step.delay_samples[c].max(1);
            }
        }
    }

    /// Set decay time in seconds. Calculates feedback gain from RT60.
    pub fn set_decay(&mut self, rt60_seconds: f32) {
        let typical_loop_ms = self.room_size_ms * 1.5;
        let loops_per_rt60 = rt60_seconds / (typical_loop_ms * 0.001);
        if loops_per_rt60 > 0.0 {
            let db_per_cycle = -60.0 / loops_per_rt60;
            self.decay_gain = (10.0f32).powf(db_per_cycle * 0.05);
            // Clamp to prevent instability
            self.decay_gain = self.decay_gain.clamp(0.0, 0.9999);
        }
    }

    /// Set or unset freeze mode. When frozen, decay_gain is 1.0 (infinite
    /// tail) and the tank input is muted so the energy-preserving loop
    /// doesn't accumulate fresh input without bound.
    /// When unfreezing, recalculate decay_gain from current room size and a default RT60.
    pub fn set_freeze(&mut self, freeze: bool) {
        self.frozen = freeze;
        if freeze {
            self.decay_gain = 1.0;
        } else if self.decay_gain >= 1.0 {
            // Was frozen — restore a sensible decay; set_decay will be called
            // next block with the actual param value, so use a safe default.
            self.decay_gain = 0.85;
        }
    }

    /// Set high-frequency damping cutoff.
    pub fn set_damping(&mut self, cutoff_hz: f32) {
        self.fdn.set_damping(cutoff_hz, self.sample_rate);
    }

    /// Set pre-delay in milliseconds.
    ///
    /// The read tap never relocates mid-signal: a change starts a
    /// linear crossfade from the old tap to the new one over
    /// `PREDELAY_FADE_MS`. Crossfading (rather than a Doppler glide)
    /// keeps the pre-delay pitch-stable — a bending tap here would bend
    /// the *entire* reverb input. The fade is linear, not equal-power:
    /// during a sweep the two taps are a few ms apart and strongly
    /// correlated, where an equal-power law would bulge by up to +3 dB
    /// at the midpoint. A change landing mid-fade is queued (newest
    /// wins) and started when the running fade completes, so a
    /// continuous sweep resolves into back-to-back short crossfades.
    pub fn set_predelay(&mut self, ms: f32) {
        let samples =
            ((ms * 0.001 * self.sample_rate) as usize).min(self.max_predelay_samples - 1);
        if samples == self.predelay_requested {
            return;
        }
        self.predelay_requested = samples;
        if !self.predelay_primed {
            // Nothing audible is in flight yet — snap, exactly like the
            // pre-crossfade code did on activation.
            self.predelay_samples = samples;
            self.predelay_fade_left = 0;
            self.predelay_pending = None;
        } else if self.predelay_fade_left > 0 {
            self.predelay_pending = Some(samples);
        } else if samples != self.predelay_samples {
            self.predelay_from = self.predelay_samples;
            self.predelay_samples = samples;
            self.predelay_fade_left = self.predelay_fade_total;
        }
    }

    /// Set modulation depth (0..1 normalized).
    pub fn set_mod_depth(&mut self, depth: f32) {
        // Map to 0..40 samples (~0.9ms at 44.1kHz) of delay modulation.
        self.mod_depth_samples = depth * 40.0;
    }

    /// Set modulation rate (Hz).
    pub fn set_mod_rate(&mut self, rate_hz: f32) {
        self.fdn.set_mod_rate(rate_hz, self.sample_rate);
    }

    /// Process a single stereo sample pair. Returns (wet_l, wet_r).
    pub fn process(
        &mut self,
        left: f32,
        right: f32,
        diffusion_amount: f32,
        width: f32,
    ) -> (f32, f32) {
        // Pre-delay. A stationary tap reads exactly as before; while a
        // crossfade is in flight both taps are read and mixed.
        let (dl, dr) = if self.predelay_fade_left > 0 {
            self.predelay_fade_left -= 1;
            let x = (self.predelay_fade_total - self.predelay_fade_left) as f32
                / self.predelay_fade_total as f32;
            let old_l = self.predelay_l.tap(self.predelay_from);
            let old_r = self.predelay_r.tap(self.predelay_from);
            let new_l = self.predelay_l.tap(self.predelay_samples);
            let new_r = self.predelay_r.tap(self.predelay_samples);
            if self.predelay_fade_left == 0 {
                if let Some(next) = self.predelay_pending.take() {
                    if next != self.predelay_samples {
                        self.predelay_from = self.predelay_samples;
                        self.predelay_samples = next;
                        self.predelay_fade_left = self.predelay_fade_total;
                    }
                }
            }
            (old_l + x * (new_l - old_l), old_r + x * (new_r - old_r))
        } else {
            (
                self.predelay_l.tap(self.predelay_samples),
                self.predelay_r.tap(self.predelay_samples),
            )
        };
        self.predelay_l.push(left);
        self.predelay_r.push(right);
        self.predelay_primed = true;

        // Early reflections: independent parallel multi-tap delay, fed from
        // the same pre-delayed input as the diffusion network.
        let (er_l, er_r) = self.er.process(dl, dr);

        // Scatter stereo input into 8 channels and diffuse it. The
        // diffusion network only ever sees *fresh input* — never FDN
        // feedback. If we ran feedback through diffusion (as the
        // earlier version did), the Hadamard-crossfade's non-unit
        // broadband gain (~0.5× for random signals) would multiply
        // into every cycle of the feedback loop, producing an
        // effective RT60 ~4× shorter than requested.
        let mut diffused = [0.0f32; CHANNELS];
        diffused[0] = dl;
        diffused[1] = dr;
        for step in &mut self.diffusion {
            step.process(&mut diffused, diffusion_amount);
        }

        // Run the diffused input through the FDN feedback loop and get
        // back the per-channel tail samples. While frozen the tank input
        // is muted (same hard gate as resonance-delay's freeze): the
        // Householder loop at decay 1.0 preserves energy, so any fresh
        // input summed in would grow the tail without bound.
        let tank_input = if self.frozen {
            [0.0f32; CHANNELS]
        } else {
            diffused
        };
        let fdn_output = self
            .fdn
            .process(&tank_input, self.mod_depth_samples, self.decay_gain);

        // The per-channel wet signal is `diffused + fdn_output`: the
        // immediate diffused signal (early reflections from the
        // cascade) plus the FDN feedback tail. This is the key bit of
        // the Signalsmith/Luff design — without summing the diffused
        // signal into the output, the only thing the user would hear
        // is the FDN tap at `fdn_delay_samples[0]` samples later, i.e.
        // the reverb onset would be pinned to `size_ms`.
        let mut output = [0.0f32; CHANNELS];
        for (c, o) in output.iter_mut().enumerate() {
            *o = diffused[c] + fdn_output[c];
        }

        // Mix 8 channels to stereo with width control
        // Even channels → left, odd channels → right
        let mut sum_l = 0.0f32;
        let mut sum_r = 0.0f32;
        for (c, &s) in output.iter().enumerate() {
            if c.is_multiple_of(2) {
                sum_l += s;
            } else {
                sum_r += s;
            }
        }
        let scale = 1.0 / (CHANNELS as f32 / 2.0).sqrt();
        sum_l *= scale;
        sum_r *= scale;

        // Sum ER into the wet bus before the width/mix stage so ER also
        // respects width and mix.
        sum_l += er_l;
        sum_r += er_r;

        // Width: 0 = mono, 1 = full stereo
        let mid = (sum_l + sum_r) * 0.5;
        let side = (sum_l - sum_r) * 0.5;
        let out_l = mid + side * width;
        let out_r = mid - side * width;

        // Wet RMS accumulator for the impulse-view live trace polygon.
        self.wet_sumsq += (out_l as f64) * (out_l as f64) + (out_r as f64) * (out_r as f64);
        self.wet_count += 2;

        (out_l, out_r)
    }

    /// Clear all internal state (delay lines, filters, feedback).
    pub fn clear(&mut self) {
        self.predelay_l.clear();
        self.predelay_r.clear();
        // Cancel any fade and land on the newest requested tap, so a
        // reset instance matches a fresh one that was configured once.
        self.predelay_fade_left = 0;
        self.predelay_pending = None;
        self.predelay_samples = self.predelay_requested;
        self.predelay_primed = false;
        for step in &mut self.diffusion {
            step.clear();
        }
        self.fdn.clear();
        self.er.clear();
        self.wet_sumsq = 0.0;
        self.wet_count = 0;
    }
}
