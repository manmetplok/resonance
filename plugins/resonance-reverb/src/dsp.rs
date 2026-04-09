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

    // Current parameters (for per-sample smoothing)
    decay_gain: f32,
    mod_depth_samples: f32,
    room_size_ms: f32,
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
            let ds = DiffusionStep::new(range_samples.max(8), (step as u64 + 1) * 0x517cc1b727220a95);
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
            decay_gain: 0.85,
            mod_depth_samples: 0.0,
            room_size_ms: 150.0,
        }
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

        // Width: 0 = mono, 1 = full stereo
        let mid = (sum_l + sum_r) * 0.5;
        let side = (sum_l - sum_r) * 0.5;
        let out_l = mid + side * width;
        let out_r = mid - side * width;

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

