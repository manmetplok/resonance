//! Small pitch/chord/contour primitives shared by the vocal passes.
//!
//! One responsibility: the stateless lookups every other pass builds on
//! — where a contour sits at time `t`, which scale tone is nearest a
//! pitch, which chord is sounding at a beat, how long the progression
//! is, and what scale to assume when the caller gives none. Nothing
//! here touches notes or the RNG.

use crate::derive::vocal::params::VocalContour;
use crate::derive::TimedChord;
use crate::scale::Scale;

/// Map a normalised time `t ∈ [0, 1]` to a unit pitch height according
/// to a contour shape. 0.0 = bottom of the range, 1.0 = top.
pub(in crate::derive::vocal) fn contour_height(contour: VocalContour, t: f32) -> f32 {
    use std::f32::consts::PI;
    let t = t.clamp(0.0, 1.0);
    match contour {
        VocalContour::Arch => (PI * t).sin().clamp(0.0, 1.0),
        VocalContour::Rise => 0.15 + 0.80 * t,
        VocalContour::Fall => 0.95 - 0.80 * t,
        VocalContour::Wave => 0.5 + 0.4 * (1.5 * 2.0 * PI * t).sin(),
        VocalContour::Flat => 0.5 + 0.05 * (8.0 * t).sin(),
    }
}

/// Snap a MIDI note to the nearest scale tone, scanning outward up to
/// 6 semitones. Falls back to the input when no scale tone is reachable.
pub(in crate::derive::vocal) fn snap_to_scale(
    note: u8,
    scale: Option<Scale>,
    lo: u8,
    hi: u8,
) -> u8 {
    let Some(scale) = scale else { return note };
    for d in 0..=6i16 {
        for &sign in &[1i16, -1] {
            let candidate = note as i16 + d * sign;
            if (lo as i16..=hi as i16).contains(&candidate)
                && scale.contains(candidate as u8)
            {
                return candidate as u8;
            }
        }
    }
    note
}

/// Find the chord active at a given beat. Returns the last chord whose
/// start ≤ beat. If none match (e.g. beat is before the first chord),
/// returns the first chord.
pub(in crate::derive::vocal) fn chord_at_beat(chords: &[TimedChord], beat: u32) -> Option<&TimedChord> {
    let mut active = chords.first();
    for c in chords {
        if c.start_beat <= beat {
            active = Some(c);
        }
    }
    active
}

/// Total beat span covered by the chord list — from beat 0 to the
/// furthest chord end.
pub(in crate::derive::vocal) fn total_beats(chords: &[TimedChord]) -> u32 {
    chords
        .iter()
        .map(|c| c.start_beat + c.duration_beats)
        .max()
        .unwrap_or(0)
}

/// Adopt the chord root + quality of the first chord as a coarse scale
/// guess when the caller doesn't pass one explicitly. Used by
/// `derive_vocal` for its in-line snapping when `stay_in_scale` is set.
pub(in crate::derive::vocal) fn scale_from_chords(chords: &[TimedChord]) -> Option<Scale> {
    use crate::scale::Mode;
    chords.first().map(|c| {
        let mode = match c.chord.quality {
            crate::chord::ChordQuality::Min | crate::chord::ChordQuality::Min7 => Mode::Minor,
            _ => Mode::Major,
        };
        Scale::new(c.chord.root, mode)
    })
}
