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
//   - the dissonance discipline (never leap both into and out of a
//     non-chord tone, `tests/embellishment.rs` contract),
//   - the strong-beat chord-tone skeleton of `align_to_harmony`
//     (`derive_basics` contract; this overlay runs *before* the
//     embellishment pass, so the stricter pre-decoration form — strong
//     beats are chord tones, full stop — still holds here),
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
use super::super::climax::SectionClimaxRule;
use super::super::{GeneratedNote, TimedChord};
use super::harmony::{
    climax_ok, dissonance_treatment_ok, leap_grammar_ok, strong_beats_ok, HarmonyGrid,
};

/// Rewrite the final two notes of a realized phrase to the planned
/// cadence formula. Best-effort: scans every in-register realization
/// of the goal's formulas (walking the compatibility fallback chain)
/// and applies the cheapest candidate that keeps the whole phrase
/// valid; leaves the phrase untouched when none survives.
///
/// `section` is the phrase's section-climax constraint: secondary
/// phrases reject candidates that reach the carrier's peak, and the
/// carrier rejects candidates that would rewrite its peak away.
#[allow(clippy::too_many_arguments)]
pub(super) fn apply_cadence_formula(
    notes: &mut [GeneratedNote],
    goal: CadenceGoal,
    phrase_chords: &[TimedChord],
    scale: &Scale,
    register: (u8, u8),
    tpb: u64,
    section: SectionClimaxRule,
) {
    let n = notes.len();
    if n < 2 || phrase_chords.is_empty() {
        return;
    }
    let grid = HarmonyGrid {
        chords: phrase_chords,
        tpb,
    };
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
    let penult_strong = grid.is_strong_beat(penult_tick);
    let final_strong = grid.is_strong_beat(final_tick);

    let Some(final_chord) = grid.chord_at(final_tick) else {
        return;
    };
    let final_chord = *final_chord;

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
        // Strong-beat notes must stay chord tones (pre-decoration
        // `align_to_harmony` skeleton contract).
        if penult_strong && !grid.is_chord_tone(penult_tick, cand.penult) {
            continue;
        }
        if final_strong && !grid.is_chord_tone(final_tick, cand.fin) {
            continue;
        }
        modified[n - 2] = cand.penult;
        modified[n - 1] = cand.fin;
        if !leap_grammar_ok(&modified)
            || !climax_ok(&modified)
            || !section.allows(&modified)
            || !dissonance_treatment_ok(&modified, notes, &grid)
            || !strong_beats_ok(&modified, notes, &grid)
        {
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
                if final_strong && !grid.is_chord_tone(final_tick, fin) {
                    continue;
                }
                modified[n - 1] = fin;
                if !leap_grammar_ok(&modified)
                    || !climax_ok(&modified)
                    || !section.allows(&modified)
                    || !dissonance_treatment_ok(&modified, notes, &grid)
                    || !strong_beats_ok(&modified, notes, &grid)
                {
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
