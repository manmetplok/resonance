// Harmony post-processing: align raw motif pitches to the current chord
// (chord-tone on strong beats, scale-tone on weak beats) and enforce the
// Open Music Theory leap grammar across the realized phrase (leaps
// resolve by an opposite step instead of being erased by fill notes).

use crate::chord::Chord;
use crate::scale::Scale;

use super::super::bass::step_scale;
use super::super::motif_bass::chord_tones_in_register;
use super::super::GeneratedNote;

/// Align a MIDI note to the current harmony based on beat strength.
pub(in crate::derive) fn align_to_harmony(
    raw_midi: u8,
    beat_position: u64,
    tpb: u64,
    chord: Chord,
    scale: Option<Scale>,
    register: (u8, u8),
) -> u8 {
    let chord_tones = chord_tones_in_register(chord, register);
    if chord_tones.is_empty() {
        return raw_midi.clamp(register.0, register.1);
    }

    // Strong beat: position is a multiple of 2 beats.
    let is_strong = tpb > 0 && beat_position.is_multiple_of(2 * tpb);

    if is_strong {
        // Must be a chord tone.
        if chord_tones.contains(&raw_midi) {
            return raw_midi;
        }
        return nearest_in_set(raw_midi, &chord_tones);
    }

    // Weak beat: allow scale tones.
    if let Some(scale) = scale {
        if scale.contains(raw_midi) {
            return raw_midi;
        }
        // Snap to nearest scale tone in register.
        let up = step_scale(&scale, raw_midi, 1);
        let down = step_scale(&scale, raw_midi, -1);
        let d_up = (up as i16 - raw_midi as i16).unsigned_abs() as u8;
        let d_down = (down as i16 - raw_midi as i16).unsigned_abs() as u8;
        let snapped = if d_up <= d_down { up } else { down };
        return snapped.clamp(register.0, register.1);
    }

    // No scale: snap to chord tone.
    nearest_in_set(raw_midi, &chord_tones)
}

/// Find the nearest value in a sorted set to the target.
pub(super) fn nearest_in_set(target: u8, set: &[u8]) -> u8 {
    let mut best = set[0];
    let mut best_dist = (target as i16 - best as i16).unsigned_abs();
    for &v in &set[1..] {
        let dist = (target as i16 - v as i16).unsigned_abs();
        if dist < best_dist {
            best = v;
            best_dist = dist;
        }
    }
    best
}

// --- Leap grammar constants (Open Music Theory: melodic line rules) ---

/// Smallest melodic move that counts as a leap, in semitones (minor 3rd).
const LEAP_MIN: i16 = 3;
/// Leaps of a 4th or larger (≥5 semitones) must recover by a step in
/// the opposite direction.
const RECOVERY_MIN: i16 = 5;
/// Largest melodic move that counts as a step, in semitones.
const STEP_MAX: i16 = 2;

/// Signed semitone move from note `a` to note `b`.
fn move_between(a: &GeneratedNote, b: &GeneratedNote) -> i16 {
    b.note as i16 - a.note as i16
}

/// Do three pitches all belong to a single major or minor triad? Used
/// to permit same-direction leap pairs that arpeggiate one harmony.
/// Shared with the cadence overlay's pure grammar validator.
pub(super) fn outlines_one_triad(a: u8, b: u8, c: u8) -> bool {
    const TRIADS: [[u8; 3]; 2] = [[0, 4, 7], [0, 3, 7]]; // major, minor
    let pcs = [a % 12, b % 12, c % 12];
    for root in 0..12u8 {
        for triad in TRIADS {
            if pcs
                .iter()
                .all(|&pc| triad.iter().any(|&iv| (root + iv) % 12 == pc))
            {
                return true;
            }
        }
    }
    false
}

/// Does a melodic outline (the span of one monotonic run) form a
/// dissonant interval — a tritone or a seventh?
fn dissonant_outline(span: i16) -> bool {
    matches!(span.abs(), 6 | 10 | 11)
}

/// Post-processing: enforce the leap grammar on a realized phrase.
///
/// Unlike the old gap-fill pass (which erased big leaps by inserting
/// stepwise passing tones), this rewrites the *following* note so the
/// leap is heard and then resolved:
///
/// - A leap of a 4th or larger (≥5 semitones) must be followed by a
///   step in the opposite direction — leap recovery.
/// - Two consecutive leaps in the same direction are allowed only when
///   all three pitches outline one major or minor triad.
/// - Three or more consecutive leaps are never allowed.
/// - The extrema of direction changes (the span of each monotonic run)
///   must not outline a tritone or a seventh.
///
/// Repairs replace the offending note's pitch with a scale step from
/// its predecessor (opposite to the leap), preserving rhythm: no notes
/// are inserted or removed.
///
/// The two passes can disturb each other (an extremum nudge may shrink
/// a recovery step into a repeat), so they alternate until a fixpoint.
/// Every repair pulls pitches closer together, so this converges in a
/// couple of rounds; the cap is belt-and-braces.
pub(super) fn apply_leap_recovery(
    notes: &mut [GeneratedNote],
    scale: &Scale,
    register: (u8, u8),
) {
    for _ in 0..16 {
        let grammar_changed = enforce_leap_grammar(notes, scale, register);
        let outline_changed = enforce_outline_consonance(notes, scale, register);
        if !grammar_changed && !outline_changed {
            break;
        }
    }
}

/// One left-to-right pass of the leap grammar (recovery, triad pairs,
/// consecutive-leap cap). At iteration `i` everything before `i` is
/// final for this pass, so each adjacent move pair is validated against
/// the pitches that actually remain. Returns whether any note changed.
fn enforce_leap_grammar(notes: &mut [GeneratedNote], scale: &Scale, register: (u8, u8)) -> bool {
    let mut changed = false;
    for i in 2..notes.len() {
        let prev_move = move_between(&notes[i - 2], &notes[i - 1]);
        let cur_move = move_between(&notes[i - 1], &notes[i]);
        let prev_is_leap = prev_move.abs() >= LEAP_MIN;
        let cur_is_leap = cur_move.abs() >= LEAP_MIN;

        // Rule: never three (or more) consecutive leaps.
        let third_leap = prev_is_leap && cur_is_leap && i >= 3 && {
            let pre = move_between(&notes[i - 3], &notes[i - 2]);
            pre.abs() >= LEAP_MIN
        };

        // Rule: a same-direction leap pair must outline one triad.
        let triad_pair = prev_is_leap
            && cur_is_leap
            && cur_move.signum() == prev_move.signum()
            && outlines_one_triad(notes[i - 2].note, notes[i - 1].note, notes[i].note);
        let bad_pair = prev_is_leap
            && cur_is_leap
            && cur_move.signum() == prev_move.signum()
            && !triad_pair;

        // Rule: a leap of a 4th or larger resolves by an opposite
        // step. A continuing triad arpeggio postpones the recovery to
        // the note after the arpeggio's top/bottom.
        let needs_recovery = prev_move.abs() >= RECOVERY_MIN && {
            let opposite_step = cur_move.signum() == -prev_move.signum()
                && cur_move.abs() <= STEP_MAX;
            !(opposite_step || (triad_pair && !third_leap))
        };

        if third_leap || bad_pair || needs_recovery {
            let dir = if prev_move > 0 { -1 } else { 1 };
            let repaired = step_scale(scale, notes[i - 1].note, dir).clamp(register.0, register.1);
            if repaired != notes[i].note {
                notes[i].note = repaired;
                changed = true;
            }
        }
    }
    changed
}

/// Direction-change extrema pass: each monotonic run's outlined span
/// must not form a tritone or a seventh; offending extrema are nudged
/// back toward the run start one scale step at a time. Every repair
/// shrinks the offending span, so the rescan loop terminates; the
/// guard is belt-and-braces against scale/register edge cases.
/// Returns whether any note changed.
fn enforce_outline_consonance(
    notes: &mut [GeneratedNote],
    scale: &Scale,
    register: (u8, u8),
) -> bool {
    let mut changed = false;
    let mut guard = notes.len().saturating_mul(8);
    'rescan: while guard > 0 {
        guard -= 1;
        let mut run_start = 0usize;
        let mut run_dir: i16 = 0;
        for i in 1..notes.len() {
            let dir = move_between(&notes[i - 1], &notes[i]).signum();
            if dir == 0 {
                continue; // repeats extend the current run
            }
            if run_dir != 0 && dir != run_dir {
                // Run ended at the extremum `i - 1`.
                if repair_outline(notes, run_start, i - 1, scale, register) {
                    changed = true;
                    continue 'rescan;
                }
                run_start = i - 1;
            }
            run_dir = dir;
        }
        if !notes.is_empty()
            && repair_outline(notes, run_start, notes.len() - 1, scale, register)
        {
            changed = true;
            continue 'rescan;
        }
        break;
    }
    changed
}

/// If the monotonic run `start..=end` outlines a tritone or seventh,
/// pull the extremum one scale step back toward the run start. Returns
/// whether a repair was made (caller rescans the run structure).
fn repair_outline(
    notes: &mut [GeneratedNote],
    start: usize,
    end: usize,
    scale: &Scale,
    register: (u8, u8),
) -> bool {
    if end <= start {
        return false;
    }
    let span = move_between(&notes[start], &notes[end]);
    if !dissonant_outline(span) {
        return false;
    }
    let dir = if span > 0 { -1 } else { 1 };
    let repaired = step_scale(scale, notes[end].note, dir).clamp(register.0, register.1);
    if repaired == notes[end].note {
        return false; // pinned by register/scale edge; avoid looping
    }
    notes[end].note = repaired;
    true
}
