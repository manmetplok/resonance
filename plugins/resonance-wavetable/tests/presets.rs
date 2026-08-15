//! Factory presets are complete snapshots.
//!
//! The shared loader only writes ids present in the preset's map, so a
//! partial preset silently leaves the previous patch's settings running —
//! the P7 finding in ba doc #275, recorded there against resonance-delay.
//! This crate has always shipped full snapshots and this test keeps it that
//! way, including for parameters added later (ba todo #1324 added six).

use resonance_wavetable::params::{WavetableParams, PARAM_COUNT};
use resonance_wavetable::presets::PRESETS;

#[test]
fn every_preset_carries_every_parameter() {
    let params = WavetableParams::new();
    let ids: Vec<&str> = (0..PARAM_COUNT).map(|i| params.param_at(i).id()).collect();

    for entry in PRESETS {
        let value: serde_json::Value =
            serde_json::from_str(entry.json).unwrap_or_else(|e| panic!("{}: {e}", entry.name));
        let map = value
            .get("params")
            .and_then(|v| v.as_object())
            .unwrap_or_else(|| panic!("{}: no \"params\" object", entry.name));

        let missing: Vec<&str> = ids
            .iter()
            .copied()
            .filter(|id| !map.contains_key(*id))
            .collect();
        assert!(
            missing.is_empty(),
            "preset {:?} is a partial recall — missing {:?}",
            entry.name,
            missing
        );

        let unknown: Vec<&String> = map.keys().filter(|k| !ids.contains(&k.as_str())).collect();
        assert!(
            unknown.is_empty(),
            "preset {:?} writes ids no parameter has: {:?}",
            entry.name,
            unknown
        );
    }
}
