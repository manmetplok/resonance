//! Markov chain generator for chord progressions.
//!
//! Given a [`MarkovTable`], a seed, and optional constraints (start / end
//! degrees, locked positions), samples a chord progression of a requested
//! length. Locked chords act as fixed waypoints: the generator fills the
//! gaps between them, biasing toward degrees that can reach the next
//! waypoint as the gap narrows.
//!
//! # Phrase-model overlay
//!
//! On top of the raw Markov walk, a phrase-model overlay (Open Music
//! Theory's T→PD→D functional arc) shapes the output:
//!
//! - Slots are grouped into phrases of [`PHRASE_SLOTS`] (a 4-bar group
//!   at the app's default of one chord per bar).
//! - Within each phrase the walk is masked to traverse the arc exactly
//!   once: it opens on tonic function, may prolong T or move to
//!   predominant (never regressing once PD is reached), is forced to PD
//!   on the penultimate slot, and places the cadential dominant on the
//!   phrase-final slot — so the dominant starts on the downbeat of the
//!   group's last bar and resolves onto the next hyper-downbeat (the
//!   following phrase's tonic opening, or the loop start). Premature
//!   D→T resolutions and T/PD ping-pong mid-phrase are impossible by
//!   construction.
//! - Harmonic rhythm accelerates into the cadence: the forced-PD slot
//!   of each full phrase is split in half (e.g. `| I | vi | IV ii | V |`),
//!   recorded as [`SplitChord`]s alongside the slot-aligned chords.
//! - User constraints always win: locked slots and start/end degrees are
//!   never masked, and when a phrase ends on a fixed tonic the cadence
//!   shifts left (`… PD D | T`) instead of fighting the constraint.

use std::collections::{BTreeMap, HashMap, HashSet};

use crate::rng::XorShift;

use super::degree::Degree;
use super::table::{HarmonicFunction, MarkovTable};
use super::{GenContext, GenerateError, GeneratedChord, GeneratedMaterial, SplitChord};

/// When within this many transitions of a fixed successor, boost the
/// probability of degrees that can reach it.
const BIAS_WINDOW: usize = 3;

/// Multiplicative boost applied to reachable degrees inside the bias
/// window. Larger values make the generator more likely to hit the
/// target but reduce variety.
const REACHABILITY_BOOST: f32 = 5.0;

/// Grid slots per phrase for the phrase-model overlay. Four slots is a
/// 4-bar hypermeasure at the app's default of one chord slot per bar,
/// and matches the groups-of-four phrase planning used by the melody
/// side. Progressions whose length is not a multiple of four end with a
/// short phrase that degrades gracefully (see [`build_phrase_plans`]).
const PHRASE_SLOTS: usize = 4;

/// Function "levels" for arc monotonicity: T = 0, PD = 1, D = 2.
fn flevel(f: HarmonicFunction) -> u8 {
    match f {
        HarmonicFunction::Tonic => 0,
        HarmonicFunction::Predominant => 1,
        HarmonicFunction::Dominant => 2,
    }
}

/// Sample a Markov chord progression.
///
/// This is the entry point called by `GeneratorSpec::MarkovProgression`.
/// The function is pure: identical inputs always produce identical output.
pub fn generate(
    length: u8,
    table_id: &str,
    order: u8,
    start: Option<Degree>,
    end: Option<Degree>,
    seed: u64,
    ctx: &GenContext,
) -> Result<GeneratedMaterial, GenerateError> {
    let table = ctx
        .registry
        .get(table_id)
        .ok_or_else(|| GenerateError::TableNotFound(table_id.to_string()))?;

    let len = length as usize;
    if len == 0 {
        return Ok(GeneratedMaterial {
            chords: vec![],
            splits: vec![],
        });
    }

    let mut rng = XorShift::new(seed);

    // --- 1. Place locked chords ----------------------------------------
    let mut output: Vec<Option<GeneratedChord>> = vec![None; len];
    for (i, slot) in ctx.locked.iter().enumerate().take(len) {
        if let Some(degree) = slot {
            output[i] = Some(GeneratedChord {
                degree: *degree,
                locked: true,
            });
        }
    }

    // --- 2. Place start / end constraints on unlocked positions --------
    if let Some(start_deg) = start {
        if output[0].is_none() {
            output[0] = Some(GeneratedChord {
                degree: start_deg,
                locked: false,
            });
        }
    }
    if let Some(end_deg) = end {
        if output[len - 1].is_none() {
            output[len - 1] = Some(GeneratedChord {
                degree: end_deg,
                locked: false,
            });
        }
    }

    // --- 3. Precompute helpers -----------------------------------------
    let order1 = marginalize_to_order1(table);
    let all_degrees = collect_all_degrees(table);

    // Snapshot of pre-placed degrees (locks + start/end constraints).
    // The phrase plans are built against these, fixed slots are never
    // masked, and a fixed slot resets the arc level (a user lock
    // legitimately restarts the phrase arc).
    let prefixed: Vec<Option<Degree>> = output
        .iter()
        .map(|slot| slot.as_ref().map(|c| c.degree))
        .collect();
    let (plans, split_slots) = build_phrase_plans(len, &prefixed, table);

    // Memoized back-off results, shared across the whole fill (the
    // table is immutable for the duration of this call). Without it,
    // every slot that backs off re-scans all of `table.transitions`
    // for the same suffix.
    let mut suffix_cache: SuffixCache = HashMap::new();

    // A user-registered table with no transitions would otherwise drive
    // `weighted_sample` past its assertions and panic in release. The
    // registry is open (third parties register via `register`), so this
    // is reachable through the public API.
    if all_degrees.is_empty() {
        return Err(GenerateError::EmptyTable(table_id.to_string()));
    }

    // Use the spec's order as the effective conditioning length, which
    // may be shorter than the table's order (forcing back-off) or longer
    // (extra history is simply ignored).
    let effective_order = order as usize;

    // --- 4. Fill gaps --------------------------------------------------
    let mut i = 0;
    while i < len {
        if output[i].is_some() {
            i += 1;
            continue;
        }

        // Found a gap at `gap_start`. Scan for its end.
        let gap_start = i;
        while i < len && output[i].is_none() {
            i += 1;
        }
        let gap_end = i; // exclusive; output[gap_end] is the successor (if in range)

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
        // `reachable[k]` = degrees that can reach `successor` in at most
        // k transitions through the order-1 graph.
        let reachable: Option<Vec<HashSet<Degree>>> =
            successor.map(|succ| precompute_reachability(&order1, succ, gap_end - gap_start));

        // Build initial history from the chord(s) preceding the gap.
        let mut history: Vec<Degree> = Vec::new();
        {
            let lookback = effective_order.max(table.order as usize);
            let start_idx = gap_start.saturating_sub(lookback);
            for chord in output[start_idx..gap_start].iter().flatten() {
                // Root position for the same reason as `successor`.
                history.push(chord.degree.root_position());
            }
        }

        // Fill left-to-right. `pos` is used for distance arithmetic,
        // not just indexing, so an iterator+enumerate is less clear.
        #[allow(clippy::needless_range_loop)]
        for pos in gap_start..gap_end {
            // Distance (in transitions) from this position to the successor.
            // pos -> pos+1 -> ... -> gap_end is (gap_end - pos) transitions.
            let dist_to_succ = gap_end - pos;

            // Condition on a sliding window over the tail of `history`
            // instead of trimming the front after every push —
            // `Vec::remove(0)` shifts all remaining elements, making the
            // fill loop O(n²) in the gap length.
            let window_start = history.len().saturating_sub(effective_order);
            let mut candidates = get_candidates(
                table,
                &history[window_start..],
                effective_order,
                &mut suffix_cache,
            );
            if candidates.is_empty() {
                candidates = all_degrees.iter().map(|&d| (d, 1.0)).collect();
            }

            // --- Phrase-model overlay: mask by harmonic function -------
            // `premask` is kept so hard constraints (locked successors,
            // end degree) can still be satisfied when the function mask
            // and the reachability filter conflict — constraints win
            // over the overlay.
            let premask = candidates.clone();
            {
                let (plan_min, plan_max) = plans[pos];
                let min = plan_min.max(phrase_level_before(pos, &output, &prefixed, table));
                if min <= plan_max {
                    let masked: Vec<(Degree, f32)> = candidates
                        .iter()
                        .filter(|(d, _)| {
                            let l = flevel(table.function_of(*d));
                            l >= min && l <= plan_max
                        })
                        .cloned()
                        .collect();
                    if !masked.is_empty() {
                        candidates = masked;
                    } else {
                        // The transition row has no degree of the
                        // required function: fall back to the table-wide
                        // pool of allowed-function degrees so the arc
                        // survives sparse rows. If the table has no such
                        // degree at all (e.g. a dominant-less user
                        // table), drop the constraint for this slot.
                        let pool: Vec<(Degree, f32)> = all_degrees
                            .iter()
                            .filter(|&&d| {
                                let l = flevel(table.function_of(d));
                                l >= min && l <= plan_max
                            })
                            .map(|&d| (d, 1.0))
                            .collect();
                        if !pool.is_empty() {
                            candidates = pool;
                        }
                    }
                }
                // min > plan_max happens only when a fixed slot forced
                // the arc past this slot's ceiling (e.g. a locked V
                // mid-phrase); the slot is left unconstrained.
            }

            // Apply reachability bias when approaching a fixed successor.
            if let Some(ref reach_levels) = reachable {
                if dist_to_succ <= BIAS_WINDOW.min(gap_end - gap_start) {
                    let level = dist_to_succ.min(reach_levels.len() - 1);
                    let reach_set = &reach_levels[level];

                    if dist_to_succ == 1 {
                        // Must directly transition to successor — filter strictly.
                        let strict: Vec<(Degree, f32)> = candidates
                            .iter()
                            .filter(|(d, _)| reach_set.contains(d))
                            .cloned()
                            .collect();
                        if !strict.is_empty() {
                            candidates = strict;
                        } else {
                            // The function mask may have excluded every
                            // degree that reaches the successor; the
                            // hard constraint outranks the overlay, so
                            // retry against the unmasked candidates.
                            let strict_premask: Vec<(Degree, f32)> = premask
                                .iter()
                                .filter(|(d, _)| reach_set.contains(d))
                                .cloned()
                                .collect();
                            if !strict_premask.is_empty() {
                                candidates = strict_premask;
                            } else if end.is_some() && successor == end.map(Degree::root_position) {
                                // The successor IS the end constraint and
                                // we can't reach it.
                                return Err(GenerateError::EndUnreachable {
                                    steps: dist_to_succ,
                                });
                            }
                            // For non-end successors (locked chords in
                            // the middle), fall through with the masked
                            // candidates — best effort.
                        }
                    } else {
                        // Boost reachable candidates.
                        for (deg, weight) in &mut candidates {
                            if reach_set.contains(deg) {
                                *weight *= REACHABILITY_BOOST;
                            }
                        }
                    }
                }
            }

            let sampled = weighted_sample(&candidates, &mut rng);
            output[pos] = Some(GeneratedChord {
                degree: sampled,
                locked: false,
            });
            history.push(sampled);
        }
    }

    // --- 5. Validate end constraint ------------------------------------
    if let Some(end_deg) = end {
        let last = output[len - 1]
            .as_ref()
            .expect("all positions should be filled");
        if last.degree != end_deg {
            return if last.locked {
                Err(GenerateError::EndConflictsWithLock)
            } else {
                Err(GenerateError::EndUnreachable { steps: 0 })
            };
        }
    }

    // --- 6. Harmonic-rhythm acceleration into the cadence ---------------
    // Split the forced-PD slot of each full phrase in half, sampling a
    // second, different predominant for the back half (e.g. `IV ii`
    // before the cadential `V`). Doubling the harmonic rhythm right
    // before the dominant is the hypermeter acceleration of bars 4/8.
    let mut splits: Vec<SplitChord> = Vec::new();
    for &slot in &split_slots {
        // Never split fixed slots: a lock's degree and duration must
        // carry through regeneration untouched.
        if prefixed[slot].is_some() {
            continue;
        }
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
        let mut candidates = get_candidates(table, &history, effective_order, &mut suffix_cache);

        // Strictly predominant-function and different from the front
        // half — a repeated chord would be no acceleration at all. No
        // pool fallback here: if the row offers no second predominant,
        // skip the split rather than break the table's voice.
        candidates.retain(|(d, _)| {
            *d != first_half && table.function_of(*d) == HarmonicFunction::Predominant
        });
        if candidates.is_empty() {
            continue;
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

        let degree = weighted_sample(&candidates, &mut rng);
        splits.push(SplitChord {
            slot: slot as u8,
            degree,
        });
    }

    // --- 7. Assemble output --------------------------------------------
    let mut chords: Vec<GeneratedChord> = output
        .into_iter()
        .map(|o| o.expect("all positions should be filled"))
        .collect();

    // --- 8. Inversion decorations (research §2C) -------------------------
    // Pre-dominant bass idioms (IV-precedes-ii ordering, ii6 walking the
    // bass 4→5) and the cadential 6/4 on phrase-final dominants. Sampled
    // material only — locked and constrained slots carry through
    // untouched. See `super::inversion`.
    super::inversion::decorate_inversions(
        &mut chords,
        &mut splits,
        &prefixed,
        &plans,
        table,
        &mut rng,
    );

    Ok(GeneratedMaterial { chords, splits })
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

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
fn build_phrase_plans(
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
fn phrase_level_before(
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

/// Memoized back-off candidate lists keyed by history suffix. The
/// back-off path merges every table key whose tail matches the suffix
/// — an O(|transitions|) scan — so results are cached per suffix for
/// the duration of one `generate` call. The empty suffix caches the
/// full-marginalization fallback. An empty cached list means "this
/// suffix matched nothing"; it is kept so the scan isn't repeated.
type SuffixCache = HashMap<Vec<Degree>, Vec<(Degree, f32)>>;

/// Get candidate transitions for the current `history` from `table`,
/// with automatic order back-off. Returns a list of (degree, weight)
/// pairs sorted by degree for deterministic sampling.
fn get_candidates(
    table: &MarkovTable,
    history: &[Degree],
    effective_order: usize,
    cache: &mut SuffixCache,
) -> Vec<(Degree, f32)> {
    let table_order = table.order as usize;

    // Try exact key match at the requested conditioning length.
    let try_order = effective_order.min(table_order);
    if history.len() >= try_order && try_order > 0 {
        let key: Vec<Degree> = history[history.len() - try_order..].to_vec();
        if let Some(transitions) = table.transitions.get(&key) {
            if !transitions.is_empty() {
                return sorted_candidates(transitions);
            }
        }
    }

    // Back off: try progressively shorter suffix matches.
    for len in (1..try_order).rev() {
        if history.len() >= len {
            let suffix = &history[history.len() - len..];
            if let Some(cached) = cache.get(suffix) {
                if cached.is_empty() {
                    continue; // known dead suffix — back off further
                }
                return cached.clone();
            }
            let mut merged: Vec<(Degree, f32)> = Vec::new();
            for (key, transitions) in &table.transitions {
                if key.len() >= len && key[key.len() - len..] == *suffix {
                    merged.extend(transitions.iter().cloned());
                }
            }
            let result = sorted_candidates(&merged);
            cache.insert(suffix.to_vec(), result.clone());
            if !result.is_empty() {
                return result;
            }
        }
    }

    // No history match: merge all transitions (marginalize completely).
    cache
        .entry(Vec::new())
        .or_insert_with(|| {
            let mut all: Vec<(Degree, f32)> = Vec::new();
            for transitions in table.transitions.values() {
                all.extend(transitions.iter().cloned());
            }
            sorted_candidates(&all)
        })
        .clone()
}

/// Merge duplicate degrees by summing their weights and sort by degree
/// for deterministic iteration order. Merging is necessary because
/// back-off can collect the same degree from multiple conditioning keys,
/// and sorting is necessary because `HashMap` iteration order is
/// non-deterministic across runs (hash randomization).
fn sorted_candidates(candidates: &[(Degree, f32)]) -> Vec<(Degree, f32)> {
    let mut map: HashMap<Degree, f32> = HashMap::new();
    for &(d, w) in candidates {
        *map.entry(d).or_insert(0.0) += w;
    }
    let mut v: Vec<(Degree, f32)> = map.into_iter().collect();
    v.sort_by_key(|a| a.0);
    v
}

/// Sample a degree from a weighted candidate list using the provided RNG.
fn weighted_sample(candidates: &[(Degree, f32)], rng: &mut XorShift) -> Degree {
    debug_assert!(!candidates.is_empty());
    let total: f32 = candidates.iter().map(|(_, w)| w).sum();
    if total <= 0.0 {
        return candidates[0].0;
    }
    let r: f32 = rng.next_f32() * total;
    let mut acc = 0.0;
    for &(deg, w) in candidates {
        acc += w;
        if r < acc {
            return deg;
        }
    }
    candidates.last().unwrap().0
}

/// Build an order-1 view of a table by marginalizing higher-order keys
/// down to the last element. For an order-1 table, returns the table's
/// transitions as-is (unwrapping the single-element keys).
///
/// Uses `BTreeMap` so downstream iteration (e.g. `precompute_reachability`)
/// is deterministic across runs.
fn marginalize_to_order1(table: &MarkovTable) -> BTreeMap<Degree, Vec<(Degree, f32)>> {
    let mut merged: BTreeMap<Degree, BTreeMap<Degree, f32>> = BTreeMap::new();
    for (key, transitions) in &table.transitions {
        if let Some(&last) = key.last() {
            let entry = merged.entry(last).or_default();
            for &(deg, w) in transitions {
                *entry.entry(deg).or_insert(0.0) += w;
            }
        }
    }
    merged
        .into_iter()
        .map(|(k, v)| (k, v.into_iter().collect()))
        .collect()
}

/// Precompute reachability from `target` in the order-1 transition graph.
///
/// Returns a vector where `result[k]` is the set of degrees that can
/// reach `target` in at most `k` transitions. `result[0]` always contains
/// just the target itself.
fn precompute_reachability(
    order1: &BTreeMap<Degree, Vec<(Degree, f32)>>,
    target: Degree,
    max_steps: usize,
) -> Vec<HashSet<Degree>> {
    let capped = max_steps.min(BIAS_WINDOW);
    let mut result = Vec::with_capacity(capped + 1);
    let mut cumulative = HashSet::new();
    cumulative.insert(target);
    result.push(cumulative.clone());

    for _ in 1..=capped {
        let prev = cumulative.clone();
        for (deg, transitions) in order1 {
            if transitions
                .iter()
                .any(|(t, w)| prev.contains(t) && *w > 0.0)
            {
                cumulative.insert(*deg);
            }
        }
        result.push(cumulative.clone());
    }

    result
}

/// Collect all unique degrees that appear in a table (both as keys and
/// as successors). Sorted for deterministic fallback sampling.
fn collect_all_degrees(table: &MarkovTable) -> Vec<Degree> {
    table.degrees()
}
