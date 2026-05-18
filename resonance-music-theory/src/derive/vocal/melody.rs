//! Melody-side helpers and post-processing for the vocal generator.
//!
//! The actual per-syllable walk lives in `super::style` and runs through
//! `walk_with_profile`. This module holds:
//!   - the public `count_syllables` helper used by the SVS pipeline and
//!     by `VocalContext`,
//!   - the motif re-skin pass (`apply_motif_pitches` + `motif_pitch`),
//!   - the post-walk `enforce_no_overlap` cleanup,
//!   - `vocal_phrase_spans` for the synth fill,
//!   - and the small chord/scale/contour helpers shared between motif
//!     application, the walker, and `VocalContext::build`.

use crate::scale::Scale;

use super::super::{GeneratedNote, TimedChord};
use super::params::VocalContour;
use super::params::VocalParams;
use super::style::{cap_interval, cadence_pitch, phrase_role};

/// Strip the syllable separator and count syllables in a lyric line. A
/// fallback for cases where `LyricLine::syllables` is 0.
pub fn count_syllables(text: &str) -> u32 {
    let dot_count = text.matches('\u{00B7}').count() as u32;
    // `n syllables = dot_count + word_count` is a reasonable approximation
    // for already-broken text; we add the dots to the word count.
    let word_count = text.split_whitespace().count() as u32;
    (dot_count + word_count).max(1)
}

/// Map a normalised time `t ∈ [0, 1]` to a unit pitch height according
/// to a contour shape. 0.0 = bottom of the range, 1.0 = top.
pub(super) fn contour_height(contour: VocalContour, t: f32) -> f32 {
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
pub(super) fn snap_to_scale(note: u8, scale: Option<Scale>, lo: u8, hi: u8) -> u8 {
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
pub(super) fn chord_at_beat(chords: &[TimedChord], beat: u32) -> Option<&TimedChord> {
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
pub(super) fn total_beats(chords: &[TimedChord]) -> u32 {
    chords
        .iter()
        .map(|c| c.start_beat + c.duration_beats)
        .max()
        .unwrap_or(0)
}

/// Replace each note's pitch with a motif-derived pitch (chord root in
/// the lane register + signed motif interval, snapped to scale and
/// clamped within an octave of the previous pitch). The terminal note
/// of every line keeps its style cadence landing so phrases still
/// resolve.
#[allow(clippy::too_many_arguments)]
pub(super) fn apply_motif_pitches(
    notes: &mut [GeneratedNote],
    motif_intervals: &[i8],
    line_syllables: &[u32],
    chords: &[TimedChord],
    section_beats: u32,
    scale: Option<Scale>,
    range: (u8, u8),
    tpb: u64,
) {
    if motif_intervals.is_empty() || notes.is_empty() {
        return;
    }
    let (lo, hi) = range;
    let centre = ((lo as u16 + hi as u16) / 2) as u8;
    let mut prev_pitch = snap_to_scale(centre, scale, lo, hi);
    let mut note_idx = 0usize;

    for (line_idx, &line_syl) in line_syllables.iter().enumerate() {
        if line_syl == 0 {
            continue;
        }
        let line_note_count = (line_syl as usize).min(notes.len() - note_idx);
        if line_note_count == 0 {
            break;
        }
        for s in 0..line_note_count {
            let n = &mut notes[note_idx + s];
            let beat = (n.start_tick / tpb) as u32;
            let beat_clamped = beat.min(section_beats.saturating_sub(1));
            let chord = chord_at_beat(chords, beat_clamped);
            let is_final = s + 1 == line_note_count;

            let raw = if is_final {
                cadence_pitch(phrase_role(line_idx), chord, scale, prev_pitch, range)
                    .unwrap_or_else(|| {
                        let interval = motif_intervals[s % motif_intervals.len()];
                        motif_pitch(interval, chord, lo, hi, prev_pitch, scale)
                    })
            } else {
                let interval = motif_intervals[s % motif_intervals.len()];
                motif_pitch(interval, chord, lo, hi, prev_pitch, scale)
            };
            let pitch = cap_interval(prev_pitch, raw, lo, hi, scale);
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

/// Group `notes` (one per syllable, in lyric order) into per-line
/// `(start_tick, end_tick)` phrase intervals using `params.draft` to
/// recover the lyric line boundaries. Each interval's start is the
/// earliest onset of any note in the line and its end is the latest
/// note's `start_tick + duration_ticks`. Lines with no syllables are
/// skipped.
///
/// Used by `MelodyParams::fill_vocal_gaps`: the synth fill needs to
/// know where the actual sung phrases sit, and the lyric line is the
/// authoritative phrase unit. Time-gap heuristics fail because the
/// vocal generator's `phrase_start_offset` can pull successive lines
/// into each other, leaving only a few-tick gap between them.
pub fn vocal_phrase_spans(
    notes: &[GeneratedNote],
    params: &VocalParams,
) -> Vec<(u64, u64)> {
    let line_syl: Vec<u32> = params
        .draft
        .iter()
        .map(|l| count_syllables(&l.text))
        .collect();
    let mut out = Vec::with_capacity(line_syl.len());
    let mut cursor = 0usize;
    for &n_syl in &line_syl {
        let n = (n_syl as usize).min(notes.len().saturating_sub(cursor));
        if n == 0 {
            continue;
        }
        let slice = &notes[cursor..cursor + n];
        let start = slice.iter().map(|x| x.start_tick).min().unwrap_or(0);
        let end = slice
            .iter()
            .map(|x| x.start_tick + x.duration_ticks)
            .max()
            .unwrap_or(start);
        out.push((start, end));
        cursor += n;
    }
    out
}

/// Final pass: each note's `start_tick + duration_ticks` must not
/// exceed the next note's `start_tick`. The `phrase_start_offset`
/// (negative pickup / anacrusis) can shift line N+1 to start before
/// line N's terminal sustain ends, which previously surfaced as
/// "doubled" notes — the SVS pipeline indexes phonemes by note slot,
/// so an overlap means two syllables claim the same time window and
/// the second one's pitch fights the first's tail.
///
/// We compute the time order via a permutation (instead of sorting
/// the notes themselves) so the original lyric order survives — the
/// app's `vocal_phrase_spans` walks notes in lyric order to recover
/// per-line phrase intervals, and a sort would mix lines together
/// when `phrase_start_offset` shifts a later line back into an
/// earlier one's tail. We trim each note's duration to leave at
/// least `tpb / 16` (a 64th note) of silence into the next-in-time
/// note's onset.
pub(super) fn enforce_no_overlap(notes: &mut [GeneratedNote], tpb: u64) {
    if notes.len() < 2 {
        return;
    }
    let mut order: Vec<usize> = (0..notes.len()).collect();
    order.sort_by_key(|&i| notes[i].start_tick);
    let min_gap = (tpb / 16).max(1);
    for w in order.windows(2) {
        let (cur_idx, next_idx) = (w[0], w[1]);
        let next_start = notes[next_idx].start_tick;
        let cur_start = notes[cur_idx].start_tick;
        let cur_end = cur_start + notes[cur_idx].duration_ticks;
        if cur_end + min_gap > next_start {
            let new_dur = next_start.saturating_sub(cur_start).saturating_sub(min_gap);
            notes[cur_idx].duration_ticks = new_dur.max(1);
        }
    }
}

/// Adopt the chord root + quality of the first chord as a coarse scale
/// guess when the caller doesn't pass one explicitly. Used by
/// `derive_vocal` for its in-line snapping when `stay_in_scale` is set.
pub(super) fn scale_from_chords(chords: &[TimedChord]) -> Option<Scale> {
    use crate::scale::Mode;
    chords.first().map(|c| {
        let mode = match c.chord.quality {
            crate::chord::ChordQuality::Min | crate::chord::ChordQuality::Min7 => Mode::Minor,
            _ => Mode::Major,
        };
        Scale::new(c.chord.root, mode)
    })
}
