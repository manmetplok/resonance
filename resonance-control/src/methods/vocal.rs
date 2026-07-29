//! `vocal.*` — lyrics, pronunciation, and SVS rendering.

use crate::ids::TrackId;
use serde::{Deserialize, Serialize};

/// `vocal.set_lyrics` — replace a vocal track's full lyric text
/// ([`SetLyricsParams`] -> `MutationAck`).
pub const SET_LYRICS: &str = "vocal.set_lyrics";
/// `vocal.set_line` — replace one lyric line
/// ([`SetLineParams`] -> `MutationAck`).
pub const SET_LINE: &str = "vocal.set_line";
/// `vocal.set_pronunciation` — set a per-word pronunciation override
/// ([`SetPronunciationParams`] -> `MutationAck`).
pub const SET_PRONUNCIATION: &str = "vocal.set_pronunciation";
/// `vocal.clear_pronunciation` — remove a per-word override
/// ([`ClearPronunciationParams`] -> `MutationAck`).
pub const CLEAR_PRONUNCIATION: &str = "vocal.clear_pronunciation";
/// `vocal.render` — kick off an SVS render
/// ([`RenderParams`] -> [`crate::job::JobStarted`]).
pub const RENDER: &str = "vocal.render";

/// All `vocal.*` method names.
pub const METHODS: &[&str] = &[
    SET_LYRICS,
    SET_LINE,
    SET_PRONUNCIATION,
    CLEAR_PRONUNCIATION,
    RENDER,
];

/// Params for `vocal.set_lyrics`: bulk text, one line per lyric line.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SetLyricsParams {
    pub track_id: TrackId,
    pub text: String,
}

/// Params for `vocal.set_line`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SetLineParams {
    pub track_id: TrackId,
    /// 0-based line index (see `song.vocal`).
    pub line_index: usize,
    pub text: String,
}

/// Params for `vocal.set_pronunciation`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SetPronunciationParams {
    /// The word being overridden (case-insensitive).
    pub word: String,
    /// Lowercase phonemes, e.g. `["l", "ih", "l", "iy", "ah"]`.
    pub phonemes: Vec<String>,
}

/// Params for `vocal.clear_pronunciation`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClearPronunciationParams {
    pub word: String,
}

/// Params for `vocal.render`. Omit `track_id` to render every vocal
/// track; the job's payload is [`RenderJobResult`].
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RenderParams {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub track_id: Option<TrackId>,
    /// Voicebank name; defaults to the app default (Lilia).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub voicebank: Option<String>,
}

/// Job payload once a `vocal.render` job completes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RenderJobResult {
    /// Tracks that were (re)rendered.
    pub track_ids: Vec<TrackId>,
    pub revision: u64,
}
