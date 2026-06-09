/// Pure DSP for the IR plugin: the partitioned FFT convolution engine
/// (overlap-add) plus [`IrEngine`], the per-block wet/dry processor that
/// owns the bypass delay alignment and the convolver-swap crossfade.
///
/// Convolution splits the IR into fixed-size segments, FFTs each one, then
/// for each input block: FFT input -> complex multiply with each IR
/// segment -> IFFT -> overlap-add.
use resonance_dsp::DelayLine;
use resonance_plugin::Smoother;
use rustfft::num_complex::Complex;
use rustfft::{Fft, FftPlanner};
use std::sync::Arc;

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

/// A single-channel partitioned convolver.
pub struct MonoConvolver {
    block_size: usize,
    fft_size: usize,
    /// FFT'd IR segments, each of length fft_size.
    ir_segments: Vec<Vec<Complex<f32>>>,
    /// Ring buffer of FFT'd input blocks, same length as ir_segments.
    input_fdl: Vec<Vec<Complex<f32>>>,
    /// Current write position in the input FDL ring buffer.
    fdl_pos: usize,
    /// Input accumulation buffer (collects samples until a full block).
    input_buf: Vec<f32>,
    /// Number of samples accumulated in input_buf.
    input_count: usize,
    /// Output overlap buffer (tail from previous blocks).
    overlap: Vec<f32>,
    /// Output buffer: processed samples ready to be read.
    output_buf: Vec<f32>,
    /// Read position in output_buf.
    output_pos: usize,
    /// Scratch buffers for FFT computation.
    fft_scratch: Vec<Complex<f32>>,
    accum_scratch: Vec<Complex<f32>>,
    time_scratch: Vec<Complex<f32>>,

    fft_forward: Arc<dyn Fft<f32> + Send + Sync>,
    fft_inverse: Arc<dyn Fft<f32> + Send + Sync>,
}

impl MonoConvolver {
    /// Create a new convolver for the given IR samples with the specified block size.
    pub fn new(ir: &[f32], block_size: usize) -> Self {
        debug_assert!(block_size.is_power_of_two());
        let fft_size = block_size * 2;

        let mut planner = FftPlanner::new();
        let fft_forward = planner.plan_fft_forward(fft_size);
        let fft_inverse = planner.plan_fft_inverse(fft_size);

        // Partition IR into segments of block_size and FFT each
        let num_segments = ir.len().div_ceil(block_size);
        let num_segments = num_segments.max(1);

        let mut ir_segments = Vec::with_capacity(num_segments);
        for seg_idx in 0..num_segments {
            let start = seg_idx * block_size;
            let mut buf = vec![Complex::new(0.0, 0.0); fft_size];

            for i in 0..block_size {
                if start + i < ir.len() {
                    buf[i] = Complex::new(ir[start + i], 0.0);
                }
            }

            fft_forward.process(&mut buf);
            ir_segments.push(buf);
        }

        let input_fdl = vec![vec![Complex::new(0.0, 0.0); fft_size]; num_segments];

        Self {
            block_size,
            fft_size,
            ir_segments,
            input_fdl,
            fdl_pos: 0,
            input_buf: vec![0.0; block_size],
            input_count: 0,
            overlap: vec![0.0; block_size],
            output_buf: vec![0.0; block_size],
            output_pos: 0,
            fft_scratch: vec![Complex::new(0.0, 0.0); fft_size],
            accum_scratch: vec![Complex::new(0.0, 0.0); fft_size],
            time_scratch: vec![Complex::new(0.0, 0.0); fft_size],
            fft_forward,
            fft_inverse,
        }
    }

    pub fn block_size(&self) -> usize {
        self.block_size
    }

    /// Process a single sample through the convolver.
    pub fn process_sample(&mut self, input: f32) -> f32 {
        let block_size = self.block_size;

        // If we have processed samples ready, return one
        if self.output_pos < block_size {
            let out = self.output_buf[self.output_pos];
            self.input_buf[self.input_count] = input;
            self.input_count += 1;
            self.output_pos += 1;

            // When we've consumed the full output block and filled the input, process next block
            if self.input_count >= block_size {
                self.process_block();
                self.input_count = 0;
                self.output_pos = 0;
            }

            return out;
        }

        // Should not reach here after initialization, but handle gracefully
        self.input_buf[self.input_count] = input;
        self.input_count += 1;
        if self.input_count >= block_size {
            self.process_block();
            self.input_count = 0;
            self.output_pos = 0;
        }
        0.0
    }

    /// Process a full block: FFT input, multiply with IR segments, IFFT, overlap-add.
    fn process_block(&mut self) {
        let block_size = self.block_size;
        let fft_size = self.fft_size;
        let num_segs = self.ir_segments.len();

        // Prepare input block in FFT buffer: [input_buf | zeros]
        for i in 0..block_size {
            self.fft_scratch[i] = Complex::new(self.input_buf[i], 0.0);
        }
        for i in block_size..fft_size {
            self.fft_scratch[i] = Complex::new(0.0, 0.0);
        }

        // FFT the input block
        self.fft_forward.process(&mut self.fft_scratch);

        // Store in the FDL (frequency-domain delay line)
        self.input_fdl[self.fdl_pos].copy_from_slice(&self.fft_scratch);

        // Accumulate: sum of (input_fdl[n] * ir_segments[n]) for all segments.
        // Pull references to separate fields so the compiler can prove
        // non-aliasing and auto-vectorize the inner loop.
        let accum = &mut self.accum_scratch[..fft_size];
        accum.fill(Complex::new(0.0, 0.0));

        let fdl_pos = self.fdl_pos;
        for seg in 0..num_segs {
            let fdl_idx = (fdl_pos + num_segs - seg) % num_segs;
            let fdl = &self.input_fdl[fdl_idx];
            let ir_seg = &self.ir_segments[seg];

            // Complex multiply-accumulate. Spelling out re/im avoids the
            // generic Complex<f32> Mul impl overhead and lets LLVM emit
            // packed FMA where available.
            for i in 0..fft_size {
                let a_re = fdl[i].re;
                let a_im = fdl[i].im;
                let b_re = ir_seg[i].re;
                let b_im = ir_seg[i].im;
                accum[i].re += a_re * b_re - a_im * b_im;
                accum[i].im += a_re * b_im + a_im * b_re;
            }
        }

        // IFFT
        self.time_scratch.copy_from_slice(accum);
        self.fft_inverse.process(&mut self.time_scratch);

        // Normalize (rustfft doesn't normalize)
        let norm = 1.0 / fft_size as f32;

        // Overlap-add: first half is output, add overlap from previous block
        for i in 0..block_size {
            self.output_buf[i] = self.time_scratch[i].re * norm + self.overlap[i];
        }

        // Save second half as overlap for next block
        for i in 0..block_size {
            self.overlap[i] = self.time_scratch[block_size + i].re * norm;
        }

        // Advance FDL write position
        self.fdl_pos = (fdl_pos + 1) % num_segs;
    }

    /// Reset all state (input buffer, FDL, overlap).
    pub fn reset(&mut self) {
        self.input_buf.fill(0.0);
        self.input_count = 0;
        self.overlap.fill(0.0);
        self.output_buf.fill(0.0);
        self.output_pos = 0;
        self.fdl_pos = 0;
        for fdl in &mut self.input_fdl {
            fdl.fill(Complex::new(0.0, 0.0));
        }
    }
}

/// Stereo convolver: handles mono IR (applied to both channels) or stereo IR.
pub struct StereoConvolver {
    pub left: MonoConvolver,
    pub right: MonoConvolver,
}

impl StereoConvolver {
    /// Create from IR data. If IR is mono, the same IR is used for both channels.
    pub fn new(left_ir: &[f32], right_ir: Option<&[f32]>, block_size: usize) -> Self {
        let left = MonoConvolver::new(left_ir, block_size);
        let right = MonoConvolver::new(right_ir.unwrap_or(left_ir), block_size);
        Self { left, right }
    }

    pub fn block_size(&self) -> usize {
        self.left.block_size()
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
    active: Option<StereoConvolver>,
    /// Convolver waiting to be swapped in after fade-out completes.
    pending: Option<StereoConvolver>,
    /// Samples remaining in fade-out before convolver swap.
    fade_out_remaining: u32,
    /// Samples remaining in fade-in after convolver swap.
    fade_in_remaining: u32,
    /// Bypass delay lines to compensate for reported latency when no convolver is active.
    bypass_delay_l: DelayLine,
    bypass_delay_r: DelayLine,
    /// Convolution block size, scaled with sample rate to keep latency ~2.7ms.
    block_size: usize,
}

impl IrEngine {
    pub fn new(block_size: usize) -> Self {
        Self {
            active: None,
            pending: None,
            fade_out_remaining: 0,
            fade_in_remaining: 0,
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
        self.active = Some(conv);
    }

    /// Hand over a freshly loaded convolver — starts the swap crossfade.
    /// If a convolver is already active it fades out first; otherwise the
    /// new one is swapped in directly and fades in.
    pub fn begin_swap(&mut self, conv: StereoConvolver) {
        self.pending = Some(conv);
        if self.active.is_some() {
            self.fade_out_remaining = SWAP_FADE_SAMPLES;
            self.fade_in_remaining = 0;
        } else {
            self.active = self.pending.take();
            self.fade_in_remaining = SWAP_FADE_SAMPLES;
        }
    }

    /// Reset the active convolver's internal state (FDL, overlap, buffers).
    pub fn reset(&mut self) {
        if let Some(conv) = &mut self.active {
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
            let fade_gain = if self.fade_out_remaining > 0 {
                self.fade_out_remaining -= 1;
                let g = self.fade_out_remaining as f32 / SWAP_FADE_SAMPLES as f32;
                if self.fade_out_remaining == 0 {
                    self.active = self.pending.take();
                    self.fade_in_remaining = SWAP_FADE_SAMPLES;
                }
                g
            } else if self.fade_in_remaining > 0 {
                self.fade_in_remaining -= 1;
                1.0 - self.fade_in_remaining as f32 / SWAP_FADE_SAMPLES as f32
            } else {
                1.0
            };

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

            match &mut self.active {
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
