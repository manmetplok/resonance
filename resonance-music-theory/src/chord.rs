use crate::pitch::PitchClass;
use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;

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

    /// This chord with the given inversion expressed as a slash bass:
    /// 1 = third in the bass, 2 = fifth, 3 = seventh (when the quality
    /// has that many tones). Inversion 0 — or one past the quality's
    /// tone count — returns the chord unchanged.
    pub fn inverted(self, inversion: u8) -> Self {
        let intervals = self.quality.intervals();
        if inversion == 0 || (inversion as usize) >= intervals.len() {
            return self;
        }
        let bass = self.root.transpose(intervals[inversion as usize] as i32);
        self.with_bass(bass)
    }

    /// Pitch classes of the chord tones, including the root and any added
    /// extensions. Lazily computed — `collect()` if a `Vec` is needed.
    pub fn pitch_classes(&self) -> impl Iterator<Item = PitchClass> + Clone + 'static {
        let root = self.root;
        self.quality
            .intervals()
            .iter()
            .map(move |&iv| root.transpose(iv as i32))
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

/// Reason a chord symbol could not be parsed by [`parse_chord`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChordParseError {
    /// The input was empty or whitespace only.
    Empty,
    /// The root note could not be read (bad letter or accidental).
    BadRoot(String),
    /// The text after the root did not match any known chord quality.
    BadQuality(String),
    /// The slash-bass note (after `/`) could not be read.
    BadBass(String),
}

impl fmt::Display for ChordParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ChordParseError::Empty => f.write_str("empty chord symbol"),
            ChordParseError::BadRoot(s) => write!(f, "invalid chord root: {s:?}"),
            ChordParseError::BadQuality(s) => write!(f, "unknown chord quality: {s:?}"),
            ChordParseError::BadBass(s) => write!(f, "invalid slash-bass note: {s:?}"),
        }
    }
}

impl std::error::Error for ChordParseError {}

/// Read a leading note name (letter `A`–`G` plus any run of accidentals)
/// from `s`, returning the resulting pitch class and the unconsumed tail.
/// Accepts ASCII `#`/`b`, the Unicode `♯`/`♭`, and `x`/`𝄪`/`𝄫` for double
/// accidentals.
fn parse_pitch_class_prefix(s: &str) -> Option<(PitchClass, &str)> {
    let mut chars = s.char_indices();
    let base = match chars.next()?.1 {
        'C' => 0,
        'D' => 2,
        'E' => 4,
        'F' => 5,
        'G' => 7,
        'A' => 9,
        'B' => 11,
        _ => return None,
    };
    let mut offset = 0i32;
    let mut end = 1; // root letter is always a single ASCII byte
    for (i, c) in chars {
        match c {
            '#' | '\u{266f}' => offset += 1,
            'b' | '\u{266d}' => offset -= 1,
            'x' | '\u{1d12a}' => offset += 2,
            '\u{1d12b}' => offset -= 2,
            _ => break,
        }
        end = i + c.len_utf8();
    }
    let semitone = (base + offset).rem_euclid(12) as u8;
    Some((PitchClass::from_semitone(semitone), &s[end..]))
}

/// Map the text following the root to a [`ChordQuality`]. Recognises every
/// `ChordQuality::suffix()` (so it round-trips with `Display`) plus the
/// common aliases (`m`/`-`, `Δ`, `°`, `ø`, `+`, …). An empty string is a
/// major triad.
fn parse_quality(s: &str) -> Result<ChordQuality, ChordParseError> {
    use ChordQuality::*;
    let q = match s {
        "" | "maj" | "M" | "Maj" => Maj,
        "m" | "min" | "-" => Min,
        "dim" | "\u{b0}" | "o" => Dim,
        "aug" | "+" => Aug,
        "maj7" | "M7" | "Maj7" | "\u{394}" | "\u{394}7" => Maj7,
        "m7" | "min7" | "-7" => Min7,
        "7" | "dom7" => Dom7,
        "mMaj7" | "mM7" | "minMaj7" | "-Maj7" => MinMaj7,
        "dim7" | "\u{b0}7" | "o7" => Dim7,
        "m7b5" | "\u{f8}" | "\u{f8}7" | "m7-5" | "min7b5" => HalfDim7,
        "sus2" => Sus2,
        "sus4" | "sus" => Sus4,
        "6" | "maj6" | "M6" => Maj6,
        "m6" | "min6" => Min6,
        "add9" => Add9,
        other => return Err(ChordParseError::BadQuality(other.to_string())),
    };
    Ok(q)
}

/// Parse a chord symbol such as `"C"`, `"F#m7"`, `"Bbmaj7"`, `"G7/B"` or
/// `"Db°7"` into a [`Chord`]. This is the inverse of [`Chord`]'s `Display`:
/// every chord round-trips through `Display` → `parse_chord` for every
/// [`ChordQuality`]. Returns a [`ChordParseError`] describing the first
/// unparseable component.
pub fn parse_chord(s: &str) -> Result<Chord, ChordParseError> {
    let s = s.trim();
    if s.is_empty() {
        return Err(ChordParseError::Empty);
    }

    // A slash bass (`/E`) is everything after the last `/`. Split it off
    // first so the remainder is just root + quality.
    let (head, bass) = match s.rsplit_once('/') {
        Some((head, bass_str)) => {
            let bass_str = bass_str.trim();
            match parse_pitch_class_prefix(bass_str) {
                Some((pc, "")) => (head.trim(), Some(pc)),
                _ => return Err(ChordParseError::BadBass(bass_str.to_string())),
            }
        }
        None => (s, None),
    };

    let (root, quality_str) =
        parse_pitch_class_prefix(head).ok_or_else(|| ChordParseError::BadRoot(head.to_string()))?;
    let quality = parse_quality(quality_str.trim())?;

    let mut chord = Chord::new(root, quality);
    if let Some(bass) = bass {
        chord = chord.with_bass(bass);
    }
    Ok(chord)
}

impl FromStr for Chord {
    type Err = ChordParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        parse_chord(s)
    }
}

