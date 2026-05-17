//! Overlap-save FFT convolution engine.
//!
//! Given a real impulse response of length ≤ `FIR_LENGTH`, the convolver
//! computes its forward FFT once and reuses the stored frequency-domain
//! response on every audio block. Per-block cost is one forward FFT, a
//! complex element-wise multiply, and one inverse FFT — all on blocks of
//! `FFT_SIZE = 8192` samples at any input sample rate.
//!
//! Streaming semantics: audio is pushed in variable-sized chunks; the
//! convolver accumulates enough samples to fill one overlap-save hop,
//! runs an FFT iteration, and stashes outputs in a `VecDeque` so the host
//! can pop any number of samples per block.

use std::collections::VecDeque;
use std::sync::Arc;

use rustfft::num_complex::Complex;
use rustfft::{Fft, FftPlanner};

/// FIR length (odd → integer group delay). A 4097-tap linear-phase FIR
/// has group delay 2048 samples ≈ 42.7 ms at 48 kHz, appropriate for
/// mastering applications.
pub const FIR_LENGTH: usize = 4097;
/// FFT size used for the overlap-save convolution. Must be a power of
/// two and large enough to fit one full FIR plus one useful output hop.
pub const FFT_SIZE: usize = 8192;
/// Number of new samples consumed per FFT iteration. With
/// `FFT_SIZE − FIR_LENGTH + 1`, the IFFT produces exactly `HOP_SIZE`
/// circular-artifact-free output samples per iteration.
pub const HOP_SIZE: usize = FFT_SIZE - FIR_LENGTH + 1;
/// Group delay of a symmetric FIR of length `FIR_LENGTH`.
pub const GROUP_DELAY: usize = (FIR_LENGTH - 1) / 2;

/// A single-channel overlap-save convolver with a stored filter.
pub struct OverlapSaveConvolver {
    fft_forward: Arc<dyn Fft<f32> + Send + Sync>,
    fft_inverse: Arc<dyn Fft<f32> + Send + Sync>,

    /// Frequency-domain filter, `FFT_SIZE` complex samples.
    filter_response: Vec<Complex<f32>>,

    /// Scratch buffer for the FFT round-trip.
    scratch: Vec<Complex<f32>>,

    /// Rolling history of the last `FIR_LENGTH - 1` input samples
    /// (prepended to each FFT iteration's new samples).
    input_history: Vec<f32>,

    /// Incoming samples accumulating toward the next FFT iteration.
    input_pending: VecDeque<f32>,
    /// Convolved samples waiting to be popped by the host.
    output_pending: VecDeque<f32>,
}

impl OverlapSaveConvolver {
    pub fn new() -> Self {
        let mut planner = FftPlanner::<f32>::new();
        let fft_forward = planner.plan_fft_forward(FFT_SIZE);
        let fft_inverse = planner.plan_fft_inverse(FFT_SIZE);

        // Initial filter: pure delta, zero-phase. The resulting FIR is a
        // single 1.0 at the centre, padded with zeros. In overlap-save
        // this gives an identity passthrough delayed by GROUP_DELAY.
        let mut impulse = vec![0.0_f32; FIR_LENGTH];
        impulse[GROUP_DELAY] = 1.0;

        let mut c = Self {
            fft_forward,
            fft_inverse,
            filter_response: vec![Complex::new(0.0, 0.0); FFT_SIZE],
            scratch: vec![Complex::new(0.0, 0.0); FFT_SIZE],
            input_history: vec![0.0; FIR_LENGTH - 1],
            input_pending: VecDeque::with_capacity(HOP_SIZE * 2),
            output_pending: VecDeque::with_capacity(HOP_SIZE * 2),
        };
        c.set_impulse_response(&impulse);
        // Pre-fill the output ring with GROUP_DELAY zeros so the reported
        // latency is `HOP_SIZE + GROUP_DELAY − GROUP_DELAY = HOP_SIZE`.
        // Combined with the input-fill delay, the convolver's total
        // latency ends up at GROUP_DELAY + HOP_SIZE samples which the
        // plugin reports through `latency_samples`.
        for _ in 0..HOP_SIZE {
            c.output_pending.push_back(0.0);
        }
        c
    }

    /// Replace the filter impulse response. `h.len()` must be ≤ `FIR_LENGTH`.
    pub fn set_impulse_response(&mut self, h: &[f32]) {
        assert!(
            h.len() <= FIR_LENGTH,
            "impulse response must fit in FIR_LENGTH ({FIR_LENGTH}) taps"
        );
        // Zero-pad h to FFT_SIZE and run the forward FFT.
        for (i, v) in self.scratch.iter_mut().enumerate() {
            if i < h.len() {
                *v = Complex::new(h[i], 0.0);
            } else {
                *v = Complex::new(0.0, 0.0);
            }
        }
        self.fft_forward.process(&mut self.scratch);
        self.filter_response.copy_from_slice(&self.scratch);
    }

    /// Clear the convolver's streaming state. Keeps the filter response.
    pub fn reset(&mut self) {
        self.input_history.fill(0.0);
        self.input_pending.clear();
        self.output_pending.clear();
        for _ in 0..HOP_SIZE {
            self.output_pending.push_back(0.0);
        }
    }

    /// Total latency in samples. Output sample `n` corresponds to the
    /// filter applied to input sample `n - latency()`.
    pub const fn latency(&self) -> usize {
        // GROUP_DELAY comes from the symmetric FIR; HOP_SIZE from the
        // fact that we buffer one full hop before producing any output.
        GROUP_DELAY + HOP_SIZE
    }

    /// Process one block of samples in place (for a single channel).
    /// The convolver consumes all input; output is written to `buffer`
    /// in the same positions. Overall, `buffer[n]` after the call holds
    /// the filter output corresponding to the input sample that entered
    /// `latency()` samples earlier.
    pub fn process_in_place(&mut self, buffer: &mut [f32]) {
        for sample in buffer.iter_mut() {
            // Stage 1: accept input.
            self.input_pending.push_back(*sample);
            if self.input_pending.len() >= HOP_SIZE {
                self.run_iteration();
            }
            // Stage 2: emit one output.
            *sample = self.output_pending.pop_front().unwrap_or(0.0);
        }
    }

    fn run_iteration(&mut self) {
        debug_assert!(self.input_pending.len() >= HOP_SIZE);

        // Assemble FFT input: [history (FIR_LENGTH - 1)] + [new HOP_SIZE].
        for i in 0..(FIR_LENGTH - 1) {
            self.scratch[i] = Complex::new(self.input_history[i], 0.0);
        }
        for i in 0..HOP_SIZE {
            let s = self.input_pending.pop_front().unwrap();
            self.scratch[FIR_LENGTH - 1 + i] = Complex::new(s, 0.0);
        }

        // Update history with the last (FIR_LENGTH - 1) of this block's
        // full input for the next iteration.
        for i in 0..(FIR_LENGTH - 1) {
            let src = FFT_SIZE - (FIR_LENGTH - 1) + i;
            self.input_history[i] = self.scratch[src].re;
        }

        // Forward FFT of the input.
        self.fft_forward.process(&mut self.scratch);
        // Element-wise multiply by the filter response.
        for i in 0..FFT_SIZE {
            self.scratch[i] *= self.filter_response[i];
        }
        // Inverse FFT.
        self.fft_inverse.process(&mut self.scratch);

        // Output is the LAST HOP_SIZE samples of the IFFT (circular
        // artifacts live in the first FIR_LENGTH - 1).
        let norm = 1.0 / FFT_SIZE as f32;
        for i in 0..HOP_SIZE {
            let y = self.scratch[FIR_LENGTH - 1 + i].re * norm;
            self.output_pending.push_back(y);
        }
    }
}

impl Default for OverlapSaveConvolver {
    fn default() -> Self {
        Self::new()
    }
}

