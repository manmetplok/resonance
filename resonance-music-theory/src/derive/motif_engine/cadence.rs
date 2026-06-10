// Cadence-formula overlay for realized instrumental phrases.
//
// Runs after the leap-grammar / single-climax fixpoint in
// `melody::derive_motif_melody_with_section` and rewrites the final
// two notes of a phrase to the planned goal cadence's melodic formula
// (see `derive::cadence` for the table). The overlay is *validated*:
// a candidate ending is only applied when the whole modified phrase
// still satisfies
//
//   - the leap grammar + outline-consonance rules
//     (`tests/leap_recovery.rs` contract),
//   - the single-climax rule (`tests/phrase_climax.rs` contract),
//   - the strong-beat chord-tone contract of `align_to_harmony`
//     (`derive_basics::motif_strong_beats_are_chord_tones`),
//   - the motif builder's approach limits (no tritone, nothing wider
//     than a perfect 5th into the approach tone).
//
// When no candidate survives, the phrase keeps the ending the earlier
// passes produced (for consequents that is the chord-root snap from
// `realize_phrase`), so the overlay can never regress the existing
// invariants — it only upgrades endings where a formula fits.

use crate::scale::Scale;

use super::super::cadence::{
    final_degree_fits_chord, formula_candidates, scale_degree_of, scale_degree_pc,
    tendency_resolution, CadenceGoal,
};
use super::super::{GeneratedNote, TimedChord};
use super::harmony::outlines_one_triad;

const LEAP_MIN: i16 = 3;
const RECOVERY_MIN: i16 = 5;
const STEP_MAX: i16 = 2;

fn is_leap(mv: i16) -> bool {
    mv.abs() >= LEAP_MIN
}

/// Pure check of the leap grammar on a pitch sequence — the same rules
/// `apply_leap_recovery` enforces by rewriting, phrased as a predicate
/// so the cadence overlay can reject candidates instead of repairing
/// them afterwards (a repair would destroy the formula).
fn leap_grammar_ok(pitches: &[u8]) -> bool {
    let mv: Vec<i16> = pitches
        .windows(2)
        .map(|w| w[1] as i16 - w[0] as i16)
        .collect();
    for i in 1..mv.len() {
        let prev = mv[i - 1];
        let cur = mv[i];
        if i >= 2 && is_leap(mv[i - 2]) && is_leap(prev) && is_leap(cur) {
            return false; // three consecutive leaps
        }
        let same_dir_pair = is_leap(prev) && is_leap(cur) && cur.signum() == prev.signum();
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
        matches!(span, 6 | 10 | 11)
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
fn climax_ok(pitches: &[u8]) -> bool {
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

/// Chord active at `tick` within the phrase.
fn chord_at_tick(phrase_chords: &[TimedChord], tpb: u64, tick: u64) -> Option<&TimedChord> {
    phrase_chords
        .iter()
        .rfind(|tc| tc.start_beat as u64 * tpb <= tick)
        .or_else(|| phrase_chords.first())
}

/// Is `tick` a strong beat of its chord (a multiple of 2 beats from
/// the chord start)? Mirrors `align_to_harmony`.
fn is_strong_beat(phrase_chords: &[TimedChord], tpb: u64, tick: u64) -> bool {
    chord_at_tick(phrase_chords, tpb, tick).is_some_and(|tc| {
        let chord_start = tc.start_beat as u64 * tpb;
        tpb > 0 && tick >= chord_start && (tick - chord_start).is_multiple_of(2 * tpb)
    })
}

/// Does `pitch` belong to the chord sounding at `tick`?
fn is_chord_tone_at(phrase_chords: &[TimedChord], tpb: u64, tick: u64, pitch: u8) -> bool {
    chord_at_tick(phrase_chords, tpb, tick).is_some_and(|tc| {
        tc.chord
            .pitch_classes()
            .any(|pc| pc.to_semitone() == pitch % 12)
    })
}

/// Rewrite the final two notes of a realized phrase to the planned
/// cadence formula. Best-effort: scans every in-register realization
/// of the goal's formulas (walking the compatibility fallback chain)
/// and applies the cheapest candidate that keeps the whole phrase
/// valid; leaves the phrase untouched when none survives.
pub(super) fn apply_cadence_formula(
    notes: &mut [GeneratedNote],
    goal: CadenceGoal,
    phrase_chords: &[TimedChord],
    scale: &Scale,
    register: (u8, u8),
    tpb: u64,
) {
    let n = notes.len();
    if n < 2 || phrase_chords.is_empty() {
        return;
    }
    let pitches: Vec<u8> = notes.iter().map(|x| x.note).collect();
    let prev = (n >= 3).then(|| pitches[n - 3]);
    // Tendency tone left hanging just before the cadence pair: prefer
    // a formula whose approach tone resolves it (7→1, 4→3, 2→1).
    let prev_resolution = prev
        .and_then(|p| scale_degree_of(scale, p))
        .and_then(tendency_resolution);
    let old_penult = pitches[n - 2];
    let old_final = pitches[n - 1];
    let penult_tick = notes[n - 2].start_tick;
    let final_tick = notes[n - 1].start_tick;
    let penult_strong = is_strong_beat(phrase_chords, tpb, penult_tick);
    let final_strong = is_strong_beat(phrase_chords, tpb, final_tick);

    let Some(final_chord) = chord_at_tick(phrase_chords, tpb, final_tick) else {
        return;
    };

    let mut best: Option<(i32, u8, u8)> = None;
    let mut modified = pitches.clone();
    for cand in formula_candidates(goal, scale, final_chord.chord, register) {
        // Keep the move into the approach tone singable; tritone and
        // outline dissonances are caught by the grammar validator
        // below (a literal tritone move embedded in a longer run is
        // legal there, matching `apply_leap_recovery`).
        if let Some(p) = prev {
            if (cand.penult as i16 - p as i16).abs() > 9 {
                continue;
            }
        }
        // Strong-beat notes must stay chord tones (`align_to_harmony`
        // contract).
        if penult_strong && !is_chord_tone_at(phrase_chords, tpb, penult_tick, cand.penult) {
            continue;
        }
        if final_strong && !is_chord_tone_at(phrase_chords, tpb, final_tick, cand.fin) {
            continue;
        }
        modified[n - 2] = cand.penult;
        modified[n - 1] = cand.fin;
        if !leap_grammar_ok(&modified) || !climax_ok(&modified) {
            continue;
        }
        let resolves = prev_resolution == Some(cand.penult_degree);
        let approach_cost = prev
            .map(|p| (cand.penult as i16 - p as i16).abs() as i32)
            .unwrap_or(0);
        let score = cand.goal_rank as i32 * 10_000
            + if resolves { 0 } else { 500 }
            + approach_cost * 20
            + (cand.fin as i16 - old_final as i16).abs() as i32
            + (cand.penult as i16 - old_penult as i16).abs() as i32;
        if best.is_none_or(|(b, _, _)| score < b) {
            best = Some((score, cand.penult, cand.fin));
        }
    }
    if let Some((_, p, f)) = best {
        notes[n - 2].note = p;
        notes[n - 1].note = f;
        return;
    }

    // No full two-note formula validated (commonly: the penult sits on
    // a strong beat, where the chord-tone contract excludes most
    // approach tones). Fall back to retargeting the final note alone
    // so the phrase still lands on the goal cadence's degree — the
    // approach just stays whatever the earlier passes produced.
    let mut best_final: Option<(i32, u8)> = None;
    let mut modified = pitches.clone();
    for (goal_rank, g) in goal.chain().iter().enumerate() {
        for &(_, f_deg) in g.formulas() {
            if !final_degree_fits_chord(scale, f_deg, final_chord.chord) {
                continue;
            }
            let pc = scale_degree_pc(scale, f_deg);
            for fin in register.0..=register.1 {
                if fin % 12 != pc {
                    continue;
                }
                let approach = (fin as i16 - old_penult as i16).abs();
                if approach > 9 {
                    continue;
                }
                if final_strong && !is_chord_tone_at(phrase_chords, tpb, final_tick, fin) {
                    continue;
                }
                modified[n - 1] = fin;
                if !leap_grammar_ok(&modified) || !climax_ok(&modified) {
                    continue;
                }
                let score = goal_rank as i32 * 10_000
                    + approach as i32 * 20
                    + (fin as i16 - old_final as i16).abs() as i32;
                if best_final.is_none_or(|(b, _)| score < b) {
                    best_final = Some((score, fin));
                }
            }
        }
    }
    if let Some((_, f)) = best_final {
        notes[n - 1].note = f;
    }
}
