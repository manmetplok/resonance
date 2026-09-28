//! Measurement DSP for Resonance mastering / metering plugins.
//!
//! All algorithms follow **ITU-R BS.1770-4** and the associated EBU R128
//! tech specs. The crate is framework-agnostic: no plugin dependencies,
//! no GUI, no I/O. Build on top of it via:
//!
//! - [`LufsMeter`] — momentary / short-term / gated-integrated LUFS
//! - [`TruePeakMeter`] — 4x oversampled inter-sample peak (BS.1770-4 Annex 2)
//! - [`LraMeter`] — EBU R128 loudness range
//! - [`SpectrumAnalyzer`] / [`SpectrumHandle`] — background-thread FFT
//! - [`CorrelationMeter`], [`CrestMeter`], [`PlrMeter`]
//! - [`MeterSnapshot`] — aggregate for lock-free publication to a UI thread
//! - [`viz`] — lock-free audio-thread → editor primitives ([`AtomicF32`],
//!   [`AtomicF32Pair`], [`AtomicF32Array`], [`AtomicHistoryRing`])
//! - [`offline`] — pure whole-buffer primitives for mix analysis
//!   ([`band_shares`], [`mono_penalty_db`], [`sample_peak_db`],
//!   [`clipped_samples`])
//! - [`detail`] — the opt-in `meter.*` detail proxies (spectrum: tilt,
//!   1/3-octave LTAS, centroid, resonances; stereo: per-band correlation,
//!   S/M, mono loss, windows, balance, one-sidedness, Haas lag)

pub mod atomic_snapshot;
pub mod correlation;
pub mod crest;
pub mod detail;
pub mod k_weighting;
pub mod lra;
pub mod lufs;
pub mod offline;
pub mod plr;
pub mod snapshot;
pub mod spectrum;
pub mod true_peak;
pub mod viz;

pub use atomic_snapshot::AtomicMeterSnapshot;
pub use correlation::CorrelationMeter;
pub use crest::CrestMeter;
pub use k_weighting::KWeightingFilter;
pub use lra::LraMeter;
pub use lufs::{LufsMeter, LufsReadout};
pub use offline::{
    band_shares, clipped_samples, mono_penalty_db, sample_peak_db, sample_peak_linear, BandShares,
};
pub use plr::{PlrMeter, PlrReadout, RangeDynamics};
pub use snapshot::MeterSnapshot;
pub use spectrum::{SpectrumAnalyzer, SpectrumHandle, SpectrumSnapshot, FFT_SIZE, NUM_OCTAVE_BINS};
pub use true_peak::TruePeakMeter;
pub use viz::{AtomicF32, AtomicF32Array, AtomicF32Pair, AtomicHistoryRing};
