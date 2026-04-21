use crate::pitch::PitchClass;
use serde::{Deserialize, Serialize};
use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ChordQuality {
    Maj,
    Min,
    Dim,
    Aug,
    Maj7,
    Min7,
    Dom7,
    MinMaj7,
    Dim7,
    HalfDim7,
    Sus2,
    Sus4,
    Maj6,
    Min6,
    Add9,
}

impl ChordQuality {
    pub const ALL: [ChordQuality; 15] = [
        ChordQuality::Maj,
        ChordQuality::Min,
        ChordQuality::Dim,
        ChordQuality::Aug,
        ChordQuality::Maj7,
        ChordQuality::Min7,
        ChordQuality::Dom7,
        ChordQuality::MinMaj7,
        ChordQuality::Dim7,
        ChordQuality::HalfDim7,
        ChordQuality::Sus2,
        ChordQuality::Sus4,
        ChordQuality::Maj6,
        ChordQuality::Min6,
        ChordQuality::Add9,
    ];

    pub fn suffix(self) -> &'static str {
        match self {
            ChordQuality::Maj => "",
            ChordQuality::Min => "m",
            ChordQuality::Dim => "dim",
            ChordQuality::Aug => "aug",
            ChordQuality::Maj7 => "maj7",
            ChordQuality::Min7 => "m7",
            ChordQuality::Dom7 => "7",
            ChordQuality::MinMaj7 => "mMaj7",
            ChordQuality::Dim7 => "dim7",
            ChordQuality::HalfDim7 => "m7b5",
            ChordQuality::Sus2 => "sus2",
            ChordQuality::Sus4 => "sus4",
            ChordQuality::Maj6 => "6",
            ChordQuality::Min6 => "m6",
            ChordQuality::Add9 => "add9",
        }
    }

    /// Semitone intervals above the root (0 is always implicit as the root).
    pub fn intervals(self) -> &'static [u8] {
        match self {
            ChordQuality::Maj => &[0, 4, 7],
            ChordQuality::Min => &[0, 3, 7],
            ChordQuality::Dim => &[0, 3, 6],
            ChordQuality::Aug => &[0, 4, 8],
            ChordQuality::Maj7 => &[0, 4, 7, 11],
            ChordQuality::Min7 => &[0, 3, 7, 10],
            ChordQuality::Dom7 => &[0, 4, 7, 10],
            ChordQuality::MinMaj7 => &[0, 3, 7, 11],
            ChordQuality::Dim7 => &[0, 3, 6, 9],
            ChordQuality::HalfDim7 => &[0, 3, 6, 10],
            ChordQuality::Sus2 => &[0, 2, 7],
            ChordQuality::Sus4 => &[0, 5, 7],
            ChordQuality::Maj6 => &[0, 4, 7, 9],
            ChordQuality::Min6 => &[0, 3, 7, 9],
            ChordQuality::Add9 => &[0, 4, 7, 14],
        }
    }
}

impl fmt::Display for ChordQuality {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.suffix())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Chord {
    pub root: PitchClass,
    pub quality: ChordQuality,
    pub bass: Option<PitchClass>,
}

impl Chord {
    pub fn new(root: PitchClass, quality: ChordQuality) -> Self {
        Self {
            root,
            quality,
            bass: None,
        }
    }

    pub fn with_bass(mut self, bass: PitchClass) -> Self {
        self.bass = Some(bass);
        self
    }

    /// Pitch classes of the chord tones, including the root and any added extensions.
    pub fn pitch_classes(&self) -> Vec<PitchClass> {
        self.quality
            .intervals()
            .iter()
            .map(|&iv| self.root.transpose(iv as i32))
            .collect()
    }
}

impl fmt::Display for Chord {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}{}", self.root, self.quality)?;
        if let Some(bass) = self.bass {
            write!(f, "/{}", bass)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_cmaj7() {
        let c = Chord::new(PitchClass::C, ChordQuality::Maj7);
        assert_eq!(c.to_string(), "Cmaj7");
    }

    #[test]
    fn display_slash() {
        let c = Chord::new(PitchClass::G, ChordQuality::Maj).with_bass(PitchClass::B);
        assert_eq!(c.to_string(), "G/B");
    }

    #[test]
    fn display_minor() {
        let c = Chord::new(PitchClass::A, ChordQuality::Min);
        assert_eq!(c.to_string(), "Am");
    }

    #[test]
    fn c_major_pitch_classes() {
        let c = Chord::new(PitchClass::C, ChordQuality::Maj);
        assert_eq!(
            c.pitch_classes(),
            vec![PitchClass::C, PitchClass::E, PitchClass::G]
        );
    }

    #[test]
    fn d_minor7_pitch_classes() {
        let c = Chord::new(PitchClass::D, ChordQuality::Min7);
        assert_eq!(
            c.pitch_classes(),
            vec![PitchClass::D, PitchClass::F, PitchClass::A, PitchClass::C]
        );
    }
}
