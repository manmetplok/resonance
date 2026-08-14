//! The per-line goal-cadence formula pass.
//!
//! One responsibility: rewriting each lyric line's final two syllables
//! into a cadence formula (PAC / IAC / HC / deceptive) that fits the
//! chord under the cadence. The formula catalogue itself lives in
//! `crate::derive::cadence`; this module decides which line gets which
//! goal, which candidate wins, and repairs the line's climax after the
//! rewrite.

use crate::derive::cadence::{
    formula_candidates, plan_cadence_goal, scale_degree_of, tendency_resolution, CadenceGoal,
};
use crate::derive::climax::{enforce_single_climax, SectionClimaxRule};
use crate::derive::vocal::params::VocalStyle;
use crate::derive::vocal::style::{phrase_role, PhraseRole, MAX_INTERVAL};
use crate::derive::{GeneratedNote, TimedChord};
use crate::rng::XorShift;
use crate::scale::Scale;

use super::primitives::chord_at_beat;

/// Per-line goal-cadence formula pass. Every lyric line gets a goal
/// cadence — weak (HC, sometimes IAC) for antecedent lines, strong
/// (PAC, ~10% deceptive) for consequent lines — and the line's final
/// two syllables are rewritten to a two-note formula compatible with
/// the chord under the cadence (2→1 / 7→1 for PAC, ends-on-3/5 for
/// IAC, 1→7 / 3→2 for HC, lands-on-6 for deceptive). This completes
/// what `cadence_pitch` starts: the style walker already lands the
/// final syllable on a formula degree, and this pass retargets the
/// penult so the approach is the formula's scale step — resolving the
/// tendency tones 7→1, 4→3, 2→1 by construction.
///
/// Runs after the per-line climax pass and the section climax pass;
/// every candidate ending is validated against the line-climax rule,
/// the line's section-climax rule (`line_rules`, from
/// [`super::climax::apply_section_climax`]; pass an empty slice to
/// skip), and the SVS `MAX_INTERVAL` adjacency cap, and lines where no
/// candidate survives keep their walked ending. The deceptive roll
/// draws from a dedicated rng seeded from the section seed so the swap
/// is deterministic and independent of the styles' draw sequences.
#[allow(clippy::too_many_arguments)]
pub(in crate::derive::vocal) fn apply_line_cadence_formulas(
    notes: &mut [GeneratedNote],
    line_syllables: &[u32],
    chords: &[TimedChord],
    section_beats: u32,
    scale: Option<Scale>,
    range: (u8, u8),
    style: VocalStyle,
    tpb: u64,
    seed: u64,
    line_rules: &[SectionClimaxRule],
) {
    let Some(scale) = scale else { return };
    // Per-style adjacency cap for the rewritten notes. Hymnal's
    // contract is strictly stepwise motion (nothing past a major
    // third); everything else is bounded by the SVS render cap.
    let max_step: i16 = match style {
        VocalStyle::Hymnal => 4,
        _ => MAX_INTERVAL,
    };
    let mut rng = XorShift::new(seed.wrapping_add(0xCADE2BAD_C0DA5EED));
    let mut note_idx = 0usize;
    for (line_idx, &line_syl) in line_syllables.iter().enumerate() {
        if line_syl == 0 {
            continue;
        }
        let n = (line_syl as usize).min(notes.len().saturating_sub(note_idx));
        if n == 0 {
            break;
        }
        let is_consequent =
            phrase_role(line_idx, line_syllables.len()) == PhraseRole::Consequent;
        let goal = plan_cadence_goal(is_consequent, &mut rng);
        if n >= 2 {
            // Predecessor of the approach tone: within the line when it
            // has one, otherwise the previous line's final note. The
            // successor is the next line's first note (the rewritten
            // final must not break the adjacency cap across the join).
            let prev = if n >= 3 {
                Some(notes[note_idx + n - 3].note)
            } else if note_idx > 0 {
                Some(notes[note_idx - 1].note)
            } else {
                None
            };
            let next = notes.get(note_idx + n).map(|x| x.note);
            let rule = line_rules
                .get(line_idx)
                .copied()
                .unwrap_or(SectionClimaxRule::Free);
            apply_one_line_cadence(
                &mut notes[note_idx..note_idx + n],
                goal,
                chords,
                section_beats,
                &scale,
                range,
                prev,
                next,
                max_step,
                tpb,
                rule,
            );
        }
        note_idx += n;
    }
}

/// Rewrite one line's final two notes to the best valid realization of
/// `goal`'s formulas (walking the chord-compatibility fallback chain).
/// Leaves the line untouched when no candidate validates.
#[allow(clippy::too_many_arguments)]
fn apply_one_line_cadence(
    line: &mut [GeneratedNote],
    goal: CadenceGoal,
    chords: &[TimedChord],
    section_beats: u32,
    scale: &Scale,
    range: (u8, u8),
    prev: Option<u8>,
    next: Option<u8>,
    max_step: i16,
    tpb: u64,
    section: SectionClimaxRule,
) {
    let n = line.len();
    if n < 2 {
        return;
    }
    let (lo, hi) = range;
    let pitches: Vec<u8> = line.iter().map(|x| x.note).collect();
    let beat = ((line[n - 1].start_tick / tpb.max(1)) as u32)
        .min(section_beats.saturating_sub(1));
    let Some(chord) = chord_at_beat(chords, beat) else {
        return;
    };
    // Tendency tone left hanging before the cadence pair: prefer the
    // formula whose approach tone resolves it (7→1, 4→3, 2→1).
    let prev_resolution = prev
        .and_then(|p| scale_degree_of(scale, p))
        .and_then(tendency_resolution);
    let old_penult = pitches[n - 2];
    let old_final = pitches[n - 1];

    let mut best: Option<(i32, u8, u8)> = None;
    let mut modified = pitches.clone();
    for cand in formula_candidates(goal, scale, chord.chord, (lo, hi)) {
        // Style adjacency cap on every join the rewrite touches: into
        // the approach tone, the formula step itself (at most an
        // augmented 2nd, but Hymnal caps at a major 3rd anyway), and
        // out of the final into the next line's first note.
        if (cand.fin as i16 - cand.penult as i16).abs() > max_step {
            continue;
        }
        if let Some(p) = prev {
            if (cand.penult as i16 - p as i16).abs() > max_step {
                continue;
            }
        }
        if let Some(nx) = next {
            if (cand.fin as i16 - nx as i16).abs() > max_step {
                continue;
            }
        }
        modified[n - 2] = cand.penult;
        modified[n - 1] = cand.fin;
        // The final (cadence) note must never end up as the line peak;
        // duplicate or first-half interior maxima are tolerated here —
        // the demote-only climax repair below restores the single
        // climax after the winning pair lands. (Requiring full
        // `line_climax_ok` of every candidate used to force the pass
        // onto pairs placed *above* the line's interior plateau — the
        // very every-line-arches-to-the-top behavior the section
        // climax plan removes — and the section cap now rejects
        // those.)
        if !cadence_tail_ok(&modified, lo) || !section.allows(&modified) {
            continue;
        }
        let resolves = prev_resolution == Some(cand.penult_degree);
        let score = cand.goal_rank as i32 * 10_000
            + if resolves { 0 } else { 500 }
            + (cand.penult as i16 - old_penult as i16).abs() as i32 * 10
            + (cand.fin as i16 - old_final as i16).abs() as i32 * 10
            + prev
                .map(|p| (cand.penult as i16 - p as i16).abs() as i32)
                .unwrap_or(0);
        if best.is_none_or(|(b, _, _)| score < b) {
            best = Some((score, cand.penult, cand.fin));
        }
    }
    if let Some((_, p, f)) = best {
        line[n - 2].note = p;
        line[n - 1].note = f;
        // Repair: demote interior duplicates/super-peaks the rewrite
        // orphaned, restoring the single second-half climax. Late
        // tie-break so a penult that ties the body max keeps the
        // formula pitch (it becomes the climax; the body copy is
        // demoted instead). Demote-only, so the section cap and the
        // adjacency analysis above keep holding.
        let applied: Vec<u8> = line.iter().map(|x| x.note).collect();
        if !line_climax_ok(&applied, lo) {
            enforce_single_climax(line, Some(*scale), range, None, false, true);
        }
    }
}

/// Relaxed per-candidate check for the cadence rewrite: mirrors
/// [`line_climax_ok`]'s exemptions (short, flat, floor-pinned lines)
/// but only insists the final note is not the line's (tied) maximum —
/// everything else is repairable by a demote-only climax pass after
/// the pair lands.
fn cadence_tail_ok(pitches: &[u8], range_lo: u8) -> bool {
    let n = pitches.len();
    if n < 3 {
        return true;
    }
    let max = *pitches.iter().max().unwrap();
    let min = *pitches.iter().min().unwrap();
    if max == min {
        return true;
    }
    let window_max = *pitches[n / 2..n - 1].iter().max().unwrap();
    if window_max <= range_lo {
        return true;
    }
    pitches[n - 1] < max
}

/// Pure check of the per-line single-climax rule, mirroring the
/// enforcement pass's skip conditions (lines under 3 syllables, flat
/// lines, and lines whose climax window sits on the register floor are
/// exempt): exactly one highest note, in the second half, never the
/// final syllable.
fn line_climax_ok(pitches: &[u8], range_lo: u8) -> bool {
    let n = pitches.len();
    if n < 3 {
        return true;
    }
    let max = *pitches.iter().max().unwrap();
    let min = *pitches.iter().min().unwrap();
    if max == min {
        return true;
    }
    let window_max = *pitches[n / 2..n - 1].iter().max().unwrap();
    if window_max <= range_lo {
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
