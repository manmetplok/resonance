/// Core reverb DSP: 8-channel diffusion network + feedback delay network.
///
/// Architecture (Signalsmith/Geraint Luff style):
///   Input -> Pre-delay -> 4-step Diffusion Network -> FDN Feedback Loop -> Stereo Output
///
/// The diffusion network blurs input into dense reflections using Hadamard mixing.
/// The FDN provides the decaying tail with Householder feedback and frequency-dependent damping.

use resonance_dsp::{DelayLine, Lfo, OnePole, SimpleRng};

const CHANNELS: usize = 8;
const DIFFUSION_STEPS: usize = 4;
const MAX_PREDELAY_SAMPLES: usize = 48000; // 1 second max pre-delay
pub const ER_TAPS: usize = 12;
const ER_BASE_MAX_MS: f32 = 220.0; // Upper bound for the longest tap at time_scale=1.0
const ER_TIME_MIN: f32 = 0.25; // er_time=0 → 0.25× base
const ER_TIME_MAX: f32 = 2.0;  // er_time=1 → 2.0× base

/// A single diffusion step: N delay lines + Hadamard mix + polarity flips.
struct DiffusionStep {
    delays: [DelayLine; CHANNELS],
    delay_samples: [usize; CHANNELS],
    flip: [bool; CHANNELS],
}

impl DiffusionStep {
    fn new(range_samples: usize, seed: u64) -> Self {
        let delays = std::array::from_fn(|_| DelayLine::new(range_samples.max(64)));
        let mut delay_samples = [0usize; CHANNELS];
        let mut flip = [false; CHANNELS];

        // Randomize delays within uniform segments (avoids clustering)
        let mut rng = SimpleRng::new(seed);
        for c in 0..CHANNELS {
            let low = range_samples * c / CHANNELS;
            let high = range_samples * (c + 1) / CHANNELS;
            delay_samples[c] = if high > low {
                low + (rng.next_u32() as usize % (high - low))
            } else {
                1
            };
            delay_samples[c] = delay_samples[c].max(1);
            flip[c] = rng.next_u32() & 1 == 1;
        }

        Self {
            delays,
            delay_samples,
            flip,
        }
    }

    fn process(&mut self, channels: &mut [f32; CHANNELS], diffusion: f32) {
        // Read from delay lines, write input
        let mut delayed = [0.0f32; CHANNELS];
        for c in 0..CHANNELS {
            delayed[c] = self.delays[c].tap(self.delay_samples[c]);
            self.delays[c].push(channels[c]);
        }

        // Save un-mixed delayed signal for crossfade
        let raw = delayed;

        // Hadamard mix + polarity flips
        hadamard_in_place(&mut delayed);
        for c in 0..CHANNELS {
            if self.flip[c] {
                delayed[c] = -delayed[c];
            }
        }

        // Crossfade: diffusion=0 → discrete echoes (raw delays), 1 → fully diffused
        for c in 0..CHANNELS {
            channels[c] = raw[c] + diffusion * (delayed[c] - raw[c]);
        }
    }

    fn clear(&mut self) {
        for d in &mut self.delays {
            d.clear();
        }
    }
}

/// Parallel multi-tap early reflections. Generates 12 discrete stereo taps
/// before the diffused tail arrives, giving the reverb its spatial signature.
struct EarlyReflections {
    delay_l: DelayLine,
    delay_r: DelayLine,
    /// Base tap times in ms (left, right), fixed at construction.
    base_times_ms: [(f32, f32); ER_TAPS],
    /// Per-tap gains (left, right), including polarity flips.
    gains: [(f32, f32); ER_TAPS],
    /// Current `er_time` multiplier applied to `base_times_ms`.
    time_scale: f32,
    /// Cached scaled tap times in samples. Recomputed when `time_scale` changes.
    scaled_samples: [(f32, f32); ER_TAPS],
    /// Cached scaled tap times in ms (for the viz).
    scaled_ms: [(f32, f32); ER_TAPS],
    /// Current `er_level` applied to the summed output before it joins the wet path.
    level: f32,
    sample_rate: f32,
}

impl EarlyReflections {
    fn new(sample_rate: f32) -> Self {
        // Enough headroom for the longest tap at time_scale=ER_TIME_MAX.
        let max_samples = ((ER_BASE_MAX_MS * ER_TIME_MAX) * 0.001 * sample_rate) as usize + 64;
        let mut rng = SimpleRng::new(0xa3f1_7b2c_0005_beef);

        // Generate tap times with a mild clustering bias: early taps
        // cluster in the first 60ms, later taps spread out toward 220ms.
        // This matches how real rooms produce dense-early / sparser-late
        // first reflections.
        let mut base_times_ms = [(0.0f32, 0.0f32); ER_TAPS];
        let mut gains = [(0.0f32, 0.0f32); ER_TAPS];
        for i in 0..ER_TAPS {
            let t = i as f32 / (ER_TAPS - 1).max(1) as f32;
            // Curve spreads the taps: sqrt gives more density early.
            let curved = t.sqrt();
            let center_ms = 4.0 + curved * (ER_BASE_MAX_MS - 4.0);

            // Small per-side jitter so L/R don't land exactly on top of each other.
            let jitter_l = ((rng.next_u32() & 0xffff) as f32 / 65535.0 - 0.5) * 8.0;
            let jitter_r = ((rng.next_u32() & 0xffff) as f32 / 65535.0 - 0.5) * 8.0;
            base_times_ms[i] = ((center_ms + jitter_l).max(1.0), (center_ms + jitter_r).max(1.0));

            // Exponential gain decay across taps with random polarity.
            let decay = (-3.0 * t).exp();
            let pol_l = if rng.next_u32() & 1 == 1 { 1.0 } else { -1.0 };
            let pol_r = if rng.next_u32() & 1 == 1 { 1.0 } else { -1.0 };
            gains[i] = (decay * pol_l, decay * pol_r);
        }

        let mut me = Self {
            delay_l: DelayLine::new(max_samples),
            delay_r: DelayLine::new(max_samples),
            base_times_ms,
            gains,
            time_scale: 1.0,
            scaled_samples: [(0.0, 0.0); ER_TAPS],
            scaled_ms: [(0.0, 0.0); ER_TAPS],
            level: 0.4,
            sample_rate,
        };
        me.recompute_scaled();
        me
    }

    fn recompute_scaled(&mut self) {
        for i in 0..ER_TAPS {
            let ms = (self.base_times_ms[i].0 * self.time_scale,
                      self.base_times_ms[i].1 * self.time_scale);
            self.scaled_ms[i] = ms;
            self.scaled_samples[i] = (
                (ms.0 * 0.001 * self.sample_rate).max(1.0),
                (ms.1 * 0.001 * self.sample_rate).max(1.0),
            );
        }
    }

    /// Map `er_time` (0..1) to a tap-time multiplier in [ER_TIME_MIN..ER_TIME_MAX].
    fn set_time(&mut self, norm: f32) {
        let scale = ER_TIME_MIN + norm.clamp(0.0, 1.0) * (ER_TIME_MAX - ER_TIME_MIN);
        if (scale - self.time_scale).abs() > 1e-4 {
            self.time_scale = scale;
            self.recompute_scaled();
        }
    }

    fn set_level(&mut self, norm: f32) {
        self.level = norm.clamp(0.0, 1.0);
    }

    /// Process a single stereo sample. Returns the ER contribution
    /// *including* `level` so the caller just sums it into the wet path.
    fn process(&mut self, left: f32, right: f32) -> (f32, f32) {
        self.delay_l.push(left);
        self.delay_r.push(right);
        let mut out_l = 0.0f32;
        let mut out_r = 0.0f32;
        for i in 0..ER_TAPS {
            let (sl, sr) = self.scaled_samples[i];
            let (gl, gr) = self.gains[i];
            out_l += self.delay_l.tap_linear(sl) * gl;
            out_r += self.delay_r.tap_linear(sr) * gr;
        }
        // 1/sqrt(N) keeps the summed broadband level predictable.
        let scale = self.level * (1.0 / (ER_TAPS as f32).sqrt());
        (out_l * scale, out_r * scale)
    }

    fn clear(&mut self) {
        self.delay_l.clear();
        self.delay_r.clear();
    }
}

/// The complete reverb processor.
pub struct ReverbDsp {
    sample_rate: f32,

    // Pre-delay
    predelay_l: DelayLine,
    predelay_r: DelayLine,
    predelay_samples: usize,

    // Diffusion network (4 cascaded steps)
    diffusion: [DiffusionStep; DIFFUSION_STEPS],
    /// Normalized tap positions (0..1) for each diffusion step + channel,
    /// computed once at creation to avoid re-randomization on size changes.
    diffusion_ratios: [[f32; CHANNELS]; DIFFUSION_STEPS],

    // FDN feedback loop
    fdn_delays: [DelayLine; CHANNELS],
    fdn_delay_samples: [usize; CHANNELS],
    fdn_damping: [OnePole; CHANNELS],
    fdn_lfos: [Lfo; CHANNELS],
    fdn_feedback: [f32; CHANNELS], // persistent feedback state

    // Early reflections (parallel multi-tap delay)
    er: EarlyReflections,

    // Current parameters (for per-sample smoothing)
    decay_gain: f32,
    mod_depth_samples: f32,
    room_size_ms: f32,

    // Viz-facing smoothed state (not audio-critical, block-rate updates).
    /// Smoothed |feedback| per FDN channel, for the tank view. Range ~0..1.
    channel_energy_smoothed: [f32; CHANNELS],
    /// Running sum-of-squares for wet RMS (reset by `take_wet_rms`).
    wet_sumsq: f64,
    wet_count: u32,
}

impl ReverbDsp {
    pub fn new(sample_rate: f32) -> Self {
        let max_delay = (sample_rate * 0.5) as usize; // 500ms max per delay line

        // Diffusion steps with halving delay ranges
        let base_diffusion_ms = 150.0;
        let mut diff_ms = base_diffusion_ms;
        let mut diffusion_ratios = [[0.0f32; CHANNELS]; DIFFUSION_STEPS];
        let diffusion = std::array::from_fn(|step| {
            diff_ms *= 0.5;
            let range_samples = (diff_ms * 0.001 * sample_rate) as usize;
            let ds = DiffusionStep::new(
                range_samples.max(8),
                (step as u64 + 1).wrapping_mul(0x517cc1b727220a95),
            );
            // Store normalized tap positions so set_size() can rescale without re-randomizing
            let rs = range_samples.max(8);
            for c in 0..CHANNELS {
                diffusion_ratios[step][c] = ds.delay_samples[c] as f32 / rs as f32;
            }
            ds
        });

        // FDN delay lines with exponential distribution
        let base_delay_ms = 150.0;
        let base_samples = (base_delay_ms * 0.001 * sample_rate) as f32;
        let mut fdn_delay_samples = [0usize; CHANNELS];
        for c in 0..CHANNELS {
            let r = c as f32 / CHANNELS as f32;
            fdn_delay_samples[c] = (2.0f32.powf(r) * base_samples) as usize;
            fdn_delay_samples[c] = fdn_delay_samples[c].max(1);
        }

        let fdn_delays = std::array::from_fn(|_| DelayLine::new(max_delay));
        let fdn_damping = std::array::from_fn(|_| OnePole::new());

        // LFOs with staggered phases and randomized rates
        let fdn_lfos = std::array::from_fn(|c| {
            let phase = c as f32 / CHANNELS as f32;
            let rate = 0.5 + 0.3 * (c as f32); // 0.5..2.6 Hz spread
            Lfo::new(rate, sample_rate, phase)
        });

        Self {
            sample_rate,
            predelay_l: DelayLine::new(MAX_PREDELAY_SAMPLES),
            predelay_r: DelayLine::new(MAX_PREDELAY_SAMPLES),
            predelay_samples: 0,
            diffusion,
            diffusion_ratios,
            fdn_delays,
            fdn_delay_samples,
            fdn_damping,
            fdn_lfos,
            fdn_feedback: [0.0; CHANNELS],
            er: EarlyReflections::new(sample_rate),
            decay_gain: 0.85,
            mod_depth_samples: 0.0,
            room_size_ms: 150.0,
            channel_energy_smoothed: [0.0; CHANNELS],
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
        self.channel_energy_smoothed
    }

    /// Current FDN delay lengths in ms (affected by `size`).
    pub fn fdn_delay_ms(&self) -> [f32; CHANNELS] {
        let mut out = [0.0f32; CHANNELS];
        for c in 0..CHANNELS {
            out[c] = self.fdn_delay_samples[c] as f32 * 1000.0 / self.sample_rate;
        }
        out
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
        // Map 0..1 to ~20ms..500ms base delay
        let size_ms = 20.0 + size_normalized * 480.0;
        self.room_size_ms = size_ms;

        let base_samples = (size_ms * 0.001 * self.sample_rate) as f32;
        for c in 0..CHANNELS {
            let r = c as f32 / CHANNELS as f32;
            self.fdn_delay_samples[c] = ((2.0f32.powf(r) * base_samples) as usize).max(1);
        }

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

    /// Set or unset freeze mode. When frozen, decay_gain is 1.0 (infinite tail).
    /// When unfreezing, recalculate decay_gain from current room size and a default RT60.
    pub fn set_freeze(&mut self, freeze: bool) {
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
        for filter in &mut self.fdn_damping {
            filter.set_cutoff(cutoff_hz, self.sample_rate);
        }
    }

    /// Set pre-delay in milliseconds.
    pub fn set_predelay(&mut self, ms: f32) {
        self.predelay_samples = ((ms * 0.001 * self.sample_rate) as usize)
            .min(MAX_PREDELAY_SAMPLES - 1)
            .max(0);
    }

    /// Set modulation depth (0..1 normalized).
    pub fn set_mod_depth(&mut self, depth: f32) {
        // Map to 0..40 samples (~0.9ms at 44.1kHz) of delay modulation.
        self.mod_depth_samples = depth * 40.0;
    }

    /// Set modulation rate (Hz).
    pub fn set_mod_rate(&mut self, rate_hz: f32) {
        for (c, lfo) in self.fdn_lfos.iter_mut().enumerate() {
            // Spread rates around the target: ±50%
            let spread = 0.5 + (c as f32 / CHANNELS as f32);
            lfo.set_rate(rate_hz * spread, self.sample_rate);
        }
    }

    /// Process a single stereo sample pair. Returns (wet_l, wet_r).
    pub fn process(&mut self, left: f32, right: f32, diffusion_amount: f32, width: f32) -> (f32, f32) {
        // Pre-delay
        let dl = self.predelay_l.tap(self.predelay_samples);
        let dr = self.predelay_r.tap(self.predelay_samples);
        self.predelay_l.push(left);
        self.predelay_r.push(right);

        // Early reflections: independent parallel multi-tap delay, fed from
        // the same pre-delayed input as the diffusion network.
        let (er_l, er_r) = self.er.process(dl, dr);

        // Scatter stereo input into 8 channels
        let mut ch = [0.0f32; CHANNELS];
        ch[0] = dl;
        ch[1] = dr;
        // Add previous FDN feedback into the input channels
        for c in 0..CHANNELS {
            ch[c] += self.fdn_feedback[c];
        }

        // Diffusion network: 4 cascaded steps
        for step in &mut self.diffusion {
            step.process(&mut ch, diffusion_amount);
        }

        // FDN: write diffused signal into delay lines, read output
        let mut output = [0.0f32; CHANNELS];
        for c in 0..CHANNELS {
            // Modulated read position
            let mod_offset = self.fdn_lfos[c].next() * self.mod_depth_samples;
            let delay_f = self.fdn_delay_samples[c] as f32 + mod_offset;
            let delay_f = delay_f.max(1.0);

            output[c] = self.fdn_delays[c].tap_linear(delay_f);
            self.fdn_delays[c].push(ch[c]);
        }

        // Householder feedback: mix output, apply damping and decay
        let mut feedback = output;
        householder_in_place(&mut feedback);
        for c in 0..CHANNELS {
            feedback[c] = self.fdn_damping[c].process(feedback[c]) * self.decay_gain;
        }
        self.fdn_feedback = feedback;

        // Smooth the |feedback| magnitudes toward a display envelope for the
        // tank view. Simple one-pole follower — cheap, no allocation. The
        // 0.995 coefficient gives a ~200-sample time constant (~4 ms @ 48 k).
        for c in 0..CHANNELS {
            let mag = self.fdn_feedback[c].abs();
            self.channel_energy_smoothed[c] =
                self.channel_energy_smoothed[c] * 0.995 + mag * 0.005;
        }

        // Mix 8 channels to stereo with width control
        // Even channels → left, odd channels → right
        let mut sum_l = 0.0f32;
        let mut sum_r = 0.0f32;
        for c in 0..CHANNELS {
            if c % 2 == 0 {
                sum_l += output[c];
            } else {
                sum_r += output[c];
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
        for step in &mut self.diffusion {
            step.clear();
        }
        for d in &mut self.fdn_delays {
            d.clear();
        }
        for f in &mut self.fdn_damping {
            f.clear();
        }
        self.fdn_feedback = [0.0; CHANNELS];
        self.er.clear();
        self.channel_energy_smoothed = [0.0; CHANNELS];
        self.wet_sumsq = 0.0;
        self.wet_count = 0;
    }
}

// --- Math utilities ---

/// In-place Hadamard transform (unitary, for power-of-2 sizes).
fn hadamard_in_place(data: &mut [f32; CHANNELS]) {
    hadamard_recursive(data, CHANNELS);
    let scale = 1.0 / (CHANNELS as f32).sqrt();
    for x in data.iter_mut() {
        *x *= scale;
    }
}

fn hadamard_recursive(data: &mut [f32], n: usize) {
    if n <= 1 {
        return;
    }
    let half = n / 2;
    hadamard_recursive(&mut data[..half], half);
    hadamard_recursive(&mut data[half..n], half);
    for i in 0..half {
        let a = data[i];
        let b = data[i + half];
        data[i] = a + b;
        data[i + half] = a - b;
    }
}

/// In-place Householder reflection: y[i] = x[i] - (2/N) * sum(x).
fn householder_in_place(data: &mut [f32; CHANNELS]) {
    let sum: f32 = data.iter().sum();
    let factor = -2.0 / CHANNELS as f32 * sum;
    for x in data.iter_mut() {
        *x += factor;
    }
}

