//! Streaming FFT convolver — uniformly partitioned overlap-save.
//!
//! One engine covers both "short FIR, single partition" filtering
//! (the mastering linear-phase EQ and crossover lowpass) and "long IR,
//! many partitions" cabinet/room convolution (resonance-ir):
//!
//! * The FFT size is `2 * hop`; each iteration consumes `hop` fresh
//!   input samples and produces `hop` convolved output samples.
//! * The impulse response is partitioned at stride `hop` and each
//!   partition is FFT'd once up front. An IR of at most `hop + 1` taps
//!   fits a single partition (the overlap-save bound `L ≤ N − H + 1`),
//!   so per-iteration work is one forward FFT, one complex multiply
//!   per partition, and one inverse FFT.
//! * Input spectra live in a frequency-domain delay line (FDL); each
//!   iteration multiply-accumulates `FDL[t − k] · partition[k]` and
//!   takes the last `hop` samples of the IFFT (the circularly clean
//!   region) as output.
//!
//! Streaming semantics: audio is pushed in arbitrary chunk sizes; the
//! convolver gathers a full hop in an input FIFO, runs an iteration,
//! and parks the results in an output FIFO, so callers can stream
//! sample-by-sample or block-by-block. Algorithmic latency is exactly
//! `hop` samples — the first `hop` outputs are zeros.
//!
//! Allocation discipline: FFT plans, rustfft scratch (used via
//! `process_with_scratch`), FIFOs, and the FDL are all allocated at
//! construction; `process_sample` / `process_in_place` never allocate.

use std::sync::Arc;

use rustfft::num_complex::Complex;
use rustfft::{Fft, FftPlanner};

/// Flat single-thread FIFO of f32 with two indices — a per-sample
/// push/pop replacement for `VecDeque`, which routes every access
/// through its own head/tail arithmetic plus a heap indirection. Fixed
/// power-of-two capacity; never allocates after construction.
struct SampleRing {
    buf: Vec<f32>,
    /// `capacity - 1`; capacity is a power of two so wrap is a bitmask.
    mask: usize,
    /// Index of the oldest sample.
    read: usize,
    /// Number of samples currently queued.
    len: usize,
}

impl SampleRing {
    fn with_capacity(capacity: usize) -> Self {
        debug_assert!(capacity.is_power_of_two());
        Self {
            buf: vec![0.0; capacity],
            mask: capacity - 1,
            read: 0,
            len: 0,
        }
    }

    #[inline]
    fn len(&self) -> usize {
        self.len
    }

    #[inline]
    fn push(&mut self, v: f32) {
        debug_assert!(self.len <= self.mask, "SampleRing overflow");
        let write = (self.read + self.len) & self.mask;
        self.buf[write] = v;
        self.len += 1;
    }

    /// Pop the oldest sample, or 0.0 when empty.
    #[inline]
    fn pop_or_zero(&mut self) -> f32 {
        if self.len == 0 {
            return 0.0;
        }
        let v = self.buf[self.read];
        self.read = (self.read + 1) & self.mask;
        self.len -= 1;
        v
    }

    fn clear(&mut self) {
        self.read = 0;
        self.len = 0;
    }
}

/// A single-channel streaming FFT convolver (see module docs).
pub struct FftConvolver {
    /// Samples consumed and produced per FFT iteration. Power of two.
    hop: usize,
    /// FFT size, always `2 * hop`.
    fft_size: usize,
    /// FFT'd IR partitions, each `fft_size` complex samples.
    segments: Vec<Vec<Complex<f32>>>,
    /// Frequency-domain delay line of input spectra, one slot per
    /// partition. Slot `fdl_pos` holds the newest spectrum.
    fdl: Vec<Vec<Complex<f32>>>,
    fdl_pos: usize,
    /// Rolling history of the last `fft_size - hop` input samples,
    /// prepended to each iteration's fresh samples (overlap-save).
    history: Vec<f32>,
    /// Incoming samples accumulating toward the next iteration.
    input_pending: SampleRing,
    /// Convolved samples waiting to be popped.
    output_pending: SampleRing,

    /// Complex scratch for the input-frame FFT and for partition design.
    scratch: Vec<Complex<f32>>,
    /// Multiply-accumulate target, IFFT'd in place.
    accum: Vec<Complex<f32>>,
    /// Pre-allocated rustfft work area (sized for both plans) so
    /// `process_with_scratch` never allocates on the audio thread.
    rustfft_scratch: Vec<Complex<f32>>,

    fft_forward: Arc<dyn Fft<f32> + Send + Sync>,
    fft_inverse: Arc<dyn Fft<f32> + Send + Sync>,
}

impl FftConvolver {
    /// Create a convolver for `ir` with the given hop size (must be a
    /// power of two). FFT size is `2 * hop`; latency is `hop` samples.
    pub fn new(ir: &[f32], hop: usize) -> Self {
        assert!(hop.is_power_of_two(), "hop must be a power of two");
        let fft_size = hop * 2;

        let mut planner = FftPlanner::new();
        let fft_forward = planner.plan_fft_forward(fft_size);
        let fft_inverse = planner.plan_fft_inverse(fft_size);

        let rustfft_scratch_len = fft_forward
            .get_inplace_scratch_len()
            .max(fft_inverse.get_inplace_scratch_len());

        let mut c = Self {
            hop,
            fft_size,
            segments: Vec::new(),
            fdl: Vec::new(),
            fdl_pos: 0,
            history: vec![0.0; fft_size - hop],
            // Jointly the FIFOs hold `hop` samples between host pushes
            // (`input_pending + output_pending = hop`), peaking at
            // `hop + 1` in the output FIFO right after an iteration.
            input_pending: SampleRing::with_capacity(2 * hop),
            output_pending: SampleRing::with_capacity(2 * hop),
            scratch: vec![Complex::new(0.0, 0.0); fft_size],
            accum: vec![Complex::new(0.0, 0.0); fft_size],
            rustfft_scratch: vec![Complex::new(0.0, 0.0); rustfft_scratch_len],
            fft_forward,
            fft_inverse,
        };
        c.set_impulse_response(ir);
        // Pre-fill the output FIFO with `hop` zeros so the convolver's
        // algorithmic latency is exactly `hop` samples from the first
        // pushed input.
        for _ in 0..hop {
            c.output_pending.push(0.0);
        }
        c
    }

    /// Number of partitions for an IR of `ir_len` taps. A single
    /// partition may hold up to `hop + 1` taps (the overlap-save
    /// validity bound); longer IRs split at stride `hop`.
    fn segment_count(ir_len: usize, hop: usize) -> usize {
        if ir_len <= hop + 1 {
            1
        } else {
            ir_len.div_ceil(hop)
        }
    }

    /// Hop size (== algorithmic latency in samples).
    pub fn hop(&self) -> usize {
        self.hop
    }

    /// Algorithmic latency in samples: output sample `n` holds the
    /// filter output for input sample `n - hop` (plus whatever group
    /// delay the IR itself carries).
    pub fn latency(&self) -> usize {
        self.hop
    }

    /// Replace the impulse response, keeping the streaming state
    /// (history, FIFOs, FDL). Allocation-free — and therefore safe on
    /// the audio thread — as long as the partition count is unchanged;
    /// a different partition count reallocates the partition tables.
    pub fn set_impulse_response(&mut self, ir: &[f32]) {
        let num_segments = Self::segment_count(ir.len(), self.hop);
        if num_segments != self.segments.len() {
            let zero = vec![Complex::new(0.0, 0.0); self.fft_size];
            self.segments.resize(num_segments, zero.clone());
            self.fdl.resize(num_segments, zero);
            self.fdl_pos %= num_segments;
        }

        for (seg_idx, seg) in self.segments.iter_mut().enumerate() {
            let start = seg_idx * self.hop;
            // The single-partition case may carry the full `hop + 1`
            // taps; multi-partition segments hold `hop` taps each.
            let end = if num_segments == 1 {
                ir.len()
            } else {
                ((seg_idx + 1) * self.hop).min(ir.len())
            };
            for (i, c) in seg.iter_mut().enumerate() {
                let tap = start + i;
                *c = if tap < end {
                    Complex::new(ir[tap], 0.0)
                } else {
                    Complex::new(0.0, 0.0)
                };
            }
            self.fft_forward
                .process_with_scratch(seg, &mut self.rustfft_scratch);
        }
    }

    /// Clear the streaming state (history, FIFOs, FDL); keeps the
    /// impulse response.
    pub fn reset(&mut self) {
        self.history.fill(0.0);
        self.input_pending.clear();
        self.output_pending.clear();
        for _ in 0..self.hop {
            self.output_pending.push(0.0);
        }
        for slot in &mut self.fdl {
            slot.fill(Complex::new(0.0, 0.0));
        }
        self.fdl_pos = 0;
    }

    /// Process a single sample. Allocation-free.
    #[inline]
    pub fn process_sample(&mut self, input: f32) -> f32 {
        self.input_pending.push(input);
        if self.input_pending.len() >= self.hop {
            self.run_iteration();
        }
        self.output_pending.pop_or_zero()
    }

    /// Process a block of samples in place. Allocation-free.
    pub fn process_in_place(&mut self, buffer: &mut [f32]) {
        for sample in buffer.iter_mut() {
            *sample = self.process_sample(*sample);
        }
    }

    fn run_iteration(&mut self) {
        debug_assert!(self.input_pending.len() >= self.hop);
        let hop = self.hop;
        let fft_size = self.fft_size;
        let history_len = fft_size - hop;
        let num_segs = self.segments.len();

        // Assemble the FFT frame: [history | hop fresh samples].
        for i in 0..history_len {
            self.scratch[i] = Complex::new(self.history[i], 0.0);
        }
        for i in 0..hop {
            let s = self.input_pending.pop_or_zero();
            self.scratch[history_len + i] = Complex::new(s, 0.0);
        }

        // The frame's last `history_len` samples become the next
        // iteration's history.
        for i in 0..history_len {
            self.history[i] = self.scratch[hop + i].re;
        }

        // Forward FFT, stored in the FDL slot for this iteration.
        self.fft_forward
            .process_with_scratch(&mut self.scratch, &mut self.rustfft_scratch);
        self.fdl[self.fdl_pos].copy_from_slice(&self.scratch);

        // Multiply-accumulate FDL[t - k] · segment[k]. Pull references
        // to separate fields so the compiler can prove non-aliasing,
        // and spell out re/im so LLVM emits packed FMA.
        let accum = &mut self.accum[..fft_size];
        let fdl_pos = self.fdl_pos;
        for seg in 0..num_segs {
            let fdl_idx = (fdl_pos + num_segs - seg) % num_segs;
            let fdl = &self.fdl[fdl_idx];
            let ir_seg = &self.segments[seg];

            if seg == 0 {
                // First partition assigns rather than accumulating into
                // zeros, exactly matching the single-partition users'
                // historical `spectrum * filter` arithmetic.
                for i in 0..fft_size {
                    let (a_re, a_im) = (fdl[i].re, fdl[i].im);
                    let (b_re, b_im) = (ir_seg[i].re, ir_seg[i].im);
                    accum[i].re = a_re * b_re - a_im * b_im;
                    accum[i].im = a_re * b_im + a_im * b_re;
                }
            } else {
                for i in 0..fft_size {
                    let (a_re, a_im) = (fdl[i].re, fdl[i].im);
                    let (b_re, b_im) = (ir_seg[i].re, ir_seg[i].im);
                    accum[i].re += a_re * b_re - a_im * b_im;
                    accum[i].im += a_re * b_im + a_im * b_re;
                }
            }
        }
        self.fdl_pos = (fdl_pos + 1) % num_segs;

        // Inverse FFT; the last `hop` samples are free of circular
        // artifacts (overlap-save discards the first `fft - hop`).
        self.fft_inverse
            .process_with_scratch(accum, &mut self.rustfft_scratch);
        let norm = 1.0 / fft_size as f32;
        for i in 0..hop {
            self.output_pending.push(accum[history_len + i].re * norm);
        }
    }
}
