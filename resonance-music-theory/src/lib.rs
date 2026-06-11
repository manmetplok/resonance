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
    generate_lyrics, motif_intervals, phrase_grammar_roles, plan_motif_transforms,
    section_climax_phrase, toggle_manual_motif_cell,
    vocal_phrase_spans, BassMotifMode, BassMotifPhrase,
    BassParams, BassStyle, ContourPreference, EmbellishmentStyle, GeneratedNote, LyricLine,
    ComposedPair, ManualMotifCell, ManualMotifNote, MelodyParams, MelodyStyle, MotifParams,
    MotifSource, MotifTransform, PadParams,
    PhraseGrammarRole, RhythmHit, SequenceKind,
    SyllableMode, TimedChord, VocalContour, VocalMood, VocalParams, VocalParamsError, VocalPov,
    VocalRhymeScheme, VocalSinger, VocalSingerMeiji, VocalStyle, VocalTimbre, VocalVoicebank,
    VoiceType,
};
pub use generator::{
    Degree, GenContext, GenerateError, GeneratedChord, GeneratedMaterial, Generator, GeneratorSpec,
    HarmonicFunction, MarkovTable, SchemaKind, SplitChord, TableRegistry,
};
pub use pitch::{midi_note_name, midi_note_name_unicode, PitchClass};
pub use progression::{
    degree_function, diatonic_chord, diatonic_triads, walk_progression, Function,
    ProgressionParams, TRANSITIONS,
};
pub use scale::{Mode, Scale};
pub use fretboard::{
    voicing as fretboard_voicing, voicing_from as fretboard_voicing_from, FretboardVoicing,
    Tuning, ALL_TUNINGS, BASS_4, BASS_5, GUITAR_6, GUITAR_8, MAX_START_FRET, WINDOW_FRETS,
};
pub use voicing::{close_voicing, nearest_midi_above, nearest_midi_to, voice_lead};
