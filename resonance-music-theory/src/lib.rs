pub mod chord;
pub mod derive;
pub mod fretboard;
pub mod generator;
pub mod pitch;
pub mod progression;
mod rng;
pub mod scale;
pub mod voicing;

pub use chord::{Chord, ChordQuality};
pub use derive::{
    derive_bass, derive_melody, derive_pad, BassParams, BassStyle, ContourPreference,
    GeneratedNote, MelodyParams, MelodyStyle, PadParams, TimedChord,
};
pub use generator::{
    Degree, GenContext, GenerateError, GeneratedChord, GeneratedMaterial, Generator, GeneratorSpec,
    MarkovTable, TableRegistry,
};
pub use pitch::PitchClass;
pub use progression::{
    degree_function, diatonic_chord, diatonic_triads, walk_progression, Function,
    ProgressionParams, TRANSITIONS,
};
pub use scale::{Mode, Scale};
pub use fretboard::{voicing as fretboard_voicing, FretboardVoicing, Tuning, GUITAR_6, GUITAR_8, BASS_4, BASS_5, ALL_TUNINGS};
pub use voicing::{close_voicing, nearest_midi_above, nearest_midi_to, voice_lead};
