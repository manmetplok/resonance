//! Scale-degree chord representation for generators.
//!
//! A [`Degree`] pairs a 1-indexed scale position with an explicit chord
//! quality, allowing generators to reason about progressions independently
//! of key. Call [`Degree::to_chord`] to project a degree into an absolute
//! [`Chord`] given a concrete [`Scale`].

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::chord::{Chord, ChordQuality};
use crate::scale::Scale;

/// A chord expressed as a scale degree with an explicit quality.
///
/// `root` is a 1-indexed diatonic position (1 = tonic through 7 = leading
/// tone). `flat` lowers the diatonic root by one semitone, producing
/// borrowed chords like bVI and bVII. `quality` is carried verbatim -- it
/// is *not* derived from the scale, so Markov tables can specify
/// non-diatonic qualities.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Degree {
    /// 1-indexed scale degree (1..=7).
    pub root: u8,
    /// Lower the diatonic root by one semitone (borrowed chords).
    pub flat: bool,
    /// Explicit chord quality.
    pub quality: ChordQuality,
    /// Inversion: 0 = root position, 1 = first inversion (third in the
    /// bass), 2 = second inversion (fifth in the bass), 3 = third
    /// inversion (seventh in the bass; seventh chords only). Projected
    /// as a slash bass by [`Degree::to_chord`]. `serde(default)` keeps
    /// material persisted before this field existed valid.
    #[serde(default)]
    pub inversion: u8,
}

// Manual Ord implementation so we can sort candidate lists for deterministic
// sampling without requiring Ord on ChordQuality (which lives in a different
// module that we don't want to modify).
impl PartialOrd for Degree {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Degree {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.root
            .cmp(&other.root)
            .then(self.flat.cmp(&other.flat))
            .then(self.quality.suffix().cmp(other.quality.suffix()))
            .then(self.inversion.cmp(&other.inversion))
    }
}

impl Degree {
    /// Root-position constructor used by the constant tables below.
    const fn deg(root: u8, flat: bool, quality: ChordQuality) -> Self {
        Self {
            root,
            flat,
            quality,
            inversion: 0,
        }
    }

    // === Major-key diatonic triads ===

    /// I -- major tonic.
    pub const I: Self = Self::deg(1, false, ChordQuality::Maj);
    /// ii -- minor supertonic.
    pub const II_MIN: Self = Self::deg(2, false, ChordQuality::Min);
    /// iii -- minor mediant.
    pub const III_MIN: Self = Self::deg(3, false, ChordQuality::Min);
    /// IV -- major subdominant.
    pub const IV: Self = Self::deg(4, false, ChordQuality::Maj);
    /// V -- major dominant.
    pub const V: Self = Self::deg(5, false, ChordQuality::Maj);
    /// vi -- minor submediant.
    pub const VI_MIN: Self = Self::deg(6, false, ChordQuality::Min);
    /// vii\u{b0} -- diminished leading tone.
    pub const VII_DIM: Self = Self::deg(7, false, ChordQuality::Dim);

    // === Borrowed / chromatic (from major-key perspective) ===

    /// bVI -- major flat submediant (borrowed from parallel minor).
    pub const FLAT_VI: Self = Self::deg(6, true, ChordQuality::Maj);
    /// bVII -- major flat subtonic (borrowed from Mixolydian / parallel minor).
    pub const FLAT_VII: Self = Self::deg(7, true, ChordQuality::Maj);

    // === Minor-key natural degrees ===
    //
    // These use `flat: false` because the scale itself (e.g. natural minor)
    // already places the root at the correct pitch. Use these with minor
    // scales; applying them to a major scale gives non-diatonic results.

    /// i -- minor tonic (natural in minor keys).
    pub const I_MIN: Self = Self::deg(1, false, ChordQuality::Min);
    /// III -- major mediant (natural in minor keys).
    pub const III_MAJ: Self = Self::deg(3, false, ChordQuality::Maj);
    /// iv -- minor subdominant (natural in minor keys).
    pub const IV_MIN: Self = Self::deg(4, false, ChordQuality::Min);
    /// VI -- major submediant (natural in minor keys; equivalent to bVI
    /// when viewed from the parallel major).
    pub const VI_MAJ: Self = Self::deg(6, false, ChordQuality::Maj);
    /// VII -- major subtonic (natural in minor keys; equivalent to bVII
    /// when viewed from the parallel major).
    pub const VII_MAJ: Self = Self::deg(7, false, ChordQuality::Maj);

    // === Seventh-chord variants ===

    /// I\u{0394}7 -- major seventh on the tonic.
    pub const I_MAJ7: Self = Self::deg(1, false, ChordQuality::Maj7);
    /// ii7 -- minor seventh on the supertonic.
    pub const II_MIN7: Self = Self::deg(2, false, ChordQuality::Min7);
    /// iii7 -- minor seventh on the mediant.
    pub const III_MIN7: Self = Self::deg(3, false, ChordQuality::Min7);
    /// IV\u{0394}7 -- major seventh on the subdominant.
    pub const IV_MAJ7: Self = Self::deg(4, false, ChordQuality::Maj7);
    /// V7 -- dominant seventh.
    pub const V_DOM7: Self = Self::deg(5, false, ChordQuality::Dom7);
    /// vi7 -- minor seventh on the submediant.
    pub const VI_MIN7: Self = Self::deg(6, false, ChordQuality::Min7);
    /// vii\u{f8}7 -- half-diminished seventh on the leading tone.
    pub const VII_HALF7: Self = Self::deg(7, false, ChordQuality::HalfDim7);

    /// The seven diatonic triads in major-key voicing order (I ii iii IV V vi vii°).
    pub const DIATONIC_TRIADS: [Self; 7] = [
        Self::I,
        Self::II_MIN,
        Self::III_MIN,
        Self::IV,
        Self::V,
        Self::VI_MIN,
        Self::VII_DIM,
    ];

    /// This degree with the given inversion (0 = root position).
    pub fn with_inversion(mut self, inversion: u8) -> Self {
        self.inversion = inversion;
        self
    }

    /// This degree in root position. Markov history conditioning and
    /// function-tag lookups normalize through this, so inverted degrees
    /// (decorations) still match root-position table entries.
    pub fn root_position(mut self) -> Self {
        self.inversion = 0;
        self
    }

    /// Is this the cadential 6/4 — the tonic triad in second inversion,
    /// placing scale degree 5 in the bass? Despite its tonic spelling it
    /// *functions* as a dominant decoration (it embellishes V and must
    /// resolve to it), which is how the phrase-model overlay treats it.
    pub fn is_cadential_six_four(self) -> bool {
        self.root == 1
            && !self.flat
            && self.inversion == 2
            && matches!(self.quality, ChordQuality::Maj | ChordQuality::Min)
    }

    /// Project this degree into an absolute [`Chord`] using the given scale.
    ///
    /// The scale's interval table determines the root pitch; the `flat`
    /// flag lowers it by a semitone; the stored `quality` is used as-is.
    /// A non-zero inversion becomes a slash bass on the chord (ii6 in C
    /// major → Dm/F).
    pub fn to_chord(self, scale: Scale) -> Chord {
        let intervals = scale.mode.intervals();
        let idx = ((self.root.saturating_sub(1)) % 7) as usize;
        let mut root_offset = intervals[idx] as i32;
        if self.flat {
            root_offset -= 1;
        }
        let root_pc = scale.root.transpose(root_offset);
        Chord::new(root_pc, self.quality).inverted(self.inversion)
    }
}

impl fmt::Display for Degree {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.flat {
            write!(f, "b")?;
        }
        let minor_like = matches!(
            self.quality,
            ChordQuality::Min
                | ChordQuality::Dim
                | ChordQuality::Min7
                | ChordQuality::MinMaj7
                | ChordQuality::Dim7
                | ChordQuality::HalfDim7
                | ChordQuality::Min6
        );
        let numeral = match self.root {
            1 => {
                if minor_like {
                    "i"
                } else {
                    "I"
                }
            }
            2 => {
                if minor_like {
                    "ii"
                } else {
                    "II"
                }
            }
            3 => {
                if minor_like {
                    "iii"
                } else {
                    "III"
                }
            }
            4 => {
                if minor_like {
                    "iv"
                } else {
                    "IV"
                }
            }
            5 => {
                if minor_like {
                    "v"
                } else {
                    "V"
                }
            }
            6 => {
                if minor_like {
                    "vi"
                } else {
                    "VI"
                }
            }
            7 => {
                if minor_like {
                    "vii"
                } else {
                    "VII"
                }
            }
            _ => "?",
        };
        write!(f, "{numeral}")?;
        // Quality split into a glyph (°, +, Δ, ø, sus…) and a numeric
        // figure ("7", "6") so figured-bass inversions can replace the
        // figure while keeping the glyph: V7 in first inversion prints
        // V65, vii° in first inversion prints vii°6.
        let (glyph, default_figure) = match self.quality {
            ChordQuality::Maj | ChordQuality::Min => ("", ""),
            ChordQuality::Dim => ("\u{b0}", ""),
            ChordQuality::Aug => ("+", ""),
            ChordQuality::Maj7 | ChordQuality::MinMaj7 => ("\u{0394}", "7"),
            ChordQuality::Min7 | ChordQuality::Dom7 => ("", "7"),
            ChordQuality::Dim7 => ("\u{b0}", "7"),
            ChordQuality::HalfDim7 => ("\u{f8}", "7"),
            ChordQuality::Sus2 => ("sus2", ""),
            ChordQuality::Sus4 => ("sus4", ""),
            ChordQuality::Maj6 | ChordQuality::Min6 => ("", "6"),
            ChordQuality::Add9 => ("add9", ""),
        };
        // Figured bass: triads 6 / 64, seventh chords 65 / 43 / 42.
        let figure = match (self.quality.intervals().len(), self.inversion) {
            (_, 0) => None,
            (3, 1) => Some("6"),
            (3, 2) => Some("64"),
            (4, 1) => Some("65"),
            (4, 2) => Some("43"),
            (4, 3) => Some("42"),
            _ => None,
        };
        write!(f, "{glyph}{}", figure.unwrap_or(default_figure))?;
        Ok(())
    }
}
