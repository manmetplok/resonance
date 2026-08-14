//! The section-climax pass.
//!
//! One responsibility: making a section have exactly one true peak.
//! The carrier line (the srdc departure) keeps it; every other line is
//! demoted below a seeded margin under it. Per-*line* climax discipline
//! lives in `crate::derive::climax`; this pass only orchestrates the
//! lines against each other.

use crate::derive::climax::{
    demote_at_or_above, enforce_single_climax, section_peak_margin, SectionClimaxRule,
};
use crate::derive::vocal::style::section_climax_line;
use crate::derive::GeneratedNote;
use crate::scale::Scale;

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
pub(in crate::derive::vocal) fn apply_section_climax(
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
