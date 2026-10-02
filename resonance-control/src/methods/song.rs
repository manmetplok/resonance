//! `song.*` — read-only introspection: the compact LLM-oriented song
//! views. None of these methods mutate; all results carry `revision` so
//! clients can detect concurrent GUI edits.

use crate::common::{
    BeatRange, KeyScale, SongPosition, TimeSignature, TrackKind, TrackOutput, TransportState,
};
use crate::ids::{
    ChordId, ClipId, NoteId, SectionDefinitionId, SectionPlacementId, SendId, TrackId,
};
use crate::methods::global::{SignatureEventView, TempoEventView};
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
#[serde(deny_unknown_fields)]
pub struct TracksParams {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub track_id: Option<TrackId>,
}

/// Params for `song.notes`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct NotesParams {
    pub clip_id: ClipId,
    /// Restrict to notes overlapping this clip-relative beat range.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub range: Option<BeatRange>,
}

/// Params for `song.vocal`; omit `track_id` for the first vocal track.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct VocalParams {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub track_id: Option<TrackId>,
}

/// Result of `song.summary`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct SongSummary {
    /// Tempo AT THE PLAYHEAD, in BPM — not a property of the song.
    ///
    /// A song can change tempo at any bar, and this field reports only
    /// the value under the cursor when the call was made: it moves when
    /// the playhead moves and the song has not changed at all.
    /// [`Self::tempo_events`] is the song's actual tempo track and is
    /// what any bar/time arithmetic must be built on.
    pub tempo_bpm: f64,
    /// Meter AT THE PLAYHEAD — not a property of the song.
    ///
    /// Same caveat as [`Self::tempo_bpm`], and the more damaging of the
    /// two: reading `4/4` here on a song that drops into 7/8 at the
    /// bridge silently shifts every bar count that follows.
    /// [`Self::signature_events`] is the song's actual signature track.
    pub time_signature: TimeSignature,
    /// The song's whole tempo track: every tempo change with the 1-based
    /// bar it takes effect at, sorted by bar.
    ///
    /// Never empty — bar 1 is the song's initial tempo and cannot be
    /// removed — so `len() == 1` means "no tempo changes; `tempo_bpm`
    /// holds throughout" and `len() > 1` means `tempo_bpm` is true only
    /// where the playhead happens to sit. That makes "does this song
    /// change tempo?" answerable from this response alone.
    /// `global.list_events` returns the same list on its own.
    #[serde(default)]
    pub tempo_events: Vec<TempoEventView>,
    /// The song's whole signature track: every meter change with the
    /// 1-based bar it takes effect at, sorted by bar.
    ///
    /// Never empty, for the same reason as [`Self::tempo_events`]: bar 1
    /// is the song's initial meter, so `len() == 1` means one meter
    /// throughout.
    #[serde(default)]
    pub signature_events: Vec<SignatureEventView>,
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
    /// `instrument` | `drums` | `vocal` | `audio` | `bus` | `external`.
    /// `external` means the track drives outboard hardware rather than a
    /// plugin — the `external.*` methods configure and inspect it.
    pub kind: TrackKind,
    /// Instrument/plugin summary, e.g. `"resonance-wavetable"`. Always
    /// present: an explicit `null` says the track has no sound source,
    /// which a client cannot distinguish from an omitted field.
    #[serde(default)]
    pub instrument: Option<String>,
    /// Present only on **sub-tracks**: the parent track this one belongs
    /// to. A multi-output instrument (e.g. the drum kit, which declares
    /// Main/Kick/Snare/Toms/Hats/Cymbals/Overhead) spawns one child track
    /// per output port past the first, each with its own fader. Absent on
    /// ordinary tracks — the field is elided rather than reported as
    /// `null`, so its presence alone identifies a sub-track.
    ///
    /// A parent and its sub-tracks are ONE instrument: mute, solo and
    /// loudness measurement only mean anything applied to the whole
    /// group.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_id: Option<TrackId>,
    pub muted: bool,
    pub soloed: bool,
    /// Linear fader gain (1.0 = unity).
    pub volume: f32,
    /// The same fader in decibels (0 dB = unity), which is what the app
    /// itself stores and what the mixer shows. Balance work is done in dB
    /// (1 LU == 1 dB), so prefer this over [`Self::volume`]; the two
    /// always agree (`volume == 10^(volume_db/20)`), with a floor around
    /// -80 dB standing in for silence.
    #[serde(default)]
    pub volume_db: f32,
    /// Stereo pan in `-1.0..=1.0` (0 = center).
    pub pan: f32,
    /// Where this track's audio goes: `"master"` or `{"bus_id": N}`.
    /// Tracks routed into a bus are summed and processed there before
    /// reaching master, so a bus's level and FX affect them all.
    #[serde(default = "default_output")]
    pub output: TrackOutput,
    pub clip_count: usize,
    /// How many automation lanes this track (or bus) carries: its own
    /// volume / pan / mute lanes plus every lane on a plugin it hosts.
    /// `automation.lanes` with this `track_id` / `bus_id` lists them.
    #[serde(default)]
    pub automation_lanes: usize,
    /// The track's identity colour as `"#rrggbb"` — the band on its Arrange
    /// header and mixer strip. Set it with `track.set_color`. Absent on
    /// busses, which carry no colour of their own.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
}

/// Routing default for peers that predate the `output` field.
fn default_output() -> TrackOutput {
    TrackOutput::Master
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
    /// Aux sends taking a tap from this track into a return bus, on top
    /// of wherever `output` sends its main signal. Empty when the track
    /// feeds nothing but its output.
    ///
    /// Persisted with the project (ba todo #1269), so a send created over
    /// the control API survives save + reload.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sends: Vec<SendView>,
    /// The track's automation lanes, compact (no points): mixer lanes
    /// and lanes on its plugins. Read the points with `automation.lanes`.
    /// Empty (and elided) when nothing on the track is automated.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub automation: Vec<super::automation::LaneSummary>,
    pub clips: Vec<ClipView>,
}

/// One aux send from a track into a return bus.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct SendView {
    /// Address this send in `track.set_send` / `track.remove_send`.
    pub send_id: SendId,
    /// The return bus this send feeds.
    pub to_bus: TrackId,
    /// Send gain in dB (0 = tapped at unity).
    pub level_db: f32,
    /// `true` taps before the track's fader, `false` after it.
    pub pre_fader: bool,
    /// A disabled send keeps its routing and level but contributes no
    /// signal.
    pub enabled: bool,
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
