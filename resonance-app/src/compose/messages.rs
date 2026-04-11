use resonance_audio::types::TrackId;
use resonance_music_theory::{
    BassStyle, Chord, ChordQuality, MelodyStyle, PitchClass, Scale,
};

use crate::compose::drumroll::DrumrollMessage;
use crate::compose::DeriveKind;
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
    OpenEditSectionDialog { definition_id: u64 },
    CancelEditSectionDialog,
    SetEditSectionName(String),
    SetEditSectionLength(String),
    ConfirmEditSection,
    CycleSectionColor { definition_id: u64 },

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

    // Instrument details panel in the Compose track area
    SelectInstrumentForDetails {
        track_id: TrackId,
    },
    ClearInstrumentDetails,

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

    // ---- Generate / derive ----
    /// Replace the section's chord list with a progression generated
    /// from the section's scale + generate_params. Does not bump the
    /// seed — callers who want a new progression should send
    /// `RerollProgression` instead.
    GenerateProgression { definition_id: u64 },

    /// Bump the progression seed and regenerate. Cascade: any derived
    /// clips already in place for this section are refreshed.
    RerollProgression { definition_id: u64 },

    /// Set the target chord count for the next generated progression.
    SetGenerateChordCount { definition_id: u64, chord_count: u32 },
    /// Set beats per chord for the next generated progression.
    SetGenerateBeatsPerChord { definition_id: u64, beats_per_chord: u32 },
    /// Toggle seventh chords in the next generated progression.
    SetGenerateSeventhChords { definition_id: u64, seventh_chords: bool },

    /// Change bass style on a section's generate params.
    SetBassStyle { definition_id: u64, style: BassStyle },
    /// Change melody style on a section's generate params.
    SetMelodyStyle { definition_id: u64, style: MelodyStyle },

    /// Derive MIDI clips for one role on every placement of this section.
    DerivePart { definition_id: u64, kind: DeriveKind },
    /// Derive pad, bass and lead parts in one shot.
    DeriveAllParts { definition_id: u64 },

    /// Set or clear a track's arrangement role.
    SetTrackRole {
        track_id: TrackId,
        role: Option<TrackRole>,
    },
}
