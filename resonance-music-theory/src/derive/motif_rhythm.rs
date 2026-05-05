// ---------------------------------------------------------------------------
// Drum motif rhythm
// ---------------------------------------------------------------------------

use crate::rng::XorShift;

use super::motif_engine::build_motif;
use super::motif_source::{manual_motif_to_motif_notes, MotifSource};
use super::TimedChord;

/// One onset in a motif-derived rhythm. Pitch is the caller's
/// responsibility — drum lanes substitute their own pad note. Accent
/// flags propagate from the motif so a downstream renderer can lift
/// accented hits' velocity without re-deriving the motif.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RhythmHit {
    pub start_tick: u64,
    pub duration_ticks: u64,
    pub accent: bool,
}

/// Project the section-shared motif onto the chord progression as a
/// pure rhythm: tile the motif's duration ratios across each chord
/// (same logic as the melody / bass renderers) and emit one
/// [`RhythmHit`] per onset.
///
/// The output rhythm is identical across chords in the progression
/// because we always use the `Identity` transform. That keeps the
/// generated drum part rhythmically locked to whatever the user is
/// playing on the melody / bass lanes — pressing a key that bumps the
/// motif seed re-derives every motif-driven lane the same way.
///
/// This function does no pitch work, so it doesn't need the section's
/// scale. Drum lanes route the hits through their pad note + velocity
/// at clip-build time.
pub fn derive_motif_rhythm(
    chords: &[TimedChord],
    motif_source: &MotifSource,
    ticks_per_beat: u32,
) -> Vec<RhythmHit> {
    if chords.is_empty() {
        return Vec::new();
    }
    let tpb = ticks_per_beat as u64;

    let motif = match motif_source {
        MotifSource::Generated(p) => {
            let mut motif_rng = XorShift::new(p.seed);
            build_motif(&mut motif_rng, chords[0].chord, None, p)
        }
        MotifSource::Manual { notes, .. } => manual_motif_to_motif_notes(notes, None),
    };
    if motif.is_empty() {
        return Vec::new();
    }

    let total_ratio: u64 = motif.iter().map(|n| n.duration_ratio as u64).sum();
    if total_ratio == 0 {
        return Vec::new();
    }
    // Drop notes shorter than a 32nd of a quarter — anything finer is
    // rhythmically unintelligible and tends to create accidental
    // double-hits when the math rounds.
    let min_duration = (tpb / 8).max(1);

    let mut out = Vec::new();
    for tc in chords {
        let chord_start = tc.start_beat as u64 * tpb;
        let chord_ticks = tc.duration_beats as u64 * tpb;
        if chord_ticks == 0 {
            continue;
        }

        let mut tick_cursor = chord_start;
        let chord_end = chord_start + chord_ticks;
        let mut motif_idx = 0;

        while tick_cursor < chord_end {
            let mn = &motif[motif_idx % motif.len()];
            let note_ticks = (chord_ticks * mn.duration_ratio as u64 / total_ratio).max(1);
            let remaining = chord_end - tick_cursor;
            let actual = note_ticks.min(remaining);
            if actual < min_duration {
                break;
            }
            if !mn.silent {
                out.push(RhythmHit {
                    start_tick: tick_cursor,
                    duration_ticks: actual,
                    accent: mn.accent,
                });
            }
            tick_cursor += actual;
            motif_idx += 1;
        }
    }
    out
}
