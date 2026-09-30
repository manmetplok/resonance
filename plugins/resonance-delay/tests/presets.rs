//! Factory presets (ba todo #1272): every baked-in preset must parse,
//! cover the full parameter surface, and load as a *complete* recall —
//! loading any preset over any other must leave nothing at the previous
//! patch's value.

use resonance_delay::params::{DelayParams, PARAM_COUNT};
use resonance_delay::presets::{load_preset, PRESETS};

/// The JSON `params` map of a preset, parsed independently of the
/// shared loader so a malformed blob fails loudly with its name.
fn params_map(
    entry: &resonance_delay::presets::PresetEntry,
) -> serde_json::Map<String, serde_json::Value> {
    let value: serde_json::Value = serde_json::from_str(&entry.state_json())
        .unwrap_or_else(|e| panic!("preset '{}' is invalid JSON: {e}", entry.name));
    value
        .get("params")
        .and_then(|v| v.as_object())
        .unwrap_or_else(|| panic!("preset '{}' lacks a \"params\" object", entry.name))
        .clone()
}

#[test]
fn factory_preset_list_is_populated() {
    assert!(!PRESETS.is_empty());
    for (i, entry) in PRESETS.iter().enumerate() {
        assert!(!entry.name.is_empty(), "preset {i} has an empty name");
    }
}

#[test]
fn every_preset_is_a_full_snapshot() {
    let fresh = DelayParams::default();
    for entry in PRESETS {
        let map = params_map(entry);
        assert_eq!(
            map.len(),
            PARAM_COUNT,
            "preset '{}' must snapshot all {PARAM_COUNT} params, found {}",
            entry.name,
            map.len()
        );
        for i in 0..PARAM_COUNT {
            let id = fresh.param_at(i).id();
            assert!(
                map.contains_key(id),
                "preset '{}' is missing param '{id}'",
                entry.name
            );
        }
    }
}

#[test]
fn loading_applies_every_value_exactly() {
    for entry in PRESETS {
        let map = params_map(entry);
        let params = DelayParams::default();
        assert!(
            load_preset(&params, entry.json),
            "loader rejected preset '{}'",
            entry.name
        );
        for i in 0..PARAM_COUNT {
            let p = params.param_at(i);
            let want = map
                .get(p.id())
                .and_then(|v| v.as_f64())
                .unwrap_or_else(|| panic!("preset '{}': '{}' is not a number", entry.name, p.id()));
            let got = p.get_plain();
            assert!(
                (got - want).abs() < 1e-6,
                "preset '{}': '{}' loaded as {got}, JSON says {want} (value out of the param's range?)",
                entry.name,
                p.id()
            );
        }
    }
}

/// The regression the finding describes: a partial preset inherits the
/// previous patch. Load every preset over every other and assert the
/// result is identical to loading it onto a fresh param set.
#[test]
fn loading_any_preset_over_any_other_leaves_nothing_behind() {
    for previous in PRESETS {
        for entry in PRESETS {
            let stacked = DelayParams::default();
            assert!(load_preset(&stacked, previous.json));
            assert!(load_preset(&stacked, entry.json));

            let clean = DelayParams::default();
            assert!(load_preset(&clean, entry.json));

            for i in 0..PARAM_COUNT {
                let a = stacked.param_at(i);
                let b = clean.param_at(i);
                assert!(
                    (a.get_plain() - b.get_plain()).abs() < 1e-6,
                    "'{}' survived from '{}' into '{}': {} vs {}",
                    a.id(),
                    previous.name,
                    entry.name,
                    a.get_plain(),
                    b.get_plain()
                );
            }
        }
    }
}
