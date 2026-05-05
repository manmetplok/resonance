use serde::{Deserialize, Serialize};

use crate::voicing::{close_voicing, voice_lead};

use super::{GeneratedNote, TimedChord};

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct PadParams {
    /// Inclusive MIDI range the pad voicings must stay inside.
    pub register: (u8, u8),
    pub velocity: f32,
}

impl Default for PadParams {
    fn default() -> Self {
        Self {
            register: (52, 76), // E3..E5 — a safe "pad" register
            velocity: 0.7,
        }
    }
}

/// Sustained, voice-led chord voicings. The first chord is spelled as a
/// close voicing anchored to the register floor; subsequent chords are
/// voice-led from the previous voicing so common tones stay put and
/// moving voices move by the smallest interval.
pub fn derive_pad(
    chords: &[TimedChord],
    params: &PadParams,
    ticks_per_beat: u32,
) -> Vec<GeneratedNote> {
    if chords.is_empty() {
        return Vec::new();
    }
    let tpb = ticks_per_beat as u64;
    let mut out = Vec::new();

    // Seed voicing: close voicing at the register floor, then clamp any
    // voices above the register ceiling by dropping them an octave.
    let mut voicing: Vec<u8> = close_voicing(chords[0].chord, params.register.0)
        .into_iter()
        .map(|n| {
            let mut m = n;
            while m > params.register.1 && m >= 12 {
                m -= 12;
            }
            m
        })
        .collect();
    voicing.sort_unstable();

    for (i, tc) in chords.iter().enumerate() {
        if i > 0 {
            voicing = voice_lead(&voicing, &tc.chord.pitch_classes(), params.register);
        }
        let start_tick = tc.start_beat as u64 * tpb;
        let duration_ticks = tc.duration_beats as u64 * tpb;
        for &note in &voicing {
            out.push(GeneratedNote {
                note,
                velocity: params.velocity,
                start_tick,
                duration_ticks,
            });
        }
    }
    out
}
