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
