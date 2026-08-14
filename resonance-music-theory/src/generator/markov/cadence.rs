//! Phase 6 of the Markov generator: harmonic-rhythm acceleration into
//! the cadence.
//!
//! One responsibility: splitting the forced-predominant slot of each
//! full phrase in half and sampling a second, different predominant for
//! the back half (e.g. `IV ii` before the cadential `V`). Doubling the
//! harmonic rhythm right before the dominant is the hypermeter
//! acceleration of bars 4/8.
//!
//! Runs after the fill, over already-filled slots, and draws from the
//! same RNG — one draw per slot that actually splits.

use crate::rng::XorShift;

use super::super::degree::Degree;
use super::super::table::{HarmonicFunction, MarkovTable};
use super::super::{GeneratedChord, SplitChord};
use super::candidates::{get_candidates, weighted_sample, Order1Graph, SuffixCache};

/// Sample the back-half chord for every splittable slot.
///
/// `split_slots` comes from the phrase plan; slots that are pre-fixed,
/// or whose transition row offers no second predominant, are skipped
/// without touching the RNG.
#[allow(clippy::too_many_arguments)]
pub(super) fn sample_cadence_splits(
    output: &[Option<GeneratedChord>],
    split_slots: &[usize],
    prefixed: &[Option<Degree>],
    table: &MarkovTable,
    order1: &Order1Graph,
    effective_order: usize,
    cache: &mut SuffixCache,
    rng: &mut XorShift,
) -> Vec<SplitChord> {
    let mut splits: Vec<SplitChord> = Vec::new();
    for &slot in split_slots {
        // Never split fixed slots: a lock's degree and duration must
        // carry through regeneration untouched.
        if prefixed[slot].is_some() {
            continue;
        }
        let Some(degree) = sample_back_half(
            output,
            slot,
            table,
            order1,
            effective_order,
            cache,
            rng,
        ) else {
            continue;
        };
        splits.push(SplitChord {
            slot: slot as u8,
            degree,
        });
    }
    splits
}

/// Pick the chord for the back half of `slot`, or `None` if this slot
/// cannot accelerate.
fn sample_back_half(
    output: &[Option<GeneratedChord>],
    slot: usize,
    table: &MarkovTable,
    order1: &Order1Graph,
    effective_order: usize,
    cache: &mut SuffixCache,
    rng: &mut XorShift,
) -> Option<Degree> {
    let first_half = output[slot]
        .as_ref()
        .expect("all positions should be filled")
        .degree;

    // Candidates for the back half, conditioned on the history up
    // to and including the front half.
    let window_start = (slot + 1).saturating_sub(effective_order.max(1));
    let history: Vec<Degree> = output[window_start..=slot]
        .iter()
        .map(|c| c.as_ref().expect("filled").degree.root_position())
        .collect();
    let mut candidates = get_candidates(table, &history, effective_order, cache);

    // Strictly predominant-function and different from the front
    // half — a repeated chord would be no acceleration at all. No
    // pool fallback here: if the row offers no second predominant,
    // skip the split rather than break the table's voice.
    candidates
        .retain(|(d, _)| *d != first_half && table.function_of(*d) == HarmonicFunction::Predominant);
    if candidates.is_empty() {
        return None;
    }

    // Prefer back halves with a direct path into the next slot's
    // chord (usually the cadential dominant).
    if let Some(next) = output.get(slot + 1).and_then(|c| c.as_ref()) {
        let next_deg = next.degree;
        let reaching: Vec<(Degree, f32)> = candidates
            .iter()
            .filter(|(d, _)| {
                order1
                    .get(d)
                    .is_some_and(|ts| ts.iter().any(|&(t, w)| t == next_deg && w > 0.0))
            })
            .cloned()
            .collect();
        if !reaching.is_empty() {
            candidates = reaching;
        }
    }

    Some(weighted_sample(&candidates, rng))
}
