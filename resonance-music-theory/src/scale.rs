use crate::pitch::PitchClass;
use serde::{Deserialize, Serialize};
use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Mode {
    Major,
    Minor,
    Dorian,
    Phrygian,
    Lydian,
    Mixolydian,
    Locrian,
    HarmonicMinor,
    MelodicMinor,
}

impl Mode {
    pub const ALL: [Mode; 9] = [
        Mode::Major,
        Mode::Minor,
        Mode::Dorian,
        Mode::Phrygian,
        Mode::Lydian,
        Mode::Mixolydian,
        Mode::Locrian,
        Mode::HarmonicMinor,
        Mode::MelodicMinor,
    ];

    /// Semitone offsets of the scale degrees above the root.
    pub fn intervals(self) -> &'static [u8] {
        match self {
            Mode::Major => &[0, 2, 4, 5, 7, 9, 11],
            Mode::Minor => &[0, 2, 3, 5, 7, 8, 10],
            Mode::Dorian => &[0, 2, 3, 5, 7, 9, 10],
            Mode::Phrygian => &[0, 1, 3, 5, 7, 8, 10],
            Mode::Lydian => &[0, 2, 4, 6, 7, 9, 11],
            Mode::Mixolydian => &[0, 2, 4, 5, 7, 9, 10],
            Mode::Locrian => &[0, 1, 3, 5, 6, 8, 10],
            Mode::HarmonicMinor => &[0, 2, 3, 5, 7, 8, 11],
            Mode::MelodicMinor => &[0, 2, 3, 5, 7, 9, 11],
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Mode::Major => "major",
            Mode::Minor => "minor",
            Mode::Dorian => "dorian",
            Mode::Phrygian => "phrygian",
            Mode::Lydian => "lydian",
            Mode::Mixolydian => "mixolydian",
            Mode::Locrian => "locrian",
            Mode::HarmonicMinor => "harmonic minor",
            Mode::MelodicMinor => "melodic minor",
        }
    }
}

impl fmt::Display for Mode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Scale {
    pub root: PitchClass,
    pub mode: Mode,
}

impl Scale {
    pub fn new(root: PitchClass, mode: Mode) -> Self {
        Self { root, mode }
    }

    /// True if the given MIDI note number belongs to the scale.
    pub fn contains(&self, midi_note: u8) -> bool {
        let semitone = midi_note % 12;
        let root_semi = self.root.to_semitone();
        let diff = (semitone + 12 - root_semi) % 12;
        self.mode.intervals().contains(&diff)
    }
}

impl fmt::Display for Scale {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} {}", self.root, self.mode)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn c_major_contains_diatonic_notes() {
        let s = Scale::new(PitchClass::C, Mode::Major);
        // C D E F G A B across any octave
        for base in [0u8, 12, 24, 36, 60, 72, 84].iter() {
            assert!(s.contains(base + 0)); // C
            assert!(s.contains(base + 2)); // D
            assert!(s.contains(base + 4)); // E
            assert!(s.contains(base + 5)); // F
            assert!(s.contains(base + 7)); // G
            assert!(s.contains(base + 9)); // A
            assert!(s.contains(base + 11)); // B
            assert!(!s.contains(base + 1)); // C#
            assert!(!s.contains(base + 3)); // D#
        }
    }

    #[test]
    fn d_dorian_contains_expected() {
        let s = Scale::new(PitchClass::D, Mode::Dorian);
        // D dorian = D E F G A B C = all white keys starting from D
        let d4 = 62u8;
        for iv in [0, 2, 3, 5, 7, 9, 10] {
            assert!(s.contains(d4 + iv), "D dorian should contain {}", iv);
        }
        assert!(!s.contains(d4 + 1));
        assert!(!s.contains(d4 + 4));
    }

    #[test]
    fn display() {
        let s = Scale::new(PitchClass::C, Mode::Minor);
        assert_eq!(s.to_string(), "C minor");
        let s = Scale::new(PitchClass::D, Mode::Dorian);
        assert_eq!(s.to_string(), "D dorian");
    }
}
