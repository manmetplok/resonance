//! Linear-phase parametric EQ stage.
//!
//! The engine is shared between the corrective and tonal EQ slots in
//! the mastering chain. Parameters specify a chain of parametric biquad
//! bands (bell / shelf / cut); the magnitude response of that chain is
//! sampled on an FFT grid and the corresponding zero-phase symmetric FIR
//! is fed to the overlap-save convolver.
//!
//! A band parameter change redesigns the FIR and crossfades to it over
//! one convolver hop (DSP-10), so an automated band morphs instead of
//! stepping at hop boundaries. Redesigns are rate-limited to one per hop:
//! while a crossfade is staged, further changes wait for it to land and
//! the newest settings are designed next.

pub mod band;
pub mod convolver;
pub mod design;

pub use band::{BandConfig, BandType};
pub use convolver::{FirGeometry, OverlapSaveConvolver, FIR_LENGTH, GROUP_DELAY, HOP_SIZE};
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
        // FIR length scales with the rate so the low bands keep their
        // resolution (DSP-06).
        let geometry = FirGeometry::for_sample_rate(sample_rate);
        Self {
            sample_rate,
            left: OverlapSaveConvolver::with_geometry(geometry),
            right: OverlapSaveConvolver::with_geometry(geometry),
            designer: FirDesigner::with_geometry(geometry),
            cached_bands: [BandConfig::off(); NUM_BANDS],
        }
    }

    pub fn reset(&mut self) {
        self.left.reset();
        self.right.reset();
    }

    /// Reported per-channel latency. Same for both channels; constant
    /// in ms across sample rates.
    pub const fn latency(&self) -> usize {
        self.left.latency()
    }

    /// Process one stereo block in place, redesigning the filter first
    /// if any band parameter has changed since the last call.
    pub fn process_stereo(
        &mut self,
        left: &mut [f32],
        right: &mut [f32],
        bands: &[BandConfig; NUM_BANDS],
    ) {
        // At most one redesign per hop: while a crossfade is staged the
        // newest settings wait (they differ from `cached_bands`, so they
        // are picked up on the first block after it lands).
        if *bands != self.cached_bands && !self.left.crossfade_pending() {
            self.cached_bands = *bands;
            // `FirDesigner::design` already iterates `bands` and skips
            // disabled entries, so we can pass the whole fixed array
            // directly. Previously we filter+collected into a fresh
            // `Vec<BandConfig>` on every band-parameter change — fine
            // for a one-off but allocates on the audio thread.
            let h = self.designer.design(bands.as_slice(), self.sample_rate);
            self.left.crossfade_to_impulse_response(h);
            self.right.crossfade_to_impulse_response(h);
        }
        self.left.process_in_place(left);
        self.right.process_in_place(right);
    }
}

