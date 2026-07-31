//! `song.*` — read-only introspection: the compact LLM-oriented song
//! views. None of these methods mutate; all results carry `revision` so
//! clients can detect concurrent GUI edits.

use crate::common::{BeatRange, KeyScale, SongPosition, TimeSignature, TrackKind, TransportState};
use crate::ids::{ChordId, ClipId, NoteId, SectionDefinitionId, SectionPlacementId, TrackId};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// `song.summary` — whole-song overview ([`SongSummary`]). No params.
pub const SUMMARY: &str = "song.summary";
/// `song.sections` — section definitions + placements ([`SectionsView`]). No params.
pub const SECTIONS: &str = "song.sections";
/// `song.tracks` — per-track detail incl. clips ([`TracksView`]).
pub const TRACKS: &str = "song.tracks";
/// `song.notes` — notes of one MIDI clip ([`NotesView`]).
pub const NOTES: &str = "song.notes";
/// `song.vocal` — lyrics/phonemes of a vocal track ([`VocalView`]).
pub const VOCAL: &str = "song.vocal";

/// All `song.*` method names.
pub const METHODS: &[&str] = &[SUMMARY, SECTIONS, TRACKS, NOTES, VOCAL];

/// Params for `song.tracks`; omit `track_id` for all tracks.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct TracksParams {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub track_id: Option<TrackId>,
}

/// Params for `song.notes`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct NotesParams {
    pub clip_id: ClipId,
    /// Restrict to notes overlapping this clip-relative beat range.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub range: Option<BeatRange>,
}

/// Params for `song.vocal`; omit `track_id` for the first vocal track.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct VocalParams {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub track_id: Option<TrackId>,
}

/// Result of `song.summary`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct SongSummary {
    pub tempo_bpm: f64,
    pub time_signature: TimeSignature,
    /// Global key, when the song defines one (otherwise key lives
    /// per-section, see [`SectionDefinitionView::scale`]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key: Option<KeyScale>,
    pub sample_rate: u32,
    /// Song length in both musical and sample units.
    pub length_bars: f64,
    pub length_samples: u64,
    pub transport: TransportState,
    pub playhead: SongPosition,
    /// Ordered section arrangement.
    pub sections: Vec<SectionPlacementView>,
    pub tracks: Vec<TrackSummary>,
    pub revision: u64,
}

/// One placed section in the arrangement.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct SectionPlacementView {
    pub id: SectionPlacementId,
    pub definition_id: SectionDefinitionId,
    /// The definition's name, denormalized for readability.
    pub name: String,
    /// 1-based bar the placement starts at.
    pub start_bar: u32,
    pub length_bars: u32,
}

/// Compact per-track line in [`SongSummary`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct TrackSummary {
    pub id: TrackId,
    pub name: String,
    pub kind: TrackKind,
    /// Instrument/plugin summary, e.g. `"resonance-wavetable"`. Always
    /// present: an explicit `null` says the track has no sound source,
    /// which a client cannot distinguish from an omitted field.
    #[serde(default)]
    pub instrument: Option<String>,
    pub muted: bool,
    pub soloed: bool,
    /// Linear fader gain (1.0 = unity).
    pub volume: f32,
    /// Stereo pan in `-1.0..=1.0` (0 = center).
    pub pan: f32,
    pub clip_count: usize,
}

/// Result of `song.sections`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct SectionsView {
    pub definitions: Vec<SectionDefinitionView>,
    pub placements: Vec<SectionPlacementView>,
    pub revision: u64,
}

/// A section definition with its chord grid.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct SectionDefinitionView {
    pub id: SectionDefinitionId,
    pub name: String,
    pub length_bars: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scale: Option<KeyScale>,
    pub chords: Vec<ChordView>,
}

/// One chord on a section's grid, e.g. `{"id":3,"start_beat":0.0,"duration_beats":4.0,"symbol":"Am7"}`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct ChordView {
    pub id: ChordId,
    /// Section-relative beat the chord starts on (0-based).
    pub start_beat: f64,
    pub duration_beats: f64,
    /// Chord symbol, e.g. `"Am7"`.
    pub symbol: String,
}

/// Result of `song.tracks`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct TracksView {
    pub tracks: Vec<TrackDetail>,
    pub revision: u64,
}

/// Per-track detail: the summary fields plus plugin chain and clips.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct TrackDetail {
    #[serde(flatten)]
    pub summary: TrackSummary,
    /// Insert chain (effects; the instrument is `summary.instrument`).
    /// Always present so the chain stays inspectable — an empty array
    /// means "no inserts", not "the field was elided".
    #[serde(default)]
    pub effects: Vec<String>,
    /// True while the track plays from a freeze cache (valid or stale).
    /// Edits to a frozen track's inputs (notes, lyrics, instrument,
    /// plugin params) are rejected by the app, so a client seeing
    /// `frozen: true` knows why its mutations bounce. Additive within
    /// v1; absent means `false`.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub frozen: bool,
    pub clips: Vec<ClipView>,
}

/// One clip on a track.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct ClipView {
    pub id: ClipId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub start: SongPosition,
    /// Clip length in both musical and sample units.
    pub length_beats: f64,
    pub length_samples: u64,
    /// True for MIDI clips (readable via `song.notes`), false for audio.
    pub midi: bool,
}

/// Result of `song.notes`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct NotesView {
    pub clip_id: ClipId,
    pub notes: Vec<NoteView>,
    pub revision: u64,
}

/// One note, with both MIDI number and name, tick and beat positions.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct NoteView {
    /// Stable note id where the app assigns one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<NoteId>,
    /// Position in the clip's note list; `notes.edit`/`notes.delete`
    /// address notes by this index.
    pub index: usize,
    /// MIDI note number (60 = C4).
    pub pitch: u8,
    /// Pitch name, e.g. `"C4"`.
    pub pitch_name: String,
    pub start_tick: u64,
    pub start_beat: f64,
    pub duration_ticks: u64,
    pub duration_beats: f64,
    pub velocity: u8,
}

/// Result of `song.vocal`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct VocalView {
    pub track_id: TrackId,
    /// Every vocal lane on this track, in placement order. Lyrics live
    /// per `(section definition, track)` lane, and the top-level fields
    /// below describe only the **first** one — the lane the `vocal.*`
    /// mutations resolve to when they are given no `section_id`. Without
    /// this list a client could not tell which lane a write had hit.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub lanes: Vec<VocalLaneView>,
    pub lines: Vec<LyricLineView>,
    /// Project-wide pronunciation overrides: word -> phoneme list
    /// (lowercase ARPAbet-style phonemes).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub pronunciation_overrides: BTreeMap<String, Vec<String>>,
    pub render_state: VocalRenderState,
    pub revision: u64,
}

/// One vocal lane — a `(section definition, track)` pair carrying a
/// Vocal lane generator (ba doc #269 FR-3/FR-7).
///
/// `note_count` and `syllable_count` are the pre-flight check for a
/// render: SVS needs one syllable per note, so a mismatch here is a
/// render-time failure a client can see and fix first. The counts can
/// disagree with a by-eye reading because the engine's G2P decides
/// syllabification (`"don't"` is one syllable, `"remember"` is three).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct VocalLaneView {
    /// The section definition this lane belongs to — pass it as
    /// `section_id` to a `vocal.*` mutation to address this lane.
    pub definition_id: SectionDefinitionId,
    /// The section's name, e.g. `"Verse"`.
    pub name: String,
    /// 1-based bar of the lane's first placement; `None` when the
    /// definition is not placed on the timeline.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start_bar: Option<u32>,
    /// Notes in the lane's derived clip; `0` means nothing is generated
    /// yet, so a render would have nothing to sing.
    pub note_count: usize,
    /// Syllables across the lane's lyric draft, as the engine's G2P
    /// splits them.
    pub syllable_count: usize,
    /// The lane has lyrics whose syllable count does not match its note
    /// count — the condition that makes an SVS render fail or mis-align.
    ///
    /// True when `syllable_count > 0 && note_count != syllable_count`,
    /// so a lane with lyrics and **no** notes flags too: that is the
    /// case that renders nothing the client expects, and it used to be
    /// reported as `false`. Distinguish "not generated yet" by
    /// `note_count == 0` rather than by this flag.
    pub counts_mismatch: bool,
    /// Voicebank this lane will render with, e.g. `"Lilia"`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub voicebank: Option<String>,
    /// The voicebank's comfortable MIDI pitch range. Notes outside it are
    /// flagged in [`Self::notes`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub comfortable_range: Option<PitchRangeView>,
    /// Per-note articulation budget — the render-time intelligibility
    /// pre-flight. Empty when the lane has no notes.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<VocalNoteView>,
    /// How many of [`Self::notes`] are `too_short`.
    #[serde(default)]
    pub short_note_count: usize,
    /// How many of [`Self::notes`] are `out_of_range`.
    #[serde(default)]
    pub out_of_range_note_count: usize,
}

/// An inclusive MIDI pitch range with human-readable note names.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct PitchRangeView {
    /// Lowest comfortable MIDI note (60 = C4).
    pub low: u8,
    /// Highest comfortable MIDI note.
    pub high: u8,
    /// e.g. `"C3"`.
    pub low_name: String,
    /// e.g. `"E5"`.
    pub high_name: String,
}

/// One note of a vocal lane, with everything needed to tell — without
/// rendering and listening — whether it will be *understood*.
///
/// Two things silently destroy intelligibility and neither surfaces as an
/// error: a note too short to articulate the phonemes assigned to it, and
/// a note outside the voicebank's comfortable range. `phoneme_count` vs
/// `min_duration_ms` vs `duration_ms` is the whole story for the first.
/// SVS sings one syllable per note, so a nine-phoneme word crammed onto
/// one note needs a duration no realistic tempo gives it; the fix is more
/// syllable breaks in the lyric (see `vocal.set_lyrics`) or a longer note,
/// not a synthesis parameter.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct VocalNoteView {
    /// 0-based index into the lane's clip note list.
    pub index: usize,
    /// The syllable sung here — `"+"` for a slur continuation.
    pub syllable: String,
    /// Lowercase ARPAbet phonemes this note sings.
    pub phonemes: Vec<String>,
    /// `phonemes.len()`, denormalized so a client can scan for crammed
    /// notes without walking the lists.
    pub phoneme_count: usize,
    pub pitch: u8,
    /// Pitch name, e.g. `"C4"`.
    pub pitch_name: String,
    /// Time this note actually has to sing, in milliseconds.
    pub duration_ms: f64,
    /// Milliseconds needed to articulate `phonemes` — the sum of each
    /// phone's audibility floor.
    pub min_duration_ms: f64,
    /// `duration_ms < min_duration_ms`: this note will be heard as a
    /// smear.
    pub too_short: bool,
    /// Outside [`VocalLaneView::comfortable_range`].
    pub out_of_range: bool,
}

/// One lyric line with its per-syllable phonemes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct LyricLineView {
    /// 0-based line index (`vocal.set_line` addresses lines by this).
    pub index: usize,
    pub text: String,
    pub syllables: Vec<SyllableView>,
}

/// One syllable of a lyric line.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct SyllableView {
    pub text: String,
    /// Lowercase phonemes, e.g. `["l", "ih"]`.
    pub phonemes: Vec<String>,
}

/// SVS render state of a vocal track, lowercase on the wire.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum VocalRenderState {
    /// No render exists yet.
    NotRendered,
    /// A render job is queued or running.
    Rendering,
    /// The rendered audio is up to date with lyrics/notes.
    Rendered,
    /// Lyrics/notes changed since the last render.
    Stale,
    /// The last render failed.
    Error,
}
