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
/// more rhythmically complex. The complexity knob's pattern-pool cap is
/// calibrated against this list — tresillo cells live in their own
/// gated pool below so adding them doesn't shift the cap.
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

/// Tresillo cells (Open Music Theory, rhythm and meter in pop music):
/// the 3+3+2 division of eight pulses, its rotations, and the double
/// tresillo 3+3+3+3+2+2 spanning sixteen pulses.
const TRESILLO_PATTERNS: &[&[u8]] = &[
    &[3, 3, 2],           // tresillo
    &[3, 2, 3],           // rotation
    &[2, 3, 3],           // rotation
    &[3, 3, 3, 3, 2, 2],  // double tresillo
];

/// Complexity floor below which tresillo cells never replace the base
/// pattern — low-complexity motifs keep the simple, even pool.
const TRESILLO_MIN_COMPLEXITY: f32 = 0.6;

/// Probability that a motif above the complexity floor swaps its base
/// rhythm pattern for a tresillo cell.
const TRESILLO_CHANCE: f32 = 0.30;

// --- Well-formed-line constants (Open Music Theory: melodic line rules) ---

/// Hard per-note interval clamp around the anchor pitch, in semitones.
const INTERVAL_CLAMP: i8 = 10;
/// Maximum total span of the motif (highest minus lowest pitch), in
/// semitones: a major 10th.
const MAX_RANGE: i16 = 16;
/// Allowed leap sizes in semitones: no tritone (6) and nothing past a
/// perfect 5th (7).
const LEAP_SIZES: [i8; 4] = [3, 4, 5, 7];
/// Probability that a step continues in the same direction as the
/// previous melodic move (step inertia).
const STEP_INERTIA: f32 = 0.62;
/// How strongly the line is pulled back toward the register center once
/// it strays past `REGISTER_EDGE` (melodic regression).
const EDGE_REGRESSION_BIAS: f32 = 0.30;
/// Distance from the anchor (in semitones) at which regression kicks in.
const REGISTER_EDGE: i8 = 6;
/// Maximum run of identical consecutive pitches.
const MAX_REPEAT_RUN: u8 = 2;
/// How many candidate draws to attempt before falling back to a
/// deterministic step toward the register center.
const CANDIDATE_ATTEMPTS: usize = 8;

/// Pick a melodic direction with step inertia (bias toward continuing
/// `last_dir`) and melodic regression (bias back toward the anchor when
/// the line sits near a register edge).
fn choose_direction(rng: &mut XorShift, last_dir: i8, current: i8) -> i8 {
    let mut p_up = if last_dir > 0 {
        STEP_INERTIA
    } else if last_dir < 0 {
        1.0 - STEP_INERTIA
    } else {
        0.5
    };
    if current >= REGISTER_EDGE {
        p_up = (p_up - EDGE_REGRESSION_BIAS).max(0.05);
    } else if current <= -REGISTER_EDGE {
        p_up = (p_up + EDGE_REGRESSION_BIAS).min(0.95);
    }
    if rng.next_f32() < p_up {
        1
    } else {
        -1
    }
}

/// Line rules for one melodic move: no tritone (6 semitones), no leap
/// past a perfect 5th (7), at most `MAX_REPEAT_RUN` identical pitches in
/// a row, and a total motif span of at most `MAX_RANGE` semitones.
fn is_legal_move(prev: i8, next: i8, repeat_run: u8, min_iv: i8, max_iv: i8) -> bool {
    let delta = (i16::from(next) - i16::from(prev)).abs();
    if delta == 6 || delta > 7 {
        return false;
    }
    if delta == 0 && repeat_run >= MAX_REPEAT_RUN {
        return false;
    }
    let new_min = i16::from(min_iv.min(next));
    let new_max = i16::from(max_iv.max(next));
    new_max - new_min <= MAX_RANGE
}

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
    // (more complex) patterns. `floor` (not `ceil`) so low complexity
    // genuinely caps the pool at the simple patterns — `ceil` admitted
    // one pattern past what the complexity knob asked for.
    let max_pattern = (motif.complexity * (RHYTHM_PATTERNS.len() - 1) as f32).floor() as usize;
    let pattern_idx = rng.next_range(max_pattern.max(1) + 1).min(RHYTHM_PATTERNS.len() - 1);
    let mut rhythm = RHYTHM_PATTERNS[pattern_idx];
    // Tresillo gate: complex motifs occasionally swap the base pattern
    // for a 3+3+2 cell (or a rotation / the double tresillo). Drawn
    // after the base pick so low-complexity motifs consume the same
    // RNG stream as before the tresillo pool existed.
    if motif.complexity >= TRESILLO_MIN_COMPLEXITY && rng.next_f32() < TRESILLO_CHANCE {
        rhythm = TRESILLO_PATTERNS[rng.next_range(TRESILLO_PATTERNS.len())];
    }

    // Build interval contour.
    let chord_intervals = chord_tone_intervals(&chord);
    let has_scale = scale.is_some();
    let mut notes = Vec::with_capacity(len);
    let mut current_interval: i8 = 0;
    // Direction of the last non-repeated move (0 = none yet).
    let mut last_dir: i8 = 0;
    // Run of identical consecutive pitches ending at the current note.
    let mut repeat_run: u8 = 1;
    // Running pitch extremes, for the total-range rule.
    let mut min_iv: i8 = 0;
    let mut max_iv: i8 = 0;

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

        // Choose: step, leap, or repeat. Draw candidates until one obeys
        // the line rules; the deterministic fallback (a half step toward
        // the register center) is always legal.
        let mut chosen: Option<i8> = None;
        for _ in 0..CANDIDATE_ATTEMPTS {
            let roll = rng.next_f32();
            let repeat_chance = 0.11;
            let step_chance = 1.0 - motif.leap_chance - repeat_chance;

            let candidate = if roll < repeat_chance {
                current_interval
            } else if roll < repeat_chance + step_chance {
                let step_size = if rng.next_f32() < 0.6 { 1 } else { 2 };
                let dir = choose_direction(rng, last_dir, current_interval);
                let candidate = current_interval + dir * step_size;
                if has_scale {
                    candidate
                } else {
                    snap_to_chord_interval(candidate, &chord_intervals)
                }
            } else {
                let leap_size = LEAP_SIZES[rng.next_range(LEAP_SIZES.len())];
                let dir = choose_direction(rng, last_dir, current_interval);
                let candidate = current_interval + dir * leap_size;
                if has_scale {
                    candidate
                } else {
                    snap_to_chord_interval(candidate, &chord_intervals)
                }
            };

            // Validate the post-snap, post-clamp value: snapping and
            // clamping can both turn a legal draw into a tritone.
            let candidate = candidate.clamp(-INTERVAL_CLAMP, INTERVAL_CLAMP);
            if is_legal_move(current_interval, candidate, repeat_run, min_iv, max_iv) {
                chosen = Some(candidate);
                break;
            }
        }
        let new_interval = chosen.unwrap_or({
            if current_interval > 0 {
                current_interval - 1
            } else {
                current_interval + 1
            }
        });

        if new_interval == current_interval {
            repeat_run += 1;
        } else {
            repeat_run = 1;
            last_dir = if new_interval > current_interval { 1 } else { -1 };
        }
        current_interval = new_interval;
        min_iv = min_iv.min(current_interval);
        max_iv = max_iv.max(current_interval);

        notes.push(MotifNote {
            interval: current_interval,
            duration_ratio,
            accent,
            silent: false,
        });
    }

    snap_last_note_to_chord(&mut notes, &chord_intervals);

    notes
}

/// Snap the motif's final note to a chord tone — but never at the cost of
/// the line rules. Considers every chord-tone interval in the octave
/// around the final pitch, keeps only candidates whose closing move stays
/// legal, and picks the nearest one. If no candidate qualifies, the final
/// note keeps its (already legal) unsnapped pitch.
fn snap_last_note_to_chord(notes: &mut [MotifNote], chord_intervals: &[i8]) {
    let Some(last_idx) = notes.len().checked_sub(1) else {
        return;
    };
    if last_idx == 0 || chord_intervals.is_empty() {
        return;
    }
    let prev = notes[last_idx - 1].interval;
    // Run of identical consecutive pitches ending at the penultimate note.
    let mut run: u8 = 1;
    for w in (1..last_idx).rev() {
        if notes[w].interval == notes[w - 1].interval {
            run += 1;
        } else {
            break;
        }
    }
    // Pitch extremes of everything except the final note.
    let mut pre_min = 0i8;
    let mut pre_max = 0i8;
    for note in &notes[..last_idx] {
        pre_min = pre_min.min(note.interval);
        pre_max = pre_max.max(note.interval);
    }

    let last = notes[last_idx].interval;
    let octave_base = i16::from(last) - i16::from(last.rem_euclid(12));
    let mut best: Option<(i16, i8)> = None;
    for &ci in chord_intervals {
        for octave_shift in [-12i16, 0, 12] {
            let candidate = octave_base + i16::from(ci) + octave_shift;
            if candidate.abs() > i16::from(INTERVAL_CLAMP) {
                continue;
            }
            let candidate = candidate as i8;
            if !is_legal_move(prev, candidate, run, pre_min, pre_max) {
                continue;
            }
            let dist = (i16::from(candidate) - i16::from(last)).abs();
            if best.is_none_or(|(best_dist, _)| dist < best_dist) {
                best = Some((dist, candidate));
            }
        }
    }
    if let Some((_, candidate)) = best {
        notes[last_idx].interval = candidate;
    }
}

/// Get the semitone intervals of a chord's pitch classes relative to
/// the root (e.g. major = [0, 4, 7]).
fn chord_tone_intervals(chord: &Chord) -> Vec<i8> {
    let root = chord.root.to_semitone() as i8;
    chord
        .pitch_classes()
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
    // Distance math in i16: chord_intervals should be 0..12, but an
    // out-of-range value would overflow i8 in the ±12 shifts (and
    // `.abs()` panics on i8::MIN).
    let norm = i16::from(interval.rem_euclid(12));
    let octave = i16::from(interval) - norm;
    let mut best = i16::from(chord_intervals[0]);
    let mut best_dist = 12i16;
    for &ci in chord_intervals {
        let ci = i16::from(ci);
        let dist = ((norm - ci).abs()).min((norm - ci + 12).abs()).min((norm - ci - 12).abs());
        if dist < best_dist {
            best_dist = dist;
            best = ci;
        }
    }
    (octave + best) as i8
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
        Transform::Syncopate => syncopate_motif(motif),
    }
}

/// Straight syncopation (OMT's canonical pop rhythm device): halve the
/// first of the cell's durations and shift every later note earlier by
/// that half, with the final note extended so the cell still spans the
/// same total time. In duration-ratio form (where the renderer derives
/// onsets from cumulative durations and normalizes the total) that is:
/// double every ratio, restore the first to its original value, and add
/// the freed half to the last. E.g. `[1, 1, 1, 1]` (four quarters)
/// becomes `[1, 2, 2, 3]` in eighths — onsets 0, 0.5, 1.5, 2.5.
///
/// The transform works at half the pattern's base division, so
/// quarter-based patterns syncopate at the eighth level and
/// eighth-based ones at the sixteenth level.
fn syncopate_motif(motif: &[MotifNote]) -> Vec<MotifNote> {
    if motif.len() < 2 {
        return motif.to_vec();
    }
    let first = motif[0].duration_ratio;
    let last_idx = motif.len() - 1;
    motif
        .iter()
        .enumerate()
        .map(|(i, note)| {
            let duration_ratio = if i == 0 {
                first.max(1)
            } else if i == last_idx {
                note.duration_ratio.saturating_mul(2).saturating_add(first).max(1)
            } else {
                note.duration_ratio.saturating_mul(2).max(1)
            };
            MotifNote {
                duration_ratio,
                ..*note
            }
        })
        .collect()
}
