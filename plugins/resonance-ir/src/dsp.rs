/// Pure DSP for the IR plugin: the stereo pairing of the shared
/// partitioned FFT convolver ([`resonance_dsp::FftConvolver`]) plus
/// [`IrEngine`], the per-block wet/dry processor that owns the bypass
/// delay alignment and the convolver-swap crossfade.
use resonance_dsp::{DelayLine, FftConvolver, SwapFader};
use resonance_plugin::Smoother;

/// Crossfade length in samples (~1.5ms at 44.1kHz) to avoid pops on convolver swap.
pub const SWAP_FADE_SAMPLES: u32 = 64;

/// Choose a block size that keeps latency around ~2.7ms regardless of sample rate.
/// Returns a power-of-two block size.
pub fn block_size_for_sample_rate(sample_rate: f32) -> usize {
    if sample_rate > 88_000.0 {
        512
    } else if sample_rate > 50_000.0 {
        256
    } else {
        128
    }
}

/// Stereo convolver: handles mono IR (applied to both channels) or stereo IR.
pub struct StereoConvolver {
    pub left: FftConvolver,
    pub right: FftConvolver,
}

impl StereoConvolver {
    /// Create from IR data. If IR is mono, the same IR is used for both
    /// channels. `block_size` is the convolution hop (and the latency).
    pub fn new(left_ir: &[f32], right_ir: Option<&[f32]>, block_size: usize) -> Self {
        Self {
            left: FftConvolver::new(left_ir, block_size),
            right: FftConvolver::new(right_ir.unwrap_or(left_ir), block_size),
        }
    }

    pub fn block_size(&self) -> usize {
        self.left.hop()
    }

    pub fn process_sample(&mut self, left_in: f32, right_in: f32) -> (f32, f32) {
        let l = self.left.process_sample(left_in);
        let r = self.right.process_sample(right_in);
        (l, r)
    }

    pub fn reset(&mut self) {
        self.left.reset();
        self.right.reset();
    }
}

/// Input/output peak magnitudes (linear) for one processed block.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct BlockPeaks {
    pub in_l: f32,
    pub in_r: f32,
    pub out_l: f32,
    pub out_r: f32,
}

/// Per-block wet/dry engine. Owns the active (and pending) convolver, the
/// bypass delay lines that keep the dry signal time-aligned with the
/// convolver's `block_size` latency, and the swap-crossfade state machine.
/// `lib.rs` hands it block slices; the per-sample loop lives here.
pub struct IrEngine {
    /// Active/pending convolver pair plus the swap crossfade envelope.
    fader: SwapFader<StereoConvolver>,
    /// Bypass delay lines to compensate for reported latency when no convolver is active.
    bypass_delay_l: DelayLine,
    bypass_delay_r: DelayLine,
    /// Convolution block size, scaled with sample rate to keep latency ~2.7ms.
    block_size: usize,
}

impl IrEngine {
    pub fn new(block_size: usize) -> Self {
        Self {
            fader: SwapFader::new(SWAP_FADE_SAMPLES),
            bypass_delay_l: DelayLine::new(block_size),
            bypass_delay_r: DelayLine::new(block_size),
            block_size,
        }
    }

    pub fn block_size(&self) -> usize {
        self.block_size
    }

    /// Reconfigure for a new convolution block size (initialize-time only —
    /// reallocates the bypass delay lines).
    pub fn set_block_size(&mut self, block_size: usize) {
        self.block_size = block_size;
        self.bypass_delay_l = DelayLine::new(block_size);
        self.bypass_delay_r = DelayLine::new(block_size);
    }

    /// Install a convolver directly, without a crossfade. Initialize-time
    /// path, before any audio has been processed.
    pub fn install(&mut self, conv: StereoConvolver) {
        self.fader.install(conv);
    }

    /// Hand over a freshly loaded convolver — starts the swap crossfade.
    /// If a convolver is already active it fades out first; otherwise the
    /// new one is swapped in directly and fades in.
    pub fn begin_swap(&mut self, conv: StereoConvolver) {
        self.fader.begin_swap(conv);
    }

    /// Reset the active convolver's internal state (FDL, overlap, buffers).
    pub fn reset(&mut self) {
        if let Some(conv) = self.fader.active_mut() {
            conv.reset();
        }
    }

    /// Process a stereo block in-place, mixing the latency-aligned dry
    /// signal with the convolved wet signal. `dry_wet` and `output_gain`
    /// are ramped per sample to avoid zippering. Returns the block's
    /// input/output peaks for metering. Allocation-free.
    pub fn process_block(
        &mut self,
        left: &mut [f32],
        right: &mut [f32],
        dry_wet: &mut Smoother,
        output_gain: &mut Smoother,
    ) -> BlockPeaks {
        let frames = left.len().min(right.len());
        let mut peaks = BlockPeaks::default();

        for i in 0..frames {
            let dry_wet = dry_wet.next();
            let output_gain = output_gain.next();

            // Crossfade envelope: fade out old convolver, swap, fade in new convolver.
            let (fade_gain, conv) = self.fader.next();

            let dry_l = left[i];
            let dry_r = right[i];
            peaks.in_l = peaks.in_l.max(dry_l.abs());
            peaks.in_r = peaks.in_r.max(dry_r.abs());

            // Always feed the bypass delay lines so the dry signal stays
            // time-aligned with the convolver's block_size latency. We
            // tap *before* pushing the current sample, so a tap of
            // `block_size - 1` reads the sample from exactly block_size
            // samples ago. (Tapping `block_size` here aliased to a
            // 1-sample delay: the buffer is exactly block_size long —
            // always a power of two — and `tap` wraps modulo its size.)
            let delayed_l = self.bypass_delay_l.tap(self.block_size - 1);
            let delayed_r = self.bypass_delay_r.tap(self.block_size - 1);
            self.bypass_delay_l.push(dry_l);
            self.bypass_delay_r.push(dry_r);

            match conv {
                Some(conv) => {
                    let (wet_l, wet_r) = conv.process_sample(dry_l, dry_r);

                    let dry_amount = 1.0 - dry_wet;
                    left[i] =
                        (delayed_l * dry_amount + wet_l * dry_wet) * output_gain * fade_gain;
                    right[i] =
                        (delayed_r * dry_amount + wet_r * dry_wet) * output_gain * fade_gain;
                }
                None => {
                    left[i] = delayed_l * output_gain * fade_gain;
                    right[i] = delayed_r * output_gain * fade_gain;
                }
            }

            peaks.out_l = peaks.out_l.max(left[i].abs());
            peaks.out_r = peaks.out_r.max(right[i].abs());
        }

        peaks
    }
}
