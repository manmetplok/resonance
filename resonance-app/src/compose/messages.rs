use resonance_audio::types::TrackId;
use resonance_music_theory::{
    BassMotifMode, BassMotifPhrase, BassStyle, Chord, ChordQuality, ContourPreference, Degree,
    MelodyStyle, PitchClass, Scale, SchemaKind, SyllableMode, VocalContour, VocalMood, VocalPov,
    VocalRhymeScheme, VocalSinger, VocalSingerMeiji, VocalStyle, VocalTimbre, VocalVoicebank,
    VoiceType,
};

use crate::compose::vocal_svs::CurveKind;
use crate::compose::{EntryLength, LaneGeneratorKindTag, PenMode, RailPanelKey, SelectedLane};

/// The two workspace group banners in the Compose lane column. Carried
/// by [`ComposeMessage::ToggleWorkspaceGroup`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkspaceGroup {
    /// SECTION banner — scale stripe, global lanes row, chord lane.
    Section,
    /// TRACKS banner — every vocal / synth / drum lane.
    Tracks,
}

/// One chord slot in a wholesale grid replacement
/// ([`ComposeMessage::ReplaceSectionChords`]). Beats are section-relative,
/// on the whole-beat chord grid.
#[derive(Debug, Clone, PartialEq)]
pub struct SectionChordSpec {
    /// Existing chord id to keep (stable across the replacement), or
    /// `None` to allocate a fresh one.
    pub id: Option<u64>,
    pub start_beat: u32,
    pub duration_beats: u32,
    pub chord: Chord,
}

#[derive(Debug, Clone)]
pub enum ComposeMessage {
    /// Drum-groups messages — project-scoped group management plus the
    /// per-group generator knobs surfaced in the right rail.
    DrumGroups(DrumGroupsMessage),

    /// Drum-arrangement messages — edit a section's ordered list of
    /// pattern entries (the sequence the drums play across its bars).
    Arrangement(ArrangementMessage),

    /// Select which arrangement entry is focused — an index into the
    /// focused section's arrangement, or `None` to clear. Emitted by
    /// clicking a span in the tiling ribbon and consumed by the right-rail
    /// Entry inspector. Pure runtime UI state — no arrangement mutation, no
    /// undo entry.
    SelectArrangementEntry(Option<usize>),

    // Create a new MIDI clip that spans the selected section on the given
    // instrument track. Used by the "+" button that appears over empty
    // instrument rows in the Compose track area.
    CreateMidiClipInSection {
        track_id: TrackId,
        start_sample: u64,
        length_bars: u32,
    },

    // Create-section inline form
    OpenCreateSectionDialog,
    CancelCreateSectionDialog,
    SetNewSectionName(String),
    SetNewSectionLength(String),
    ConfirmCreateSection,

    // Edit-section inline form (for the currently selected placement)
    OpenEditSectionDialog {
        definition_id: u64,
    },
    CancelEditSectionDialog,
    SetEditSectionName(String),
    SetEditSectionLength(String),
    ConfirmEditSection,
    CycleSectionColor {
        definition_id: u64,
    },

    // Section definitions
    CreateSection {
        name: String,
        length_bars: u32,
        color: [u8; 3],
        /// Also place the new definition after the last placement. True
        /// for the GUI dialog (a section the user just made should show
        /// up on the timeline); the control endpoint's `section.create`
        /// exposes it as `place` so a client can build a definition
        /// library first and place it deliberately (ba doc #269 FR-6).
        place: bool,
    },
    RenameSection {
        definition_id: u64,
        name: String,
    },
    ResizeSection {
        definition_id: u64,
        length_bars: u32,
    },
    DeleteSectionDefinition {
        definition_id: u64,
    },
    SetSectionScale {
        definition_id: u64,
        scale: Option<Scale>,
    },

    // Section placements
    PlaceSection {
        definition_id: u64,
        start_bar: u32,
    },
    DeleteSectionPlacement {
        placement_id: u64,
    },
    SelectSectionPlacement {
        placement_id: u64,
    },

    // Chord selection (drives the editor row under the chord lane)
    SelectChord {
        chord_id: u64,
    },
    ClearChordSelection,

    // ---- Lane selection (unified) ----
    /// Select a lane in the Compose view. Updates the right-hand inspector.
    SelectLane(SelectedLane),

    /// Fold / unfold one right-rail panel card. Runtime UI state only.
    ToggleRailPanel(RailPanelKey),
    /// Fold / unfold a workspace group banner (SECTION / TRACKS lanes).
    /// Runtime UI state only.
    ToggleWorkspaceGroup(WorkspaceGroup),

    /// Expand a track into the full-width inline piano-roll editor.
    ExpandTrack {
        track_id: TrackId,
    },
    /// Collapse the expanded editor back to the compact overview.
    CollapseTrack,
    /// Scroll the expanded editor horizontally.
    ExpandedScrollX(f32),
    /// Scroll the expanded editor vertically.
    ExpandedScrollY(f32),
    /// The Compose workspace `Scrollable` reported its viewport: the
    /// horizontal offset and visible width, in content pixels (FU-V2c).
    WorkspaceScrolled { offset_x: f32, width: f32 },
    /// Adjust vertical zoom of the expanded editor.
    ExpandedZoomY(f32),

    // Chords inside a section definition
    AddChord {
        definition_id: u64,
        start_beat: u32,
        duration_beats: u32,
        root: PitchClass,
        quality: ChordQuality,
    },
    EditChord {
        definition_id: u64,
        chord_id: u64,
        chord: Chord,
    },
    MoveChord {
        definition_id: u64,
        chord_id: u64,
        start_beat: u32,
    },
    ResizeChord {
        definition_id: u64,
        chord_id: u64,
        duration_beats: u32,
    },
    DeleteChord {
        definition_id: u64,
        chord_id: u64,
    },
    /// Replace a section definition's whole chord grid in one undoable
    /// step. Used by the control endpoint's `harmony.*` methods (ba doc
    /// #265, todo #1153) so a progression apply — or a chord edit that
    /// changes symbol, position, and length at once — is a single undo
    /// entry with a single lane-regeneration cascade.
    ReplaceSectionChords {
        definition_id: u64,
        chords: Vec<SectionChordSpec>,
    },

    /// Delete a section definition together with every placement that
    /// references it, in one undoable step. Used by the control
    /// endpoint's `section.delete` (ba doc #265, todo #1153); the GUI
    /// path (`DeleteSectionDefinition`) instead refuses while placements
    /// exist.
    DeleteSectionWithPlacements {
        definition_id: u64,
    },

    /// Install a Bass / Melody / Pad generator on a track within a
    /// section and derive its MIDI onto every placement — one undoable
    /// step. Used by the control endpoint's `generate.part` (ba doc
    /// #265, todo #1154). Carries the fully-built lane config the caller
    /// assembled from the wire params.
    GenerateSectionPart {
        definition_id: u64,
        track_id: TrackId,
        config: Box<crate::compose::LaneGeneratorConfig>,
    },

    /// Install (or clear) the generator on a `(section definition,
    /// track)` lane — one undoable step, with **no** MIDI derive. Used
    /// by the control endpoint's `section.set_lane_generator` (ba doc
    /// #268, todo #1168). `None` is the Manual kind: remove the lane's
    /// generator entry. Unlike [`ComposeMessage::GenerateSectionPart`]
    /// this neither requires chords nor generates notes — a vocal lane
    /// is filled in later via `vocal.set_lyrics` / `vocal.render`.
    SetLaneGenerator {
        definition_id: u64,
        track_id: TrackId,
        config: Option<Box<crate::compose::LaneGeneratorConfig>>,
    },

    /// Assign a drum pattern to a section, (re)seed + generate its
    /// groups, and materialize the drum clips — one undoable step. Used
    /// by the control endpoint's `generate.drums` (ba doc #265, todo
    /// #1154). `pattern_id` selects the pattern (defaulting to the
    /// section's primary, then the project default); `seed` seeds the
    /// groups deterministically when set.
    GenerateSectionDrums {
        definition_id: u64,
        pattern_id: Option<u64>,
        /// A built-in groove name to install as authored, instead of
        /// rolling a bank pattern. Mutually exclusive with `pattern_id`.
        builtin: Option<String>,
        /// `0.0..=1.0`; thins a built-in's authored steps, or drives the
        /// Euclidean generator for a bank pattern.
        density: Option<f32>,
        seed: Option<u64>,
    },

    /// Generate a vocal lane's melody — and, unless `lyrics` is false, a
    /// fresh lyric draft — into its derived clip, as one undoable step.
    /// Used by the control endpoint's `vocal.generate` (ba doc #269
    /// FR-2). `seed` pins the result; `None` advances the lane's seed the
    /// way the GUI's generate button does.
    ControlGenerateVocal {
        definition_id: u64,
        track_id: TrackId,
        seed: Option<u64>,
        lyrics: bool,
    },

    /// Replace a vocal lane's full lyric draft from bulk text (one line
    /// per lyric line) — one undoable step. Used by the control
    /// endpoint's `vocal.set_lyrics` (ba doc #265, todo #1156).
    ControlSetVocalLyrics {
        definition_id: u64,
        track_id: TrackId,
        text: String,
    },

    /// Replace a single lyric line (0-based) on a vocal lane — one
    /// undoable step. Used by the control endpoint's `vocal.set_line`
    /// (ba doc #265, todo #1156).
    ControlSetVocalLine {
        definition_id: u64,
        track_id: TrackId,
        line_index: usize,
        text: String,
    },

    /// Set (or replace) a per-word pronunciation override in the project
    /// dictionary — one undoable step. Used by the control endpoint's
    /// `vocal.set_pronunciation` (ba doc #265, todo #1156). Phonemes are
    /// already canonicalised to `&'static str`.
    ControlSetPronunciation {
        word: String,
        phonemes: Vec<&'static str>,
    },

    /// Remove a per-word pronunciation override from the project
    /// dictionary — one undoable step. Used by the control endpoint's
    /// `vocal.clear_pronunciation` (ba doc #265, todo #1156).
    ControlClearPronunciation {
        word: String,
    },

    /// Kick off an SVS render for one vocal lane — the state-mutating
    /// part (voicebank selection, epoch bump) is undoable; the async
    /// audio arrives later via `VocalAudioReady`. Used by the control
    /// endpoint's `vocal.render` (ba doc #265, todo #1156).
    ControlRenderVocal {
        definition_id: u64,
        track_id: TrackId,
        voicebank: VocalVoicebank,
    },

    // ---- Chord lane inspector ----
    ChordInspector {
        definition_id: u64,
        msg: ChordInspectorMsg,
    },

    // ---- Per-track lane inspector ----
    LaneInspector {
        definition_id: u64,
        track_id: TrackId,
        msg: LaneInspectorMsg,
    },

    /// Vocal Expression-dock edits for the lane `(definition_id, track_id)`:
    /// active-curve select, pen/snap tool state, breakpoint add/move/remove,
    /// depth/smoothing, and reset-to-generated. See [`ExpressionMessage`].
    Expression {
        definition_id: u64,
        track_id: TrackId,
        msg: ExpressionMessage,
    },

    /// SVS rendering completed off-thread — install the WAV as an audio
    /// clip on every placement the renderer was launched for. Boxed
    /// because the payload is large (samples vec).
    VocalAudioReady(Box<VocalAudioReadyData>),
    /// SVS render failed; surface the error to the user and fail the
    /// control jobs waiting on the lane. Carries the same lane identity
    /// + epoch snapshot as [`VocalAudioReadyData`]: without them the
    /// handler could only guess which `vocal.render` job the failure
    /// belonged to, and a GUI regeneration of lane B erroring killed a
    /// control job that covered only lane A.
    VocalAudioFailed {
        definition_id: u64,
        track_id: TrackId,
        /// Snapshot of the lane's render epoch taken when the render was
        /// queued (see [`VocalAudioReadyData::render_epoch`]). A failure
        /// arriving with a stale epoch belongs to a superseded render —
        /// a newer one is already in flight for the lane — so it fails
        /// nothing.
        render_epoch: u64,
        error: String,
    },
    /// SVS render finished with nothing to install: no voicebank is
    /// installed, so the lane falls back to playing its MIDI. Not an
    /// error for the GUI (no banner), but a `vocal.render` job waiting on
    /// the lane must still resolve and the lane's in-flight entry clear
    /// (code review FU-M11a). Same lane identity + epoch as
    /// [`Self::VocalAudioFailed`].
    VocalAudioUnavailable {
        definition_id: u64,
        track_id: TrackId,
        render_epoch: u64,
    },
}

impl ComposeMessage {
    /// How this message interacts with the undo history (`undo::classify`
    /// delegates here). Exhaustive on purpose — no `_` arm — so a new
    /// variant does not compile until someone decides what undo does with
    /// it (ARCH-06 A6-4).
    pub(crate) fn undo_action(&self) -> crate::undo::UndoAction {
        use crate::undo::UndoAction;
        match self {
            // Form input, selections, panel open/close: UI only.
            Self::OpenCreateSectionDialog
            | Self::CancelCreateSectionDialog
            | Self::SetNewSectionName(..)
            | Self::SetNewSectionLength(..)
            | Self::OpenEditSectionDialog { .. }
            | Self::CancelEditSectionDialog
            | Self::SetEditSectionName(..)
            | Self::SetEditSectionLength(..)
            | Self::SelectSectionPlacement { .. }
            | Self::SelectChord { .. }
            | Self::ClearChordSelection
            | Self::SelectLane(..)
            | Self::ToggleRailPanel(..)
            | Self::ToggleWorkspaceGroup(..)
            | Self::ExpandTrack { .. }
            | Self::CollapseTrack
            | Self::ExpandedScrollX(..)
            | Self::ExpandedScrollY(..)
            | Self::WorkspaceScrolled { .. }
            | Self::ExpandedZoomY(..) => UndoAction::Skip,
            // Drum-ribbon span selection is view state too (code review
            // VIEW-18): recording it wiped the redo stack.
            Self::SelectArrangementEntry(..) => UndoAction::Skip,
            // A vocal render finishing is the tail of the edit that queued it,
            // seconds later — not a new edit, so it must not clear the redo
            // stack an undo in the meantime filled (VIEW-18). An accepted
            // install still marks the project dirty and bumps the revision in
            // its handler: it changed the project's clips.
            Self::VocalAudioReady(..)
            | Self::VocalAudioFailed { .. }
            | Self::VocalAudioUnavailable { .. } => UndoAction::Skip,
            // The nested editors classify themselves.
            Self::DrumGroups(m) => m.undo_action(),
            Self::Arrangement(m) => m.undo_action(),
            Self::Expression { msg, .. } => msg.undo_action(),
            // Keyed by lane (definition + track) so a lane-inspector text
            // field can coalesce (FU-A10a) — `LaneInspectorMsg` itself
            // doesn't carry the lane identity.
            Self::LaneInspector {
                definition_id,
                track_id,
                msg,
            } => msg.undo_action(*definition_id, *track_id),
            // Keyed by section definition so a chord-inspector slider can
            // coalesce (FU-A10b) — `ChordInspectorMsg` itself doesn't carry
            // the section identity.
            Self::ChordInspector { definition_id, msg } => msg.undo_action(*definition_id),
            // Everything else in Compose mutates project state.
            Self::CreateMidiClipInSection { .. }
            | Self::ConfirmCreateSection
            | Self::ConfirmEditSection
            | Self::CycleSectionColor { .. }
            | Self::CreateSection { .. }
            | Self::RenameSection { .. }
            | Self::ResizeSection { .. }
            | Self::DeleteSectionDefinition { .. }
            | Self::SetSectionScale { .. }
            | Self::PlaceSection { .. }
            | Self::DeleteSectionPlacement { .. }
            | Self::AddChord { .. }
            | Self::EditChord { .. }
            | Self::MoveChord { .. }
            | Self::ResizeChord { .. }
            | Self::DeleteChord { .. }
            | Self::ReplaceSectionChords { .. }
            | Self::DeleteSectionWithPlacements { .. }
            | Self::GenerateSectionPart { .. }
            | Self::SetLaneGenerator { .. }
            | Self::GenerateSectionDrums { .. }
            | Self::ControlGenerateVocal { .. }
            | Self::ControlSetVocalLyrics { .. }
            | Self::ControlSetVocalLine { .. }
            | Self::ControlSetPronunciation { .. }
            | Self::ControlClearPronunciation { .. }
            | Self::ControlRenderVocal { .. } => UndoAction::Record,
        }
    }
}

/// Payload dispatched when the background SVS render finishes. Carries
/// the freshly-written WAV path and the placements that should mmap it.
#[derive(Debug, Clone)]
pub struct VocalAudioReadyData {
    pub definition_id: u64,
    pub track_id: TrackId,
    pub wav_path: std::path::PathBuf,
    /// `(placement_id, start_sample)` snapshot taken when the render was
    /// queued. The render task is fire-and-forget — by the time it
    /// finishes the user may have edited placements, so the install
    /// re-resolves each id: a deleted placement is skipped and a moved
    /// one gets its audio at its current start (VIEW-04, VIEW-19).
    pub placements: Vec<(u64, u64)>,
    pub clip_name: String,
    /// Leading frames to skip on playback. The renderer adds a short
    /// AP padding to every segment so the model ramps in cleanly; the
    /// trim hides it from the timeline.
    pub trim_start_frames: u64,
    /// Trailing frames to skip on playback, mirroring `trim_start`.
    pub trim_end_frames: u64,
    /// The lane's lead-in (tick of its first note) the audio is offset
    /// by from the section start. Kept so the install can re-place the
    /// audio at a placement that moved while the render ran (VIEW-19).
    pub lead_ticks: u64,
    /// Snapshot of `compose.vocal_audio.render_epoch[(def, track)]` taken
    /// when the render was queued. The completion handler compares
    /// this to the current epoch and drops stale renders so the user
    /// doesn't end up with two audio clips stacked on the same lane.
    pub render_epoch: u64,
    /// The section tempo the audio was sung at. A placement can move into
    /// another tempo region (or the tempo can be edited) while the render
    /// runs; the install compares this to the section's tempo *now* and
    /// re-renders on a mismatch rather than installing audio at the wrong
    /// tempo (FU-V2d).
    pub bpm: f32,
}

// ---------------------------------------------------------------------------
// Vocal Expression-dock sub-messages
// ---------------------------------------------------------------------------

/// Edits to a vocal lane's expression curves, dispatched by
/// [`ComposeMessage::Expression`] (doc #154, todo #336). The tool-state
/// arms (`SelectCurve` / `SetPenMode` / `SetSnap`) touch only the
/// [`ExpressionDockState`](crate::compose::ExpressionDockState); the rest
/// mutate the lane's
/// [`ExpressionCurves`](crate::compose::ExpressionCurves) and request a
/// vocal re-render so the WAV reflects the edit.
///
/// Each curve-mutating arm names its [`CurveKind`] explicitly rather than
/// relying on the dock's active curve, so the transition is deterministic
/// and unit-testable.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ExpressionMessage {
    /// Make `kind` the active curve in the dock (canvas + inspector).
    SelectCurve(CurveKind),
    /// Switch the pen mode used for canvas edits.
    SetPenMode(PenMode),
    /// Toggle snap-to-syllables. When on, breakpoint times quantise to the
    /// lane's note onsets on add/move.
    SetSnap(bool),
    /// Add a breakpoint to `kind`'s overlay at normalised time `t` (snapped
    /// when snap is on) and `value`.
    AddBreakpoint { kind: CurveKind, t: f32, value: f32 },
    /// Move `kind`'s overlay breakpoint at `index` to (`t`, `value`); `t`
    /// snaps when snap is on and clamps to its neighbours so the index is
    /// stable.
    MoveBreakpoint {
        kind: CurveKind,
        index: usize,
        t: f32,
        value: f32,
    },
    /// Remove `kind`'s overlay breakpoint at `index`.
    RemoveBreakpoint { kind: CurveKind, index: usize },
    /// Set `kind`'s depth-over-baseline inspector value.
    SetDepth { kind: CurveKind, depth: f32 },
    /// Set `kind`'s smoothing window, in milliseconds.
    SetSmoothing { kind: CurveKind, smoothing: f32 },
    /// Reset `kind` to its generated baseline: drop the overlay and depth/
    /// smoothing, flipping the curve's status back to `Auto`.
    Reset { kind: CurveKind },
}

impl ExpressionMessage {
    /// How this message interacts with the undo history (`undo::classify`
    /// delegates here). Exhaustive on purpose — no `_` arm — so a new
    /// variant does not compile until someone decides what undo does with
    /// it (ARCH-06 A6-4).
    pub(crate) fn undo_action(&self) -> crate::undo::UndoAction {
        use crate::undo::UndoAction;
        match self {
            // The Expression dock's tool state (active curve, pen, snap) is
            // view state (code review VIEW-18): recording it wiped the redo
            // stack.
            Self::SelectCurve(..) | Self::SetPenMode(..) | Self::SetSnap(..) => UndoAction::Skip,
            // Every breakpoint / depth / smoothing edit mutates the lane.
            Self::AddBreakpoint { .. }
            | Self::MoveBreakpoint { .. }
            | Self::RemoveBreakpoint { .. }
            | Self::SetDepth { .. }
            | Self::SetSmoothing { .. }
            | Self::Reset { .. } => UndoAction::Record,
        }
    }
}

// ---------------------------------------------------------------------------
// Chord lane inspector sub-messages
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub enum ChordInspectorMsg {
    /// Switch the progression generator between its Markov-table and
    /// pop-schema modes. Length carries over; everything else takes the
    /// target mode's defaults (table "pop" / schema Axis).
    SetGeneratorKind(GeneratorKind),
    /// Select which Markov table to use.
    SetTable(String),
    /// Select which pop schema to realize. Resets length to the
    /// schema's natural loop length and rotation to 0.
    SetSchemaKind(SchemaKind),
    /// Rotate the schema's base loop by this many positions.
    SetSchemaRotation(u8),
    /// Per-position probability (0..=1) of a function-preserving chord
    /// substitution.
    SetSchemaSubstitution(f32),
    /// Set the number of chords to generate.
    SetLength(u8),
    /// Set the beat duration of each generated chord.
    SetBeatsPerChord(u32),
    /// Toggle seventh chords on/off.
    SetSeventhChords(bool),
    /// Set the start-degree constraint (None = any).
    SetStartDegree(Option<Degree>),
    /// Set the end-degree constraint (None = any).
    SetEndDegree(Option<Degree>),
    /// First-time generation: create a GeneratorSpec from current controls.
    Generate,
    /// Bump seed and regenerate (respecting locks).
    Regenerate,

    // Section-shared motif knobs (consumed by every Motif-style lane).
    SetMotifComplexity(f32),
    SetMotifLen(u8),
    SetMotifLeapChance(f32),
    /// Bump the section motif seed and re-derive every Motif-style lane.
    RegenerateMotif,

    // Generated-vs-manual motif source.
    /// Switch between a procedurally generated motif and a hand-drawn one.
    SetMotifSourceKind(MotifSourceKind),
    /// Toggle a cell in the manual-motif canvas. `beat_16` is the start
    /// beat in sixteenth-notes from the motif start; `scale_step` is the
    /// target row (0 = anchor, +n = up the scale, −n = down).
    ToggleManualMotifCell { scale_step: i8, beat_16: u8 },
    /// Toggle a rest at the given beat position. Same insert/replace/
    /// remove semantics as [`ToggleManualMotifCell`], but the entry has
    /// no pitch — it advances the motif cursor without emitting a note.
    ToggleManualMotifRest { beat_16: u8 },
    /// Cycle the duration of the indexed manual-motif note (1 → 2 → 3 → 4
    /// → 1 sixteenths).
    CycleManualMotifNoteDuration { index: usize },
    /// Toggle the accent flag on the indexed manual-motif note.
    ToggleManualMotifAccent { index: usize },
    /// Wipe every note from the manual motif.
    ClearManualMotif,
}

impl ChordInspectorMsg {
    /// How this message interacts with the undo history (`undo::classify`
    /// delegates here via `ComposeMessage::ChordInspector`). Exhaustive on
    /// purpose — no `_` arm — so a new variant does not compile until
    /// someone decides what undo does with it (ARCH-06 A6-4).
    ///
    /// Takes `definition_id` — the enclosing `ComposeMessage::ChordInspector`'s
    /// section identity — because the sliders below coalesce per section,
    /// and `ChordInspectorMsg` itself doesn't carry that identity (FU-A10b,
    /// the `LaneInspectorMsg::undo_action` template).
    pub(crate) fn undo_action(&self, definition_id: u64) -> crate::undo::UndoAction {
        use crate::undo::{ChordParamKnob, CoalesceKey, UndoAction};
        match self {
            // The motif/schema sliders deliver one message per step with no
            // begin/commit pair; coalesce per section *and* per knob so
            // dragging complexity then leap chance is two entries, not one,
            // but repeated steps on the same slider merge (FU-A10b; was one
            // entry per step).
            Self::SetMotifComplexity(..) => UndoAction::RecordCoalesced(CoalesceKey::ChordParam(
                definition_id,
                ChordParamKnob::MotifComplexity,
            )),
            Self::SetMotifLeapChance(..) => UndoAction::RecordCoalesced(CoalesceKey::ChordParam(
                definition_id,
                ChordParamKnob::MotifLeapChance,
            )),
            Self::SetSchemaSubstitution(..) => {
                UndoAction::RecordCoalesced(CoalesceKey::ChordParam(
                    definition_id,
                    ChordParamKnob::SchemaSubstitution,
                ))
            }
            // Every other chord-inspector edit is a discrete pick, toggle
            // or action — not a slider drag — and records atomically like
            // the rest of Compose.
            Self::SetGeneratorKind(..)
            | Self::SetTable(..)
            | Self::SetSchemaKind(..)
            | Self::SetSchemaRotation(..)
            | Self::SetLength(..)
            | Self::SetBeatsPerChord(..)
            | Self::SetSeventhChords(..)
            | Self::SetStartDegree(..)
            | Self::SetEndDegree(..)
            | Self::Generate
            | Self::Regenerate
            | Self::SetMotifLen(..)
            | Self::RegenerateMotif
            | Self::SetMotifSourceKind(..)
            | Self::ToggleManualMotifCell { .. }
            | Self::ToggleManualMotifRest { .. }
            | Self::CycleManualMotifNoteDuration { .. }
            | Self::ToggleManualMotifAccent { .. }
            | Self::ClearManualMotif => UndoAction::Record,
        }
    }
}

/// Which kind of motif a section is using. Drives a radio in the chord
/// inspector and decides whether the manual-motif canvas is editable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MotifSourceKind {
    Generated,
    Manual,
}

impl MotifSourceKind {
    pub fn as_str(self) -> &'static str {
        match self {
            MotifSourceKind::Generated => "Generated",
            MotifSourceKind::Manual => "Manual",
        }
    }
}

impl std::fmt::Display for MotifSourceKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Which progression generator a section's chord lane is using. UI-side
/// discriminant for [`resonance_music_theory::GeneratorSpec`] — drives
/// the GENERATOR dropdown in the chord inspector.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GeneratorKind {
    /// Sample from a Markov style table (Pop / Jazz / ...).
    Markov,
    /// Realize a canonical pop schema (axis, doo-wop, 12-bar blues, ...).
    Schema,
}

impl GeneratorKind {
    /// All kinds, in display order (for the cached pick_list options).
    pub const ALL: [GeneratorKind; 2] = [GeneratorKind::Markov, GeneratorKind::Schema];

    pub fn as_str(self) -> &'static str {
        match self {
            GeneratorKind::Markov => "Style table",
            GeneratorKind::Schema => "Schema",
        }
    }
}

impl std::fmt::Display for GeneratorKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

// ---------------------------------------------------------------------------
// Per-track lane inspector sub-messages
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub enum LaneInspectorMsg {
    /// Switch the generator type for this lane.
    SetGenerator(LaneGeneratorKindTag),

    // Bass
    SetBassStyle(BassStyle),
    SetBassBaseNote(u8),
    SetBassVelocity(f32),
    SetBassMotifMode(BassMotifMode),
    SetBassMotifPhrase(BassMotifPhrase),

    // Melody
    SetMelodyStyle(MelodyStyle),
    SetMelodyRegisterLow(u8),
    SetMelodyRegisterHigh(u8),
    SetMelodyNoteValue(u32),
    SetMelodyRestDensity(f32),
    SetMelodyVelocity(f32),
    SetMelodyArticulation(f32),
    SetMelodyContour(ContourPreference),
    SetMelodyPhraseLen(u8),
    ToggleMelodyFillVocalGaps,

    // Pad
    SetPadRegisterLow(u8),
    SetPadRegisterHigh(u8),
    SetPadVelocity(f32),

    // Vocal — lyrics
    SetVocalTheme(String),
    SetVocalMood(VocalMood),
    SetVocalPov(VocalPov),
    SetVocalRhyme(VocalRhymeScheme),
    SetVocalLines(u8),
    SetVocalSyllablesMin(u8),
    SetVocalSyllablesMax(u8),
    ToggleVocalMatchSyllables,
    ToggleVocalAvoidCliches,
    ToggleVocalLockLine(u8),
    /// Replace the text of a single draft line (1-based) and auto-lock it so
    /// the next re-roll preserves the user's wording.
    SetVocalLineText(u8, String),
    /// Edit action coming from the section-level "bulk lyrics" text editor.
    /// Carries the raw iced action so the update handler can apply it to
    /// the lane's editor `Content` and (when the action is an edit) re-parse
    /// the buffer into individual `LyricLine`s.
    VocalBulkLyricsAction(iced::widget::text_editor::Action),
    RerollUnlockedLyrics,
    /// Insert `·` syllable markers into every lyric line based on
    /// each word's CMU dictionary syllable count. Words that already
    /// have enough dots are left alone.
    AutoSyllabifyLyrics,

    // Vocal — melody
    SetVocalVoiceType(VoiceType),
    SetVocalRangeLow(u8),
    SetVocalRangeHigh(u8),
    SetVocalStyle(VocalStyle),
    SetVocalContour(VocalContour),
    SetVocalSyllableMode(SyllableMode),
    SetVocalChordToneAnchor(f32),
    SetVocalLeapRange(f32),
    SetVocalPhraseLength(u8),
    SetVocalBreath(f32),
    ToggleVocalStayInScale,
    ToggleVocalAvoidClashes,
    ToggleVocalUseSectionMotif,

    // Vocal — voice & delivery
    SetVocalTimbre(VocalTimbre),
    SetVocalVoicebank(VocalVoicebank),
    SetVocalSinger(VocalSinger),
    SetVocalSingerMeiji(VocalSingerMeiji),
    SetVocalVibrato(f32),
    SetVocalVibratoRate(f32),
    SetVocalTension(f32),
    SetVocalTensionVelocityAmount(f32),
    SetVocalTensionContourAmount(f32),
    SetVocalPortamentoMs(f32),
    SetVocalArticulation(f32),
    SetVocalConsonantEmphasis(f32),

    // Vocal — actions
    GenerateVocalAll,
    GenerateVocalLyricsOnly,
    GenerateVocalMelodyOnly,
    /// Re-render the audio only, reusing the existing MIDI clip notes
    /// (which may have been hand-edited in the vocal roll). Doesn't
    /// bump the lane seed or re-derive notes from chords; just feeds
    /// the current notes + current `VocalParams` through the SVS
    /// pipeline. Use this when the user wants to audition their own
    /// edits without losing them to a re-roll.
    RerenderVocalAudio,

    /// Regenerate this lane from its generator spec + section chords.
    Regenerate,
}

impl LaneInspectorMsg {
    /// How this message interacts with the undo history (`undo::classify`
    /// delegates here via `ComposeMessage::LaneInspector`). Exhaustive on
    /// purpose — no `_` arm — so a new variant does not compile until
    /// someone decides what undo does with it (ARCH-06 A6-4).
    ///
    /// Takes `definition_id`/`track_id` — the enclosing
    /// `ComposeMessage::LaneInspector`'s lane identity — because the two
    /// text fields below coalesce per lane, and `LaneInspectorMsg` itself
    /// doesn't carry that identity.
    pub(crate) fn undo_action(
        &self,
        definition_id: u64,
        track_id: TrackId,
    ) -> crate::undo::UndoAction {
        use crate::undo::{CoalesceKey, LaneParamKnob, UndoAction};
        match self {
            // Typed straight into the project on every keystroke, with no
            // begin/commit pair — coalesce per lane (FU-A10a; was one
            // undo entry per keystroke).
            Self::SetVocalTheme(_) => {
                UndoAction::RecordCoalesced(CoalesceKey::VocalTheme(definition_id, track_id))
            }
            // Same as `SetVocalTheme`, keyed additionally by which draft
            // line so editing two lines in a row is two entries but
            // retyping one is one (FU-A10a).
            Self::SetVocalLineText(line_n, _) => UndoAction::RecordCoalesced(
                CoalesceKey::VocalLineText(definition_id, track_id, *line_n),
            ),
            // The bulk-lyrics text editor delivers one message per
            // interaction, including cursor moves and selection that
            // never touch the draft — those are no edit at all (FU-A10b;
            // every action used to record, so clicking around the editor
            // filled the undo history with no-op entries). An edit
            // action coalesces per lane, same idiom as the two fields
            // above.
            Self::VocalBulkLyricsAction(action) => {
                if action.is_edit() {
                    UndoAction::RecordCoalesced(CoalesceKey::VocalBulkLyrics(
                        definition_id,
                        track_id,
                    ))
                } else {
                    UndoAction::Skip
                }
            }
            // The lane inspector's numeric sliders deliver one message per
            // step with no begin/commit pair; coalesce per lane *and* per
            // knob so dragging one slider then another on the same lane is
            // two entries, not one, but repeated steps on the same slider
            // merge (FU-A10b; was one entry per step). The register/note
            // pickers beside some of these are `pick_list`s — discrete
            // picks, not a drag — and stay plain `Record` below.
            Self::SetBassVelocity(..) => UndoAction::RecordCoalesced(CoalesceKey::LaneParam(
                definition_id,
                track_id,
                LaneParamKnob::BassVelocity,
            )),
            Self::SetMelodyRestDensity(..) => UndoAction::RecordCoalesced(CoalesceKey::LaneParam(
                definition_id,
                track_id,
                LaneParamKnob::MelodyRestDensity,
            )),
            Self::SetMelodyVelocity(..) => UndoAction::RecordCoalesced(CoalesceKey::LaneParam(
                definition_id,
                track_id,
                LaneParamKnob::MelodyVelocity,
            )),
            Self::SetMelodyArticulation(..) => UndoAction::RecordCoalesced(CoalesceKey::LaneParam(
                definition_id,
                track_id,
                LaneParamKnob::MelodyArticulation,
            )),
            Self::SetPadVelocity(..) => UndoAction::RecordCoalesced(CoalesceKey::LaneParam(
                definition_id,
                track_id,
                LaneParamKnob::PadVelocity,
            )),
            Self::SetVocalChordToneAnchor(..) => {
                UndoAction::RecordCoalesced(CoalesceKey::LaneParam(
                    definition_id,
                    track_id,
                    LaneParamKnob::VocalChordToneAnchor,
                ))
            }
            Self::SetVocalLeapRange(..) => UndoAction::RecordCoalesced(CoalesceKey::LaneParam(
                definition_id,
                track_id,
                LaneParamKnob::VocalLeapRange,
            )),
            Self::SetVocalBreath(..) => UndoAction::RecordCoalesced(CoalesceKey::LaneParam(
                definition_id,
                track_id,
                LaneParamKnob::VocalBreath,
            )),
            Self::SetVocalVibrato(..) => UndoAction::RecordCoalesced(CoalesceKey::LaneParam(
                definition_id,
                track_id,
                LaneParamKnob::VocalVibrato,
            )),
            Self::SetVocalVibratoRate(..) => UndoAction::RecordCoalesced(CoalesceKey::LaneParam(
                definition_id,
                track_id,
                LaneParamKnob::VocalVibratoRate,
            )),
            Self::SetVocalTension(..) => UndoAction::RecordCoalesced(CoalesceKey::LaneParam(
                definition_id,
                track_id,
                LaneParamKnob::VocalTension,
            )),
            Self::SetVocalTensionVelocityAmount(..) => {
                UndoAction::RecordCoalesced(CoalesceKey::LaneParam(
                    definition_id,
                    track_id,
                    LaneParamKnob::VocalTensionVelocityAmount,
                ))
            }
            Self::SetVocalTensionContourAmount(..) => {
                UndoAction::RecordCoalesced(CoalesceKey::LaneParam(
                    definition_id,
                    track_id,
                    LaneParamKnob::VocalTensionContourAmount,
                ))
            }
            Self::SetVocalPortamentoMs(..) => UndoAction::RecordCoalesced(CoalesceKey::LaneParam(
                definition_id,
                track_id,
                LaneParamKnob::VocalPortamentoMs,
            )),
            Self::SetVocalArticulation(..) => UndoAction::RecordCoalesced(CoalesceKey::LaneParam(
                definition_id,
                track_id,
                LaneParamKnob::VocalArticulation,
            )),
            Self::SetVocalConsonantEmphasis(..) => {
                UndoAction::RecordCoalesced(CoalesceKey::LaneParam(
                    definition_id,
                    track_id,
                    LaneParamKnob::VocalConsonantEmphasis,
                ))
            }
            // Every other lane-inspector edit is a discrete pick, toggle
            // or action — not a keystroke/knob burst — and records
            // atomically like the rest of Compose.
            Self::SetGenerator(..)
            | Self::SetBassStyle(..)
            | Self::SetBassBaseNote(..)
            | Self::SetBassMotifMode(..)
            | Self::SetBassMotifPhrase(..)
            | Self::SetMelodyStyle(..)
            | Self::SetMelodyRegisterLow(..)
            | Self::SetMelodyRegisterHigh(..)
            | Self::SetMelodyNoteValue(..)
            | Self::SetMelodyContour(..)
            | Self::SetMelodyPhraseLen(..)
            | Self::ToggleMelodyFillVocalGaps
            | Self::SetPadRegisterLow(..)
            | Self::SetPadRegisterHigh(..)
            | Self::SetVocalMood(..)
            | Self::SetVocalPov(..)
            | Self::SetVocalRhyme(..)
            | Self::SetVocalLines(..)
            | Self::SetVocalSyllablesMin(..)
            | Self::SetVocalSyllablesMax(..)
            | Self::ToggleVocalMatchSyllables
            | Self::ToggleVocalAvoidCliches
            | Self::ToggleVocalLockLine(..)
            | Self::RerollUnlockedLyrics
            | Self::AutoSyllabifyLyrics
            | Self::SetVocalVoiceType(..)
            | Self::SetVocalRangeLow(..)
            | Self::SetVocalRangeHigh(..)
            | Self::SetVocalStyle(..)
            | Self::SetVocalContour(..)
            | Self::SetVocalSyllableMode(..)
            | Self::SetVocalPhraseLength(..)
            | Self::ToggleVocalStayInScale
            | Self::ToggleVocalAvoidClashes
            | Self::ToggleVocalUseSectionMotif
            | Self::SetVocalTimbre(..)
            | Self::SetVocalVoicebank(..)
            | Self::SetVocalSinger(..)
            | Self::SetVocalSingerMeiji(..)
            | Self::GenerateVocalAll
            | Self::GenerateVocalLyricsOnly
            | Self::GenerateVocalMelodyOnly
            | Self::RerenderVocalAudio
            | Self::Regenerate => UndoAction::Record,
        }
    }
}

// ---------------------------------------------------------------------------
// Drum groups
// ---------------------------------------------------------------------------

/// Messages targeting the project-scoped drum pattern bank. Patterns
/// (id, name, color, groups) live on
/// [`crate::compose::ComposeState::drum_patterns`]; group definitions
/// (id, name, color, pads, grid, cycle, phase) live on each
/// [`crate::compose::DrumPattern::groups`]. The manager modal, the
/// drum-lane pattern picker, and the right-rail generator all route
/// through these.
#[derive(Debug, Clone)]
pub enum DrumGroupsMessage {
    /// Mark a group as the active selection for the right-rail generator
    /// and the lane's highlight stripe.
    SelectGroup { group_id: u64 },

    // ---- Pattern bank ----
    /// Switch which pattern the manager modal is currently editing.
    /// Doesn't change the assignment on any section — that's
    /// [`AssignPattern`].
    SelectPattern { pattern_id: u64 },
    /// Assign a pattern to a section. `pattern_id: None` reverts the
    /// section to "use the project default" (resolved via
    /// `ComposeState::pattern_for_definition`).
    AssignPattern {
        definition_id: u64,
        pattern_id: Option<u64>,
    },
    /// Append a fresh empty pattern to the bank and focus it in the
    /// manager modal.
    AddPattern,
    /// Clone an existing pattern (groups + per-pad patterns) into a new
    /// bank entry. Each group inside the duplicate gets a fresh id so
    /// future toggles don't affect both.
    DuplicatePattern { pattern_id: u64 },
    /// Remove a pattern from the bank. Sections that pointed at the
    /// deleted pattern fall back to the project default. Refuses to
    /// remove the last remaining pattern.
    DeletePattern { pattern_id: u64 },
    /// Replace a pattern's display name.
    RenamePattern { pattern_id: u64, name: String },
    /// Update a pattern's accent color.
    SetPatternColor { pattern_id: u64, color: [u8; 3] },
    /// Open the inline rename input on the lane's pattern picker chip.
    BeginRenamePattern { pattern_id: u64 },
    /// Update the in-progress rename text. Keyed off
    /// `drumroll.renaming_pattern_id`.
    UpdateRenamePatternText(String),
    /// Apply the rename input and clear the in-progress state.
    CommitRenamePattern,
    /// Discard the rename input without applying.
    CancelRenamePattern,

    // ---- Manager modal ----
    /// Show the modal in "manage groups" mode.
    OpenManager,
    /// Hide the modal.
    CloseManager,
    /// Pick which group the manager is editing on the right-hand column.
    ManagerSelectGroup { group_id: u64 },
    /// Filter text typed into the kit pad search box.
    ManagerSetFilter(String),
    /// Add a fresh empty group and select it for editing.
    AddGroup,
    /// Delete a group from the project. If the deleted group was selected
    /// the focus falls back to the first remaining group.
    DeleteGroup { group_id: u64 },
    /// Rename a group (the manager's name input).
    RenameGroup { group_id: u64, name: String },
    /// Cycle the group's color through the palette by clicking a swatch.
    SetGroupColor { group_id: u64, color: [u8; 3] },
    /// Toggle whether a kit pad belongs to the given group. Adding a pad
    /// to one group removes it from any other.
    TogglePadAssignment { group_id: u64, note: u8 },
    /// Remove every pad from a group (the "Clear pads" button).
    ClearGroupPads { group_id: u64 },

    // ---- Generator knobs (right rail) ----
    SetGroupGrid { group_id: u64, grid: u8 },
    SetGroupCycle { group_id: u64, cycle: u32 },
    SetGroupPhase { group_id: u64, phase: u32 },
    /// Combined grid+cycle preset — used by the polyrhythm/polymeter
    /// preset chips. Set both at once so the chip can produce a single
    /// click that lands on a consistent meter rather than nudging only
    /// one axis.
    SetGroupMeter { group_id: u64, grid: u8, cycle: u32 },
    SetGroupDensity { group_id: u64, density: f32 },
    SetGroupSwing { group_id: u64, swing: f32 },
    SetGroupAccent { group_id: u64, accent: f32 },
    SetGroupHumanize { group_id: u64, humanize: f32 },
    SetGroupFills { group_id: u64, fills: f32 },
    /// Update an articulation pad's weight (0..=100).
    SetPadWeight {
        group_id: u64,
        pad_index: usize,
        weight: u32,
    },
    /// Run the group's pattern generator (density/style aware) and bump
    /// the per-group seed so repeated presses yield variation.
    GenerateGroup { group_id: u64 },
    /// Run the generator on every group at once.
    GenerateAllGroups,
    /// Flip a single pattern step on / off for one pad. Used by the
    /// drum lane's cell click. `step` is the visible, section-relative
    /// step (`bar × steps_per_bar + step_in_bar`); the handler adds the
    /// group's phase and wraps modulo its cycle to get the pattern index.
    TogglePadStep {
        group_id: u64,
        pad_index: usize,
        step: usize,
    },
}

impl DrumGroupsMessage {
    /// How this message interacts with the undo history (`undo::classify`
    /// delegates here). Exhaustive on purpose — no `_` arm — so a new
    /// variant does not compile until someone decides what undo does with
    /// it (ARCH-06 A6-4).
    pub(crate) fn undo_action(&self) -> crate::undo::UndoAction {
        use crate::undo::{CoalesceKey, DrumGroupKnob, UndoAction};
        match self {
            // Focus, the manager modal and the inline rename's begin /
            // typing / cancel only touch `DrumrollViewState`, which is
            // session UI and never in the project (A-10: these used to
            // record — a rename was one entry per keystroke plus two).
            // The rename lands as one entry, on `CommitRenamePattern`.
            Self::SelectGroup { .. }
            | Self::SelectPattern { .. }
            | Self::BeginRenamePattern { .. }
            | Self::UpdateRenamePatternText(..)
            | Self::CancelRenamePattern
            | Self::OpenManager
            | Self::CloseManager
            | Self::ManagerSelectGroup { .. }
            | Self::ManagerSetFilter(..) => UndoAction::Skip,
            // The manager's group-name field records one keystroke at a
            // time straight into the project, with no begin/commit pair
            // like the pattern chip's rename above — coalesce it instead
            // (FU-A10a; was one undo entry per keystroke).
            Self::RenameGroup { group_id, .. } => {
                UndoAction::RecordCoalesced(CoalesceKey::DrumGroupName(*group_id))
            }
            // The right-rail generator knobs deliver one message per
            // slider step; coalesce per group *and* per knob so dragging
            // density then swing on the same group is two entries, not
            // one, but repeated steps on the same knob merge (FU-A10a;
            // was one entry per step).
            Self::SetGroupDensity { group_id, .. } => UndoAction::RecordCoalesced(
                CoalesceKey::DrumGroupParam(*group_id, DrumGroupKnob::Density),
            ),
            Self::SetGroupSwing { group_id, .. } => UndoAction::RecordCoalesced(
                CoalesceKey::DrumGroupParam(*group_id, DrumGroupKnob::Swing),
            ),
            Self::SetGroupAccent { group_id, .. } => UndoAction::RecordCoalesced(
                CoalesceKey::DrumGroupParam(*group_id, DrumGroupKnob::Accent),
            ),
            Self::SetGroupHumanize { group_id, .. } => UndoAction::RecordCoalesced(
                CoalesceKey::DrumGroupParam(*group_id, DrumGroupKnob::Humanize),
            ),
            Self::SetGroupFills { group_id, .. } => UndoAction::RecordCoalesced(
                CoalesceKey::DrumGroupParam(*group_id, DrumGroupKnob::Fills),
            ),
            // Pattern-bank and group edits mutate the persisted drum
            // patterns / section assignment.
            Self::AssignPattern { .. }
            | Self::AddPattern
            | Self::DuplicatePattern { .. }
            | Self::DeletePattern { .. }
            | Self::RenamePattern { .. }
            | Self::SetPatternColor { .. }
            | Self::CommitRenamePattern
            | Self::AddGroup
            | Self::DeleteGroup { .. }
            | Self::SetGroupColor { .. }
            | Self::TogglePadAssignment { .. }
            | Self::ClearGroupPads { .. }
            | Self::SetGroupGrid { .. }
            | Self::SetGroupCycle { .. }
            | Self::SetGroupPhase { .. }
            | Self::SetGroupMeter { .. }
            | Self::SetPadWeight { .. }
            | Self::GenerateGroup { .. }
            | Self::GenerateAllGroups
            | Self::TogglePadStep { .. } => UndoAction::Record,
        }
    }
}

// ---------------------------------------------------------------------------
// Drum arrangement
// ---------------------------------------------------------------------------

/// Messages that edit a section's drum *arrangement* — the ordered
/// [`Vec<PatternEntry>`](crate::compose::PatternEntry) on
/// [`SectionDefinitionState::arrangement`](crate::compose::SectionDefinitionState::arrangement)
/// describing which pattern plays over which bars. Every variant carries
/// the target `definition_id` so the edit lands on a specific section
/// rather than relying on the current focus. Each routes through
/// `update::compose::drum_groups::handle_arrangement`, mutates the
/// arrangement, and re-materializes the drum clips so playback stays in
/// sync; the existing undo machinery records a snapshot per edit.
#[derive(Debug, Clone)]
pub enum ArrangementMessage {
    /// Append a fresh single-repeat entry playing `pattern_id` (picked
    /// from the pattern bank). Ignored if the pattern doesn't exist.
    AddEntry { definition_id: u64, pattern_id: u64 },
    /// Remove the entry at `index`.
    RemoveEntry { definition_id: u64, index: usize },
    /// Move the entry at `from` to position `to`, shifting the rest. Drives
    /// move-up / move-down buttons and drag-to-reorder.
    MoveEntry {
        definition_id: u64,
        from: usize,
        to: usize,
    },
    /// Select the arrangement entry at `index` for the right-rail Entry
    /// inspector (or clear the selection with `None`). Pure UI state — does
    /// not mutate the arrangement, so it is skipped by the undo machinery.
    /// Emitted by the arrangement strip's entry chips and (implicitly) when
    /// a bank pattern is added.
    SelectEntry { index: Option<usize> },
    /// Swap the pattern played by the entry at `index`, keeping its length
    /// mode + fill. Drives the Entry inspector's pattern picker dropdown.
    /// Ignored if the chosen pattern doesn't exist.
    SetEntryPattern {
        definition_id: u64,
        index: usize,
        pattern_id: u64,
    },
    /// Set the length mode + value of the entry at `index`
    /// (`RepeatN(n)` / `Bars(b)`).
    SetEntryLength {
        definition_id: u64,
        index: usize,
        length: EntryLength,
    },
    /// Set the entry's fill pattern (`Some(id)` to enable + choose, `None`
    /// to clear). Ignored if a chosen fill pattern doesn't exist.
    SetEntryFill {
        definition_id: u64,
        index: usize,
        fill: Option<u64>,
    },
    /// Insert a copy of the entry at `index` right after it.
    DuplicateEntry { definition_id: u64, index: usize },
    /// Remediation: extend the arrangement (last entry or a new one) so it
    /// covers the section exactly, closing any trailing gap.
    FillToEnd { definition_id: u64 },
    /// Remediation: shrink / drop trailing entries so the arrangement lands
    /// exactly on the section boundary, dropping any overflow.
    TrimToFit { definition_id: u64 },
}

impl ArrangementMessage {
    /// How this message interacts with the undo history (`undo::classify`
    /// delegates here). Exhaustive on purpose — no `_` arm — so a new
    /// variant does not compile until someone decides what undo does with
    /// it (ARCH-06 A6-4).
    pub(crate) fn undo_action(&self) -> crate::undo::UndoAction {
        use crate::undo::UndoAction;
        match self {
            // Selecting an arrangement entry is pure UI state (the right-rail
            // inspector focus); it never mutates the project.
            Self::SelectEntry { .. } => UndoAction::Skip,
            // Every other entry edit mutates the section's drum arrangement.
            Self::AddEntry { .. }
            | Self::RemoveEntry { .. }
            | Self::MoveEntry { .. }
            | Self::SetEntryPattern { .. }
            | Self::SetEntryLength { .. }
            | Self::SetEntryFill { .. }
            | Self::DuplicateEntry { .. }
            | Self::FillToEnd { .. }
            | Self::TrimToFit { .. } => UndoAction::Record,
        }
    }
}
