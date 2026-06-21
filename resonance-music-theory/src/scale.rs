use crate::pitch::PitchClass;
use serde::{Deserialize, Serialize};
use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Mode {
    Chromatic,
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
    pub const ALL: [Mode; 10] = [
        Mode::Chromatic,
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
            Mode::Chromatic => &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11],
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
            Mode::Chromatic => "chromatic",
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

    /// Snaps a pitch (as a fractional MIDI note number) to the nearest in-scale
    /// degree, with a correction strength factor.
    ///
    /// - `pitch`: The input pitch as a fractional MIDI note number (e.g., 60.0 = C4,
    ///   60.5 = C#4, 60.05 ≈ 5 cents above C4).
    /// - `strength`: Correction strength in `[0.0, 1.0]`.
    ///   - 0.0: Returns the original pitch unchanged (identity).
    ///   - 1.0: Returns the pitch snapped exactly to the nearest scale degree.
    ///   - Intermediate values: Linear interpolation between original and snapped.
    ///
    /// For `Mode::Chromatic`, every semitone is in the scale, so this is a no-op
    /// (always returns the original pitch).
    ///
    /// # Tie-breaking
    /// When the input pitch is equidistant between two scale degrees, the
    /// higher degree is chosen.
    pub fn snap_pitch(&self, pitch: f32, strength: f32) -> f32 {
        // Clamp strength to [0.0, 1.0] for safety
        let strength = strength.clamp(0.0, 1.0);

        // For chromatic scale, every note is valid - no snapping needed
        if self.mode == Mode::Chromatic {
            return pitch;
        }

        // For strength 0, return the original pitch (identity)
        if strength == 0.0 {
            return pitch;
        }

        // For strength 1, return the snapped pitch
        if strength == 1.0 {
            return self.snap_pitch_hard(pitch);
        }

        // For intermediate strength, interpolate
        let snapped = self.snap_pitch_hard(pitch);
        pitch + (snapped - pitch) * strength
    }

    /// Internal helper: snaps a pitch to the nearest in-scale degree without
    /// considering strength (always hard snap).
    fn snap_pitch_hard(&self, pitch: f32) -> f32 {
        // Find the nearest integer MIDI note in the scale
        let pitch_floor = pitch.floor() as i32;
        
        // Search for the best match in a window around the pitch
        // The scale repeats every 12 semitones, so we check notes in [pitch_floor-6, pitch_floor+6]
        let mut best_note: i32 = pitch_floor;
        let mut best_distance = f32::INFINITY;
        
        // Check notes in a range around the input pitch
        for offset in -6..=6 {
            let candidate = pitch_floor + offset;
            if self.contains(candidate as u8) {
                let distance = (pitch - candidate as f32).abs();
                // Use <= for tie-breaking: prefer higher notes
                if distance < best_distance || (distance == best_distance && candidate > best_note) {
                    best_distance = distance;
                    best_note = candidate;
                }
            }
        }
        
        best_note as f32
    }
}

impl fmt::Display for Scale {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} {}", self.root, self.mode)
    }
}
