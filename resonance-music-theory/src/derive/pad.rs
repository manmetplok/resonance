use serde::{Deserialize, Serialize};

use crate::satb::satb_voicings;
use crate::scale::Scale;

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

/// Sustained, SATB-style voiced harmony. Chords render as voiced parts
/// rather than parallel block stacks: the bass is planned first, the
/// top voice is planned backwards from the cadence with correct
/// tendency-tone resolutions, and the inner voices take the nearest
/// chord tones — with parallel fifths/octaves forbidden, the leading
/// tone and chordal sevenths never doubled, and contrary motion
/// preferred against a rising 4→5 bass (see `crate::satb`).
///
/// `scale` enables the key-dependent rules; pass `None` to voice-lead
/// without leading-tone/tonic awareness. Registers narrower than 16
/// semitones drop from four voices to three so close voicings still
/// fit.
pub fn derive_pad(
    chords: &[TimedChord],
    scale: Option<Scale>,
    params: &PadParams,
    ticks_per_beat: u32,
) -> Vec<GeneratedNote> {
    if chords.is_empty() {
        return Vec::new();
    }
    let tpb = ticks_per_beat as u64;
    let progression: Vec<_> = chords.iter().map(|tc| tc.chord).collect();
    let voicings = satb_voicings(&progression, scale, params.register);

    let mut out = Vec::new();
    for (tc, voicing) in chords.iter().zip(voicings.iter()) {
        let start_tick = tc.start_beat as u64 * tpb;
        let duration_ticks = tc.duration_beats as u64 * tpb;
        for &note in voicing {
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
