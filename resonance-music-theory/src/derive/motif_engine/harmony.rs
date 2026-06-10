// Harmony post-processing: build the chord-tone skeleton (chord-tone on
// strong beats, scale-tone on weak beats — the raw material the
// embellishment pass later re-classifies), and enforce the Open Music
// Theory leap grammar across the realized phrase (leaps resolve by an
// opposite step instead of being erased by fill notes). The grammar is
// harmony-aware: a non-chord tone may never be both leaped into and
// left by leap (OMT dissonance discipline).
//
// This module also owns the shared *pure* validators (leap grammar,
// single climax, dissonance treatment, strong-beat contract) that the
// cadence and embellishment overlays use to reject candidates instead
// of repairing them afterwards.
//
// The strong-beat contract: a strong-beat note is either a chord tone
// (the skeleton `align_to_harmony` produces) or a dissonance that
// resolves *down by step* to a chord tone — the latter only ever
// introduced by the embellishment pass (appoggiatura / suspension).

use crate::chord::Chord;
use crate::scale::Scale;

use super::super::bass::step_scale;
use super::super::motif_bass::chord_tones_in_register;
use super::super::{GeneratedNote, TimedChord};

/// Harmonic lookup grid shared by the grammar pass and the validated
/// overlays (cadence, embellishment): which chord sounds at a tick,
/// whether the tick is a strong beat of its chord, and whether a pitch
/// is a chord tone there.
pub(super) struct HarmonyGrid<'a> {
    pub(super) chords: &'a [TimedChord],
    pub(super) tpb: u64,
}

impl HarmonyGrid<'_> {
    /// Chord active at `tick`.
    pub(super) fn chord_at(&self, tick: u64) -> Option<&TimedChord> {
        self.chords
            .iter()
            .rfind(|tc| tc.start_beat as u64 * self.tpb <= tick)
            .or_else(|| self.chords.first())
    }

    /// Is `tick` a strong beat of its chord (a multiple of 2 beats
    /// from the chord start)? Mirrors `align_to_harmony`.
    pub(super) fn is_strong_beat(&self, tick: u64) -> bool {
        self.chord_at(tick).is_some_and(|tc| {
            let chord_start = tc.start_beat as u64 * self.tpb;
            self.tpb > 0
                && tick >= chord_start
                && (tick - chord_start).is_multiple_of(2 * self.tpb)
        })
    }

    /// Does `pitch` belong to the chord sounding at `tick`?
    pub(super) fn is_chord_tone(&self, tick: u64, pitch: u8) -> bool {
        self.chord_at(tick).is_some_and(|tc| {
            tc.chord
                .pitch_classes()
                .any(|pc| pc.to_semitone() == pitch % 12)
        })
    }
}

/// Align a MIDI note to the current harmony based on beat strength.
/// This produces the *skeleton*: chord tones on strong beats, scale
/// tones on weak beats. The embellishment pass then re-classifies the
/// surface from the OMT embellishing-tone table instead of leaving the
/// blanket snap as the final word.
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
/// - A non-chord tone (against the chord sounding at its tick) may
///   never be both leaped into and left by leap — the OMT dissonance
///   discipline. The triad-arpeggio exemption does *not* apply: an
///   arpeggio through a dissonance still sounds wrong.
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
    grid: &HarmonyGrid<'_>,
) {
    for _ in 0..16 {
        let grammar_changed = enforce_leap_grammar(notes, scale, register, grid);
        let outline_changed = enforce_outline_consonance(notes, scale, register);
        if !grammar_changed && !outline_changed {
            break;
        }
    }
}

/// One left-to-right pass of the leap grammar (recovery, triad pairs,
/// consecutive-leap cap, dissonance discipline). At iteration `i`
/// everything before `i` is final for this pass, so each adjacent move
/// pair is validated against the pitches that actually remain. Returns
/// whether any note changed.
fn enforce_leap_grammar(
    notes: &mut [GeneratedNote],
    scale: &Scale,
    register: (u8, u8),
    grid: &HarmonyGrid<'_>,
) -> bool {
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

        // Rule: never leap both into and out of a dissonance. The
        // repair (an opposite step from the dissonance) turns the exit
        // into a step, so the dissonance is left by step.
        let dissonant_leap_pair = prev_is_leap
            && cur_is_leap
            && !grid.is_chord_tone(notes[i - 1].start_tick, notes[i - 1].note);

        // Rule: a strong-beat dissonance resolves by step — the
        // appoggiatura/suspension shape the strong-beat contract
        // demands. A leap away or a dissonant repetition is repaired
        // into the resolution step.
        let strong_unresolved = !(1..=STEP_MAX).contains(&cur_move.abs())
            && grid.is_strong_beat(notes[i - 1].start_tick)
            && !grid.is_chord_tone(notes[i - 1].start_tick, notes[i - 1].note);

        if third_leap || bad_pair || needs_recovery || dissonant_leap_pair || strong_unresolved {
            // Preferred repair direction: opposite to the approach
            // (the recovery step); a held approach (only the
            // strong-dissonance rule triggers there) resolves down,
            // the suspension shape. `needs_recovery` *requires* the
            // opposite direction; the other triggers accept a step
            // either way, so the fallback direction is tried when the
            // preferred one would itself create a dissonant run
            // outline (the repair/outline ping-pong that otherwise
            // stalls the fixpoint) or lift a pitch up to the phrase
            // maximum (which would break the single-climax
            // alternation this pass participates in).
            let preferred: i32 = if prev_move >= 0 { -1 } else { 1 };
            let dirs: &[i32] = if needs_recovery {
                &[preferred]
            } else {
                &[preferred, -preferred]
            };
            let phrase_max = notes.iter().map(|n| n.note).max().unwrap_or(0);
            let mut repaired = None;
            for &dir in dirs {
                let cand =
                    step_scale(scale, notes[i - 1].note, dir).clamp(register.0, register.1);
                if cand == notes[i - 1].note {
                    continue; // pinned by a scale/register edge
                }
                if cand > notes[i].note && cand >= phrase_max {
                    continue; // would create a new phrase maximum
                }
                if dissonant_outline(run_span_ending_at(notes, i, cand)) {
                    continue; // would hand the outline pass a tritone/7th
                }
                repaired = Some(cand);
                break;
            }
            let repaired = repaired.unwrap_or_else(|| {
                step_scale(scale, notes[i - 1].note, preferred).clamp(register.0, register.1)
            });
            if repaired != notes[i].note {
                notes[i].note = repaired;
                changed = true;
            }
        }
    }
    changed
}

/// Pitch span of the monotonic run (repeats extend it) that would end
/// at `end` if it held `end_value`. Mirrors the outline pass's run
/// segmentation so a grammar repair can avoid creating the dissonant
/// outline that pass would immediately undo.
fn run_span_ending_at(notes: &[GeneratedNote], end: usize, end_value: u8) -> i16 {
    let pitch = |k: usize| -> i16 {
        if k == end {
            end_value as i16
        } else {
            notes[k].note as i16
        }
    };
    if end == 0 {
        return 0;
    }
    let dir = (pitch(end) - pitch(end - 1)).signum();
    if dir == 0 {
        return 0;
    }
    let mut j = end;
    while j > 0 {
        let step = pitch(j) - pitch(j - 1);
        if step.signum() == dir || step == 0 {
            j -= 1;
        } else {
            break;
        }
    }
    pitch(end) - pitch(j)
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

// --- Shared pure validators -------------------------------------------------
//
// The validated overlays (cadence formulas, embellishments) propose a
// candidate pitch sequence and only commit it when the whole phrase
// still satisfies every invariant the repair passes above enforce.
// `pitches` is the candidate; `notes` supplies the (unchanged) ticks.

/// Pure check of the leap grammar on a pitch sequence — the same rules
/// `apply_leap_recovery` enforces by rewriting (minus the harmony-aware
/// dissonance rule, which lives in [`dissonance_treatment_ok`]).
pub(super) fn leap_grammar_ok(pitches: &[u8]) -> bool {
    let mv: Vec<i16> = pitches
        .windows(2)
        .map(|w| w[1] as i16 - w[0] as i16)
        .collect();
    for i in 1..mv.len() {
        let prev = mv[i - 1];
        let cur = mv[i];
        if i >= 2 && mv[i - 2].abs() >= LEAP_MIN && prev.abs() >= LEAP_MIN && cur.abs() >= LEAP_MIN
        {
            return false; // three consecutive leaps
        }
        let same_dir_pair = prev.abs() >= LEAP_MIN
            && cur.abs() >= LEAP_MIN
            && cur.signum() == prev.signum();
        let triad_pair =
            same_dir_pair && outlines_one_triad(pitches[i - 1], pitches[i], pitches[i + 1]);
        if same_dir_pair && !triad_pair {
            return false;
        }
        if prev.abs() >= RECOVERY_MIN {
            let opposite_step = cur.signum() == -prev.signum() && cur.abs() <= STEP_MAX;
            if !(opposite_step || triad_pair) {
                return false;
            }
        }
    }
    // Direction-change extrema: no monotonic run may outline a tritone
    // or a seventh.
    let dissonant = |s: usize, e: usize| {
        let span = (pitches[e] as i16 - pitches[s] as i16).abs();
        dissonant_outline(span)
    };
    let mut run_start = 0usize;
    let mut run_dir: i16 = 0;
    for i in 1..pitches.len() {
        let dir = (pitches[i] as i16 - pitches[i - 1] as i16).signum();
        if dir == 0 {
            continue;
        }
        if run_dir != 0 && dir != run_dir {
            if dissonant(run_start, i - 1) {
                return false;
            }
            run_start = i - 1;
        }
        run_dir = dir;
    }
    if !pitches.is_empty() && dissonant(run_start, pitches.len() - 1) {
        return false;
    }
    true
}

/// Pure check of the single-climax rule: exactly one highest note, in
/// the second half, never the final note. Phrases too short or flat
/// are exempt, mirroring `enforce_single_climax`'s skip conditions.
pub(super) fn climax_ok(pitches: &[u8]) -> bool {
    let n = pitches.len();
    if n < 3 {
        return true;
    }
    let max = *pitches.iter().max().unwrap();
    let min = *pitches.iter().min().unwrap();
    if max == min {
        return true;
    }
    let peaks: Vec<usize> = pitches
        .iter()
        .enumerate()
        .filter(|(_, &p)| p == max)
        .map(|(i, _)| i)
        .collect();
    peaks.len() == 1 && peaks[0] >= n / 2 && peaks[0] != n - 1
}

/// Pure check of the dissonance discipline: a non-chord tone (against
/// the chord sounding at its tick) is never both leaped into and left
/// by leap. First and last notes are exempt — they lack one of the two
/// moves, so the "both" condition cannot hold.
pub(super) fn dissonance_treatment_ok(
    pitches: &[u8],
    notes: &[GeneratedNote],
    grid: &HarmonyGrid<'_>,
) -> bool {
    debug_assert_eq!(pitches.len(), notes.len());
    for i in 1..pitches.len().saturating_sub(1) {
        if grid.is_chord_tone(notes[i].start_tick, pitches[i]) {
            continue;
        }
        let leap_in = (pitches[i] as i16 - pitches[i - 1] as i16).abs() >= LEAP_MIN;
        let leap_out = (pitches[i + 1] as i16 - pitches[i] as i16).abs() >= LEAP_MIN;
        if leap_in && leap_out {
            return false;
        }
    }
    true
}

/// Pure check of the strong-beat contract (the evolution of "strong
/// beats are chord tones"): every strong-beat note is a chord tone, or
/// a dissonance that resolves *by step* — the appoggiatura/suspension
/// shape. The embellishment pass constructs its strong-beat
/// dissonances with downward chord-tone resolutions; the contract
/// itself only demands the step. A strong-beat dissonance with no
/// following note has no resolution and fails.
pub(super) fn strong_beats_ok(
    pitches: &[u8],
    notes: &[GeneratedNote],
    grid: &HarmonyGrid<'_>,
) -> bool {
    debug_assert_eq!(pitches.len(), notes.len());
    for i in 0..pitches.len() {
        if !grid.is_strong_beat(notes[i].start_tick)
            || grid.is_chord_tone(notes[i].start_tick, pitches[i])
        {
            continue;
        }
        let Some(&next) = pitches.get(i + 1) else {
            return false;
        };
        let resolution = (next as i16 - pitches[i] as i16).abs();
        if !(1..=STEP_MAX).contains(&resolution) {
            return false;
        }
    }
    true
}
