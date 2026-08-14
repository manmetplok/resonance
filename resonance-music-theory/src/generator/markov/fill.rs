//! Phase 4 of the Markov generator: filling the gaps between fixed
//! chords.
//!
//! One responsibility: walking every unfixed slot left-to-right and
//! sampling a degree for it. Each slot combines three inputs, in this
//! order:
//!
//! 1. the back-off candidate list for the current history
//!    ([`super::candidates::get_candidates`]);
//! 2. the phrase-model function mask ([`super::phrase::mask_to_arc`]);
//! 3. the reachability bias toward the gap's fixed successor.
//!
//! Exactly one RNG draw happens per slot, at the end. Generated
//! material is persisted in user projects, so neither the number nor
//! the order of those draws may change.

use std::collections::HashSet;

use crate::rng::XorShift;

use super::super::degree::Degree;
use super::super::table::MarkovTable;
use super::super::{GenerateError, GeneratedChord};
use super::candidates::{
    get_candidates, precompute_reachability, weighted_sample, Order1Graph, SuffixCache,
};
use super::phrase::{mask_to_arc, phrase_level_before};
use super::{BIAS_WINDOW, REACHABILITY_BOOST};

/// Everything phase 4 reads but never changes: the table, the
/// precomputed graphs and the user's constraints.
pub(super) struct FillContext<'a> {
    /// Transition table being sampled.
    pub table: &'a MarkovTable,
    /// Order-1 view of `table`, used for reachability.
    pub order1: &'a Order1Graph,
    /// Every degree the table knows, for the empty-row fallback.
    pub all_degrees: &'a [Degree],
    /// Conditioning length requested by the spec.
    pub effective_order: usize,
    /// Per-slot function windows from the phrase model.
    pub plans: &'a [(u8, u8)],
    /// Degrees fixed before sampling started (locks + start/end).
    pub prefixed: &'a [Option<Degree>],
    /// The spec's end constraint, if any.
    pub end: Option<Degree>,
}

/// The state phase 4 mutates as it walks the progression.
pub(super) struct FillState<'a> {
    /// Slots being filled; `None` entries are the gaps.
    pub output: &'a mut Vec<Option<GeneratedChord>>,
    /// Back-off memo shared across the whole fill.
    pub cache: &'a mut SuffixCache,
    /// The generator's RNG.
    pub rng: &'a mut XorShift,
}

/// One run of unfixed slots, plus what is known about its far end.
struct Gap {
    /// First unfixed slot.
    start: usize,
    /// One past the last unfixed slot; `output[end]` is the successor.
    end: usize,
    /// Root-position degree of the chord after the gap, if in range.
    successor: Option<Degree>,
    /// `reachable[k]` = degrees that can reach `successor` in at most
    /// `k` transitions. `None` when the gap runs to the end.
    reachable: Option<Vec<HashSet<Degree>>>,
}

/// Fill every gap in `state.output`, left to right.
pub(super) fn fill_gaps(
    ctx: &FillContext<'_>,
    state: &mut FillState<'_>,
) -> Result<(), GenerateError> {
    let len = state.output.len();
    let mut i = 0;
    while i < len {
        if state.output[i].is_some() {
            i += 1;
            continue;
        }

        // Found a gap at `gap_start`. Scan for its end.
        let gap_start = i;
        while i < len && state.output[i].is_none() {
            i += 1;
        }
        let gap_end = i; // exclusive; output[gap_end] is the successor (if in range)

        let gap = build_gap(ctx, state.output, gap_start, gap_end, len);
        fill_gap(ctx, state, &gap)?;
    }
    Ok(())
}

/// Describe a gap: its fixed successor and the reachability sets used
/// to bias toward it.
fn build_gap(
    ctx: &FillContext<'_>,
    output: &[Option<GeneratedChord>],
    gap_start: usize,
    gap_end: usize,
    len: usize,
) -> Gap {
    // Successor chord (first fixed position after the gap, if any).
    // Normalized to root position: a locked chord may carry an
    // inversion decoration from a previous generation, but the
    // transition graph only knows root-position degrees.
    let successor = if gap_end < len {
        output[gap_end].as_ref().map(|c| c.degree.root_position())
    } else {
        None
    };

    // Precompute reachability from the successor for biasing.
    let reachable = successor
        .map(|succ| precompute_reachability(ctx.order1, succ, gap_end - gap_start));

    Gap {
        start: gap_start,
        end: gap_end,
        successor,
        reachable,
    }
}

/// Sample every slot of one gap.
fn fill_gap(
    ctx: &FillContext<'_>,
    state: &mut FillState<'_>,
    gap: &Gap,
) -> Result<(), GenerateError> {
    // Build initial history from the chord(s) preceding the gap.
    let mut history: Vec<Degree> = Vec::new();
    {
        let lookback = ctx.effective_order.max(ctx.table.order as usize);
        let start_idx = gap.start.saturating_sub(lookback);
        for chord in state.output[start_idx..gap.start].iter().flatten() {
            // Root position for the same reason as `Gap::successor`.
            history.push(chord.degree.root_position());
        }
    }

    // Fill left-to-right. `pos` is used for distance arithmetic,
    // not just indexing, so an iterator+enumerate is less clear.
    #[allow(clippy::needless_range_loop)]
    for pos in gap.start..gap.end {
        let sampled = sample_slot(ctx, state, gap, pos, &history)?;
        state.output[pos] = Some(GeneratedChord {
            degree: sampled,
            locked: false,
        });
        history.push(sampled);
    }
    Ok(())
}

/// Pick the degree for a single unfixed slot: back-off lookup, phrase
/// mask, reachability bias, then one weighted draw.
fn sample_slot(
    ctx: &FillContext<'_>,
    state: &mut FillState<'_>,
    gap: &Gap,
    pos: usize,
    history: &[Degree],
) -> Result<Degree, GenerateError> {
    // Condition on a sliding window over the tail of `history`
    // instead of trimming the front after every push —
    // `Vec::remove(0)` shifts all remaining elements, making the
    // fill loop O(n²) in the gap length.
    let window_start = history.len().saturating_sub(ctx.effective_order);
    let mut candidates = get_candidates(
        ctx.table,
        &history[window_start..],
        ctx.effective_order,
        state.cache,
    );
    if candidates.is_empty() {
        candidates = ctx.all_degrees.iter().map(|&d| (d, 1.0)).collect();
    }

    // Phrase-model overlay. `premask` is kept so hard constraints
    // (locked successors, end degree) can still be satisfied when the
    // function mask and the reachability filter conflict — constraints
    // win over the overlay.
    let premask = candidates.clone();
    let reached = phrase_level_before(pos, state.output, ctx.prefixed, ctx.table);
    if let Some(masked) = mask_to_arc(
        &candidates,
        ctx.plans[pos],
        reached,
        ctx.table,
        ctx.all_degrees,
    ) {
        candidates = masked;
    }

    apply_reachability_bias(ctx, gap, pos, &premask, &mut candidates)?;

    Ok(weighted_sample(&candidates, state.rng))
}

/// Steer the candidate list toward the gap's fixed successor.
///
/// Within [`BIAS_WINDOW`] transitions of the successor, degrees that
/// can still reach it are boosted; on the last transition before it,
/// non-reaching degrees are filtered out entirely (and, if the mask
/// left nothing that reaches, the unmasked list is retried — a hard
/// constraint outranks the phrase overlay).
fn apply_reachability_bias(
    ctx: &FillContext<'_>,
    gap: &Gap,
    pos: usize,
    premask: &[(Degree, f32)],
    candidates: &mut Vec<(Degree, f32)>,
) -> Result<(), GenerateError> {
    let Some(ref reach_levels) = gap.reachable else {
        return Ok(());
    };

    // Distance (in transitions) from this position to the successor.
    // pos -> pos+1 -> ... -> gap.end is (gap.end - pos) transitions.
    let dist_to_succ = gap.end - pos;
    if dist_to_succ > BIAS_WINDOW.min(gap.end - gap.start) {
        return Ok(());
    }

    let level = dist_to_succ.min(reach_levels.len() - 1);
    let reach_set = &reach_levels[level];

    if dist_to_succ > 1 {
        // Boost reachable candidates.
        for (deg, weight) in candidates.iter_mut() {
            if reach_set.contains(deg) {
                *weight *= REACHABILITY_BOOST;
            }
        }
        return Ok(());
    }

    // Must directly transition to successor — filter strictly.
    let strict: Vec<(Degree, f32)> = candidates
        .iter()
        .filter(|(d, _)| reach_set.contains(d))
        .cloned()
        .collect();
    if !strict.is_empty() {
        *candidates = strict;
        return Ok(());
    }

    // The function mask may have excluded every degree that reaches
    // the successor; the hard constraint outranks the overlay, so
    // retry against the unmasked candidates.
    let strict_premask: Vec<(Degree, f32)> = premask
        .iter()
        .filter(|(d, _)| reach_set.contains(d))
        .cloned()
        .collect();
    if !strict_premask.is_empty() {
        *candidates = strict_premask;
    } else if ctx.end.is_some() && gap.successor == ctx.end.map(Degree::root_position) {
        // The successor IS the end constraint and we can't reach it.
        return Err(GenerateError::EndUnreachable {
            steps: dist_to_succ,
        });
    }
    // For non-end successors (locked chords in the middle), fall
    // through with the masked candidates — best effort.
    Ok(())
}
