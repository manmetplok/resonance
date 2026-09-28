//! `render.*` — offline audio export to explicit paths.
//!
//! Both methods return [`crate::job::JobStarted`]; the job's terminal
//! `result` payload is [`MixdownResult`] / [`StemsResult`]. Overwriting
//! an existing file requires `"overwrite": true`, otherwise the server
//! answers `needs_confirmation`.

use crate::common::PositionSpec;
use serde::{Deserialize, Serialize};

/// `render.mixdown` — bounce the master mix to a WAV file
/// ([`MixdownParams`] -> job -> [`MixdownResult`]).
pub const MIXDOWN: &str = "render.mixdown";
/// `render.stems` — export per-track stems into a directory
/// ([`StemsParams`] -> job -> [`StemsResult`]).
///
/// **Not implemented on this build**, and deliberately NOT in
/// [`METHODS`] because of that: `control.hello`'s `capabilities` is a
/// contract, and a name listed there that always answers `unsupported`
/// makes the whole list untrustworthy — a client cannot then use it to
/// decide anything (ba doc #275 P4). The method name stays defined so
/// the handler can keep returning a specific `unsupported` message
/// rather than `method_not_found`, and so it lands back in `METHODS`
/// unchanged when the export plumbing is wired.
///
/// The engine half already exists (`AudioCommand::ExportStems`, with
/// per-target progress events); what is missing is the control-side job
/// plumbing.
pub const STEMS: &str = "render.stems";

/// All `render.*` method names. `STEMS` is excluded until it is
/// implemented — see its doc comment.
pub const METHODS: &[&str] = &[MIXDOWN];

/// A render time range; omitted ends default to song start/end.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct RangeSpec {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start: Option<PositionSpec>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end: Option<PositionSpec>,
}

/// Params for `render.mixdown`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct MixdownParams {
    /// Absolute path of the WAV file to write.
    pub path: String,
    /// Defaults to the whole song.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub range: Option<RangeSpec>,
    /// Required (`true`) to replace an existing file at `path`.
    #[serde(default)]
    pub overwrite: bool,
    /// Loudness-normalize the file: gain to `target_lufs` integrated,
    /// then a true-peak limiter at `ceiling_dbtp`. Omit for the plain
    /// mix. Mutually exclusive with `platform`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub normalize: Option<NormalizeParams>,
    /// Normalize to a delivery platform's published target — shorthand
    /// for `normalize` (see [`Platform::targets`]). Mutually exclusive
    /// with `normalize`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub platform: Option<Platform>,
}

/// Loudness normalization for `render.mixdown` (warmth-width-depth.md
/// §7.7).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct NormalizeParams {
    /// Integrated loudness to reach, LUFS, -40..-5.
    pub target_lufs: f64,
    /// True-peak ceiling of the post-gain limiter, dBTP, -12..0.
    pub ceiling_dbtp: f64,
}

/// A delivery platform with a published loudness target.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(rename_all = "lowercase")]
pub enum Platform {
    /// -14 LUFS, -1 dBTP.
    Spotify,
    /// -16 LUFS, -1 dBTP.
    Apple,
    /// -14 LUFS, -1 dBTP.
    Youtube,
    /// -14 LUFS, -1 dBTP.
    Tidal,
    /// -14 LUFS, -2 dBTP.
    Amazon,
    /// -15 LUFS, -1 dBTP.
    Deezer,
    /// A club master: -8 LUFS, -0.3 dBTP.
    Club,
}

impl Platform {
    /// `(target_lufs, ceiling_dbtp)`.
    ///
    /// Loudness targets are the platforms' published normalization
    /// levels (warmth-width-depth.md §3.4). Ceilings are -1 dBTP, the
    /// common lossy-codec headroom recommendation, except Amazon (-2,
    /// its own guidance) and club (-0.3: a loud master played from a
    /// lossless file, where the last dB of headroom buys nothing).
    pub fn targets(self) -> (f64, f64) {
        match self {
            Platform::Spotify | Platform::Youtube | Platform::Tidal => (-14.0, -1.0),
            Platform::Apple => (-16.0, -1.0),
            Platform::Amazon => (-14.0, -2.0),
            Platform::Deezer => (-15.0, -1.0),
            Platform::Club => (-8.0, -0.3),
        }
    }
}

/// What a normalized `render.mixdown` asked for and what it achieved.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct NormalizeReport {
    /// The platform shorthand used, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub platform: Option<Platform>,
    /// The loudness asked for, LUFS.
    pub target_lufs: f64,
    /// The true-peak ceiling asked for, dBTP.
    pub ceiling_dbtp: f64,
    /// Integrated loudness of the written file, re-measured after the
    /// limiter, LUFS. Can sit below the target when reaching it would
    /// need more limiting than the ceiling allows; `null` for silence.
    pub achieved_lufs: Option<f64>,
    /// True peak of the written file, dBTP.
    pub achieved_dbtp: f64,
}

/// Job payload once a `render.mixdown` job completes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct MixdownResult {
    /// Absolute path of the written WAV file.
    pub path: String,
    /// Rendered audio length in seconds.
    pub duration_s: f64,
    /// Sample rate of the written WAV file.
    pub sample_rate: u32,
    /// Tracks that were SOLOED for this bounce. A non-empty list means
    /// the file is **not the mix** — it contains only these tracks. The
    /// render honours solo (matching what you would hear), while every
    /// per-track meter reads the same either way, so nothing else in a
    /// report would reveal it (ba doc #275 P1.6).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub soloed_track_ids: Vec<crate::ids::TrackId>,
    /// Present when the mixdown was normalized: the target and what the
    /// file actually measures.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub normalize: Option<NormalizeReport>,
}

/// Params for `render.stems`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct StemsParams {
    /// Absolute path of the directory to write stem WAVs into.
    pub dir: String,
    /// Defaults to the whole song.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub range: Option<RangeSpec>,
    /// Required (`true`) to replace existing files in `dir`.
    #[serde(default)]
    pub overwrite: bool,
}

/// Job payload once a `render.stems` job completes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct StemsResult {
    /// Absolute paths of the written stem files.
    pub paths: Vec<String>,
    /// Rendered audio length in seconds.
    pub duration_s: f64,
}
