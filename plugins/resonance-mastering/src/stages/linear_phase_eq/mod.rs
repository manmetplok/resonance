//! Linear-phase parametric EQ stage.
//!
//! The engine is shared between the corrective and tonal EQ slots in
//! the mastering chain. Parameters specify a chain of parametric biquad
//! bands (bell / shelf / cut); the magnitude response of that chain is
//! sampled on an FFT grid and the corresponding zero-phase symmetric FIR
//! is fed to the overlap-save convolver.
//!
//! A band parameter change marks the filter dirty; the next `process`
//! call redesigns the FIR (one extra FFT pair) before convolving.

pub mod band;
pub mod convolver;
pub mod design;

pub use band::{BandConfig, BandType};
pub use convolver::{OverlapSaveConvolver, FIR_LENGTH, GROUP_DELAY, HOP_SIZE};
pub use design::FirDesigner;

/// Number of parametric bands exposed by the plugin per EQ instance.
/// Phase 3 ships with four bands; the chain can grow later without
/// touching the convolver or designer — they're band-count-agnostic.
pub const NUM_BANDS: usize = 4;

/// Stereo linear-phase parametric EQ.
///
/// Owns two [`OverlapSaveConvolver`] instances (one per channel), a
/// [`FirDesigner`], and a cached snapshot of the band parameters used
/// for the currently-loaded FIR. Any difference between the supplied
/// `bands` slice and the cache triggers a redesign on the next
/// `process_stereo` call.
pub struct LinearPhaseEq {
    sample_rate: f32,
    left: OverlapSaveConvolver,
    right: OverlapSaveConvolver,
    designer: FirDesigner,
    /// Band parameters used for the currently-loaded FIR. Compared on
    /// every `process_stereo` to decide whether to redesign.
    cached_bands: [BandConfig; NUM_BANDS],
}

impl LinearPhaseEq {
    pub fn new(sample_rate: f32) -> Self {
        Self {
            sample_rate,
            left: OverlapSaveConvolver::new(),
            right: OverlapSaveConvolver::new(),
            designer: FirDesigner::new(),
            cached_bands: [BandConfig::off(); NUM_BANDS],
        }
    }

    pub fn reset(&mut self) {
        self.left.reset();
        self.right.reset();
    }

    /// Reported per-channel latency. Same for both channels.
    pub const fn latency(&self) -> usize {
        GROUP_DELAY + HOP_SIZE
    }

    /// Process one stereo block in place, redesigning the filter first
    /// if any band parameter has changed since the last call.
    pub fn process_stereo(
        &mut self,
        left: &mut [f32],
        right: &mut [f32],
        bands: &[BandConfig; NUM_BANDS],
    ) {
        if *bands != self.cached_bands {
            self.cached_bands = *bands;
            let enabled: Vec<BandConfig> = bands.iter().copied().filter(|b| b.enabled).collect();
            // Split the borrow so the designer result (which borrows
            // from `self.designer`) can coexist with mutable borrows
            // of the two convolver fields.
            let h = self.designer.design(&enabled, self.sample_rate);
            self.left.set_impulse_response(h);
            self.right.set_impulse_response(h);
        }
        self.left.process_in_place(left);
        self.right.process_in_place(right);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_eq_is_pure_delay() {
        let mut eq = LinearPhaseEq::new(48_000.0);
        let latency = eq.latency();
        let n = latency + 2048;
        let mut left = vec![0.0_f32; n];
        let mut right = vec![0.0_f32; n];
        for i in 0..n {
            let s = (i as f32 * 0.05).sin() * 0.3;
            left[i] = s;
            right[i] = s;
        }
        let input = left.clone();
        eq.process_stereo(&mut left, &mut right, &[BandConfig::off(); NUM_BANDS]);
        // Output at latency offset matches input with small tolerance.
        let mut max_err = 0.0_f32;
        for i in latency..n {
            max_err = max_err.max((left[i] - input[i - latency]).abs());
            max_err = max_err.max((right[i] - input[i - latency]).abs());
        }
        assert!(max_err < 5e-3, "default EQ error = {max_err}");
    }

    #[test]
    fn bell_cut_attenuates_centre_frequency() {
        // -12 dB bell at 1 kHz: a 1 kHz sine should come out ~4× quieter
        // after the EQ (tolerance 2 dB for FIR truncation/windowing).
        let sr = 48_000.0;
        let mut eq = LinearPhaseEq::new(sr);
        let latency = eq.latency();
        let n = latency + 8192;
        let mut l = vec![0.0_f32; n];
        let mut r = vec![0.0_f32; n];
        let freq = 1000.0;
        let omega = std::f32::consts::TAU * freq / sr;
        for i in 0..n {
            l[i] = (i as f32 * omega).sin() * 0.5;
            r[i] = l[i];
        }
        let mut bands = [BandConfig::off(); NUM_BANDS];
        bands[0] = BandConfig {
            enabled: true,
            band_type: BandType::Bell,
            freq_hz: 1000.0,
            q: 1.0,
            gain_db: -12.0,
        };
        eq.process_stereo(&mut l, &mut r, &bands);
        // Measure RMS of the settled tail.
        let tail_start = latency + 2048;
        let tail_len = n - tail_start;
        let mut sum_sq = 0.0_f64;
        for &s in &l[tail_start..n] {
            sum_sq += (s as f64) * (s as f64);
        }
        let rms = (sum_sq / tail_len as f64).sqrt() as f32;
        let input_rms = 0.5 / 2.0_f32.sqrt();
        let gain_db = 20.0 * (rms / input_rms).log10();
        assert!(
            (gain_db - -12.0).abs() < 2.0,
            "gain at 1 kHz = {gain_db} dB (expected -12 ± 2)"
        );
    }
}
