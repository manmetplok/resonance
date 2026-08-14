//! Unit tests for the phrase-model overlay, extracted out of the
//! Markov fill loop in ba todo #1249.
//!
//! `generator::markov::phrase` decides which harmonic functions a slot
//! may take; it never samples, so it can be checked directly here
//! instead of only through generated progressions.

use std::collections::BTreeMap;

use resonance_music_theory::generator::markov::phrase::{
    build_phrase_plans, flevel, mask_to_arc, PHRASE_SLOTS,
};
use resonance_music_theory::generator::table::MarkovTable;
use resonance_music_theory::generator::{Degree, HarmonicFunction};

/// A minimal major-key table; functions come from the built-in
/// major-key defaults (I/iii/vi = T, ii/IV = PD, V/vii° = D).
fn table() -> MarkovTable {
    let mut transitions: BTreeMap<Vec<Degree>, Vec<(Degree, f32)>> = BTreeMap::new();
    for d in Degree::DIATONIC_TRIADS {
        transitions.insert(
            vec![d],
            Degree::DIATONIC_TRIADS.iter().map(|&t| (t, 1.0)).collect(),
        );
    }
    MarkovTable {
        id: "test".to_string(),
        order: 1,
        transitions,
        functions: BTreeMap::new(),
    }
}

/// A table whose degrees are all tonic-function: no PD, no D anywhere.
fn tonic_only_table() -> MarkovTable {
    let mut functions = BTreeMap::new();
    for d in Degree::DIATONIC_TRIADS {
        functions.insert(d, HarmonicFunction::Tonic);
    }
    MarkovTable {
        functions,
        ..table()
    }
}

fn all(t: &MarkovTable) -> Vec<Degree> {
    t.degrees()
}

fn weighted(degrees: &[Degree]) -> Vec<(Degree, f32)> {
    degrees.iter().map(|&d| (d, 1.0)).collect()
}

// ---------------------------------------------------------------------------
// mask_to_arc
// ---------------------------------------------------------------------------

#[test]
fn mask_keeps_only_degrees_inside_the_window() {
    let t = table();
    let candidates = weighted(&Degree::DIATONIC_TRIADS);

    // Phrase-opening slot: tonic function only.
    let masked = mask_to_arc(&candidates, (0, 0), 0, &t, &all(&t)).expect("mask applies");
    assert!(!masked.is_empty());
    for (d, _) in &masked {
        assert_eq!(
            t.function_of(*d),
            HarmonicFunction::Tonic,
            "{d} is not tonic-function"
        );
    }
}

#[test]
fn mask_forces_the_cadential_dominant() {
    let t = table();
    let candidates = weighted(&Degree::DIATONIC_TRIADS);

    let masked = mask_to_arc(&candidates, (2, 2), 0, &t, &all(&t)).expect("mask applies");
    for (d, _) in &masked {
        assert_eq!(flevel(t.function_of(*d)), 2, "{d} is not dominant-function");
    }
}

#[test]
fn reached_level_ratchets_the_minimum_up() {
    let t = table();
    let candidates = weighted(&Degree::DIATONIC_TRIADS);

    // A mid-phrase slot may be T or PD, but once the arc has reached PD
    // it must not regress to T.
    let free = mask_to_arc(&candidates, (0, 1), 0, &t, &all(&t)).expect("mask applies");
    assert!(free
        .iter()
        .any(|(d, _)| t.function_of(*d) == HarmonicFunction::Tonic));

    let ratcheted = mask_to_arc(&candidates, (0, 1), 1, &t, &all(&t)).expect("mask applies");
    for (d, _) in &ratcheted {
        assert_eq!(
            t.function_of(*d),
            HarmonicFunction::Predominant,
            "{d} regressed below the reached level"
        );
    }
}

#[test]
fn mask_declines_when_a_fixed_slot_pushed_the_arc_past_the_ceiling() {
    let t = table();
    let candidates = weighted(&Degree::DIATONIC_TRIADS);

    // Ceiling is PD but a locked V already took the arc to D: the slot
    // is left unconstrained rather than fighting the user's lock.
    assert!(mask_to_arc(&candidates, (0, 1), 2, &t, &all(&t)).is_none());
}

#[test]
fn sparse_rows_fall_back_to_the_table_wide_pool() {
    let t = table();
    // A transition row offering only tonic-function degrees, on a slot
    // that demands the cadential dominant.
    let candidates = weighted(&[Degree::I, Degree::VI_MIN]);

    let masked = mask_to_arc(&candidates, (2, 2), 0, &t, &all(&t)).expect("pool fallback applies");
    assert!(!masked.is_empty());
    for (d, weight) in &masked {
        assert_eq!(flevel(t.function_of(*d)), 2);
        assert_eq!(*weight, 1.0, "pool entries carry flat weights");
    }
}

#[test]
fn mask_declines_when_the_table_has_no_degree_of_the_required_function() {
    let t = tonic_only_table();
    let candidates = weighted(&Degree::DIATONIC_TRIADS);

    // Dominant-less table: the arc constraint is dropped for the slot
    // instead of leaving the sampler with nothing to draw from.
    assert!(mask_to_arc(&candidates, (2, 2), 0, &t, &all(&t)).is_none());
}

#[test]
fn mask_never_invents_a_degree_outside_the_candidate_list() {
    let t = table();
    let candidates = weighted(&[Degree::I, Degree::II_MIN, Degree::V]);

    let masked = mask_to_arc(&candidates, (0, 1), 0, &t, &all(&t)).expect("mask applies");
    for (d, _) in &masked {
        assert!(
            candidates.iter().any(|(c, _)| c == d),
            "{d} was not among the candidates"
        );
    }
}

// ---------------------------------------------------------------------------
// build_phrase_plans
// ---------------------------------------------------------------------------

#[test]
fn full_phrase_plans_traverse_the_arc_once() {
    let t = table();
    let prefixed = vec![None; PHRASE_SLOTS];
    let (plans, splits) = build_phrase_plans(PHRASE_SLOTS, &prefixed, &t);

    assert_eq!(plans, vec![(0, 0), (0, 1), (1, 1), (2, 2)]);
    // The forced-PD slot is the one that accelerates.
    assert_eq!(splits, vec![2]);
}

#[test]
fn a_fixed_final_tonic_shifts_the_cadence_left() {
    let t = table();
    let mut prefixed = vec![None; PHRASE_SLOTS];
    prefixed[PHRASE_SLOTS - 1] = Some(Degree::I);
    let (plans, splits) = build_phrase_plans(PHRASE_SLOTS, &prefixed, &t);

    // … PD D | fixed T, and the fixed slot is left unmasked.
    assert_eq!(plans, vec![(0, 0), (1, 1), (2, 2), (0, 2)]);
    assert_eq!(splits, vec![1]);
}

#[test]
fn short_remainder_phrases_do_not_accelerate() {
    let t = table();
    let len = PHRASE_SLOTS + 2;
    let prefixed = vec![None; len];
    let (plans, splits) = build_phrase_plans(len, &prefixed, &t);

    assert_eq!(plans.len(), len);
    // Only the first, full phrase splits.
    assert_eq!(splits, vec![2]);
}

#[test]
fn single_slot_phrases_are_pinned_to_tonic() {
    let t = table();
    let (plans, splits) = build_phrase_plans(1, &[None], &t);

    assert_eq!(plans, vec![(0, 0)]);
    assert!(splits.is_empty());
}
