//! `meter.*` — measure the mix without bouncing a file.
//!
//! The app already runs a full BS.1770 / EBU R128 stack
//! (`resonance-metering`) for its GUI meters; this namespace is what
//! puts those numbers on the wire, so a client that cannot hear can
//! still tell whether a mix is too loud, too squashed, out of phase or
//! bottom-heavy (ba doc #273, todos #1219 / #1220).
//!
//! It replaces the bounce-and-analyse loop entirely: no WAV is written,
//! nothing is mutated, and no external analyser is involved. Because a
//! measurement renders the requested slice offline it runs as a job
//! ([`crate::job::JobStarted`]) exactly like `render.mixdown`; the job's
//! terminal `result` payload is [`MeasureResult`] for
//! [`MEASURE`] and [`StemsResult`] for [`STEMS`].
//!
//! [`STEMS`] is a whole balance pass in one call: every track and the
//! master, measured over ONE shared range in ONE engine pass, so the
//! numbers are directly comparable and no track can bleed into another
//! one's figures. "One pass" means one command, one range and one set
//! of results — not one render: inside it the engine renders each
//! target in turn, so the *cost* scales with the number of tracks even
//! though the *measurement* is a single, self-consistent pass.
//!
//! ## Units, in one place
//!
//! * **LUFS / LU** — loudness. 1 LU is exactly 1 dB, so a track 3 LU
//!   below another is 3 dB quieter. Integrated loudness is *gated*: a
//!   part that only plays in two of nine sections reports how loud it is
//!   **while it plays**, not an average diluted by its silence, which is
//!   what makes these numbers usable for balance.
//! * **dBFS / dBTP** — level. `0` is digital full scale; true peak
//!   (dBTP) is 4x-oversampled and can exceed sample peak between
//!   samples.
//! * **`bands`** — shares of total energy, summing to 1.0. They are
//!   relative by construction: compare them against each other or
//!   against a reference mix, never against an absolute target.
//!
//! ## Opt-in detail
//!
//! `detail: ["spectrum", "stereo", "dynamics"]` (any subset) on either
//! method adds a per-detail object to
//! every result (warmth-width-depth.md §7.1) — see [`MeasureDetail`].
//! Without `detail` the payload is exactly what it always was: the
//! detail objects are omitted, not `null`, so the default reply stays
//! small for token cost. Details need the whole rendered buffer, so they
//! are refused with `source: "live"`.

use crate::ids::TrackId;
use crate::methods::render::RangeSpec;
use serde::{Deserialize, Serialize};

/// `meter.measure` — measure one slice of the mix
/// ([`MeasureParams`] -> job -> [`MeasureResult`]).
pub const MEASURE: &str = "meter.measure";
/// `meter.stems` — measure every track plus the master in one pass
/// ([`StemsParams`] -> job -> [`StemsResult`]).
pub const STEMS: &str = "meter.stems";
/// `meter.snapshot` — measure one slice and keep the numbers for a later
/// `meter.compare` ([`SnapshotParams`] -> job -> [`SnapshotResult`]).
pub const SNAPSHOT: &str = "meter.snapshot";
/// `meter.compare` — loudness-matched deltas between two measurements
/// ([`CompareParams`] -> job -> [`CompareResult`]).
pub const COMPARE: &str = "meter.compare";
/// `meter.probe` — harmonic signature of an insert chain
/// ([`ProbeParams`] -> job -> [`ProbeResult`]).
pub const PROBE: &str = "meter.probe";

/// All `meter.*` method names.
pub const METHODS: &[&str] = &[MEASURE, STEMS, SNAPSHOT, COMPARE, PROBE];

/// Which slice of the mix to measure.
///
/// On the wire this is the string `"master"`, or the object
/// `{"track_id": N}` / `{"bus_id": N}` — the same id space
/// `song.summary` reports every track and bus under. A track target
/// includes that track's sub-tracks, so a multi-output instrument is
/// measured whole. `{"reference": N}` measures a loaded reference track
/// instead (`reference.load`; `meter.measure` only).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub enum MeasureTarget {
    /// The full mix, with master FX, master volume and the final
    /// hard-clip applied — identical to what `render.mixdown` writes.
    #[default]
    #[serde(rename = "master")]
    Master,
    /// One track plus its sub-tracks, before master FX and the master
    /// fader. A bus id is accepted here too, since busses share the
    /// track id space in `song.summary`.
    #[serde(rename = "track_id")]
    Track(TrackId),
    /// One group / return bus: every track routed into it, through the
    /// bus's own FX chain, before master FX and the master fader.
    #[serde(rename = "bus_id")]
    Bus(TrackId),
    /// A reference track loaded with `reference.load`, measured whole
    /// from its decoded audio, exactly as a clip of the same file would
    /// measure. `meter.measure` only; `range` and `source: "live"` do not
    /// apply to it.
    #[serde(rename = "reference")]
    Reference(crate::ids::ReferenceId),
}

/// Where the numbers come from.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(rename_all = "lowercase")]
pub enum MeasureSource {
    /// Render the target offline over the range and measure the result
    /// (the default). Deterministic and repeatable, and the only source
    /// that can measure anything other than the master. Requires a
    /// stopped transport and no other offline render in progress.
    #[default]
    Render,
    /// Read the engine's live master meter — "what just played" — with
    /// no render at all. Only valid for `target: "master"`. It is a
    /// streaming tap, so several whole-buffer figures do not exist on
    /// this path: see [`MeasureResult`] for exactly which, and check
    /// [`MeasureResult::source`] before trusting a field.
    Live,
}

/// An opt-in detail block for `meter.measure` / `meter.stems`.
///
/// Each one adds its own object to every result, named after it; each
/// costs one extra spectral analysis of the rendered buffer (shared
/// between details), not an extra render.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(rename_all = "lowercase")]
pub enum MeasureDetail {
    /// [`SpectrumDetail`]: 1/3-octave LTAS, spectral tilt, centroid,
    /// low-mid/presence ratio, presence peakiness, air ratio and the
    /// strongest narrow resonances — the warmth and harshness proxies.
    Spectrum,
    /// [`StereoDetail`]: per-band correlation, side/mid and mono loss, a
    /// windowed-correlation summary, balance, a one-sided flag and a Haas
    /// (static inter-channel delay) detector — width and mono safety.
    Stereo,
    /// [`DynamicsDetail`]: PLR and PSR.
    Dynamics,
    /// [`DepthDetail`]: HF tilt, and for tracks a direct-to-reverberant
    /// ESTIMATE from their sends plus a front/middle/back layer hint.
    /// Meant for `meter.stems`, where the layer hint ranks the tracks;
    /// each return a track sends to is rendered once more per pass.
    Depth,
}

/// Params for `meter.measure`. Every field is optional: the default is
/// the whole song's master mix, rendered offline.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct MeasureParams {
    /// Defaults to `"master"`. `{"reference": N}` measures a loaded
    /// reference track.
    #[serde(default)]
    pub target: MeasureTarget,
    /// Defaults to the whole song. A range reaching past the end of the
    /// song is clamped to it rather than refused. Refused for a
    /// reference, which is always measured whole.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub range: Option<RangeSpec>,
    /// Defaults to `"render"`.
    #[serde(default)]
    pub source: MeasureSource,
    /// Opt-in detail blocks, any of `["spectrum", "stereo", "dynamics"]`.
    /// Defaults to none, which
    /// keeps the reply to the standard figures. Requires `source:
    /// "render"`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub detail: Vec<MeasureDetail>,
}

/// Shares of the measured range's energy in the four AES tonal bands.
///
/// Raw energy shares of the mono sum, summing to 1.0 (all four are 0.0
/// for silence). Unweighted — no equal-loudness curve — so they describe
/// spectral *balance*, not perceived brightness, and are only meaningful
/// compared against another measurement.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct Bands {
    /// 20 Hz - 250 Hz.
    pub low: f32,
    /// 250 Hz - 2 kHz.
    pub mid: f32,
    /// 2 kHz - 8 kHz.
    pub high: f32,
    /// 8 kHz - 20 kHz.
    pub air: f32,
}

/// Nominal ISO centre frequencies of [`SpectrumDetail::third_octave`],
/// in Hz, lowest band first.
pub const THIRD_OCTAVE_HZ: [f32; 31] = [
    20.0, 25.0, 31.5, 40.0, 50.0, 63.0, 80.0, 100.0, 125.0, 160.0, 200.0, 250.0, 315.0, 400.0,
    500.0, 630.0, 800.0, 1_000.0, 1_250.0, 1_600.0, 2_000.0, 2_500.0, 3_150.0, 4_000.0, 5_000.0,
    6_300.0, 8_000.0, 10_000.0, 12_500.0, 16_000.0, 20_000.0,
];

/// One narrow resonance in [`SpectrumDetail::peaks`].
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct SpectralPeak {
    /// Frequency of the resonance, Hz.
    pub freq_hz: f64,
    /// How far its 1/6-octave band stands above the smoothed spectrum
    /// around it (the mean level of the octave either side), dB.
    pub excess_db: f64,
}

/// The `spectrum` detail: tonal balance and warmth proxies.
///
/// Numbers in detail blocks are `f64` and pre-rounded (0.1 dB for band
/// levels, 0.01 for ratios, 1 Hz for frequencies) so they serialize
/// short: a rounded `f32` widens to digits like `-30.100000381469727`.
///
/// Read off the stereo long-term average spectrum (the mean of the left
/// and right power spectra), so unlike [`Bands`] it keeps anti-phase
/// side content. `null` fields mean silence.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct SpectrumDetail {
    /// Level of the 31 ISO 1/3-octave bands, 20 Hz to 20 kHz (centres in
    /// [`THIRD_OCTAVE_HZ`]), dB, where a full-scale sine in the band
    /// reads 0. Pink noise reads FLAT here; a band with no energy reads
    /// -120.
    pub third_octave: Vec<f64>,
    /// Spectral tilt over 100 Hz-10 kHz, dB/octave: the slope of the
    /// power density (the 1/3-octave regression slope minus 3.01).
    /// Pink noise -3.0, white 0; more negative is darker / warmer.
    /// Commercial pop averages about -4.5 to -5.
    pub tilt_db_per_oct: Option<f64>,
    /// Power-weighted mean frequency, Hz. Falls as a mix gets warmer.
    pub centroid_hz: Option<f64>,
    /// Energy 150-500 Hz over energy 2-5 kHz, dB. Rises with warmth (or
    /// mud), falls with harshness.
    pub lowmid_presence_db: Option<f64>,
    /// Spectral crest inside 2-5 kHz at 1/6-octave resolution (loudest
    /// band over the mean), dB. 0 is perfectly even; high means a
    /// presence resonance, the usual cause of harshness.
    pub presence_peakiness_db: Option<f64>,
    /// Energy 8-16 kHz over the whole 20 Hz-20 kHz energy, dB (always
    /// negative).
    pub air_ratio_db: Option<f64>,
    /// Up to 5 narrow resonances, strongest first: 1/6-octave bands at
    /// least 1 dB above the smoothed spectrum around them. Empty when
    /// nothing stands out.
    pub peaks: Vec<SpectralPeak>,
}

/// One of the eight [`StereoDetail::bands`].
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct StereoBand {
    /// Lower band edge, Hz.
    pub lo_hz: f64,
    /// Upper band edge, Hz.
    pub hi_hz: f64,
    /// L/R correlation inside the band, -1..+1. `null` when the band is
    /// empty (under -70 dB of the signal) or one-sided (one channel 40 dB
    /// or more below the other), where it would be 0/0.
    pub correlation: Option<f64>,
    /// Side over mid power inside the band, dB, clamped to +-60: -60 is
    /// mono, 0 hard-panned or uncorrelated, +60 anti-phase. With equal
    /// L/R energy, `correlation = (1 - rho)/(1 + rho)` where `rho =
    /// 10^(side_mid_db/10)`. `null` for an empty band.
    pub side_mid_db: Option<f64>,
    /// Level the band loses folded to mono, dB (negative is a loss): 0
    /// for mono, about -3 for uncorrelated or hard-panned content, -60
    /// (the floor) for anti-phase. `null` for an empty band.
    pub mono_loss_db: Option<f64>,
}

/// The 400 ms windowed correlation in [`StereoDetail`].
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct CorrelationWindows {
    /// Windows counted; silent and one-sided windows are skipped.
    pub windows: u32,
    /// Percentage of counted windows with correlation below +0.3.
    pub pct_below_0_3: f64,
    /// Lowest window correlation.
    pub worst: f64,
    /// Start of that window, seconds from the start of the measured range.
    pub worst_at_seconds: f64,
}

/// The `stereo` detail: width and mono-safety proxies.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct StereoDetail {
    /// Eight bands with edges 20, 60, 150, 400, 1k, 2.5k, 5k, 10k, 20k Hz.
    pub bands: Vec<StereoBand>,
    /// Windowed correlation summary. `null` when no window could be
    /// counted (silence, or a one-sided signal).
    pub correlation_windows: Option<CorrelationWindows>,
    /// Left over right energy, dB, clamped to +-60; positive leans left,
    /// +-60 means one channel is silent. `null` for silence.
    pub balance_db: Option<f64>,
    /// One channel is (nearly) silent: the other is at least 40 dB louder,
    /// i.e. hard-panned mono. Every 0/0 correlation is then `null` — read
    /// neither the top-level `correlation` nor a band's as a width
    /// figure; read `balance_db` for which side.
    pub one_sided: bool,
    /// A static delay between the channels (Haas), ms: the lag of the
    /// strongest normalized L/R cross-correlation peak between 1 and 35
    /// ms, when it exceeds 0.5 and beats the zero-lag correlation.
    /// Positive means the RIGHT channel is late. A static delay of `d` ms
    /// combs in mono with nulls at `(2k+1) * 1000/(2d)` Hz. `null` when
    /// there is none.
    pub haas_lag_ms: Option<f64>,
}

/// One of a track's sends in [`DepthDetail::sends`].
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct DepthSendInfo {
    /// The return bus it feeds.
    pub bus_id: TrackId,
    /// The send's level, dB.
    pub send_level_db: f64,
    /// Tapped before the track's fader.
    pub pre_fader: bool,
    /// The return's measured gain in this pass, dB: its output energy
    /// over the energy its sends feed it, so it includes the return's
    /// chain and fader. `null` when the return came out silent.
    pub return_gain_db: Option<f64>,
}

/// A track's place front-to-back, relative to the other tracks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(rename_all = "lowercase")]
pub enum LayerHint {
    /// The driest third.
    Front,
    /// The middle third.
    Middle,
    /// The wettest third.
    Back,
}

/// The `depth` detail (warmth-width-depth.md §7.6, decision D5).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct DepthDetail {
    /// Energy 6-16 kHz over 1-4 kHz, dB. Falls from front to back.
    pub hf_tilt_db: Option<f64>,
    /// Direct-to-reverberant ESTIMATE, dB — not a measurement: `-(send
    /// level + return gain)` per send, summed in power over the track's
    /// sends, a pre-fader send adding the track's fader. Only the track's
    /// OWN sends count (a send from a bus it feeds does not). Send levels
    /// are their current static values. `null` for a master or bus
    /// target, for a dry-only track, and when every return it feeds is
    /// silent. Read it for ORDERING: rough targets front >= +10, middle
    /// +3..+8, back <= 0.
    pub drr_db_estimate: Option<f64>,
    /// A track with no enabled sends: no reverberant path. Its
    /// `drr_db_estimate` is `null`, and it ranks as the driest (front).
    pub dry_only: bool,
    /// Front / middle / back from `drr_db_estimate` tertiles across the
    /// tracks of one `meter.stems` pass (dry-only tracks rank front).
    /// `null` on `meter.measure`, which has nothing to rank against, and
    /// on the master and bus entries.
    pub layer_hint: Option<LayerHint>,
    /// The track's enabled sends, with each return's measured gain.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sends: Vec<DepthSendInfo>,
}

/// The `dynamics` detail.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct DynamicsDetail {
    /// Peak-to-loudness ratio: `true_peak_db - lufs_integrated`, dB.
    /// `null` when `lufs_integrated` is.
    pub plr_db: Option<f64>,
    /// Peak-to-short-term ratio: `true_peak_db - lufs_short_max`, dB.
    /// `null` when `lufs_short_max` is.
    pub psr_db: Option<f64>,
}

/// Everything one measurement pass reports about one slice of the mix.
///
/// ## Why so many fields are nullable
///
/// A `null` here always means "this number does not exist for this
/// measurement", never "zero". Two things produce one:
///
/// 1. **Silence, or a range shorter than the meter's window.** Gated
///    loudness is undefined below the gate, and a 3 s short-term window
///    cannot be filled by a 1 s range. Those come back as `null` rather
///    than as the `-inf` the meters actually carry, which JSON cannot
///    represent.
/// 2. **`source: "live"`.** The live master meter is a streaming tap
///    with no access to a whole buffer, so `sample_peak_db`,
///    `clipped_samples`, `mono_penalty_db`, `bands` and
///    `measured_seconds` are all `null` on that path; so are
///    `crest_db` and `correlation`, which that tap simply does not
///    run a meter for; and so are `lufs_short_max` /
///    `lufs_momentary_max`, because the tap keeps no history. Its
///    *current* readings are reported separately as
///    [`lufs_short_term_now`](Self::lufs_short_term_now) /
///    [`lufs_momentary_now`](Self::lufs_momentary_now), which are
///    present only on the live path. Filling any of these with zeros
///    would read as a real measurement and silently corrupt a balance
///    decision — and `0.0` is especially dangerous for `crest_db` and
///    `correlation`, because it sits *inside* each one's plausible
///    range and cannot be told from a reading.
///
/// ## What a "live" number describes
///
/// The live tap is never reset per measurement, so the three figures
/// that do survive that path — [`lufs_integrated`](Self::lufs_integrated),
/// [`lra`](Self::lra) and [`true_peak_db`](Self::true_peak_db) — are
/// **session-cumulative**: they describe everything played since the
/// engine started, not a window anyone asked for. `source: "live"`
/// answers "how has this session been going"; only `source: "render"`
/// measures a *range*.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct MeasureResult {
    /// The slice measured, echoed back so a result stands alone.
    pub target: MeasureTarget,
    /// Which path produced these numbers. Read it before trusting a
    /// field — see the nullability rules above.
    pub source: MeasureSource,

    /// Gated integrated loudness over the range, LUFS. The single most
    /// useful number for balance. `null` for silence or for a range
    /// shorter than one 400 ms gating block. On `source: "live"` it is
    /// SESSION-CUMULATIVE, not a figure for a range: the gated loudness
    /// of everything played since the engine started, which answers
    /// "how loud has this session been", never "how loud is this
    /// passage".
    pub lufs_integrated: Option<f32>,
    /// Loudest 3 s short-term window in the range, LUFS. `null` when the
    /// range is shorter than 3 s.
    pub lufs_short_max: Option<f32>,
    /// Loudest 400 ms momentary window in the range, LUFS. `null` when
    /// the range is shorter than 400 ms.
    pub lufs_momentary_max: Option<f32>,
    /// The live meter's CURRENT 3 s short-term reading, LUFS. Present
    /// only when `source` is `"live"`; it is an instantaneous value, not
    /// a maximum.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lufs_short_term_now: Option<f32>,
    /// The live meter's CURRENT 400 ms momentary reading, LUFS. Present
    /// only when `source` is `"live"`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lufs_momentary_now: Option<f32>,
    /// EBU R128 loudness range, LU — how much the loudness moves across
    /// the range. `0.0` when there are too few short-term windows to
    /// form a range. On `source: "live"` it is SESSION-CUMULATIVE, not
    /// a figure for a range: how much the loudness has moved since the
    /// engine started.
    pub lra: f32,

    /// Maximum inter-sample (true) peak, dBTP, 4x oversampled. This is
    /// the number a lossy encoder cares about. On `source: "live"` it
    /// is SESSION-CUMULATIVE, not a figure for a range: the highest
    /// peak since the engine started, which answers "did this ever go
    /// over while I listened", never "how hot is this passage".
    pub true_peak_db: f32,
    /// Maximum absolute sample value, dBFS. Always at or below
    /// `true_peak_db`.
    pub sample_peak_db: Option<f32>,
    /// Peak-to-RMS ratio over the whole range, dB — how much dynamic
    /// movement survives. `0.0` for silence. `null` on `source:
    /// "live"`: the streaming tap runs no crest meter at all, so there
    /// is no number to report.
    pub crest_db: Option<f32>,
    /// Channel samples at or beyond digital full scale (`|x| >= 1.0`),
    /// counted per channel sample. Anything above 0 on the master means
    /// audible clipping.
    pub clipped_samples: Option<u64>,
    /// Pearson correlation of L against R, in `[-1, 1]`. `+1` is
    /// mono-identical, `0` uncorrelated, negative means anti-phase
    /// content that a mono listener loses. `null` on `source: "live"`:
    /// the streaming tap runs no correlation meter at all, and `0`
    /// there would read as "perfectly wide".
    pub correlation: Option<f32>,
    /// Loudness lost when the range is folded to mono, dB. Negative
    /// means level disappears in mono; near `0` is mono-safe.
    pub mono_penalty_db: Option<f32>,
    /// Energy shares across the four AES tonal bands.
    pub bands: Option<Bands>,
    /// Length of the measured range in seconds — the same value for
    /// every entry of one measurement pass, which is what makes the
    /// numbers comparable.
    pub measured_seconds: Option<f64>,

    /// Tracks that were SOLOED when this was measured, present only on a
    /// `master` target (a track or bus measurement renders its own audio
    /// regardless of solo, so solo cannot skew it).
    ///
    /// A non-empty list means the master here is **not the mix** — it is
    /// only these tracks, exactly as `render.mixdown` would write it.
    /// Every per-track number stays correct while the master silently
    /// becomes a solo bounce, which is how a "mastered export" came out
    /// containing nothing but the lead synth and looked plausible in
    /// every per-track metric (ba doc #275 P1.6).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub soloed_track_ids: Vec<TrackId>,

    /// The `spectrum` detail, present only when `detail` asked for it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spectrum: Option<SpectrumDetail>,
    /// The `stereo` detail, present only when `detail` asked for it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stereo: Option<StereoDetail>,
    /// The `dynamics` detail, present only when `detail` asked for it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dynamics: Option<DynamicsDetail>,
    /// The `depth` detail, present only when `detail` asked for it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub depth: Option<DepthDetail>,
}

// ---------------------------------------------------------------------------
// meter.stems
// ---------------------------------------------------------------------------

/// Params for `meter.stems`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct StemsParams {
    /// Defaults to the whole song. Every entry is measured over this ONE
    /// range, which is what makes the numbers directly comparable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub range: Option<RangeSpec>,
    /// Also measure each group / return bus (default `false`). A bus and
    /// the tracks feeding it BOTH appear then, describing the same audio
    /// at two stages — the bus is its members summed through the bus FX
    /// chain, so the entries overlap and must never be added together.
    #[serde(default)]
    pub include_busses: bool,
    /// Opt-in detail blocks, computed for the master and every entry —
    /// see [`MeasureParams::detail`].
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub detail: Vec<MeasureDetail>,
}

/// One line of a [`StemsResult`] — a track (or bus) and its numbers.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct TrackMeasurement {
    /// The track or bus id, as `song.summary` reports it.
    pub track_id: TrackId,
    /// Its name, so a report reads without a second lookup.
    pub name: String,
    /// Sub-tracks folded into this entry — the extra output ports of a
    /// multi-output instrument, whose audio is INCLUDED in these
    /// numbers. Empty for an ordinary track.
    ///
    /// Those ids get no line of their own (see [`StemsResult::tracks`]),
    /// so this is what tells a client where they went.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub includes_track_ids: Vec<TrackId>,
    /// The measurement itself, flattened into this object: every field
    /// of [`MeasureResult`] appears alongside `track_id` and `name`.
    #[serde(flatten)]
    pub measurement: MeasureResult,
}

/// Job payload once a `meter.stems` job completes: the whole mix plus
/// every track, all measured over one shared range in one pass — one
/// command and one set of results, inside which the engine renders each
/// target in turn, so a pass over a large project takes proportionally
/// longer than a single `meter.measure`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct StemsResult {
    /// The full mix, identical to `meter.measure` with the default
    /// target — the reference every track entry is read against.
    pub master: MeasureResult,
    /// One entry per TOP-LEVEL track, in mixer order, plus the busses
    /// when `include_busses` was set.
    ///
    /// A sub-track never gets its own entry. A multi-output instrument's
    /// extra output ports carry no material of their own — their audio
    /// is produced while the parent renders — so they are measured as
    /// part of the parent and listed in its
    /// [`includes_track_ids`](TrackMeasurement::includes_track_ids).
    /// That is the whole point of measuring this way: the mute-and-bounce
    /// approach it replaces left a whole drum kit bleeding into every
    /// other stem and produced a plausible, completely wrong balance
    /// table.
    pub tracks: Vec<TrackMeasurement>,
}

// ---------------------------------------------------------------------------
// meter.snapshot / meter.compare (warmth-width-depth.md §7.2)
// ---------------------------------------------------------------------------

/// Params for `meter.snapshot`: what to measure and keep.
///
/// Snapshots live in the running app's memory for the session: they are
/// never saved with the project, do not survive a restart, and the
/// oldest-used ones are evicted past [`SNAPSHOT_CAPACITY`]. They do
/// survive opening another project, so a snapshot of one song can be
/// compared against another.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct SnapshotParams {
    /// Defaults to `"master"`.
    #[serde(default)]
    pub target: MeasureTarget,
    /// Defaults to the whole song. The resolved sample range is stored
    /// with the snapshot, and a later `"current"` side of a compare
    /// renders exactly that range again.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub range: Option<RangeSpec>,
    /// Detail blocks to keep. Defaults to ALL of them (`spectrum`,
    /// `stereo`, `dynamics`), since a snapshot exists to be compared and
    /// a detail it lacks has no delta.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<Vec<MeasureDetail>>,
}

/// How many snapshots the app keeps; past it the least recently used
/// one is evicted.
pub const SNAPSHOT_CAPACITY: usize = 32;

/// Job payload once a `meter.snapshot` job completes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct SnapshotResult {
    /// Pass this as `a` or `b` of `meter.compare`.
    pub snapshot_id: u64,
    /// The stored measurement, as `meter.measure` would report it.
    pub measurement: MeasureResult,
}

/// Marker for the `"current"` side of a compare.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(rename_all = "lowercase")]
pub enum Current {
    /// Render the project as it is now.
    Current,
}

/// One side of a `meter.compare`: `"current"` (render the project now)
/// or a `snapshot_id` from `meter.snapshot`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(untagged)]
pub enum CompareSide {
    /// A stored snapshot.
    Snapshot(u64),
    /// The project as it is now.
    Current(Current),
}

impl CompareSide {
    /// The `"current"` side.
    pub const CURRENT: Self = CompareSide::Current(Current::Current);
}

impl Default for CompareSide {
    fn default() -> Self {
        Self::CURRENT
    }
}

/// How `meter.compare` levels B against A.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(rename_all = "lowercase")]
pub enum MatchMode {
    /// Gain-match B to A's integrated loudness first (the default): every
    /// delta then describes a change in *character*, not in level.
    #[default]
    Lufs,
    /// Compare as measured.
    None,
}

/// Params for `meter.compare`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct CompareParams {
    /// Only needed when both sides are `"current"`; a snapshot side
    /// carries its own target, and a `"current"` side renders that same
    /// target. Given together with a snapshot, it must match it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<MeasureTarget>,
    /// Same rule as `target`: a snapshot side fixes the range, and a
    /// `"current"` side re-renders exactly that sample range.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub range: Option<RangeSpec>,
    /// The reference: usually a `snapshot_id` taken before a change.
    pub a: CompareSide,
    /// The candidate. Defaults to `"current"`.
    #[serde(default)]
    pub b: CompareSide,
    /// Defaults to `"lufs"`.
    #[serde(rename = "match", default)]
    pub match_mode: MatchMode,
}

/// One side of a [`CompareResult`], echoed with its loudness.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct CompareSideInfo {
    /// What was compared.
    pub side: CompareSide,
    /// Its integrated loudness as measured (before any match gain).
    pub lufs_integrated: Option<f64>,
}

/// Deltas of the four [`Bands`] shares.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct BandsDelta {
    /// Change in the 20-250 Hz share.
    pub low: f64,
    /// Change in the 250 Hz-2 kHz share.
    pub mid: f64,
    /// Change in the 2-8 kHz share.
    pub high: f64,
    /// Change in the 8-20 kHz share.
    pub air: f64,
}

/// Deltas of the [`SpectrumDetail`] figures.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct SpectrumDelta {
    /// Per 1/3-octave band, dB (centres in [`THIRD_OCTAVE_HZ`]); `null`
    /// where either side has no energy in the band.
    pub third_octave: Vec<Option<f64>>,
    /// Change in spectral tilt, dB/oct. Negative is warmer.
    pub tilt_db_per_oct: Option<f64>,
    /// Change in centroid, Hz.
    pub centroid_hz: Option<f64>,
    /// Change in centroid, percent of A's.
    pub centroid_pct: Option<f64>,
    /// Change in the low-mid / presence ratio, dB.
    pub lowmid_presence_db: Option<f64>,
    /// Change in presence peakiness, dB. Negative is less harsh.
    pub presence_peakiness_db: Option<f64>,
    /// Change in the air ratio, dB.
    pub air_ratio_db: Option<f64>,
}

/// Deltas of one [`StereoBand`].
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct StereoBandDelta {
    /// Lower band edge, Hz.
    pub lo_hz: f64,
    /// Upper band edge, Hz.
    pub hi_hz: f64,
    /// Change in the band's correlation.
    pub correlation: Option<f64>,
    /// Change in the band's side/mid ratio, dB. Positive is wider.
    pub side_mid_db: Option<f64>,
    /// Change in the band's mono loss, dB. Negative is less mono-safe.
    pub mono_loss_db: Option<f64>,
}

/// Deltas of the [`StereoDetail`] figures.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct StereoDelta {
    /// Per band, same edges as [`StereoDetail::bands`].
    pub bands: Vec<StereoBandDelta>,
    /// Change in balance, dB.
    pub balance_db: Option<f64>,
    /// Change in the percentage of 400 ms windows below +0.3.
    pub pct_below_0_3: Option<f64>,
    /// Change in the worst window's correlation.
    pub worst_window_correlation: Option<f64>,
}

/// Deltas of the [`DynamicsDetail`] figures.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct DynamicsDelta {
    /// Change in PLR, dB.
    pub plr_db: Option<f64>,
    /// Change in PSR, dB.
    pub psr_db: Option<f64>,
}

/// `B − A` for every proxy, B taken at the match gain.
///
/// A delta is `null` when either side lacks the number (silence, a
/// window the range cannot fill, a detail the snapshot did not keep).
/// Level figures (the LUFS fields, `true_peak_db`, `sample_peak_db`,
/// `third_octave`) move with the match gain; shape figures (crest, LRA,
/// correlation, `bands`, tilt and the other ratios, PLR/PSR, the whole
/// stereo block) do not, since a pure gain cannot change them.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct CompareDeltas {
    /// ~0 by construction when matched.
    pub lufs_integrated: Option<f64>,
    /// Loudest 3 s window, LU.
    pub lufs_short_max: Option<f64>,
    /// Loudest 400 ms window, LU.
    pub lufs_momentary_max: Option<f64>,
    /// Loudness range, LU.
    pub lra: Option<f64>,
    /// True peak, dB.
    pub true_peak_db: Option<f64>,
    /// Sample peak, dB.
    pub sample_peak_db: Option<f64>,
    /// Crest factor, dB. Negative is denser.
    pub crest_db: Option<f64>,
    /// Change in clipped samples AS MEASURED — a clip count cannot be
    /// re-derived at another gain, so this one is never matched.
    pub clipped_samples: Option<i64>,
    /// Whole-range correlation.
    pub correlation: Option<f64>,
    /// Mono penalty, dB.
    pub mono_penalty_db: Option<f64>,
    /// Energy-share deltas of the four tonal bands.
    pub bands: Option<BandsDelta>,
    /// Present when both sides carry the `spectrum` detail.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spectrum: Option<SpectrumDelta>,
    /// Present when both sides carry the `stereo` detail.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stereo: Option<StereoDelta>,
    /// Present when both sides carry the `dynamics` detail.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dynamics: Option<DynamicsDelta>,
}

/// Job payload once a `meter.compare` job completes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct CompareResult {
    /// The slice both sides measure.
    pub target: MeasureTarget,
    /// Length of the compared range, seconds.
    pub measured_seconds: Option<f64>,
    /// The reference side.
    pub a: CompareSideInfo,
    /// The candidate side.
    pub b: CompareSideInfo,
    /// The match mode asked for.
    #[serde(rename = "match")]
    pub match_mode: MatchMode,
    /// Whether the match gain was applied. `false` with `match: "none"`,
    /// or when either side has no integrated loudness (silence), in which
    /// case the deltas are as measured.
    pub matched: bool,
    /// Gain applied to B, dB: `a.lufs_integrated − b.lufs_integrated`
    /// when matched, else 0. A +3 dB louder B reads about -3 here.
    pub match_gain_db: f64,
    /// `B − A` for every proxy.
    pub deltas: CompareDeltas,
}

// ---------------------------------------------------------------------------
// meter.probe (warmth-width-depth.md §7.3)
// ---------------------------------------------------------------------------

fn default_probe_freq_hz() -> f64 {
    1_000.0
}

fn default_probe_level_dbfs() -> f64 {
    -12.0
}

/// Params for `meter.probe`: which insert chain, and the stimulus.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct ProbeParams {
    /// Whose insert chain: `"master"` (default), `{track_id}` or
    /// `{bus_id}`. A track's instrument is not part of it.
    #[serde(default)]
    pub target: MeasureTarget,
    /// Probe tone, Hz (default 1000), snapped to the analysis grid
    /// (0.73 Hz at 48 kHz; the result echoes the exact value). Use e.g.
    /// 5000 to expose aliasing: harmonics past Nyquist fold back.
    #[serde(default = "default_probe_freq_hz")]
    pub freq_hz: f64,
    /// Peak level of the tone, dBFS (default -12), -80..0. Distortion
    /// depends on it, so probe at the level the chain really sees.
    #[serde(default = "default_probe_level_dbfs")]
    pub level_dbfs: f64,
    /// Also run the SMPTE pair (60 Hz + 7 kHz, 4:1, summed peak at
    /// `level_dbfs`) and report `imd_pct` (default false).
    #[serde(default)]
    pub imd: bool,
}

impl Default for ProbeParams {
    fn default() -> Self {
        Self {
            target: MeasureTarget::Master,
            freq_hz: default_probe_freq_hz(),
            level_dbfs: default_probe_level_dbfs(),
            imd: false,
        }
    }
}

/// One stage the probe ran through.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct ProbeStageInfo {
    /// The plugin's CLAP id, as `*.plugin_params` reports it.
    pub plugin_id: String,
    /// Its index among same-id plugins on the chain.
    pub occurrence: u32,
    /// Its display name.
    pub name: String,
    /// Whether the live plugin's current state was copied into the probe's
    /// clone. `false` means the plugin has no state extension and was
    /// probed at its defaults.
    pub state_copied: bool,
}

/// A slot the probe left out, and why.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct ProbeSkipped {
    /// The plugin's CLAP id.
    pub plugin_id: String,
    /// Its index among same-id plugins on the chain.
    pub occurrence: u32,
    /// `"bypassed"`, `"chain bypassed"` or `"missing"`.
    pub reason: String,
}

/// Job payload once a `meter.probe` job completes: the chain's harmonic
/// signature at the probed frequency and level.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct ProbeResult {
    /// The chain probed.
    pub target: MeasureTarget,
    /// The exact probe frequency, Hz.
    pub freq_hz: f64,
    /// The tone's input peak level, dBFS.
    pub level_dbfs: f64,
    /// The stages the tone went through, in order. Empty means the chain
    /// was a straight wire.
    pub stages: Vec<ProbeStageInfo>,
    /// Slots on the chain that were not probed.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub skipped: Vec<ProbeSkipped>,
    /// Chain gain at the probe frequency: output fundamental minus input
    /// level, dB.
    pub gain_db: f64,
    /// Total harmonic distortion over H2..H9 (in-band ones), %. Targets:
    /// master 0.1-1, bus 0.5-3, single track 3-10.
    pub thd_pct: f64,
    /// H2..H9 in dBc (`h[0]` is H2), floored at -160; `null` for a
    /// harmonic above Nyquist (it aliases instead).
    pub h: Vec<Option<f64>>,
    /// H2 minus H3, dB. Positive is even-dominant (the "warm" signature).
    pub h2_h3_db: Option<f64>,
    /// How fast the series falls, dB per order (positive = falling);
    /// fitted over harmonics above -140 dBc. Aim for 6 or more.
    pub decay_db_per_order: Option<f64>,
    /// Strongest non-harmonic, non-DC bin, dBc: aliasing plus any noise
    /// or inharmonic product. Aim for -90 or lower.
    pub aliasing_floor_dbc: f64,
    /// SMPTE intermodulation, %, when `imd` was asked for.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub imd_pct: Option<f64>,
    /// Summed latency of the probed stages, samples.
    pub latency_samples: u32,
}
