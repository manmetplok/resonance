//! Plugin-side pitch quantizer (ba todo #1078, doc #252 §3, doc #253):
//! snaps a per-grain effective transpose (base Pitch + random Spread,
//! in semitones) to integer semitones or to the degrees of a
//! `resonance-music-theory` scale before the grain latches it at spawn.
//!
//! Scale mode treats the transpose as an interval above the scale's
//! root: a transpose is allowed when root + transpose lands on a scale
//! degree, so the allowed lattice is the mode's interval set repeated
//! per octave (e.g. C major admits +0, +2, +4, +5, +7, +9, +11 st;
//! A minor admits +0, +2, +3, +5, +7, +8, +10 st). Everything here is
//! pure arithmetic over the scale's static interval tables — no
//! allocation, safe on the audio thread.

use resonance_music_theory::{Mode, PitchClass, Scale};

/// Quantization applied to the per-grain effective transpose at spawn
/// (the `pitch_quantize` parameter).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PitchQuantize {
    /// Transpose values pass through untouched.
    #[default]
    Off,
    /// Snap to the nearest integer semitone.
    Semitones,
    /// Snap to the nearest degree of the configured scale.
    Scale,
}

impl PitchQuantize {
    /// Map the `pitch_quantize` parameter value (0/1/2) to a mode.
    pub fn from_index(index: i32) -> Self {
        match index {
            1 => Self::Semitones,
            2 => Self::Scale,
            _ => Self::Off,
        }
    }
}

/// Map the `root` parameter (0–11 = C..B) to a pitch class.
pub fn root_from_index(index: i32) -> PitchClass {
    PitchClass::from_semitone(index.rem_euclid(12) as u8)
}

/// Map the `scale` parameter to a `resonance-music-theory` mode
/// (indexes [`Mode::ALL`], clamped).
pub fn mode_from_index(index: i32) -> Mode {
    Mode::ALL[(index.max(0) as usize).min(Mode::ALL.len() - 1)]
}

/// MIDI anchor used to evaluate transposes against a scale: the
/// transpose is applied to the scale root in a mid-range octave, so a
/// value is in-scale exactly when it is a scale degree above the root.
fn scale_anchor(scale: Scale) -> f32 {
    60.0 + scale.root.to_semitone() as f32
}

/// Quantize an effective transpose in (fractional) semitones.
///
/// * `Off` returns the value untouched.
/// * `Semitones` rounds to the nearest integer semitone (ties away
///   from zero, `f32::round`).
/// * `Scale` snaps to the nearest scale degree above/below the root
///   via [`Scale::snap_pitch`]; ties resolve upward (its documented
///   tie-break). `Mode::Chromatic` — a no-op in `snap_pitch` — is
///   treated as "every semitone allowed" and rounds like `Semitones`.
pub fn quantize_transpose(semitones: f32, quantize: PitchQuantize, scale: Scale) -> f32 {
    match quantize {
        PitchQuantize::Off => semitones,
        PitchQuantize::Semitones => semitones.round(),
        PitchQuantize::Scale => {
            if scale.mode == Mode::Chromatic {
                return semitones.round();
            }
            let anchor = scale_anchor(scale);
            scale.snap_pitch(anchor + semitones, 1.0) - anchor
        }
    }
}
