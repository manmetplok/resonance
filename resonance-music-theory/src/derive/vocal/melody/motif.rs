//! The motif re-skin pass.
//!
//! One responsibility: replacing a walked phrase's pitches with a
//! motif's relative-interval contour, anchored on the chord root
//! nearest the previous pitch. The rhythm, dynamics and cadence
//! landings the style walker produced are left alone.

use crate::derive::vocal::style::{cadence_pitch, cap_interval, phrase_role};
use crate::derive::{GeneratedNote, TimedChord};
use crate::scale::Scale;

use super::primitives::{chord_at_beat, snap_to_scale};

/// Bundle of immutable inputs to [`apply_motif_pitches`]. Carries the
/// motif's interval pattern, the section's per-line syllable counts,
/// and the harmonic + register context the pitch picker needs. Held
/// together so callers can fan out a single `VocalContext` into one
/// `MotifPitchContext` rather than threading seven parallel args.
pub(in crate::derive::vocal) struct MotifPitchContext<'a> {
    pub(in crate::derive::vocal) motif_intervals: &'a [i8],
    pub(in crate::derive::vocal) line_syllables: &'a [u32],
    pub(in crate::derive::vocal) chords: &'a [TimedChord],
    pub(in crate::derive::vocal) section_beats: u32,
    pub(in crate::derive::vocal) scale: Option<Scale>,
    pub(in crate::derive::vocal) range: (u8, u8),
    pub(in crate::derive::vocal) tpb: u64,
}

/// Re-skin a vocal phrase's pitches with a motif interval pattern.
/// Non-terminal syllables follow the motif's relative-interval contour
/// (anchored on the chord root nearest the previous pitch and clamped
/// to the lane register, snapped to scale). The terminal note of every
/// line keeps its style cadence landing so phrases still resolve.
pub(in crate::derive::vocal) fn apply_motif_pitches(
    notes: &mut [GeneratedNote],
    ctx: &MotifPitchContext<'_>,
) {
    if ctx.motif_intervals.is_empty() || notes.is_empty() {
        return;
    }
    let (lo, hi) = ctx.range;
    let centre = ((lo as u16 + hi as u16) / 2) as u8;
    let mut prev_pitch = snap_to_scale(centre, ctx.scale, lo, hi);
    let mut note_idx = 0usize;

    for (line_idx, &line_syl) in ctx.line_syllables.iter().enumerate() {
        if line_syl == 0 {
            continue;
        }
        let line_note_count = (line_syl as usize).min(notes.len() - note_idx);
        if line_note_count == 0 {
            break;
        }
        for s in 0..line_note_count {
            let n = &mut notes[note_idx + s];
            let beat = (n.start_tick / ctx.tpb) as u32;
            let beat_clamped = beat.min(ctx.section_beats.saturating_sub(1));
            let chord = chord_at_beat(ctx.chords, beat_clamped);
            let is_final = s + 1 == line_note_count;

            let raw = if is_final {
                let role = phrase_role(line_idx, ctx.line_syllables.len());
                cadence_pitch(role, chord, ctx.scale, prev_pitch, ctx.range)
                    .unwrap_or_else(|| {
                        let interval = ctx.motif_intervals[s % ctx.motif_intervals.len()];
                        motif_pitch(interval, chord, lo, hi, prev_pitch, ctx.scale)
                    })
            } else {
                let interval = ctx.motif_intervals[s % ctx.motif_intervals.len()];
                motif_pitch(interval, chord, lo, hi, prev_pitch, ctx.scale)
            };
            let pitch = cap_interval(prev_pitch, raw, lo, hi, ctx.scale);
            n.note = pitch;
            prev_pitch = pitch;
        }
        note_idx += line_note_count;
        if note_idx >= notes.len() {
            break;
        }
    }
}

/// Anchor pitch + signed motif interval, snapped to scale and range.
/// The anchor is the chord root in the lane register nearest to the
/// previous pitch (so motif transposes follow the chord progression
/// and the line stays in tessitura).
fn motif_pitch(
    interval: i8,
    chord: Option<&TimedChord>,
    lo: u8,
    hi: u8,
    prev: u8,
    scale: Option<Scale>,
) -> u8 {
    let anchor = chord
        .map(|c| {
            let root_pc = c.chord.root.to_semitone() as i16;
            // Find the in-range MIDI note nearest `prev` whose pitch
            // class equals the chord root.
            (lo..=hi)
                .filter(|p| (*p as i16 - root_pc).rem_euclid(12) == 0)
                .min_by_key(|p| (*p as i16 - prev as i16).abs())
                .unwrap_or(prev)
        })
        .unwrap_or(prev);
    let candidate = (anchor as i16 + interval as i16).clamp(lo as i16, hi as i16) as u8;
    snap_to_scale(candidate, scale, lo, hi)
}
