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

/// All `notes.*` method names.
pub const METHODS: &[&str] = &[INSERT, EDIT, DELETE, CREATE_CLIP];

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
