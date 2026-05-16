use resonance_audio::types::TrackId;
use resonance_music_theory::{
    BassMotifMode, BassMotifPhrase, BassStyle, Chord, ChordQuality, ContourPreference, Degree,
    MelodyStyle, PitchClass, Scale, SyllableMode, VocalContour, VocalMood, VocalPov,
    VocalRhymeScheme, VocalSinger, VocalSingerMeiji, VocalStyle, VocalTimbre, VocalVoicebank,
    VoiceType,
};

use crate::compose::drumroll::DrumrollMessage;
use crate::compose::{DrumVoiceMode, LaneGeneratorKindTag, SelectedLane};
use crate::state::TrackRole;

#[derive(Debug, Clone)]
#[allow(dead_code)] // Drumroll(...) is retained for projects that load the old flat-pad model.
pub enum ComposeMessage {
    /// Drumroll view messages (toggle hit, generate euclidean, etc.).
    Drumroll(DrumrollMessage),

    /// Drum-groups messages — project-scoped group management plus the
    /// per-group generator knobs surfaced in the right rail.
    DrumGroups(DrumGroupsMessage),

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

    /// Set or clear a track's arrangement role.
    #[allow(dead_code)]
    SetTrackRole {
        track_id: TrackId,
        role: Option<TrackRole>,
    },

    /// SVS rendering completed off-thread — install the WAV as an audio
    /// clip on every placement the renderer was launched for. Boxed
    /// because the payload is large (samples vec).
    VocalAudioReady(Box<VocalAudioReadyData>),
    /// SVS render failed; surface the error to the user.
    VocalAudioFailed { error: String },
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
    /// finishes the user may have edited placements, but the snapshot
    /// keeps that drift from blowing up the install step.
    pub placements: Vec<(u64, u64)>,
    pub clip_name: String,
    /// Leading frames to skip on playback. The renderer adds a short
    /// AP padding to every segment so the model ramps in cleanly; the
    /// trim hides it from the timeline.
    pub trim_start_frames: u64,
    /// Trailing frames to skip on playback, mirroring `trim_start`.
    pub trim_end_frames: u64,
    /// Snapshot of `compose.vocal_render_epoch[(def, track)]` taken
    /// when the render was queued. The completion handler compares
    /// this to the current epoch and drops stale renders so the user
    /// doesn't end up with two audio clips stacked on the same lane.
    pub render_epoch: u64,
}

// ---------------------------------------------------------------------------
// Chord lane inspector sub-messages
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub enum ChordInspectorMsg {
    /// Select which Markov table to use.
    SetTable(String),
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
    /// Toggle the lock on a chord at the given index in generated_material.
    #[allow(dead_code)]
    ToggleLock(usize),
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

    // Drum euclidean (per-voice) — legacy flat-pad path. Kept around so the
    // update handler stays compilable; the new grouped UI uses
    // `DrumGroupsMessage` instead.
    #[allow(dead_code)]
    SetDrumVoiceMode {
        pad_index: usize,
        mode: DrumVoiceMode,
    },
    #[allow(dead_code)]
    SetDrumEuclidSteps {
        pad_index: usize,
        steps: u32,
    },
    #[allow(dead_code)]
    SetDrumEuclidHits {
        pad_index: usize,
        hits: u32,
    },
    #[allow(dead_code)]
    SetDrumEuclidRotation {
        pad_index: usize,
        rotation: i32,
    },

    /// Regenerate this lane from its generator spec + section chords.
    Regenerate,
}

// ---------------------------------------------------------------------------
// Drum groups
// ---------------------------------------------------------------------------

/// Messages targeting the project-scoped drum groups. Group definitions
/// (id, name, color, pads, grid, cycle, phase) live on
/// [`crate::compose::ComposeState::drum_groups`]. The manager modal and
/// the right-rail Drum generator both route through these.
#[derive(Debug, Clone)]
pub enum DrumGroupsMessage {
    /// Mark a group as the active selection for the right-rail generator
    /// and the lane's highlight stripe.
    SelectGroup { group_id: u64 },

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
    /// drum lane's cell click. `step` is relative to the visible 1-bar
    /// preview (0..BEATS_IN_LANE × grid) and gets mapped to the underlying
    /// pattern index modulo cycle inside the handler.
    TogglePadStep {
        group_id: u64,
        pad_index: usize,
        step: usize,
    },
}
