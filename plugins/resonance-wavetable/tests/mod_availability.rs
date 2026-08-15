//! What the modulation matrix can and cannot actually do (ba todo #1278).
//!
//! Two mod sources and two mod destinations are offered by the picker but
//! produce no modulation in this build. These tests pin the set, pin the
//! fact that routing through one changes nothing, and pin the factory
//! presets that reference one so the editor's warning has a known job.

use resonance_wavetable::dsp::modulation::{
    evaluate_mod_matrix, ModDest, ModSlot, ModSource, NUM_MOD_SLOTS,
};
use resonance_wavetable::params::WavetableParams;
use resonance_wavetable::presets::PRESETS;

fn slot(source: ModSource, dest: ModDest, amount: f32) -> ModSlot {
    ModSlot {
        source,
        dest,
        amount,
    }
}

// ---------------------------------------------------------------------------
// The unavailable set
// ---------------------------------------------------------------------------

#[test]
fn exactly_two_sources_are_unavailable() {
    let unavailable: Vec<&str> = (0..ModSource::LABELS.len())
        .map(|i| ModSource::from_int(i as i32))
        .filter(|s| !s.is_available())
        .map(|s| s.label())
        .collect();
    assert_eq!(unavailable, vec!["Mod Wheel", "Aftertouch"]);
}

#[test]
fn every_destination_is_available() {
    // "Osc Balance" and "Unison Detune" were the last two holdouts; ba todo
    // #1323 wired both into the oscillator.
    let unavailable: Vec<&str> = (0..ModDest::LABELS.len())
        .map(|i| ModDest::from_int(i as i32))
        .filter(|d| !d.is_available())
        .map(|d| d.label())
        .collect();
    assert!(unavailable.is_empty(), "unimplemented: {unavailable:?}");
}

#[test]
fn every_unavailable_reason_names_the_todo_that_implements_it() {
    // The DoD for #1278: each carries a comment naming the todo. The reason
    // string is what the user sees, so it carries it too.
    for i in 0..ModSource::LABELS.len() {
        if let Some(reason) = ModSource::from_int(i as i32).unavailable_reason() {
            assert!(
                reason.contains("ba todo #"),
                "source reason must name a todo: {reason:?}"
            );
        }
    }
    for i in 0..ModDest::LABELS.len() {
        if let Some(reason) = ModDest::from_int(i as i32).unavailable_reason() {
            assert!(
                reason.contains("ba todo #"),
                "destination reason must name a todo: {reason:?}"
            );
        }
    }
}

#[test]
fn a_slot_is_effective_only_when_both_ends_are_wired_and_implemented() {
    assert!(slot(ModSource::Lfo1, ModDest::FilterCutoff, 1.0).is_effective());
    assert!(!slot(ModSource::None, ModDest::FilterCutoff, 1.0).is_effective());
    assert!(!slot(ModSource::Lfo1, ModDest::None, 1.0).is_effective());
    assert!(!slot(ModSource::ModWheel, ModDest::FilterCutoff, 1.0).is_effective());
    assert!(!slot(ModSource::Aftertouch, ModDest::FilterCutoff, 1.0).is_effective());
    // Both implemented by ba todo #1323.
    assert!(slot(ModSource::Lfo1, ModDest::OscBalance, 1.0).is_effective());
    assert!(slot(ModSource::Lfo1, ModDest::UnisonDetune, 1.0).is_effective());
}

// ---------------------------------------------------------------------------
// They really do nothing
// ---------------------------------------------------------------------------

#[test]
fn an_unavailable_source_contributes_no_modulation() {
    let with = vec![
        slot(ModSource::Lfo1, ModDest::FilterCutoff, 0.5),
        slot(ModSource::ModWheel, ModDest::FilterCutoff, 1.0),
        slot(ModSource::Aftertouch, ModDest::Osc1Position, 1.0),
    ];
    let without = vec![slot(ModSource::Lfo1, ModDest::FilterCutoff, 0.5)];

    let a = evaluate_mod_matrix(&with, 0.8, 0.0, 0.0, 0.4, 0.9, 72.0);
    let b = evaluate_mod_matrix(&without, 0.8, 0.0, 0.0, 0.4, 0.9, 72.0);
    assert_eq!(a.filter_cutoff, b.filter_cutoff);
    assert_eq!(a.osc1_position, b.osc1_position);
}

#[test]
fn osc_balance_and_unison_detune_accumulate() {
    // ba todo #1323: these two used to be filtered out as unavailable.
    let slots = vec![
        slot(ModSource::Lfo1, ModDest::OscBalance, 1.0),
        slot(ModSource::Env2, ModDest::UnisonDetune, 1.0),
    ];
    let state = evaluate_mod_matrix(&slots, 0.75, 0.0, 0.0, 1.0, 1.0, 60.0);
    assert_eq!(state.osc_balance, 0.75);
    assert_eq!(state.unison_detune, 1.0);
}

#[test]
fn available_routings_still_evaluate() {
    // Guard against the availability filter swallowing working routings.
    let slots = vec![slot(ModSource::Lfo1, ModDest::FilterCutoff, 0.5)];
    let state = evaluate_mod_matrix(&slots, 1.0, 0.0, 0.0, 0.0, 1.0, 60.0);
    assert_eq!(state.filter_cutoff, 0.5);
}

// ---------------------------------------------------------------------------
// Factory presets that reference one
// ---------------------------------------------------------------------------

/// Preset routings the editor has to flag as inert.
///
/// This was three entries (two to Osc Balance / Unison Detune) until ba todo
/// #1323 implemented both destinations; those presets now sound as their
/// designer intended. It stays empty unless a preset picks up a `Mod Wheel`
/// or `Aftertouch` source before ba todo #1301 lands.
const KNOWN_INERT_PRESET_ROUTINGS: &[(&str, usize, &str)] = &[];

#[test]
fn factory_presets_reference_only_the_known_inert_routings() {
    let params = WavetableParams::new();
    let mut found: Vec<(String, usize, String)> = Vec::new();

    for entry in PRESETS {
        resonance_plugin::presets::load(entry.json, 87, |i| params.param_at(i));
        for i in 0..NUM_MOD_SLOTS {
            let s = &params.mod_slots[i];
            let source = ModSource::from_int(s.source.value());
            let dest = ModDest::from_int(s.destination.value());
            if source == ModSource::None || dest == ModDest::None {
                continue;
            }
            if !source.is_available() {
                found.push((entry.name.to_string(), i + 1, source.label().to_string()));
            }
            if !dest.is_available() {
                found.push((entry.name.to_string(), i + 1, dest.label().to_string()));
            }
        }
    }

    let expected: Vec<(String, usize, String)> = KNOWN_INERT_PRESET_ROUTINGS
        .iter()
        .map(|(n, s, l)| (n.to_string(), *s, l.to_string()))
        .collect();
    found.sort();
    let mut expected = expected;
    expected.sort();
    assert_eq!(
        found, expected,
        "a factory preset gained or lost an inert modulation routing"
    );
}
