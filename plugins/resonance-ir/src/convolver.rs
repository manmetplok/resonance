/// Partitioned FFT convolution engine using overlap-add.
///
/// Splits the IR into fixed-size segments, FFTs each one, then for each input block:
/// FFT input -> complex multiply with each IR segment -> IFFT -> overlap-add.
use rustfft::num_complex::Complex;
use rustfft::{Fft, FftPlanner};
use std::sync::Arc;

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
