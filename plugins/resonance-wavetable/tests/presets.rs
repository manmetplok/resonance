//! Factory presets are complete snapshots.
//!
//! The shared loader only writes ids present in the preset's map, so a
//! partial preset silently leaves the previous patch's settings running —
//! the P7 finding in ba doc #275, recorded there against resonance-delay.
//! This crate has always shipped full snapshots and this test keeps it that
//! way, including for parameters added later (ba todo #1324 added six).

use resonance_wavetable::params::{WavetableParams, PARAM_COUNT};
use resonance_wavetable::presets::PRESETS;

/// Every place in this crate that calls the shared preset loader.
/// Compiled in, so the guard below can never drift from the sources that
/// actually run.
///
/// The editor used to be on this list. It now loads through
/// `presets::PresetBank` (ba todo #1358), which takes the parameter slice
/// and derives the count from its length, so the editor cannot get the
/// count wrong any more — there is nothing left for it to spell out.
const LOADER_CALL_SITES: [(&str, &str); 1] = [(
    "tests/mod_availability.rs",
    include_str!("mod_availability.rs"),
)];

/// The count `presets::load` is given must be the constant, never a
/// literal.
///
/// `load` iterates `0..count`, so a stale number stops short and leaves
/// the tail of the parameter list at its constructor default — the
/// preset is applied incomplete and nothing fails. That is exactly what
/// happened when ba todo #1324 moved the count from 87 to 93 and
/// `mod_availability.rs` kept the literal: six effect parameters stopped
/// being restored, and `param_at_exposes_every_parameter_exactly_once`
/// could not see it, because it checks the table and not its callers.
#[test]
fn no_caller_spells_the_parameter_count_out_by_hand() {
    let mut checked = 0;
    for (what, src) in LOADER_CALL_SITES {
        for line in src.lines() {
            // Skip prose: the doc comments here talk *about* the call.
            if line.trim_start().starts_with("//") {
                continue;
            }
            let Some(rest) = line.split_once("presets::load(") else {
                continue;
            };
            checked += 1;
            let args: Vec<&str> = rest.1.split(',').map(str::trim).collect();
            assert!(
                args.len() >= 2,
                "{what}: cannot read the count argument of `{}`",
                line.trim()
            );
            assert_eq!(
                args[1], "PARAM_COUNT",
                "{what}: the count must be PARAM_COUNT, found `{}` in `{}`",
                args[1],
                line.trim()
            );
        }
    }
    assert_eq!(
        checked, 1,
        "expected the mod-matrix test to be the only remaining direct call \
         site; add any new one to LOADER_CALL_SITES"
    );
}

#[test]
fn every_preset_carries_every_parameter() {
    let params = WavetableParams::new();
    let ids: Vec<&str> = (0..PARAM_COUNT).map(|i| params.param_at(i).id()).collect();

    for entry in PRESETS {
        let value: serde_json::Value =
            serde_json::from_str(&entry.state_json()).unwrap_or_else(|e| panic!("{}: {e}", entry.name));
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
