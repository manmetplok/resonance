//! Pure part generators: given a chord progression, produce MIDI notes
//! for a pad, bass line, or melody.
//!
//! The functions here do not depend on any DAW types. They take a
//! `TimedChord` list and return `GeneratedNote`s with ticks measured
//! from the start of the containing clip. The app crate is responsible
//! for converting between these and the engine's `MidiClip` / `MidiNote`.

use crate::chord::Chord;

mod bass;
mod cadence;
mod climax;
mod melody;
mod motif_bass;
mod motif_engine;
mod motif_rhythm;
mod motif_source;
mod pad;
mod vocal;

pub use bass::{derive_bass, BassMotifMode, BassMotifPhrase, BassParams, BassStyle};
pub use melody::{
    derive_melody, derive_melody_fill_vocal, ContourPreference, EmbellishmentStyle, MelodyParams,
    MelodyStyle,
};
pub use motif_bass::derive_bass_motif;
pub use motif_engine::{
    derive_motif_melody_with_section, motif_intervals, phrase_grammar_roles, section_climax_phrase,
    PhraseGrammarRole,
};
pub use motif_rhythm::{derive_motif_rhythm, RhythmHit};
pub use motif_source::{
    toggle_manual_motif_cell, ManualMotifCell, ManualMotifNote, MotifParams, MotifSource,
};
pub use pad::{derive_pad, PadParams};
pub use vocal::{
    count_syllables, derive_vocal, derive_vocal_with_meter, derive_vocal_with_motif,
    generate_lyrics, vocal_phrase_spans, LyricLine, SyllableMode, VocalContour, VocalMood,
    VocalParams, VocalParamsError, VocalPov, VocalRhymeScheme, VocalSinger, VocalSingerMeiji,
    VocalStyle, VocalTimbre, VocalVoicebank, VoiceType,
};

/// A chord positioned on the section's beat grid. Mirrors the app's
/// `ChordState` so callers don't have to take a dependency on the app
/// crate just to use these generators.
#[derive(Debug, Clone, Copy)]
pub struct TimedChord {
    pub chord: Chord,
    pub start_beat: u32,
    pub duration_beats: u32,
}

/// DAW-agnostic MIDI note. Matches `resonance_audio::types::MidiNote`
/// field-for-field; converted at the app boundary.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GeneratedNote {
    pub note: u8,
    pub velocity: f32,
    pub start_tick: u64,
    pub duration_ticks: u64,
}


