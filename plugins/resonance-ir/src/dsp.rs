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
///
/// This is the [`LatencyMode::Normal`] base; the other modes scale it — see
/// [`block_size_for`].
pub fn block_size_for_sample_rate(sample_rate: f32) -> usize {
    if sample_rate > 88_000.0 {
        512
    } else if sample_rate > 50_000.0 {
        256
    } else {
        128
    }
}

/// Smallest convolution block the plugin will run (0.7 ms at 44.1 kHz).
/// Below this the per-sample FFT cost stops being worth the milliseconds.
pub const MIN_BLOCK_SIZE: usize = 32;
/// Largest convolution block the plugin will run (46 ms at 44.1 kHz).
pub const MAX_BLOCK_SIZE: usize = 2048;

/// How much latency the user is willing to spend on convolution — ba todo
/// #1300, audit finding I1.
///
/// The convolution block size *is* this plugin's reported latency (the
/// uniformly-partitioned convolver's algorithmic delay is exactly one hop),
/// and until this existed it was derived from the sample rate alone with no
/// user control and no readout: a player tracking a cabinet through the
/// plugin ate ~2.9 ms and could not see it, let alone shorten it.
///
/// The trade is latency against CPU, *not* against sound: a partitioned
/// convolver computes the same convolution whatever the hop, it just runs
/// more, smaller FFTs when the hop is short. That is why the long block is
/// called `Efficient` rather than "HQ" — it is not higher quality, it is
/// cheaper.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LatencyMode {
    /// A quarter of the normal block: the lowest latency the plugin offers,
    /// for playing or singing through it live. Costs roughly 4x the
    /// convolution CPU.
    Tracking,
    /// The historical behaviour and the default — ~2.7-2.9 ms, which is
    /// inaudible against monitoring latency while mixing.
    #[default]
    Normal,
    /// Four times the normal block: the cheapest mode, for a session that is
    /// past tracking and short on CPU.
    Efficient,
}

/// Display labels, in range order. Handed to `IntParam::with_choices` in
/// `params.rs`, which is what makes the editor, a host automation lane and
/// the control API all read "Tracking" instead of "0".
pub const LATENCY_MODE_LABELS: &[&str] = &["Tracking", "Normal", "Efficient"];

impl LatencyMode {
    /// Every mode, in the order [`LATENCY_MODE_LABELS`] declares them.
    pub const ALL: [LatencyMode; 3] = [Self::Tracking, Self::Normal, Self::Efficient];

    /// The mode a parameter value selects. Out-of-range values fall back to
    /// [`LatencyMode::Normal`] rather than to an end of the table, so a
    /// nonsense automation value cannot silently park the plugin at 46 ms.
    pub fn from_index(index: i32) -> Self {
        match index {
            0 => Self::Tracking,
            2 => Self::Efficient,
            _ => Self::Normal,
        }
    }

    /// This mode's parameter value — its position in [`LATENCY_MODE_LABELS`].
    pub fn index(self) -> i32 {
        match self {
            Self::Tracking => 0,
            Self::Normal => 1,
            Self::Efficient => 2,
        }
    }

    /// Power-of-two shift applied to the sample-rate base block size.
    fn shift(self) -> i32 {
        match self {
            Self::Tracking => -2,
            Self::Normal => 0,
            Self::Efficient => 2,
        }
    }
}

/// The convolution block size — and therefore the reported latency in
/// samples — for a sample rate and a mode.
pub fn block_size_for(sample_rate: f32, mode: LatencyMode) -> usize {
    let base = block_size_for_sample_rate(sample_rate);
    let shift = mode.shift();
    let scaled = if shift < 0 {
        base >> (-shift) as u32
    } else {
        base << shift as u32
    };
    scaled.clamp(MIN_BLOCK_SIZE, MAX_BLOCK_SIZE)
}

/// A block size expressed as milliseconds of latency. The single place that
/// conversion is written: the editor's readout and the tests both call it.
pub fn latency_ms(block_size: usize, sample_rate: f32) -> f32 {
    if sample_rate <= 0.0 {
        return 0.0;
    }
    block_size as f32 * 1000.0 / sample_rate
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
        // A displaced convolver is a large heap free (the partitioned
        // FDL); route it to a janitor thread so `begin_swap`/`next`
        // never run that free inside the audio thread's sample loop.
        let mut fader = SwapFader::new(SWAP_FADE_SAMPLES);
        fader.set_retire_sink(SwapFader::spawn_retire_janitor("resonance-ir-janitor"));
        Self {
            fader,
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

    /// Reset the active convolver's internal state (FDL, overlap, buffers)
    /// and clear the dry path's latency-compensation delay, so no
    /// pre-reset audio is replayed.
    pub fn reset(&mut self) {
        if let Some(conv) = self.fader.active_mut() {
            conv.reset();
        }
        // Overwrite rather than reallocate: `reset` may run on the audio
        // thread. The dry path reads `block_size` pushes back, so that
        // many zeros flush everything it can still reach.
        for _ in 0..self.block_size {
            self.bypass_delay_l.push(0.0);
            self.bypass_delay_r.push(0.0);
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

            // The swap fade scales the WET share only: while a convolver
            // fades out/in the mix leans toward the dry signal, which is
            // never interrupted. With no convolver the output is fully
            // dry, so the first load's fade-in is continuous with it.
            match conv {
                Some(conv) => {
                    let (wet_l, wet_r) = conv.process_sample(dry_l, dry_r);

                    let wet_amount = dry_wet * fade_gain;
                    let dry_amount = 1.0 - wet_amount;
                    left[i] = (delayed_l * dry_amount + wet_l * wet_amount) * output_gain;
                    right[i] = (delayed_r * dry_amount + wet_r * wet_amount) * output_gain;
                }
                None => {
                    left[i] = delayed_l * output_gain;
                    right[i] = delayed_r * output_gain;
                }
            }

            peaks.out_l = peaks.out_l.max(left[i].abs());
            peaks.out_r = peaks.out_r.max(right[i].abs());
        }

        peaks
    }
}
