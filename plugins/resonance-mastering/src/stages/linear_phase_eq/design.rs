//! FIR design from a cascade of parametric biquads.
//!
//! Produces a zero-phase (symmetric) FIR that matches the magnitude
//! response of the biquad chain. The design runs whenever any band
//! parameter changes — cheap enough to run inside `process` since it's
//! a single N-point IFFT plus a Hann window application.
//!
//! Algorithm:
//! 1. Evaluate the composite magnitude response at `FFT_SIZE / 2 + 1`
//!    positive-frequency bins (product of per-band biquad magnitudes).
//! 2. Build a Hermitian-symmetric complex array: real part = magnitude,
//!    imaginary part = 0.
//! 3. Inverse-FFT to get a real impulse response.
//! 4. Circular-shift by `FFT_SIZE / 2` so the center of symmetry lands
//!    in the middle of the FIR.
//! 5. Truncate to `FIR_LENGTH` taps, Hann-window to taper the edges.
//!
//! # Mid/side bands
//!
//! A band set to [`MsMode::Mid`] or [`MsMode::Side`] filters only that
//! part of the image. With `Hm` the cascade of the stereo and mid bands
//! and `Hs` that of the stereo and side bands, the stereo outputs are
//!
//! ```text
//! L' = A·L + B·R,   R' = A·R + B·L,   A = (Hm + Hs)/2,   B = (Hm − Hs)/2
//! ```
//!
//! (decode `M' = Hm·M`, `S' = Hs·S` back to L/R). Both `A` and `B` are
//! real and zero-phase, so each is one symmetric FIR of the same length:
//! [`FirPart::Direct`] designs `A`, [`FirPart::Cross`] designs `B`. With
//! every band on `Stereo`, `Hm` and `Hs` are the same product taken in
//! the same order, so `A` is bit-for-bit the plain cascade and `B` is
//! exactly zero.

use resonance_dsp::BiquadCoeffs;
use rustfft::num_complex::Complex;
use rustfft::{Fft, FftPlanner};

use super::band::{BandConfig, MsMode};
use super::convolver::FirGeometry;

/// Which of the two M/S filters a designer produces (see the module
/// docs). `Direct` is the plain cascade when no band is mid/side.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FirPart {
    /// `(Hm + Hs)/2`: applied to each channel itself.
    #[default]
    Direct,
    /// `(Hm − Hs)/2`: applied to the opposite channel and added.
    Cross,
}

/// Stateful FIR designer. Owns the inverse FFT plan and scratch
/// buffers so we can redesign without allocating.
pub struct FirDesigner {
    geometry: FirGeometry,
    ifft: std::sync::Arc<dyn Fft<f32> + Send + Sync>,
    scratch: Vec<Complex<f32>>,
    /// Pre-allocated rustfft scratch so `process_with_scratch` never
    /// allocates when a redesign runs inside `process`.
    fft_scratch: Vec<Complex<f32>>,
    hann: Vec<f32>,
    /// Reusable impulse-response buffer. Returned as a borrow from
    /// [`design`], avoiding a fresh heap allocation on every call.
    h: Vec<f32>,
    /// Reusable per-band biquad buffer (with each band's M/S mode) so
    /// each band is designed once per redesign instead of once per
    /// frequency bin.
    biquads: Vec<(BiquadCoeffs, MsMode)>,
    part: FirPart,
}

impl FirDesigner {
    /// A designer for the base (≤ 48 kHz) geometry.
    pub fn new() -> Self {
        Self::with_geometry(FirGeometry::BASE)
    }

    pub fn with_geometry(geometry: FirGeometry) -> Self {
        Self::with_part(geometry, FirPart::Direct)
    }

    /// A designer for one of the two M/S filters.
    pub fn with_part(geometry: FirGeometry, part: FirPart) -> Self {
        let mut planner = FftPlanner::<f32>::new();
        let ifft = planner.plan_fft_inverse(geometry.fft_size);
        let hann = resonance_dsp::hann_window(geometry.fir_len);
        Self {
            geometry,
            fft_scratch: vec![Complex::new(0.0, 0.0); ifft.get_inplace_scratch_len()],
            ifft,
            scratch: vec![Complex::new(0.0, 0.0); geometry.fft_size],
            hann,
            h: vec![0.0; geometry.fir_len],
            // Reserved up front: `design` may run on the audio thread
            // (the inline fallback), where the first push must not
            // allocate.
            biquads: Vec::with_capacity(2 * super::NUM_BANDS),
            part,
        }
    }

    /// Design a symmetric FIR of the geometry's length whose magnitude
    /// response matches the cascaded biquad chain described by `bands`.
    /// Returns a borrow of the internal impulse-response buffer so
    /// back-to-back redesigns do not allocate.
    pub fn design(&mut self, bands: &[BandConfig], sample_rate: f32) -> &[f32] {
        let fft_size = self.geometry.fft_size;
        let fir_len = self.geometry.fir_len;
        let half = fft_size / 2;

        // Design each enabled band's biquad once up front; the per-bin
        // loop below only evaluates magnitudes.
        self.biquads.clear();
        self.biquads.extend(
            bands
                .iter()
                .filter(|b| b.enabled)
                .map(|b| (b.to_coeffs(sample_rate), b.ms)),
        );

        // Compute composite magnitude response at each positive-frequency
        // bin. The biquad chain is cascaded by multiplying magnitudes,
        // once for the mid and once for the side (a stereo band is in
        // both), then combined into this designer's part.
        //
        // Evaluated in f64 from f64 designs: the f32 form loses up to
        // ~7 dB in a 20 Hz high-pass stopband at 192 kHz (DSP2-07).
        for k in 0..=half {
            let half_w = std::f64::consts::PI * k as f64 / fft_size as f64;
            let phi = half_w.sin().powi(2);
            let mut mid = 1.0_f64;
            let mut side = 1.0_f64;
            for (bq, ms) in &self.biquads {
                let m = bq.magnitude_at_sin2(phi);
                match ms {
                    MsMode::Stereo => {
                        mid *= m;
                        side *= m;
                    }
                    MsMode::Mid => mid *= m,
                    MsMode::Side => side *= m,
                }
            }
            let mag = match self.part {
                FirPart::Direct => (mid + side) * 0.5,
                FirPart::Cross => (mid - side) * 0.5,
            } as f32;
            self.scratch[k] = Complex::new(mag, 0.0);
            // Mirror to the negative-frequency half (Hermitian symmetry).
            if k > 0 && k < half {
                self.scratch[fft_size - k] = Complex::new(mag, 0.0);
            }
        }

        // Inverse FFT → real impulse response (imaginary part ≈ 0).
        self.ifft
            .process_with_scratch(&mut self.scratch, &mut self.fft_scratch);
        let norm = 1.0 / fft_size as f32;

        // The IFFT output is a zero-phase impulse response centred at
        // index 0 (i.e. samples [0..FIR_LENGTH/2] come from positive
        // offsets, samples [FFT_SIZE - FIR_LENGTH/2..FFT_SIZE] from
        // negative offsets). Circular-shift by `FFT_SIZE / 2` so the
        // centre lands at index `FIR_LENGTH / 2` of our FIR output.
        let half_fir = fir_len / 2;
        for i in 0..fir_len {
            let src = ((i as isize - half_fir as isize).rem_euclid(fft_size as isize)) as usize;
            self.h[i] = self.scratch[src].re * norm * self.hann[i];
        }
        &self.h
    }
}

impl Default for FirDesigner {
    fn default() -> Self {
        Self::new()
    }
}

