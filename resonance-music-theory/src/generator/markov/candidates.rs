//! Transition-table lookups and weighted sampling for the Markov walk.
//!
//! One responsibility: turning a [`MarkovTable`] plus a history into a
//! deterministic candidate list, and drawing from that list. Nothing
//! here knows about phrases, gaps or constraints — it is the raw
//! probabilistic layer the fill and cadence phases sit on.
//!
//! Determinism is the whole point of this module: candidate lists are
//! always merged and sorted by [`Degree`] before they reach the RNG, so
//! `HashMap` iteration order can never leak into generated material.

use std::collections::{BTreeMap, HashMap, HashSet};

use crate::rng::XorShift;

use super::super::degree::Degree;
use super::super::table::MarkovTable;
use super::BIAS_WINDOW;

/// An order-1 view of a transition table: last degree -> successors.
///
/// `BTreeMap` (not `HashMap`) so downstream iteration — notably
/// [`precompute_reachability`] — is deterministic across runs.
pub(super) type Order1Graph = BTreeMap<Degree, Vec<(Degree, f32)>>;

/// Memoized back-off candidate lists keyed by history suffix. The
/// back-off path merges every table key whose tail matches the suffix
/// — an O(|transitions|) scan — so results are cached per suffix for
/// the duration of one `generate` call. The empty suffix caches the
/// full-marginalization fallback. An empty cached list means "this
/// suffix matched nothing"; it is kept so the scan isn't repeated.
pub(super) type SuffixCache = HashMap<Vec<Degree>, Vec<(Degree, f32)>>;

/// Get candidate transitions for the current `history` from `table`,
/// with automatic order back-off. Returns a list of (degree, weight)
/// pairs sorted by degree for deterministic sampling.
pub(super) fn get_candidates(
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
///
/// Draws exactly one `f32` from `rng` per call: the phases above must
/// keep the number and order of these draws stable, since generated
/// material is persisted in user projects.
pub(super) fn weighted_sample(candidates: &[(Degree, f32)], rng: &mut XorShift) -> Degree {
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
pub(super) fn marginalize_to_order1(table: &MarkovTable) -> Order1Graph {
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
pub(super) fn precompute_reachability(
    order1: &Order1Graph,
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
pub(super) fn collect_all_degrees(table: &MarkovTable) -> Vec<Degree> {
    table.degrees()
}
