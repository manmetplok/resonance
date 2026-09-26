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
//! stepping at hop boundaries. The design runs on a background worker
//! and lands on the next hop boundary, falling back to an identical
//! inline design if the worker is late, so the output never depends on
//! thread timing (FU-M2a, see [`worker`]). Redesigns are rate-limited
//! to one per hop: while one waits for its boundary, further changes
//! wait and the newest settings are designed next.

pub mod band;
pub mod convolver;
pub mod design;
pub mod worker;

pub use band::{BandConfig, BandType};
pub use convolver::{FirGeometry, OverlapSaveConvolver, FIR_LENGTH, GROUP_DELAY, HOP_SIZE};
pub use design::FirDesigner;
pub use worker::{DesignWorker, SpectrumDesigner, StereoFir};

use std::sync::Arc;

/// Number of parametric bands exposed by the plugin per EQ instance.
/// Phase 3 ships with four bands; the chain can grow later without
/// touching the convolver or designer — they're band-count-agnostic.
pub const NUM_BANDS: usize = 4;

/// Stereo linear-phase parametric EQ.
///
/// A [`StereoFir`] (two convolvers plus the off-thread designer) and a
/// cached snapshot of the band parameters the current filter was
/// designed for. Any difference between the supplied `bands` and the
/// cache requests a redesign on the next `process_stereo` call.
pub struct LinearPhaseEq {
    fir: StereoFir,
    /// Band parameters of the current (or pending) FIR. Compared on
    /// every `process_stereo` to decide whether to redesign.
    cached_bands: [BandConfig; NUM_BANDS],
}

impl LinearPhaseEq {
    /// An EQ with its own design worker thread.
    pub fn new(sample_rate: f32) -> Self {
        Self::with_worker(sample_rate, Some(&DesignWorker::spawn()))
    }

    /// An EQ designing through `worker` (shared with other filters), or
    /// always inline with `None`. The output is identical either way.
    pub fn with_worker(sample_rate: f32, worker: Option<&Arc<DesignWorker>>) -> Self {
        // FIR length scales with the rate so the low bands keep their
        // resolution (DSP-06).
        Self {
            fir: StereoFir::new(sample_rate, worker),
            cached_bands: [BandConfig::off(); NUM_BANDS],
        }
    }

    pub fn reset(&mut self) {
        self.fir.reset();
    }

    /// Reported per-channel latency. Same for both channels; constant
    /// in ms across sample rates.
    pub fn latency(&self) -> usize {
        self.fir.latency()
    }

    /// Designs taken from the worker vs. designed inline (diagnostics).
    pub fn design_counts(&self) -> (u64, u64) {
        self.fir.design_counts()
    }

    /// Process one stereo block in place, requesting a redesign first if
    /// any band parameter has changed since the last one.
    pub fn process_stereo(
        &mut self,
        left: &mut [f32],
        right: &mut [f32],
        bands: &[BandConfig; NUM_BANDS],
    ) {
        // At most one redesign per hop: while one is pending the newest
        // settings wait (they differ from `cached_bands`, so they are
        // picked up on the first block after it lands).
        if *bands != self.cached_bands && self.fir.request(bands) {
            self.cached_bands = *bands;
        }
        self.fir.process(left, right);
    }
}
