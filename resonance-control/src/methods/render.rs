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
pub const STEMS: &str = "render.stems";

/// All `render.*` method names.
pub const METHODS: &[&str] = &[MIXDOWN, STEMS];

/// A render time range; omitted ends default to song start/end.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct RangeSpec {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start: Option<PositionSpec>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end: Option<PositionSpec>,
}

/// Params for `render.mixdown`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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
pub struct MixdownResult {
    /// Absolute path of the written WAV file.
    pub path: String,
    /// Rendered audio length in seconds.
    pub duration_s: f64,
    /// Sample rate of the written WAV file.
    pub sample_rate: u32,
}

/// Params for `render.stems`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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
pub struct StemsResult {
    /// Absolute paths of the written stem files.
    pub paths: Vec<String>,
    /// Rendered audio length in seconds.
    pub duration_s: f64,
}
