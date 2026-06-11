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

use crate::rng::XorShift;
use crate::scale::Scale;

use super::super::cadence::{
    formula_candidates, plan_cadence_goal, scale_degree_of, tendency_resolution, CadenceGoal,
};
use super::super::climax::{
    demote_at_or_above, enforce_single_climax, section_peak_margin, SectionClimaxRule,
};
use super::super::{GeneratedNote, TimedChord};
use super::params::VocalContour;
use super::params::{VocalParams, VocalStyle};
use super::style::{
    cap_interval, cadence_pitch, phrase_role, section_climax_line, PhraseRole, MAX_INTERVAL,
};

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
/// Bundle of immutable inputs to [`apply_motif_pitches`]. Carries the
/// motif's interval pattern, the section's per-line syllable counts,
/// and the harmonic + register context the pitch picker needs. Held
/// together so callers can fan out a single `VocalContext` into one
/// `MotifPitchContext` rather than threading seven parallel args.
pub(super) struct MotifPitchContext<'a> {
    pub(super) motif_intervals: &'a [i8],
    pub(super) line_syllables: &'a [u32],
    pub(super) chords: &'a [TimedChord],
    pub(super) section_beats: u32,
    pub(super) scale: Option<Scale>,
    pub(super) range: (u8, u8),
    pub(super) tpb: u64,
}

/// Re-skin a vocal phrase's pitches with a motif interval pattern.
/// Non-terminal syllables follow the motif's relative-interval contour
/// (anchored on the chord root nearest the previous pitch and clamped
/// to the lane register, snapped to scale). The terminal note of every
/// line keeps its style cadence landing so phrases still resolve.
pub(super) fn apply_motif_pitches(notes: &mut [GeneratedNote], ctx: &MotifPitchContext<'_>) {
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

/// Pop srdc section layout (statement–restatement–departure–
/// conclusion, OMT phrase archetypes): lyric lines group in fours as
/// aaba / aabc. The restatement re-sings the statement's contour
/// (offsets from the line's first pitch, index-scaled across differing
/// syllable counts, scale-snapped and adjacency-capped), the departure
/// keeps its own walked material so the group has real contrast, and
/// the conclusion either restates the statement (aaba, ~50% per group,
/// seeded) or keeps its own material (aabc). Trailing 3-line groups
/// are s r c; pairs and singles are left alone.
///
/// Runs before the per-line climax and cadence passes, so the copied
/// lines still get a valid single climax and the conclusion's
/// weak→strong ending swap: a copied conclusion ends on the
/// statement's *open* contour until `apply_line_cadence_formulas`
/// rewrites its final two notes toward PAC — the period principle
/// (same material, different ending) at section level.
pub(super) fn apply_srdc_layout(
    notes: &mut [GeneratedNote],
    line_syllables: &[u32],
    scale: Option<Scale>,
    range: (u8, u8),
    seed: u64,
) {
    // Per-line note spans, in lyric order (one note per syllable).
    let mut spans: Vec<(usize, usize)> = Vec::with_capacity(line_syllables.len());
    let mut cursor = 0usize;
    for &syl in line_syllables {
        let n = (syl as usize).min(notes.len().saturating_sub(cursor));
        spans.push((cursor, cursor + n));
        cursor += n;
    }
    let mut rng = XorShift::new(seed.wrapping_add(0x00AA_BAAA_BC5D_C0DE));
    let total = spans.len();
    let mut g = 0usize;
    while g < total {
        let group_len = (total - g).min(4);
        // One draw per group, taken unconditionally so the aaba/aabc
        // shape of later groups doesn't depend on earlier group sizes.
        let aaba = rng.next_f32() < 0.5;
        if group_len >= 3 {
            copy_line_contour(notes, spans[g], spans[g + 1], scale, range);
            if group_len == 4 && aaba {
                copy_line_contour(notes, spans[g], spans[g + 3], scale, range);
            }
        }
        g += group_len;
    }
}

/// Re-skin `dst`'s pitches with `src`'s contour: signed offsets from
/// the source line's first pitch, index-scaled onto the destination's
/// syllable count, anchored at the destination's own first pitch (so
/// register continuity from the previous line survives), snapped to
/// scale and capped to the SVS adjacency limit. The destination's
/// first note is the anchor and keeps its walked pitch.
fn copy_line_contour(
    notes: &mut [GeneratedNote],
    src: (usize, usize),
    dst: (usize, usize),
    scale: Option<Scale>,
    range: (u8, u8),
) {
    let (lo, hi) = range;
    let src_n = src.1 - src.0;
    let dst_n = dst.1 - dst.0;
    if src_n == 0 || dst_n < 2 {
        return;
    }
    let src_first = notes[src.0].note as i16;
    let offsets: Vec<i16> = (src.0..src.1)
        .map(|i| notes[i].note as i16 - src_first)
        .collect();
    let anchor = notes[dst.0].note as i16;
    let mut prev = notes[dst.0].note;
    for s in 1..dst_n {
        let idx = s * src_n / dst_n;
        let raw = (anchor + offsets[idx]).clamp(lo as i16, hi as i16) as u8;
        let snapped = snap_to_scale(raw, scale, lo, hi);
        let pitch = cap_interval(prev, snapped, lo, hi, scale);
        notes[dst.0 + s].note = pitch;
        prev = pitch;
    }
}

/// Section-level climax orchestration for vocal lines (Open Music
/// Theory v2: one climax per *section*): the designated carrier line
/// (the srdc departure — line 3 of 4) keeps the section's highest
/// note, and every other line's pitches are demoted strictly below it
/// so the four lines stop arching identically. The secondary cap sits
/// a seeded per-group margin (1–3 semitones) under the carrier's peak;
/// per *group* rather than per line so the statement/restatement echo
/// is demoted as a pair and survives intact.
///
/// Demote-only, like the per-line climax pass it runs after: nothing
/// is ever raised, so the styles' walked contours, the SVS adjacency
/// cap (`max_adjacent`; pass 4 for Hymnal's strictly-stepwise
/// contract, `MAX_INTERVAL` otherwise), and the register floor all
/// survive. Lines whose peaks already sit below their cap are left
/// untouched — natural contour variation stays. After demotion the
/// per-line single-climax rule is re-asserted on changed lines.
///
/// Returns the per-line [`SectionClimaxRule`]s for the downstream
/// cadence-formula pass, which validates its candidates against them
/// so a rewritten ending cannot reintroduce a demoted peak (or rewrite
/// the carrier's peak away). Degenerate sections — fewer than two
/// lines, or a carrier whose peak sits on the register floor — return
/// all-`Free` rules and change nothing.
pub(super) fn apply_section_climax(
    notes: &mut [GeneratedNote],
    line_syllables: &[u32],
    scale: Option<Scale>,
    range: (u8, u8),
    max_adjacent: i16,
    seed: u64,
) -> Vec<SectionClimaxRule> {
    let total = line_syllables.len();
    let mut rules = vec![SectionClimaxRule::Free; total];
    if total < 2 || notes.is_empty() {
        return rules;
    }
    // Per-line note spans, in lyric order (one note per syllable).
    let mut spans: Vec<(usize, usize)> = Vec::with_capacity(total);
    let mut cursor = 0usize;
    for &syl in line_syllables {
        let n = (syl as usize).min(notes.len().saturating_sub(cursor));
        spans.push((cursor, cursor + n));
        cursor += n;
    }
    let carrier = section_climax_line(total);
    let (cs, ce) = spans[carrier];
    let Some(peak) = notes[cs..ce].iter().map(|n| n.note).max() else {
        return rules;
    };
    let lo = range.0;
    if peak < lo + 2 {
        return rules;
    }
    rules[carrier] = SectionClimaxRule::Carrier { peak };
    for (li, &(s, e)) in spans.iter().enumerate() {
        if li == carrier {
            continue;
        }
        let margin = section_peak_margin(seed, li / 4);
        let cap = peak.saturating_sub(margin).max(lo + 1);
        rules[li] = SectionClimaxRule::Capped { cap };
        if s >= e {
            continue;
        }
        if demote_at_or_above(&mut notes[s..e], cap, None, scale, range, None, max_adjacent) {
            // Demotion can leave duplicate maxima inside the line;
            // re-assert the per-line single-climax rule (demote-only,
            // so the section cap keeps holding). Early tie-break: the
            // repaired climax must stay clear of the penult or the
            // cadence-formula pass can't rewrite the line ending
            // without orphaning the peak.
            enforce_single_climax(&mut notes[s..e], scale, range, None, false, false);
        }
    }
    rules
}

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
/// [`apply_section_climax`]; pass an empty slice to skip), and the SVS
/// `MAX_INTERVAL` adjacency cap, and lines where no candidate survives
/// keep their walked ending. The deceptive roll draws from a dedicated
/// rng seeded from the section seed so the swap is deterministic and
/// independent of the styles' draw sequences.
#[allow(clippy::too_many_arguments)]
pub(super) fn apply_line_cadence_formulas(
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
