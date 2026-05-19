pub mod chord;
pub mod derive;
pub mod fretboard;
pub mod g2p;
pub mod generator;
pub mod pitch;
pub mod progression;
mod rng;
pub mod scale;
pub mod voicing;

pub use chord::{Chord, ChordQuality};
pub use derive::{
    count_syllables, derive_bass, derive_bass_motif, derive_melody,
    derive_motif_melody_with_section, derive_motif_rhythm, derive_pad, derive_vocal,
    derive_melody_fill_vocal, derive_vocal_with_meter, derive_vocal_with_motif,
    generate_lyrics, motif_intervals, toggle_manual_motif_cell, vocal_phrase_spans,
    BassMotifMode, BassMotifPhrase,
    BassParams, BassStyle, ContourPreference, GeneratedNote, LyricLine, ManualMotifCell,
    ManualMotifNote, MelodyParams, MelodyStyle, MotifParams, MotifSource, PadParams, RhythmHit,
    SyllableMode, TimedChord, VocalContour, VocalMood, VocalParams, VocalParamsError, VocalPov,
    VocalRhymeScheme, VocalSinger, VocalSingerMeiji, VocalStyle, VocalTimbre, VocalVoicebank,
    VoiceType,
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
