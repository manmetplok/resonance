//! Markov chain generator for chord progressions.
//!
//! Given a [`MarkovTable`], a seed, and optional constraints (start / end
//! degrees, locked positions), samples a chord progression of a requested
//! length. Locked chords act as fixed waypoints: the generator fills the
//! gaps between them, biasing toward degrees that can reach the next
//! waypoint as the gap narrows.

use std::collections::{BTreeMap, HashMap, HashSet};

use rand::rngs::SmallRng;
use rand::{Rng, SeedableRng};

use super::degree::Degree;
use super::table::MarkovTable;
use super::{GenContext, GenerateError, GeneratedChord, GeneratedMaterial};

/// When within this many transitions of a fixed successor, boost the
/// probability of degrees that can reach it.
const BIAS_WINDOW: usize = 3;

/// Multiplicative boost applied to reachable degrees inside the bias
/// window. Larger values make the generator more likely to hit the
/// target but reduce variety.
const REACHABILITY_BOOST: f32 = 5.0;

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
        return Ok(GeneratedMaterial { chords: vec![] });
    }

    let mut rng = SmallRng::seed_from_u64(seed);

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
        let successor = if gap_end < len {
            output[gap_end].as_ref().map(|c| c.degree)
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
                history.push(chord.degree);
            }
        }

        // Fill left-to-right. `pos` is used for distance arithmetic,
        // not just indexing, so an iterator+enumerate is less clear.
        #[allow(clippy::needless_range_loop)]
        for pos in gap_start..gap_end {
            // Distance (in transitions) from this position to the successor.
            // pos -> pos+1 -> ... -> gap_end is (gap_end - pos) transitions.
            let dist_to_succ = gap_end - pos;

            let mut candidates = get_candidates(table, &history, effective_order);
            if candidates.is_empty() {
                candidates = all_degrees.iter().map(|&d| (d, 1.0)).collect();
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
                        } else if end.is_some() && successor == end {
                            // The successor IS the end constraint and we
                            // can't reach it.
                            return Err(GenerateError::EndUnreachable {
                                steps: dist_to_succ,
                            });
                        }
                        // For non-end successors (locked chords in the
                        // middle), fall through with the original
                        // candidates — best effort.
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

            // Trim history to the effective conditioning length.
            while history.len() > effective_order {
                history.remove(0);
            }
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

    // --- 6. Assemble output --------------------------------------------
    let chords = output
        .into_iter()
        .map(|o| o.expect("all positions should be filled"))
        .collect();

    Ok(GeneratedMaterial { chords })
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Get candidate transitions for the current `history` from `table`,
/// with automatic order back-off. Returns a list of (degree, weight)
/// pairs sorted by degree for deterministic sampling.
fn get_candidates(
    table: &MarkovTable,
    history: &[Degree],
    effective_order: usize,
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
            let mut merged: Vec<(Degree, f32)> = Vec::new();
            for (key, transitions) in &table.transitions {
                if key.len() >= len && key[key.len() - len..] == *suffix {
                    merged.extend(transitions.iter().cloned());
                }
            }
            if !merged.is_empty() {
                return sorted_candidates(&merged);
            }
        }
    }

    // No history match: merge all transitions (marginalize completely).
    let mut all: Vec<(Degree, f32)> = Vec::new();
    for transitions in table.transitions.values() {
        all.extend(transitions.iter().cloned());
    }
    sorted_candidates(&all)
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
fn weighted_sample(candidates: &[(Degree, f32)], rng: &mut SmallRng) -> Degree {
    debug_assert!(!candidates.is_empty());
    let total: f32 = candidates.iter().map(|(_, w)| w).sum();
    if total <= 0.0 {
        return candidates[0].0;
    }
    let r: f32 = rng.gen::<f32>() * total;
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
