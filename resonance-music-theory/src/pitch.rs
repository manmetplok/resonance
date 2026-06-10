use serde::{Deserialize, Serialize};
use std::fmt;

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum PitchClass {
    C = 0,
    Cs = 1,
    D = 2,
    Ds = 3,
    E = 4,
    F = 5,
    Fs = 6,
    G = 7,
    Gs = 8,
    A = 9,
    As = 10,
    B = 11,
}

impl PitchClass {
    pub const ALL: [PitchClass; 12] = [
        PitchClass::C,
        PitchClass::Cs,
        PitchClass::D,
        PitchClass::Ds,
        PitchClass::E,
        PitchClass::F,
        PitchClass::Fs,
        PitchClass::G,
        PitchClass::Gs,
        PitchClass::A,
        PitchClass::As,
        PitchClass::B,
    ];

    pub fn from_semitone(semitone: u8) -> Self {
        Self::ALL[(semitone % 12) as usize]
    }

    pub fn to_semitone(self) -> u8 {
        self as u8
    }

    pub fn transpose(self, semitones: i32) -> Self {
        let raw = (self as i32 + semitones).rem_euclid(12) as u8;
        Self::from_semitone(raw)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            PitchClass::C => "C",
            PitchClass::Cs => "C#",
            PitchClass::D => "D",
            PitchClass::Ds => "D#",
            PitchClass::E => "E",
            PitchClass::F => "F",
            PitchClass::Fs => "F#",
            PitchClass::G => "G",
            PitchClass::Gs => "G#",
            PitchClass::A => "A",
            PitchClass::As => "A#",
            PitchClass::B => "B",
        }
    }
}

impl fmt::Display for PitchClass {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Human-readable MIDI note name with ASCII sharps (e.g. `"C4"`,
/// `"F#3"`). Middle C (MIDI 60) renders as `"C4"`, matching the
/// convention used by keyboard plugins and most DAWs (note 0 is
/// `"C-1"`, note 127 is `"G9"`).
pub fn midi_note_name(note: u8) -> String {
    let octave = (note / 12) as i8 - 1;
    format!("{}{}", PitchClass::from_semitone(note).as_str(), octave)
}

/// Like [`midi_note_name`] but renders sharps with the Unicode sharp
/// sign (`♯`, U+266F) for UI labels (e.g. `"F♯3"`).
pub fn midi_note_name_unicode(note: u8) -> String {
    midi_note_name(note).replace('#', "\u{266f}")
}

