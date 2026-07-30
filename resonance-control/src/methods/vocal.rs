//! `vocal.*` — lyrics, pronunciation, and SVS rendering.
//!
//! # One lane per (section, track)
//!
//! A vocal lane must first be installed with `section.set_lane_generator`
//! (`kind = vocal`); nothing else creates one. Lyrics and the SVS voice
//! live per **(section definition, track)** vocal lane. The lane-addressed
//! mutations here take an optional `section_id` to pick one; omitting it
//! resolves the track's **first** vocal lane in placement order.
//! `song.vocal` lists a track's lanes with their ids, so a client can tell
//! which lane a write will hit.

use crate::ids::{ClipId, SectionDefinitionId, TrackId};
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
/// `vocal.generate` — generate a vocal lane's melody (and lyrics) into
/// its derived clip ([`GenerateParams`] -> [`GenerateResult`]).
pub const GENERATE: &str = "vocal.generate";

/// All `vocal.*` method names.
pub const METHODS: &[&str] = &[
    SET_LYRICS,
    SET_LINE,
    SET_PRONUNCIATION,
    CLEAR_PRONUNCIATION,
    RENDER,
    GENERATE,
];

/// Params for `vocal.set_lyrics`: bulk text, one line per lyric line.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct SetLyricsParams {
    pub track_id: TrackId,
    /// Which of the track's vocal lanes to write (`definition_id` from
    /// `song.vocal`'s `lanes`). Omitted resolves the track's first
    /// vocal lane in placement order.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub section_id: Option<SectionDefinitionId>,
    pub text: String,
}

/// Params for `vocal.set_line`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct SetLineParams {
    pub track_id: TrackId,
    /// Which of the track's vocal lanes to write (`definition_id` from
    /// `song.vocal`'s `lanes`). Omitted resolves the track's first
    /// vocal lane in placement order.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub section_id: Option<SectionDefinitionId>,
    /// 0-based line index within the resolved lane (see `song.vocal`).
    pub line_index: usize,
    pub text: String,
}

/// Params for `vocal.set_pronunciation`. Pronunciation overrides are
/// project-wide, not per lane, so this takes no `section_id`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct SetPronunciationParams {
    /// The word being overridden (case-insensitive).
    pub word: String,
    /// Lowercase phonemes, e.g. `["l", "ih", "l", "iy", "ah"]`.
    pub phonemes: Vec<String>,
}

/// Params for `vocal.clear_pronunciation`. Project-wide, like
/// [`SetPronunciationParams`] — no `section_id`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct ClearPronunciationParams {
    pub word: String,
}

/// Params for `vocal.render`. A render covers **one lane**: omitting
/// `track_id` resolves the project's first vocal track and omitting
/// `section_id` that track's first lane, so render each lane you want
/// updated. The job's payload is [`RenderJobResult`].
///
/// Render synthesises the notes already in the lane's clip; it never
/// generates or rewrites them (`vocal.generate` does that).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct RenderParams {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub track_id: Option<TrackId>,
    /// Which of the track's vocal lanes to render. Omitted resolves the
    /// track's first vocal lane in placement order — the same rule the
    /// rest of the namespace uses. Without this a track that sings in
    /// several sections could only ever re-render its first lane, so
    /// every other lane stayed frozen at its first render.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub section_id: Option<SectionDefinitionId>,
    /// Voicebank name. Omitted keeps the lane's current voicebank
    /// (falling back to the app default, Lilia, for a lane that has
    /// never chosen one) — so a plain re-render never resets it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub voicebank: Option<String>,
}

/// Params for `vocal.generate`.
///
/// `generate.part` refuses vocal tracks (drums have their own method,
/// vocals their own namespace), and vocal rendering reads its notes from
/// the lane's **derived clip** — so without this method a vocal lane had
/// no notes and `vocal.render` had nothing to sing (doc #269 FR-2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct GenerateParams {
    pub track_id: TrackId,
    /// Which of the track's vocal lanes to generate. Omitted resolves
    /// the track's first vocal lane in placement order.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub section_id: Option<SectionDefinitionId>,
    /// Explicit RNG seed for a reproducible result; omitted advances the
    /// lane's seed, so repeated calls give different material.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seed: Option<u64>,
    /// Also roll a fresh lyric draft from the lane's theme brief.
    /// Defaults to `true`, matching the GUI's "generate" button. Pass
    /// `false` to generate the **melody only** and leave lyrics you
    /// wrote with `vocal.set_lyrics` untouched — generation writes both
    /// by default, so doing it the other way round silently discards
    /// them.
    #[serde(default = "default_lyrics")]
    pub lyrics: bool,
}

fn default_lyrics() -> bool {
    true
}

/// Result of `vocal.generate`: the lane's derived MIDI clip, ready for
/// `song.notes` / `notes.*` edits and then `vocal.render`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct GenerateResult {
    /// The derived clip at the lane's first placement. A lane derives
    /// one clip per placement of its section, all carrying the same
    /// material.
    pub clip_id: ClipId,
    pub revision: u64,
}

/// Job payload once a `vocal.render` job completes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct RenderJobResult {
    /// Tracks that were (re)rendered.
    pub track_ids: Vec<TrackId>,
    pub revision: u64,
}
