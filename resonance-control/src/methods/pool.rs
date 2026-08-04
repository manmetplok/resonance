//! `pool.*` — the project's media pool: the imported audio files a
//! sample can be placed from.
//!
//! An audio file is not usable straight off disk. The app imports it
//! first — decoding, up/down-mixing to stereo, resampling to the project
//! rate, and copying the result into `{project_dir}/audio/` under a
//! stable name — and the result is a *pool asset* with an id. Placing a
//! sample on the timeline is then [`crate::methods::clip::PLACE`] against
//! that asset.
//!
//! `clip.place` also accepts a bare `path` and imports it on the way, so
//! a client that just wants a file on a track never has to touch this
//! namespace. `pool.list` matters when reusing one sample many times
//! (import once, place N times) and for seeing what a project already
//! carries.

use crate::ids::AssetId;
use serde::{Deserialize, Serialize};

/// `pool.list` — the project's imported audio assets ([`PoolView`]).
/// Read-only.
pub const LIST: &str = "pool.list";
/// `pool.import` — import audio files into the pool without placing them
/// ([`ImportParams`] -> a job whose result is [`ImportResult`]).
pub const IMPORT: &str = "pool.import";

/// All `pool.*` method names.
pub const METHODS: &[&str] = &[LIST, IMPORT];

/// Largest number of files one `pool.import` accepts. Refused above this
/// rather than truncated — a partial import is worse than a rejected one.
pub const MAX_IMPORT_FILES: usize = 64;

/// Result of `pool.list`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct PoolView {
    pub assets: Vec<PoolAssetView>,
    pub revision: u64,
}

/// One imported audio file in the project's media pool.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct PoolAssetView {
    pub id: AssetId,
    /// Absolute path of the source file that was imported. This is what
    /// `clip.place`'s `path` matches against to reuse an asset instead of
    /// importing the same file twice.
    pub original_path: String,
    /// Filename stem of the source, the name a placed clip gets.
    pub name: String,
    /// Length of the imported (project-rate) audio in frames — how long a
    /// clip placed from this asset is before any trim.
    pub duration_frames: u64,
    /// The same length in seconds, at the project rate.
    pub duration_seconds: f64,
    /// Channel count of the ORIGINAL source file (the pooled copy is
    /// always stereo at the project rate).
    pub channels: u16,
    /// Sample rate of the original source file, in Hz.
    pub source_sample_rate: u32,
    /// Container/codec family of the original source, lowercase
    /// (`"wav"`, `"flac"`, `"mp3"`, ...).
    pub format: String,
    /// How many clips in the project were placed from this asset.
    pub usage_count: u32,
    /// True when the backing file is gone from disk (e.g. the project was
    /// moved without its `audio/` folder). A missing asset is kept, not
    /// dropped, but placing from it produces a silent clip until it is
    /// relinked in the GUI.
    pub missing: bool,
}

/// Params for `pool.import`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct ImportParams {
    /// Absolute paths to audio files on the machine running the app, at
    /// most [`MAX_IMPORT_FILES`]. A file already in the pool (same
    /// `original_path`) is imported again as a second asset — check
    /// `pool.list` first if that is not what you want.
    pub paths: Vec<String>,
}

/// Job result of `pool.import` — the assets that landed, ready to pass to
/// `clip.place`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct ImportResult {
    pub assets: Vec<PoolAssetView>,
    pub revision: u64,
}
