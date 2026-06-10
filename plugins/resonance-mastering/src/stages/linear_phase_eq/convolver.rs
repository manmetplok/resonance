//! Overlap-save FFT convolution engine — a thin wrapper around the
//! shared [`resonance_dsp::FftConvolver`] pinned to the mastering
//! chain's FIR geometry.
//!
//! Given a real impulse response of length ≤ `FIR_LENGTH`, the convolver
//! computes its forward FFT once and reuses the stored frequency-domain
//! response on every audio block: a 4097-tap FIR fits a single
//! `FFT_SIZE = 8192` partition (hop 4096), so per-block cost is one
//! forward FFT, a complex element-wise multiply, and one inverse FFT at
//! any input sample rate.
//!
//! Streaming semantics: audio is pushed in variable-sized chunks; the
//! convolver accumulates enough samples to fill one overlap-save hop,
//! runs an FFT iteration, and stashes outputs in a flat ring buffer so
//! the host can pop any number of samples per block. No allocation
//! happens after construction (`set_impulse_response` keeps the single
//! partition in place).

use resonance_dsp::FftConvolver;

/// FIR length (odd → integer group delay). A 4097-tap linear-phase FIR
/// has group delay 2048 samples ≈ 42.7 ms at 48 kHz, appropriate for
/// mastering applications.
pub const FIR_LENGTH: usize = 4097;
/// FFT size used for the overlap-save convolution.
pub const FFT_SIZE: usize = 8192;
/// Number of new samples consumed per FFT iteration. With
/// `FFT_SIZE − FIR_LENGTH + 1`, the IFFT produces exactly `HOP_SIZE`
/// circular-artifact-free output samples per iteration.
pub const HOP_SIZE: usize = FFT_SIZE - FIR_LENGTH + 1;
/// Group delay of a symmetric FIR of length `FIR_LENGTH`.
pub const GROUP_DELAY: usize = (FIR_LENGTH - 1) / 2;

// The shared convolver fixes its FFT size at twice the hop; the FIR
// geometry above must agree (4097 taps = hop + 1, the single-partition
// maximum).
const _: () = assert!(FFT_SIZE == 2 * HOP_SIZE);
const _: () = assert!(FIR_LENGTH == HOP_SIZE + 1);

/// A single-channel overlap-save convolver with a stored filter.
pub struct OverlapSaveConvolver {
    inner: FftConvolver,
}

impl OverlapSaveConvolver {
    pub fn new() -> Self {
        // Initial filter: pure delta, zero-phase. The resulting FIR is a
        // single 1.0 at the centre, padded with zeros. In overlap-save
        // this gives an identity passthrough delayed by GROUP_DELAY.
        let mut impulse = vec![0.0_f32; FIR_LENGTH];
        impulse[GROUP_DELAY] = 1.0;
        Self {
            inner: FftConvolver::new(&impulse, HOP_SIZE),
        }
    }

    /// Replace the filter impulse response. `h.len()` must be ≤ `FIR_LENGTH`.
    pub fn set_impulse_response(&mut self, h: &[f32]) {
        assert!(
            h.len() <= FIR_LENGTH,
            "impulse response must fit in FIR_LENGTH ({FIR_LENGTH}) taps"
        );
        self.inner.set_impulse_response(h);
    }

    /// Clear the convolver's streaming state. Keeps the filter response.
    pub fn reset(&mut self) {
        self.inner.reset();
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
        self.inner.process_in_place(buffer);
    }
}

impl Default for OverlapSaveConvolver {
    fn default() -> Self {
        Self::new()
    }
}
