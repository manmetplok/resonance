//! Input diffusion network: cascaded Schroeder-style allpass-like stages
//! using delay lines + Hadamard mixing + per-channel polarity flips.
//!
//! The diffusion network blurs input into dense reflections before the
//! signal enters the FDN tail.

use resonance_dsp::{DelayLine, SimpleRng};

use super::{CHANNELS, TAP_SLEW_PER_SAMPLE};

/// A single diffusion step: N delay lines + Hadamard mix + polarity flips.
pub(super) struct DiffusionStep {
    delays: [DelayLine; CHANNELS],
    /// Tap-length *targets*. `ReverbDsp::set_size` rewrites these per
    /// block; the actual read taps glide toward them in `process`.
    pub(super) delay_samples: [usize; CHANNELS],
    /// Gliding fractional read positions (same scheme as the FDN
    /// bank's `read_pos`: slew-limited motion, exact integer at rest so
    /// a stationary step reads bit-identically to the pre-glide code).
    read_pos: [f32; CHANNELS],
    /// False until the first processed sample (activation and `clear`
    /// snap the read positions instead of gliding).
    primed: bool,
    flip: [bool; CHANNELS],
}

impl DiffusionStep {
    pub(super) fn new(range_samples: usize, seed: u64) -> Self {
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

        let mut read_pos = [0.0f32; CHANNELS];
        for (c, pos) in read_pos.iter_mut().enumerate() {
            *pos = delay_samples[c] as f32;
        }

        Self {
            delays,
            delay_samples,
            read_pos,
            primed: false,
            flip,
        }
    }

    pub(super) fn process(&mut self, channels: &mut [f32; CHANNELS], diffusion: f32) {
        if !self.primed {
            self.primed = true;
            for c in 0..CHANNELS {
                self.read_pos[c] = self.delay_samples[c] as f32;
            }
        }

        // Read from delay lines, write input. A stationary tap takes
        // the integer read (bit-identical to the pre-glide code); a
        // moving one slews toward its target and interpolates.
        let mut delayed = [0.0f32; CHANNELS];
        for c in 0..CHANNELS {
            let target = self.delay_samples[c] as f32;
            let pos = self.read_pos[c];
            delayed[c] = if pos == target {
                self.delays[c].tap(self.delay_samples[c])
            } else {
                let pos = if (target - pos).abs() <= TAP_SLEW_PER_SAMPLE {
                    target
                } else if target > pos {
                    pos + TAP_SLEW_PER_SAMPLE
                } else {
                    pos - TAP_SLEW_PER_SAMPLE
                };
                self.read_pos[c] = pos;
                self.delays[c].tap_linear(pos)
            };
            self.delays[c].push(channels[c]);
        }

        // Save un-mixed delayed signal for crossfade
        let raw = delayed;

        // Hadamard mix + polarity flips
        hadamard_in_place(&mut delayed);
        for (c, d) in delayed.iter_mut().enumerate() {
            if self.flip[c] {
                *d = -*d;
            }
        }

        // Crossfade: diffusion=0 → discrete echoes (raw delays), 1 → fully diffused
        for (c, ch) in channels.iter_mut().enumerate() {
            *ch = raw[c] + diffusion * (delayed[c] - raw[c]);
        }
    }

    pub(super) fn clear(&mut self) {
        for d in &mut self.delays {
            d.clear();
        }
        self.primed = false;
    }
}

/// In-place 8-point Hadamard (unitary). Three straight-line butterfly
/// stages with independent adds per stage, so LLVM can schedule them
/// into AVX2 SIMD under `target-cpu=native`. The final `1/√8` scale is
/// folded into the last stage.
#[inline]
fn hadamard_in_place(data: &mut [f32; 8]) {
    // Stage 1: pairs within each half.
    let (a0, a1) = (data[0] + data[1], data[0] - data[1]);
    let (a2, a3) = (data[2] + data[3], data[2] - data[3]);
    let (a4, a5) = (data[4] + data[5], data[4] - data[5]);
    let (a6, a7) = (data[6] + data[7], data[6] - data[7]);

    // Stage 2: pairs of pairs.
    let (b0, b2) = (a0 + a2, a0 - a2);
    let (b1, b3) = (a1 + a3, a1 - a3);
    let (b4, b6) = (a4 + a6, a4 - a6);
    let (b5, b7) = (a5 + a7, a5 - a7);

    // Stage 3: top half vs bottom half, with 1/√8 folded in.
    const S: f32 = 0.353_553_39; // 1 / sqrt(8)
    data[0] = (b0 + b4) * S;
    data[1] = (b1 + b5) * S;
    data[2] = (b2 + b6) * S;
    data[3] = (b3 + b7) * S;
    data[4] = (b0 - b4) * S;
    data[5] = (b1 - b5) * S;
    data[6] = (b2 - b6) * S;
    data[7] = (b3 - b7) * S;
}
