use resonance_audio::types::TrackId;
use resonance_music_theory::{
    BassMotifMode, BassMotifPhrase, BassStyle, Chord, ChordQuality, ContourPreference, Degree,
    MelodyStyle, PitchClass, Scale,
};

use crate::compose::drumroll::DrumrollMessage;
use crate::compose::{DrumVoiceMode, LaneGeneratorKindTag, SelectedLane};
use crate::state::TrackRole;

#[derive(Debug, Clone)]
pub enum ComposeMessage {
    /// Drumroll view messages (toggle hit, generate euclidean, etc.).
    Drumroll(DrumrollMessage),

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

    // Pad
    SetPadRegisterLow(u8),
    SetPadRegisterHigh(u8),
    SetPadVelocity(f32),

    // Drum euclidean (per-voice)
    SetDrumVoiceMode {
        pad_index: usize,
        mode: DrumVoiceMode,
    },
    SetDrumEuclidSteps {
        pad_index: usize,
        steps: u32,
    },
    SetDrumEuclidHits {
        pad_index: usize,
        hits: u32,
    },
    SetDrumEuclidRotation {
        pad_index: usize,
        rotation: i32,
    },

    /// Regenerate this lane from its generator spec + section chords.
    Regenerate,
}
