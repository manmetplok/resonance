//! Factory presets (ba todo #1137): every baked-in preset JSON must
//! parse, cover the full 29-parameter surface, and load
//! deterministically through the shared loader onto a fresh param set.

use resonance_granular_delay::params::{GranularDelayParams, PARAM_COUNT};
use resonance_granular_delay::presets::{load_preset, PRESETS};

#[test]
fn factory_preset_list_is_populated() {
    assert!(
        (5..=8).contains(&PRESETS.len()),
        "expected 5-8 factory presets, found {}",
        PRESETS.len()
    );
    for (i, entry) in PRESETS.iter().enumerate() {
        assert!(!entry.name.is_empty(), "preset {i} has an empty name");
    }
}

#[test]
fn every_preset_is_a_full_snapshot_and_loads() {
    let fresh = GranularDelayParams::default();
    for entry in PRESETS {
        // Parse the raw JSON independently of the loader so a malformed
        // file fails loudly with its name.
        let value: serde_json::Value = serde_json::from_str(entry.json)
            .unwrap_or_else(|e| panic!("preset '{}' is invalid JSON: {e}", entry.name));
        let map = value
            .get("params")
            .and_then(|v| v.as_object())
            .unwrap_or_else(|| panic!("preset '{}' lacks a \"params\" object", entry.name));

        // Full snapshot: every declared param id must be present, no
        // strays may remain.
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

        // Loading applies every value exactly (set_plain clamps to the
        // param range; factory values must already be in range so the
        // load is deterministic).
        let params = GranularDelayParams::default();
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
                .unwrap_or_else(|| panic!("preset '{}': param '{}' not a number", entry.name, p.id()));
            let got = p.get_plain();
            assert!(
                (got - want).abs() < 1e-6,
                "preset '{}': param '{}' loaded as {got}, JSON says {want}",
                entry.name,
                p.id()
            );
        }
    }
}
