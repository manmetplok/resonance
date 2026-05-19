// Motif construction primitives: build a fresh motif cell, transform an
// existing one, and the chord-interval snapping used to bias notes
// toward chord tones during construction.

use crate::chord::Chord;
use crate::rng::XorShift;
use crate::scale::Scale;

use super::super::motif_source::MotifParams;
use super::types::{MotifNote, Transform};

/// Rhythm pattern library: each pattern is a list of duration ratios.
/// The ratios are scaled to fill the available time. Higher indices are
/// more rhythmically complex.
const RHYTHM_PATTERNS: &[&[u8]] = &[
    &[1, 1, 1, 1],       // steady
    &[2, 1, 1],           // long-short-short
    &[1, 1, 2],           // short-short-long
    &[1, 2, 1],           // short-long-short
    &[3, 1, 2, 2],        // dotted feel
    &[1, 1, 1, 1, 2],     // four eighths + quarter
    &[2, 1, 1, 2, 2],     // varied
    &[1, 1, 2, 1, 1],     // syncopated center
];

/// Build a motif: a short melodic cell of 2-6 notes with relative intervals
/// and a rhythmic pattern. Intervals are unbounded by lane register — each
/// lane clamps to its own register at render time, so two lanes built from
/// the same `MotifParams` and chord get identical interval shapes.
pub(in crate::derive) fn build_motif(
    rng: &mut XorShift,
    chord: Chord,
    scale: Option<Scale>,
    motif: &MotifParams,
) -> Vec<MotifNote> {
    let len = if motif.motif_len > 0 {
        (motif.motif_len as usize).clamp(2, 6)
    } else {
        (2.0 + motif.complexity * 4.0).round() as usize
    };

    // Pick a rhythm pattern. Higher complexity biases toward later
    // (more complex) patterns.
    let max_pattern = (motif.complexity * (RHYTHM_PATTERNS.len() - 1) as f32).ceil() as usize;
    let pattern_idx = rng.next_range(max_pattern.max(1) + 1).min(RHYTHM_PATTERNS.len() - 1);
    let rhythm = RHYTHM_PATTERNS[pattern_idx];

    // Build interval contour.
    let chord_intervals = chord_tone_intervals(&chord);
    let has_scale = scale.is_some();
    let mut notes = Vec::with_capacity(len);
    let mut current_interval: i8 = 0;

    for i in 0..len {
        let duration_ratio = rhythm[i % rhythm.len()];
        let accent = i == 0 || duration_ratio >= 2;

        if i == 0 {
            notes.push(MotifNote {
                interval: 0,
                duration_ratio,
                accent,
                silent: false,
            });
            continue;
        }

        // Choose: step, leap, or repeat.
        let roll = rng.next_f32();
        let repeat_chance = 0.11;
        let step_chance = 1.0 - motif.leap_chance - repeat_chance;

        let new_interval = if roll < repeat_chance {
            current_interval
        } else if roll < repeat_chance + step_chance {
            let step_size = if rng.next_f32() < 0.6 { 1 } else { 2 };
            let dir: i8 = if rng.next_f32() < 0.5 { 1 } else { -1 };
            let candidate = current_interval + dir * step_size;
            if has_scale {
                candidate
            } else {
                snap_to_chord_interval(candidate, &chord_intervals)
            }
        } else {
            let leap_size = 3 + (rng.next_f32() * 4.0) as i8;
            let dir: i8 = if rng.next_f32() < 0.5 { 1 } else { -1 };
            let candidate = current_interval + dir * leap_size;
            if has_scale {
                candidate
            } else {
                snap_to_chord_interval(candidate, &chord_intervals)
            }
        };

        current_interval = new_interval.clamp(-10, 10);

        notes.push(MotifNote {
            interval: current_interval,
            duration_ratio,
            accent,
            silent: false,
        });
    }

    if let Some(last) = notes.last_mut() {
        last.interval = snap_to_chord_interval(last.interval, &chord_intervals);
    }

    notes
}

/// Get the semitone intervals of a chord's pitch classes relative to
/// the root (e.g. major = [0, 4, 7]).
fn chord_tone_intervals(chord: &Chord) -> Vec<i8> {
    let root = chord.root.to_semitone() as i8;
    chord
        .pitch_classes()
        .iter()
        .map(|pc| {
            let diff = pc.to_semitone() as i8 - root;
            if diff < 0 { diff + 12 } else { diff }
        })
        .collect()
}

/// Snap an interval to the nearest chord-tone interval (mod 12).
fn snap_to_chord_interval(interval: i8, chord_intervals: &[i8]) -> i8 {
    if chord_intervals.is_empty() {
        return interval;
    }
    let norm = interval.rem_euclid(12);
    let octave = interval - norm;
    let mut best = chord_intervals[0];
    let mut best_dist = 12i8;
    for &ci in chord_intervals {
        let dist = ((norm - ci).abs()).min((norm - ci + 12).abs()).min((norm - ci - 12).abs());
        if dist < best_dist {
            best_dist = dist;
            best = ci;
        }
    }
    octave + best
}

/// Apply a transformation to a motif, returning a new motif.
pub(in crate::derive) fn transform_motif(motif: &[MotifNote], transform: Transform) -> Vec<MotifNote> {
    match transform {
        Transform::Identity => motif.to_vec(),
        Transform::TransposeUp(n) => motif
            .iter()
            .map(|note| MotifNote {
                interval: note.interval + n,
                ..*note
            })
            .collect(),
        Transform::TransposeDown(n) => motif
            .iter()
            .map(|note| MotifNote {
                interval: note.interval - n,
                ..*note
            })
            .collect(),
        Transform::Invert => motif
            .iter()
            .map(|note| MotifNote {
                interval: -note.interval,
                ..*note
            })
            .collect(),
        Transform::Retrograde => {
            let mut reversed = motif.to_vec();
            reversed.reverse();
            reversed
        }
        Transform::Augment => motif
            .iter()
            .map(|note| MotifNote {
                duration_ratio: note.duration_ratio.saturating_mul(2).max(1),
                ..*note
            })
            .collect(),
        Transform::Diminish => motif
            .iter()
            .map(|note| MotifNote {
                duration_ratio: (note.duration_ratio / 2).max(1),
                ..*note
            })
            .collect(),
        Transform::Fragment(n) => motif[..n.min(motif.len())].to_vec(),
    }
}
