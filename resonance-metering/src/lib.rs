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

pub mod correlation;
pub mod crest;
pub mod k_weighting;
pub mod lra;
pub mod lufs;
pub mod plr;
pub mod snapshot;
pub mod spectrum;
pub mod true_peak;

pub use correlation::CorrelationMeter;
pub use crest::CrestMeter;
pub use k_weighting::KWeightingFilter;
pub use lra::LraMeter;
pub use lufs::{LufsMeter, LufsReadout};
pub use plr::{PlrMeter, PlrReadout};
pub use snapshot::MeterSnapshot;
pub use spectrum::{SpectrumAnalyzer, SpectrumHandle, SpectrumSnapshot, FFT_SIZE, NUM_OCTAVE_BINS};
pub use true_peak::TruePeakMeter;
