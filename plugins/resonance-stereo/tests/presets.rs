//! Factory presets: every baked-in preset parses, is a **full** snapshot
//! of the declared parameter surface, sits inside every declared range,
//! loads exactly, and actually renders (none of them is silent, and all
//! but Init change the sound).

mod common;

use common::*;
use resonance_stereo::params::{StereoParams, PARAM_COUNT};
use resonance_stereo::presets::{load_preset, PRESETS};

#[test]
fn the_bank_has_the_named_presets_and_no_duplicates() {
    for want in ["Mono Bass Below 120", "Widen Mono Source", "Master — Gentle Width"] {
        assert!(PRESETS.iter().any(|e| e.name == want), "missing preset {want:?}");
    }
    let mut names: Vec<&str> = PRESETS.iter().map(|e| e.name).collect();
    names.sort_unstable();
    let n = names.len();
    names.dedup();
    assert_eq!(names.len(), n, "two factory presets share a name");
}

#[test]
fn every_preset_is_a_full_snapshot_and_loads_exactly() {
    let fresh = StereoParams::default();
    for entry in PRESETS {
        let value: serde_json::Value = serde_json::from_str(entry.json)
            .unwrap_or_else(|e| panic!("preset '{}' is invalid JSON: {e}", entry.name));
        let map = value
            .get("params")
            .and_then(|v| v.as_object())
            .unwrap_or_else(|| panic!("preset '{}' lacks a \"params\" object", entry.name));
        assert_eq!(map.len(), PARAM_COUNT, "preset '{}' is not a full snapshot", entry.name);
        for i in 0..PARAM_COUNT {
            let id = fresh.param_at(i).id();
            assert!(map.contains_key(id), "preset '{}' is missing '{id}'", entry.name);
        }

        let params = StereoParams::default();
        assert!(load_preset(&params, entry.json), "loader rejected '{}'", entry.name);
        for i in 0..PARAM_COUNT {
            let p = params.param_at(i);
            let want = map[p.id()].as_f64().unwrap();
            assert!(
                (p.get_plain() - want).abs() < 1e-6,
                "preset '{}': '{}' loaded as {}, JSON says {want} (out of range?)",
                entry.name,
                p.id(),
                p.get_plain()
            );
        }
    }
}

#[test]
fn init_is_the_defaults_and_every_other_preset_changes_the_sound() {
    let n = 24_000;
    let a = noise(n, 0.3, 1);
    let b = noise(n, 0.3, 2);
    let tone = sine(330.0, 0.3, n);
    let bass = sine(60.0, 0.3, n);
    let l: Vec<f32> = (0..n).map(|i| tone[i] + bass[i] + a[i]).collect();
    let r: Vec<f32> = (0..n).map(|i| tone[i] + 0.7 * bass[i] + b[i]).collect();

    for entry in PRESETS {
        let (ol, or) = render(|p| assert!(load_preset(p, entry.json)), &l, &r);
        assert!(energy(&ol) + energy(&or) > 1.0, "preset '{}' renders silence", entry.name);
        let is_init = entry.name.starts_with("Init");
        if is_init {
            let d = StereoParams::default();
            let params = StereoParams::default();
            load_preset(&params, entry.json);
            for i in 0..PARAM_COUNT {
                assert_eq!(params.param_at(i).get_plain(), d.param_at(i).default_plain());
            }
            assert_eq!((ol, or), (l.clone(), r.clone()), "Init must be a passthrough");
        } else {
            assert!(ol != l || or != r, "preset '{}' does nothing", entry.name);
        }
    }
}
