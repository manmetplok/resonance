//! Overlap-save FFT convolution engine — a thin wrapper around the
//! shared [`resonance_dsp::FftConvolver`] pinned to the mastering
//! chain's FIR geometry.
//!
//! Given a real impulse response of length ≤ the geometry's FIR length,
//! the convolver computes its forward FFT once and reuses the stored
//! frequency-domain response on every audio block: a `hop + 1`-tap FIR
//! fits a single `2·hop` partition, so per-hop cost is one forward FFT,
//! a complex element-wise multiply, and one inverse FFT.
//!
//! The geometry scales with the sample rate ([`FirGeometry`]): the base
//! 4097 taps at ≤ 48 kHz, doubled per octave of rate above that, so the
//! FIR's time span — and its low-frequency resolution — stays ~85 ms
//! and the latency stays constant in ms (DSP-06).
//!
//! Streaming semantics: audio is pushed in variable-sized chunks; the
//! convolver accumulates enough samples to fill one overlap-save hop,
//! runs an FFT iteration, and stashes outputs in a flat ring buffer so
//! the host can pop any number of samples per block. No allocation
//! happens after construction (`set_impulse_response` keeps the single
//! partition in place).

use resonance_dsp::FftConvolver;

/// Base (≤ 48 kHz) FIR length (odd → integer group delay). A 4097-tap
/// linear-phase FIR has group delay 2048 samples ≈ 42.7 ms at 48 kHz,
/// appropriate for mastering applications. See [`FirGeometry`] for the
/// rate-scaled geometry.
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

/// Largest rate multiple the geometry scales to (8 × 48 kHz = 384 kHz):
/// bounds the FIR at 32769 taps and the per-EQ latency at 49152 samples.
const MAX_GEOMETRY_SCALE: usize = 8;

/// FIR/FFT geometry of one linear-phase filter: an odd `fir_len`-tap
/// symmetric FIR convolved in a single `fft_size = 2·hop` partition.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FirGeometry {
    pub fir_len: usize,
    pub hop: usize,
    pub fft_size: usize,
    pub group_delay: usize,
}

impl FirGeometry {
    /// The base geometry: [`FIR_LENGTH`] taps, hop [`HOP_SIZE`].
    pub const BASE: Self = Self::scaled(1);

    const fn scaled(scale: usize) -> Self {
        let hop = HOP_SIZE * scale;
        Self {
            fir_len: hop + 1,
            hop,
            fft_size: 2 * hop,
            group_delay: hop / 2,
        }
    }

    /// Geometry for `sample_rate`: the base up to 48 kHz, doubled per
    /// octave above it (8192 taps' worth at 96 kHz, 16384 at 192 kHz),
    /// capped at 8×. The FIR's time span and the latency in ms thereby
    /// stay constant, where a fixed length in samples lost its low-band
    /// resolution at high rates (DSP-06).
    pub fn for_sample_rate(sample_rate: f32) -> Self {
        let ratio = (sample_rate / 48_000.0).max(1.0);
        let scale = (ratio.ceil() as usize)
            .next_power_of_two()
            .min(MAX_GEOMETRY_SCALE);
        Self::scaled(scale)
    }

    /// Total latency of a convolver with this geometry: the FIR's
    /// group delay plus one buffered hop.
    pub const fn latency(&self) -> usize {
        self.group_delay + self.hop
    }
}

/// A single-channel overlap-save convolver with a stored filter.
pub struct OverlapSaveConvolver {
    inner: FftConvolver,
    geometry: FirGeometry,
}

impl OverlapSaveConvolver {
    /// A convolver with the base (≤ 48 kHz) geometry.
    pub fn new() -> Self {
        Self::with_geometry(FirGeometry::BASE)
    }

    pub fn with_geometry(geometry: FirGeometry) -> Self {
        // Initial filter: pure delta, zero-phase. The resulting FIR is a
        // single 1.0 at the centre, padded with zeros. In overlap-save
        // this gives an identity passthrough delayed by the group delay.
        let mut impulse = vec![0.0_f32; geometry.fir_len];
        impulse[geometry.group_delay] = 1.0;
        Self {
            inner: FftConvolver::new(&impulse, geometry.hop),
            geometry,
        }
    }

    pub fn geometry(&self) -> FirGeometry {
        self.geometry
    }

    /// Replace the filter impulse response. `h.len()` must be ≤ the
    /// geometry's FIR length.
    pub fn set_impulse_response(&mut self, h: &[f32]) {
        assert!(
            h.len() <= self.geometry.fir_len,
            "impulse response must fit in {} taps",
            self.geometry.fir_len
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
        // The group delay comes from the symmetric FIR; the hop from the
        // fact that we buffer one full hop before producing any output.
        self.geometry.latency()
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
