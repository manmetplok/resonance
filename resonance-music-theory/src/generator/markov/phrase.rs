//! The phrase-model overlay: Open Music Theory's T -> PD -> D functional
//! arc, expressed as a per-slot window of allowed harmonic functions.
//!
//! One responsibility: deciding *which functions a slot may take*. It
//! plans the arc for the whole progression ([`build_phrase_plans`]),
//! tracks how far the arc has already advanced inside the current
//! phrase ([`phrase_level_before`]) and masks a candidate list down to
//! the allowed functions ([`mask_to_arc`]). It never samples, so it can
//! be exercised on its own without an RNG.

use super::super::degree::Degree;
use super::super::table::{HarmonicFunction, MarkovTable};
use super::super::GeneratedChord;

/// Grid slots per phrase for the phrase-model overlay. Four slots is a
/// 4-bar hypermeasure at the app's default of one chord slot per bar,
/// and matches the groups-of-four phrase planning used by the melody
/// side. Progressions whose length is not a multiple of four end with a
/// short phrase that degrades gracefully (see [`build_phrase_plans`]).
pub const PHRASE_SLOTS: usize = 4;

/// Function "levels" for arc monotonicity: T = 0, PD = 1, D = 2.
pub fn flevel(f: HarmonicFunction) -> u8 {
    match f {
        HarmonicFunction::Tonic => 0,
        HarmonicFunction::Predominant => 1,
        HarmonicFunction::Dominant => 2,
    }
}

/// Build per-slot function-level windows `(min, max)` for the
/// phrase-model overlay, plus the slots eligible for harmonic-rhythm
/// splitting (one forced-PD slot per full [`PHRASE_SLOTS`] phrase).
///
/// Standard plan for an `n`-slot phrase (levels: T = 0, PD = 1, D = 2):
///
/// - slot `0`: `(0, 0)` — open on tonic function;
/// - slots `1..n-2`: `(0, 1)` — prolong T or move to PD, never D early
///   (the dynamic minimum in the sampler prevents PD→T regression);
/// - slot `n-2`: `(1, 1)` — forced predominant prepares the cadence;
/// - slot `n-1`: `(2, 2)` — cadential dominant on the phrase-final
///   slot, resolving onto the next hyper-downbeat.
///
/// When the phrase-final slot is pre-fixed with a tonic-function degree
/// (an `end: I` constraint or a lock), the cadence shifts left so the
/// arc still completes inside the phrase: `… (1,1) (2,2) [fixed T]`.
/// Single-slot phrases are pinned to tonic (they sit on a
/// hyper-downbeat right after the previous phrase's dominant).
pub fn build_phrase_plans(
    len: usize,
    prefixed: &[Option<Degree>],
    table: &MarkovTable,
) -> (Vec<(u8, u8)>, Vec<usize>) {
    let mut plans = vec![(0u8, 2u8); len];
    let mut split_slots = Vec::new();

    let mut start = 0;
    while start < len {
        let n = PHRASE_SLOTS.min(len - start);
        let final_fixed_tonic = n >= 2
            && prefixed[start + n - 1]
                .is_some_and(|d| table.function_of(d) == HarmonicFunction::Tonic);

        // Phrase-relative positions of the cadential dominant and the
        // forced predominant that prepares it.
        let (d_slot, pd_slot) = if n == 1 {
            (None, None)
        } else if final_fixed_tonic {
            (Some(n - 2), n.checked_sub(3))
        } else {
            (Some(n - 1), n.checked_sub(2).filter(|&pd| pd >= 1))
        };

        for rel in 0..n {
            plans[start + rel] = if Some(rel) == d_slot {
                (2, 2)
            } else if Some(rel) == pd_slot {
                (1, 1)
            } else if final_fixed_tonic && rel == n - 1 {
                (0, 2) // pre-fixed anyway; never masked
            } else if rel == 0 {
                (0, 0)
            } else {
                (0, 1)
            };
        }

        // Only full phrases accelerate — splitting the already-short
        // remainder phrases would over-crowd them.
        if n == PHRASE_SLOTS {
            if let Some(pd) = pd_slot {
                split_slots.push(start + pd);
            }
        }
        start += n;
    }

    (plans, split_slots)
}

/// Highest function level reached so far in `pos`'s phrase, scanning
/// the (already filled) slots before `pos`. Sampled slots ratchet the
/// level up (arc monotonicity); pre-fixed slots *reset* it to their own
/// function, because a user lock or start/end constraint legitimately
/// restarts the arc.
pub fn phrase_level_before(
    pos: usize,
    output: &[Option<GeneratedChord>],
    prefixed: &[Option<Degree>],
    table: &MarkovTable,
) -> u8 {
    let phrase_start = (pos / PHRASE_SLOTS) * PHRASE_SLOTS;
    let mut level = 0u8;
    for slot in phrase_start..pos {
        let Some(chord) = output[slot].as_ref() else {
            continue;
        };
        let l = flevel(table.function_of(chord.degree));
        level = if prefixed[slot].is_some() { l } else { level.max(l) };
    }
    level
}

/// Mask a candidate list down to the harmonic functions this slot's
/// phrase plan allows.
///
/// `plan` is the slot's `(min, max)` window from [`build_phrase_plans`]
/// and `reached_level` is [`phrase_level_before`] for the slot; the
/// effective minimum is the larger of the two, which is what keeps the
/// arc monotonic (no PD→T regression mid-phrase).
///
/// Returns `Some(candidates)` when the overlay applies and `None` when
/// the slot must be left unconstrained, which happens when:
///
/// - a fixed slot already forced the arc past this slot's ceiling
///   (e.g. a locked V mid-phrase) — user constraints outrank the
///   overlay; or
/// - neither the transition row *nor* the whole table offers a degree
///   of the required function (e.g. a dominant-less user table).
///
/// When the transition row itself has no degree of the required
/// function, the mask falls back to the table-wide pool of
/// allowed-function degrees (weight 1.0 each) so the arc survives
/// sparse rows.
pub fn mask_to_arc(
    candidates: &[(Degree, f32)],
    plan: (u8, u8),
    reached_level: u8,
    table: &MarkovTable,
    all_degrees: &[Degree],
) -> Option<Vec<(Degree, f32)>> {
    let (plan_min, plan_max) = plan;
    let min = plan_min.max(reached_level);
    if min > plan_max {
        return None;
    }

    let allowed = |d: Degree| {
        let l = flevel(table.function_of(d));
        l >= min && l <= plan_max
    };

    let masked: Vec<(Degree, f32)> = candidates
        .iter()
        .filter(|(d, _)| allowed(*d))
        .cloned()
        .collect();
    if !masked.is_empty() {
        return Some(masked);
    }

    let pool: Vec<(Degree, f32)> = all_degrees
        .iter()
        .filter(|&&d| allowed(d))
        .map(|&d| (d, 1.0))
        .collect();
    if pool.is_empty() {
        None
    } else {
        Some(pool)
    }
}
