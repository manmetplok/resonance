//! Factory presets (ba todo #1330, audit finding C4): every baked-in
//! preset must parse, be a **full** snapshot of the declared parameter
//! surface, sit inside every parameter's declared range, and load
//! deterministically through the shared loader onto a fresh param set.
//!
//! The full-snapshot assertion is the point of the file. The shared
//! loader writes only the ids it finds, so a preset that omits an id
//! leaves the previous patch's value in place — the recall bug the
//! delay's presets shipped with (finding P7).

use resonance_gate::params::{GateParams, PARAM_COUNT};
use resonance_gate::presets::{load_preset, PRESETS};

#[test]
fn factory_preset_list_is_populated() {
    assert!(
        PRESETS.len() >= 6,
        "expected a real factory bank, found {} preset(s)",
        PRESETS.len()
    );
    for (i, entry) in PRESETS.iter().enumerate() {
        assert!(!entry.name.is_empty(), "preset {i} has an empty name");
    }
    let mut names: Vec<&str> = PRESETS.iter().map(|e| e.name).collect();
    names.sort_unstable();
    let unique = names.len();
    names.dedup();
    assert_eq!(names.len(), unique, "two factory presets share a name");
}

#[test]
fn every_preset_is_a_full_snapshot_and_loads() {
    let fresh = GateParams::default();
    for entry in PRESETS {
        // Parse the raw JSON independently of the loader so a malformed
        // file fails loudly with its name.
        let value: serde_json::Value = serde_json::from_str(&entry.state_json())
            .unwrap_or_else(|e| panic!("preset '{}' is invalid JSON: {e}", entry.name));
        let map = value
            .get("params")
            .and_then(|v| v.as_object())
            .unwrap_or_else(|| panic!("preset '{}' lacks a \"params\" object", entry.name));

        // Full snapshot: every declared param id present, no strays.
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

        // Loading applies every value exactly: `set_plain` clamps to the
        // declared range, so a value that survives the round trip
        // unchanged is also proof it was in range to begin with.
        let params = GateParams::default();
        assert!(
            load_preset(&params, entry.json),
            "loader rejected preset '{}'",
            entry.name
        );
        for i in 0..PARAM_COUNT {
            let p = params.param_at(i);
            let want = map.get(p.id()).and_then(|v| v.as_f64()).unwrap_or_else(|| {
                panic!(
                    "preset '{}': param '{}' is not a number",
                    entry.name,
                    p.id()
                )
            });
            let got = p.get_plain();
            assert!(
                (got - want).abs() < 1e-6,
                "preset '{}': param '{}' loaded as {got}, JSON says {want} \
                 (out of range, or the loader dropped it)",
                entry.name,
                p.id()
            );
        }
    }
}

#[test]
fn the_init_preset_restores_the_declared_defaults() {
    let init = PRESETS
        .iter()
        .find(|e| e.name.starts_with("Init"))
        .expect("the bank has no Init preset to reset from");

    // Move every parameter off its default, then load Init and check it
    // came all the way back.
    let params = GateParams::default();
    let defaults: Vec<f64> = (0..PARAM_COUNT)
        .map(|i| params.param_at(i).default_plain())
        .collect();
    for i in 0..PARAM_COUNT {
        let p = params.param_at(i);
        p.set_plain(p.max_plain());
    }
    assert!(load_preset(&params, init.json), "Init preset did not load");
    for (i, &want) in defaults.iter().enumerate() {
        let p = params.param_at(i);
        assert!(
            (p.get_plain() - want).abs() < 1e-6,
            "Init left '{}' at {} instead of its default {want}",
            p.id(),
            p.get_plain(),
        );
    }
}
