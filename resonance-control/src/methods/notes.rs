//! `notes.*` — piano-roll-level note editing so the AI can compose
//! directly, not only via generators.
//!
//! Notes are addressed by their `index` in the clip's note list as
//! reported by `song.notes` ([`crate::methods::song::NoteView::index`]);
//! re-read after edits that reorder notes.

use crate::ids::{ClipId, SectionPlacementId, TrackId};
use serde::{Deserialize, Serialize};

/// `notes.insert` — insert one note ([`InsertParams`] -> [`InsertResult`]).
pub const INSERT: &str = "notes.insert";
/// `notes.edit` — edit one note in place ([`EditParams`] -> `MutationAck`).
pub const EDIT: &str = "notes.edit";
/// `notes.delete` — delete one note ([`DeleteParams`] -> `MutationAck`).
pub const DELETE: &str = "notes.delete";
/// `notes.create_clip` — create an empty MIDI clip on a track
/// ([`CreateClipParams`] -> [`CreateClipResult`]).
pub const CREATE_CLIP: &str = "notes.create_clip";
/// `notes.move_clip` — reposition a MIDI clip on the timeline
/// ([`MoveClipParams`] -> `MutationAck`).
pub const MOVE_CLIP: &str = "notes.move_clip";
/// `notes.insert_many` — insert a batch of notes as ONE undoable edit
/// ([`InsertManyParams`] -> [`InsertManyResult`]).
pub const INSERT_MANY: &str = "notes.insert_many";
/// `notes.replace_all` — replace a clip's whole note list as ONE
/// undoable edit ([`ReplaceAllParams`] -> [`InsertManyResult`]).
pub const REPLACE_ALL: &str = "notes.replace_all";

/// All `notes.*` method names.
pub const METHODS: &[&str] = &[
    INSERT,
    EDIT,
    DELETE,
    CREATE_CLIP,
    MOVE_CLIP,
    INSERT_MANY,
    REPLACE_ALL,
];

fn default_velocity() -> u8 {
    100
}

/// Params for `notes.insert`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct InsertParams {
    pub clip_id: ClipId,
    /// MIDI note number (60 = C4).
    pub pitch: u8,
    /// Clip-relative start beat (0-based).
    pub start_beat: f64,
    pub duration_beats: f64,
    /// Defaults to 100.
    #[serde(default = "default_velocity")]
    pub velocity: u8,
}

/// Result of `notes.insert`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct InsertResult {
    /// Index of the inserted note in the clip's note list.
    pub index: usize,
    pub revision: u64,
}

/// One note in a bulk write ([`InsertManyParams`] /
/// [`ReplaceAllParams`]) — the single-note shape of [`InsertParams`]
/// without the clip, which the batch carries once.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct NoteSpec {
    /// MIDI note number (60 = C4).
    pub pitch: u8,
    /// Clip-relative start beat (0-based).
    pub start_beat: f64,
    pub duration_beats: f64,
    /// Defaults to 100.
    #[serde(default = "default_velocity")]
    pub velocity: u8,
}

/// Params for `notes.insert_many`: add every note in `notes` to the clip
/// in ONE undoable edit.
///
/// Prefer this over N `notes.insert` calls. Each control mutation is its
/// own undoable transaction, so a 400-note part written note-by-note
/// leaves 400 undo entries and 400 read-after-write round trips; this is
/// one of each.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct InsertManyParams {
    pub clip_id: ClipId,
    pub notes: Vec<NoteSpec>,
}

/// Params for `notes.replace_all`: make `notes` the clip's entire note
/// list, in ONE undoable edit.
///
/// Destructive — every existing note in the clip is dropped. Clearing
/// and rewriting in one transaction also avoids the highest-index-first
/// ordering trap of a delete loop.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct ReplaceAllParams {
    pub clip_id: ClipId,
    pub notes: Vec<NoteSpec>,
}

/// Result of `notes.insert_many` / `notes.replace_all`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct InsertManyResult {
    /// Where each submitted note landed in the clip's sorted note list,
    /// in the order the notes were given — so `indices[i]` addresses
    /// `notes[i]` for a follow-up `notes.edit` / `notes.delete`.
    pub indices: Vec<usize>,
    pub revision: u64,
}

/// Params for `notes.edit`; omitted fields stay unchanged.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct EditParams {
    pub clip_id: ClipId,
    pub index: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pitch: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start_beat: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_beats: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub velocity: Option<u8>,
}

/// Params for `notes.delete`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct DeleteParams {
    pub clip_id: ClipId,
    pub index: usize,
}

/// Params for `notes.create_clip`. Position the clip either inside a
/// section placement (`placement_id`) or at an explicit `start_bar`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct CreateClipParams {
    pub track_id: TrackId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub placement_id: Option<SectionPlacementId>,
    /// 1-based bar; ignored when `placement_id` is given.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start_bar: Option<u32>,
    /// Defaults to the section length (with `placement_id`) or one bar.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub length_beats: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

/// Result of `notes.create_clip`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct CreateClipResult {
    pub clip_id: ClipId,
    pub revision: u64,
}

/// Params for `notes.move_clip`: reposition an existing MIDI clip on the
/// timeline. Give **exactly one** of `start_bar` (1-based) or
/// `placement_id` (anchor the clip to a section placement's start).
///
/// The target is snapped to the tempo map's bar grid, so this also
/// re-grids a clip whose start drifted. The clip's track, length and
/// notes are unchanged.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct MoveClipParams {
    pub clip_id: ClipId,
    /// 1-based target bar.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start_bar: Option<u32>,
    /// Move the clip to this section placement's start bar.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub placement_id: Option<SectionPlacementId>,
}
