/// Partitioned FFT convolution engine using overlap-add.
///
/// Splits the IR into fixed-size segments, FFTs each one, then for each input block:
/// FFT input -> complex multiply with each IR segment -> IFFT -> overlap-add.

use rustfft::num_complex::Complex;
use rustfft::{Fft, FftPlanner};
use std::sync::Arc;

/// Block size for partitioned convolution. Must be a power of two.
pub const BLOCK_SIZE: usize = 128;
/// FFT size = 2 * BLOCK_SIZE for linear convolution via circular convolution.
const FFT_SIZE: usize = BLOCK_SIZE * 2;

/// A single-channel partitioned convolver.
pub struct MonoConvolver {
    /// FFT'd IR segments, each of length FFT_SIZE.
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

    fft_forward: Arc<dyn Fft<f32>>,
    fft_inverse: Arc<dyn Fft<f32>>,
}

impl MonoConvolver {
    /// Create a new convolver for the given IR samples.
    pub fn new(ir: &[f32]) -> Self {
        let mut planner = FftPlanner::new();
        let fft_forward = planner.plan_fft_forward(FFT_SIZE);
        let fft_inverse = planner.plan_fft_inverse(FFT_SIZE);

        // Partition IR into segments of BLOCK_SIZE and FFT each
        let num_segments = (ir.len() + BLOCK_SIZE - 1) / BLOCK_SIZE;
        let num_segments = num_segments.max(1);

        let mut ir_segments = Vec::with_capacity(num_segments);
        for seg_idx in 0..num_segments {
            let start = seg_idx * BLOCK_SIZE;
            let mut buf = vec![Complex::new(0.0, 0.0); FFT_SIZE];

            for i in 0..BLOCK_SIZE {
                if start + i < ir.len() {
                    buf[i] = Complex::new(ir[start + i], 0.0);
                }
            }

            fft_forward.process(&mut buf);
            ir_segments.push(buf);
        }

        let input_fdl = vec![vec![Complex::new(0.0, 0.0); FFT_SIZE]; num_segments];

        Self {
            ir_segments,
            input_fdl,
            fdl_pos: 0,
            input_buf: vec![0.0; BLOCK_SIZE],
            input_count: 0,
            overlap: vec![0.0; BLOCK_SIZE],
            output_buf: vec![0.0; BLOCK_SIZE],
            output_pos: 0,
            fft_scratch: vec![Complex::new(0.0, 0.0); FFT_SIZE],
            accum_scratch: vec![Complex::new(0.0, 0.0); FFT_SIZE],
            time_scratch: vec![Complex::new(0.0, 0.0); FFT_SIZE],
            fft_forward,
            fft_inverse,
        }
    }

    /// Process a single sample through the convolver.
    pub fn process_sample(&mut self, input: f32) -> f32 {
        // If we have processed samples ready, return one
        if self.output_pos < BLOCK_SIZE {
            let out = self.output_buf[self.output_pos];
            self.input_buf[self.input_count] = input;
            self.input_count += 1;
            self.output_pos += 1;

            // When we've consumed the full output block and filled the input, process next block
            if self.input_count >= BLOCK_SIZE {
                self.process_block();
                self.input_count = 0;
                self.output_pos = 0;
            }

            return out;
        }

        // Should not reach here after initialization, but handle gracefully
        self.input_buf[self.input_count] = input;
        self.input_count += 1;
        if self.input_count >= BLOCK_SIZE {
            self.process_block();
            self.input_count = 0;
            self.output_pos = 0;
        }
        0.0
    }

    /// Process a full block: FFT input, multiply with IR segments, IFFT, overlap-add.
    fn process_block(&mut self) {
        let num_segs = self.ir_segments.len();

        // Prepare input block in FFT buffer: [input_buf | zeros]
        for i in 0..BLOCK_SIZE {
            self.fft_scratch[i] = Complex::new(self.input_buf[i], 0.0);
        }
        for i in BLOCK_SIZE..FFT_SIZE {
            self.fft_scratch[i] = Complex::new(0.0, 0.0);
        }

        // FFT the input block
        self.fft_forward.process(&mut self.fft_scratch);

        // Store in the FDL (frequency-domain delay line)
        self.input_fdl[self.fdl_pos].copy_from_slice(&self.fft_scratch);

        // Accumulate: sum of (input_fdl[n] * ir_segments[n]) for all segments
        self.accum_scratch.fill(Complex::new(0.0, 0.0));

        for seg in 0..num_segs {
            // FDL index: current block convolved with segment 0,
            // previous block convolved with segment 1, etc.
            let fdl_idx = (self.fdl_pos + num_segs - seg) % num_segs;

            for i in 0..FFT_SIZE {
                self.accum_scratch[i] += self.input_fdl[fdl_idx][i] * self.ir_segments[seg][i];
            }
        }

        // IFFT
        self.time_scratch.copy_from_slice(&self.accum_scratch);
        self.fft_inverse.process(&mut self.time_scratch);

        // Normalize (rustfft doesn't normalize)
        let norm = 1.0 / FFT_SIZE as f32;

        // Overlap-add: first half is output, add overlap from previous block
        for i in 0..BLOCK_SIZE {
            self.output_buf[i] = self.time_scratch[i].re * norm + self.overlap[i];
        }

        // Save second half as overlap for next block
        for i in 0..BLOCK_SIZE {
            self.overlap[i] = self.time_scratch[BLOCK_SIZE + i].re * norm;
        }

        // Advance FDL write position
        self.fdl_pos = (self.fdl_pos + 1) % num_segs;
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
    pub fn new(left_ir: &[f32], right_ir: Option<&[f32]>) -> Self {
        let left = MonoConvolver::new(left_ir);
        let right = MonoConvolver::new(right_ir.unwrap_or(left_ir));
        Self { left, right }
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

// Safety: MonoConvolver contains no thread-unsafe types.
// Arc<dyn Fft> is Send+Sync, and all buffers are owned Vecs.
unsafe impl Send for MonoConvolver {}
unsafe impl Send for StereoConvolver {}
