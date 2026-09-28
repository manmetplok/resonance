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

use resonance_metering::detail::{SpectrumDetail, StereoDetail};
use resonance_metering::RangeDynamics;
use resonance_metering::offline::BandShares;

use super::{BusId, SamplePos, StemSource};

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

/// Which opt-in details a measurement computes on top of the default
/// figures (warmth-width-depth.md §7.1). All off by default, so a plain
/// measurement costs exactly what it did before details existed.
///
/// Details exist only on the [`MeasureSource::Render`] path: every one of
/// them needs the whole rendered buffer. The live path ignores the set.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DetailSet {
    /// 1/3-octave LTAS, tilt, centroid, low-mid/presence, presence
    /// peakiness, air ratio and resonance peaks.
    pub spectrum: bool,
    /// Per-band correlation, side/mid and mono loss, windowed
    /// correlation, balance, one-sidedness and the Haas detector.
    pub stereo: bool,
    /// PLR and PSR.
    pub dynamics: bool,
    /// HF tilt and, for track targets, the DRR estimate from sends and
    /// each return's measured gain (renders every return a track sends
    /// to, once per pass).
    pub depth: bool,
}

impl DetailSet {
    /// True when at least one detail is requested.
    pub fn any(self) -> bool {
        self.spectrum || self.stereo || self.dynamics || self.depth
    }
}

/// One of a track's aux sends, as the depth estimate saw it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DepthSend {
    /// The return bus it feeds.
    pub bus_id: BusId,
    /// Its level, dB (the static value; send automation is not read).
    pub send_level_db: f32,
    /// Tapped before the source's fader.
    pub pre_fader: bool,
    /// The return's measured gain, dB: output energy over the energy its
    /// sends feed it, over the same range. `None` when the return came
    /// out silent (no chain output, or muted by its own fader).
    pub return_gain_db: Option<f32>,
}

/// The `depth` detail of one measurement (warmth-width-depth.md §7.6).
#[derive(Debug, Clone, PartialEq)]
pub struct DepthDetail {
    /// `E(6–16 kHz) / E(1–4 kHz)`, dB.
    pub hf_tilt_db: Option<f32>,
    /// Direct-to-reverberant ESTIMATE, dB
    /// ([`drr_db_estimate`][resonance_metering::detail::depth::drr_db_estimate]).
    /// `None` for anything but a track target, for a dry-only track, and
    /// when every return it feeds came out silent.
    pub drr_db_estimate: Option<f32>,
    /// A track target with no enabled sends: no reverberant path at all.
    pub dry_only: bool,
    /// The track's enabled sends. Empty for a master or bus target.
    pub sends: Vec<DepthSend>,
}

/// The opt-in details of one [`MixMeasurement`]: `Some` exactly for the
/// details its [`DetailSet`] asked for (and only on the render path).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct MeasurementDetail {
    /// See [`SpectrumDetail`].
    pub spectrum: Option<SpectrumDetail>,
    /// See [`StereoDetail`].
    pub stereo: Option<StereoDetail>,
    /// PLR / PSR of the range, from the measurement's own true peak and
    /// loudness ([`PlrMeter::range`][resonance_metering::PlrMeter::range]).
    pub dynamics: Option<RangeDynamics>,
    /// See [`DepthDetail`].
    pub depth: Option<DepthDetail>,
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
/// (`FLOOR_DBFS`), `crest_db` (`0.0`), `clipped_samples` (`0`),
/// `correlation` (`0.0`), `mono_penalty_db` (`0.0`), `bands`
/// ([`BandShares::SILENT`]), and `range_start` / `range_end` / `frames`
/// (all `0`, since the live meter integrates from the start of the
/// session, not over a range). `lufs_short_term_max` and
/// `lufs_momentary_max` carry the meter's *current* short-term and
/// momentary readings, not maxima. Read [`source`](Self::source) to know
/// which set of rules applies.
///
/// A consumer must treat every field in that list as ABSENT on the live
/// path, not as a measurement. `crest_db` and `correlation` are the two
/// that bite, because their placeholder `0.0` is a plausible reading:
/// 0 dB crest means a square wave and 0 correlation means a fully wide
/// image, so a client that reports them verbatim tells the user the mix
/// is squashed and decorrelated when nothing was measured at all. That
/// is not hypothetical — `meter.measure` shipped a first cut doing
/// exactly this, caught in review (todos #1219, #1247), because these
/// two were missing from the list above.
///
/// Ground truth for the two: `ABMeterTap::snapshot` fills only the
/// loudness and true-peak fields and ends with `..MeterSnapshot::
/// default()`, and neither `CrestMeter` nor `CorrelationMeter` is
/// instantiated anywhere on the live path.
#[derive(Debug, Clone, PartialEq)]
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
    /// Peak-to-RMS ratio over the WHOLE range, dB. `0.0` for silence —
    /// and `0.0` as a PLACEHOLDER on [`MeasureSource::Live`], where
    /// nothing measures it. See the availability rules on the struct.
    pub crest_db: f32,
    /// Number of channel samples at or beyond digital full scale
    /// (`|x| >= 1.0`), counted per channel sample rather than per frame.
    pub clipped_samples: u64,
    /// Pearson correlation of L against R over the whole range, in
    /// `[-1, 1]`. `+1` is mono-identical, `0` uncorrelated, `-1` fully
    /// anti-phase. `0.0` for a silent or single-sided range — and `0.0`
    /// as a PLACEHOLDER on [`MeasureSource::Live`], where nothing
    /// measures it. See the availability rules on the struct.
    pub correlation: f32,
    /// Loudness lost when the range is folded to mono, dB (negative means
    /// level is lost). See
    /// [`mono_penalty_db`][resonance_metering::offline::mono_penalty_db].
    pub mono_penalty_db: f32,
    /// Fraction of the range's energy in each of the four AES tonal
    /// bands. Sums to 1.0 except for silence, where all four are 0.
    pub bands: BandShares,
    /// The opt-in details the command's [`DetailSet`] asked for. All
    /// `None` on the live path and for a plain measurement.
    pub detail: MeasurementDetail,
}
