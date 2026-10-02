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
/// undoable edit; destructive on a non-empty clip, requires
/// `"confirm": true` there ([`ReplaceAllParams`] -> [`InsertManyResult`]).
pub const REPLACE_ALL: &str = "notes.replace_all";

/// `notes.import_midi` — import a Standard MIDI File
/// ([`ImportMidiParams`] -> [`ImportMidiResult`]).
pub const IMPORT_MIDI: &str = "notes.import_midi";

/// All `notes.*` method names.
pub const METHODS: &[&str] = &[
    INSERT,
    EDIT,
    DELETE,
    CREATE_CLIP,
    MOVE_CLIP,
    INSERT_MANY,
    REPLACE_ALL,
    IMPORT_MIDI,
];

/// Largest Standard MIDI File `notes.import_midi` accepts, in bytes.
/// Refused above this rather than truncated — a half-imported part is
/// worse than a rejected one.
pub const MAX_MIDI_BYTES: usize = 4 * 1024 * 1024;

/// Most notes one `notes.insert_many` / `notes.replace_all` batch
/// accepts — the same bound `notes.import_midi` puts on an SMF track.
/// Refused above this rather than truncated, for the same reason as
/// [`MAX_MIDI_BYTES`].
pub const MAX_BATCH_NOTES: usize = 100_000;

/// Largest beat value (position or duration) any `notes.*` method
/// accepts. A million beats is over two thousand hours at 120 BPM —
/// far past anything musical — while a truly huge value would overflow
/// the engine's tick arithmetic and corrupt the note on save.
pub const MAX_BEATS: f64 = 1_000_000.0;

/// Params for `notes.import_midi`.
///
/// Give the file **either** as `path` (an absolute path the app can
/// read) **or** as `data_base64`; supplying both, or neither, is
/// `invalid_params`.
///
/// Target the import **either** at an existing `clip_id` — whose notes
/// are replaced — **or** at a `track_id` (+ optional `start_bar`), which
/// creates a clip long enough to hold the imported part.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct ImportMidiParams {
    /// Absolute path to a `.mid` file on the machine running the app.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    /// The file's bytes, base64-encoded. Capped at [`MAX_MIDI_BYTES`]
    /// decoded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data_base64: Option<String>,
    /// Import into this existing MIDI clip, replacing its notes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub clip_id: Option<ClipId>,
    /// Import onto this track as a new clip. Mutually exclusive with
    /// `clip_id`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub track_id: Option<TrackId>,
    /// 1-based bar the new clip starts at; defaults to bar 1. Ignored
    /// when `clip_id` is given.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start_bar: Option<u32>,
    /// Which track of a multi-track SMF to import, 0-based as reported
    /// in the error a multi-track file without this produces. A file
    /// with exactly one note-carrying track needs no selector; one with
    /// several is REFUSED rather than silently flattened.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_track: Option<usize>,
    /// Name for a newly created clip.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

/// Result of `notes.import_midi` — what actually landed, so the client
/// needs no follow-up `song.notes` to find out.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct ImportMidiResult {
    /// The clip the notes went into — newly created, or the `clip_id`
    /// that was given.
    pub clip_id: ClipId,
    pub track_id: TrackId,
    /// How many notes were imported.
    pub note_count: usize,
    /// The SMF track that was imported, 0-based.
    pub source_track: usize,
    /// That track's name from the file, when it carried one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_track_name: Option<String>,
    /// Length of the imported material in beats.
    pub length_beats: f64,
    pub revision: u64,
}

fn default_velocity() -> u8 {
    100
}

/// Params for `notes.insert`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct InsertParams {
    pub clip_id: ClipId,
    /// MIDI note number (60 = C4).
    pub pitch: u8,
    /// Clip-relative start beat (0-based). Capped at [`MAX_BEATS`].
    pub start_beat: f64,
    /// Capped at [`MAX_BEATS`].
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
    /// Clip-relative start beat (0-based). Capped at [`MAX_BEATS`].
    pub start_beat: f64,
    /// Capped at [`MAX_BEATS`].
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
#[serde(deny_unknown_fields)]
pub struct InsertManyParams {
    pub clip_id: ClipId,
    /// At most [`MAX_BATCH_NOTES`] notes.
    pub notes: Vec<NoteSpec>,
}

/// Params for `notes.replace_all`: make `notes` the clip's entire note
/// list, in ONE undoable edit.
///
/// Destructive — every existing note in the clip is dropped, so a
/// non-empty clip requires `"confirm": true` (an empty one has nothing
/// to lose and needs no confirmation). Clearing and rewriting in one
/// transaction also avoids the highest-index-first ordering trap of a
/// delete loop.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct ReplaceAllParams {
    pub clip_id: ClipId,
    /// At most [`MAX_BATCH_NOTES`] notes.
    pub notes: Vec<NoteSpec>,
    /// Required (`true`) when the clip already has notes; the error
    /// otherwise summarizes what would be dropped.
    #[serde(default)]
    pub confirm: bool,
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
#[serde(deny_unknown_fields)]
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
#[serde(deny_unknown_fields)]
pub struct DeleteParams {
    pub clip_id: ClipId,
    pub index: usize,
}

/// Params for `notes.create_clip`. Position the clip either inside a
/// section placement (`placement_id`) or at an explicit `start_bar`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
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
#[serde(deny_unknown_fields)]
pub struct MoveClipParams {
    pub clip_id: ClipId,
    /// 1-based target bar.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start_bar: Option<u32>,
    /// Move the clip to this section placement's start bar.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub placement_id: Option<SectionPlacementId>,
}
