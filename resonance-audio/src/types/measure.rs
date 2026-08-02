//! Offline mix-measurement protocol descriptors (ba todo #1218, doc #273).
//!
//! The command surface for "measure a mix slice without writing a file":
//! [`AudioCommand::MeasureMix`] carries a list of [`StemSource`] targets
//! plus a shared range, and the engine answers with one
//! [`AudioEvent::MixMeasured`] carrying one [`MixMeasurement`] per target.
//!
//! Pure data — the engine's measurement worker (`engine::bounce::measure`)
//! hangs its behaviour off these types, exactly as the stem exporter does
//! off [`StemSource`] / [`StemTarget`][super::StemTarget].
//!
//! **Why the targets are `StemSource` and not `StemTarget`:** a measurement
//! writes nothing to disk, so a per-target path would be a dead field. The
//! list is still a *vector* so that the control layer's `meter.stems`
//! (todo #1220) is a pure enumeration of sources over ONE engine pass —
//! every target is rendered over the same range, so the numbers are
//! directly comparable — rather than a second render path.
//!
//! [`AudioCommand::MeasureMix`]: super::AudioCommand::MeasureMix
//! [`AudioEvent::MixMeasured`]: super::AudioEvent::MixMeasured

use resonance_metering::offline::BandShares;

use super::{SamplePos, StemSource};

/// Where the numbers in a [`MixMeasurement`] come from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MeasureSource {
    /// Render the target offline over the requested range and measure the
    /// rendered buffer. Deterministic and repeatable: the same project and
    /// range always produce the same numbers. Requires a stopped transport
    /// (the offline renderer shares plugin instances with live playback).
    Render,
    /// Read the engine's live master meter
    /// (`SharedState::mix_meter`) — "what just played", with no render at
    /// all. Only valid for [`StemSource::Master`] and only for a single
    /// target; anything else is refused with
    /// [`AudioEvent::MixMeasureError`] rather than silently rendering.
    ///
    /// The live meter is a streaming BS.1770 tap, so it carries no
    /// whole-buffer figures: see [`MixMeasurement`] for exactly which
    /// fields are unavailable on this path and what they report instead.
    ///
    /// [`AudioEvent::MixMeasureError`]: super::AudioEvent::MixMeasureError
    Live,
}

/// Everything the mix report needs about one measured slice of the mix.
///
/// Produced by [`AudioCommand::MeasureMix`][super::AudioCommand::MeasureMix]
/// and carried back on
/// [`AudioEvent::MixMeasured`][super::AudioEvent::MixMeasured]. Loudness is
/// BS.1770-4 / EBU R128 throughout.
///
/// ## Availability on the [`MeasureSource::Live`] path
///
/// The live master meter is a streaming tap with no access to the whole
/// buffer, so the whole-range figures cannot be derived from it. On that
/// path — and only there — the following fields carry documented
/// placeholders rather than measurements: `sample_peak_db`
/// (`FLOOR_DBFS`), `clipped_samples` (`0`), `mono_penalty_db` (`0.0`),
/// `bands` ([`BandShares::SILENT`]), and `range_start` / `range_end` /
/// `frames` (all `0`, since the live meter integrates from the start of
/// the session, not over a range). `lufs_short_term_max` and
/// `lufs_momentary_max` carry the meter's *current* short-term and
/// momentary readings, not maxima. Read [`source`](Self::source) to know
/// which set of rules applies.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MixMeasurement {
    /// Which slice of the mix this measures.
    pub target: StemSource,
    /// Which path produced the numbers — see the availability rules above.
    pub source: MeasureSource,
    /// First sample of the measured range (engine samples, absolute
    /// timeline position). `0` on the live path.
    pub range_start: SamplePos,
    /// One past the last sample of the measured range. `0` on the live
    /// path.
    pub range_end: SamplePos,
    /// Length of the measured range in sample frames (`range_end -
    /// range_start`), i.e. the number of frames actually fed to the
    /// meters. `0` on the live path.
    pub frames: u64,

    /// Gated integrated loudness over the range, LUFS.
    /// `f32::NEG_INFINITY` for silence or for a range shorter than one
    /// 400 ms gating block.
    pub lufs_integrated: f32,
    /// Loudest 3 s short-term window in the range, LUFS.
    /// `f32::NEG_INFINITY` when the range is shorter than 3 s.
    pub lufs_short_term_max: f32,
    /// Loudest 400 ms momentary window in the range, LUFS.
    /// `f32::NEG_INFINITY` when the range is shorter than 400 ms.
    pub lufs_momentary_max: f32,
    /// EBU R128 loudness range, LU. `0.0` when there are too few
    /// short-term windows to form a range.
    pub lra_lu: f32,

    /// Maximum inter-sample (true) peak, dBTP, 4x oversampled. Floored at
    /// -120 for silence.
    pub true_peak_dbtp: f32,
    /// Maximum absolute sample value, dBFS. Floored at
    /// [`FLOOR_DBFS`][resonance_metering::offline::FLOOR_DBFS].
    pub sample_peak_db: f32,
    /// Peak-to-RMS ratio over the WHOLE range, dB. `0.0` for silence.
    pub crest_db: f32,
    /// Number of channel samples at or beyond digital full scale
    /// (`|x| >= 1.0`), counted per channel sample rather than per frame.
    pub clipped_samples: u64,
    /// Pearson correlation of L against R over the whole range, in
    /// `[-1, 1]`. `+1` is mono-identical, `0` uncorrelated, `-1` fully
    /// anti-phase. `0.0` for a silent or single-sided range.
    pub correlation: f32,
    /// Loudness lost when the range is folded to mono, dB (negative means
    /// level is lost). See
    /// [`mono_penalty_db`][resonance_metering::offline::mono_penalty_db].
    pub mono_penalty_db: f32,
    /// Fraction of the range's energy in each of the four AES tonal
    /// bands. Sums to 1.0 except for silence, where all four are 0.
    pub bands: BandShares,
}
