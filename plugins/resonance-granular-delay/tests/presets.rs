//! Factory presets (ba todo #1137): every baked-in preset JSON must
//! parse, cover the full 29-parameter surface, and load
//! deterministically through the shared loader onto a fresh param set.

use resonance_granular_delay::params::{GranularDelayParams, PARAM_COUNT};
use resonance_granular_delay::presets::{load_preset, PRESETS};
use resonance_granular_delay::ResonanceGranularDelay;
use resonance_plugin::{EventIterator, OutputBuffer, ResonancePlugin, TempoInfo};

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
    let mut names = std::collections::HashSet::new();
    for entry in PRESETS {
        assert!(names.insert(entry.name), "duplicate preset name '{}'", entry.name);
    }
}

// --- Mode coverage (ba todo #1339) ------------------------------------
//
// The audit's LOW finding: quality was always Normal, filter_type always
// LP, time_mode never Repitch and fb_route never Output-only — five
// shipped modes reachable but never demonstrated, so a user browsing the
// presets would not know they exist. These two tests keep the factory
// set a tour of the plugin rather than six variations on one patch.

/// Every mode parameter's declared values, and how many each has.
const MODE_PARAMS: &[(&str, usize)] = &[
    ("time_mode", 3),      // Fade / Repitch / Per-Grain
    ("fb_route", 3),       // Wet->Buffer / Output-only / Ping-pong
    ("scheduler", 3),      // Sync / Async / Pitch-Sync
    ("pitch_quantize", 3), // Off / Semitones / Scale
    ("filter_type", 2),    // LP / HP
    ("quality", 3),        // Lo-fi / Normal / HQ
];

/// Capability switches that must be demonstrated both on and off.
const TOGGLES: &[&str] = &["sync", "fb_pitch", "density_sync", "freeze", "align"];

fn preset_values(id: &str) -> Vec<f64> {
    PRESETS
        .iter()
        .map(|entry| {
            let value: serde_json::Value = serde_json::from_str(entry.json)
                .unwrap_or_else(|e| panic!("preset '{}' is invalid JSON: {e}", entry.name));
            value
                .get("params")
                .and_then(|m| m.get(id))
                .and_then(|v| v.as_f64())
                .unwrap_or_else(|| panic!("preset '{}' is missing param '{id}'", entry.name))
        })
        .collect()
}

#[test]
fn the_preset_set_demonstrates_every_shipped_mode() {
    for &(id, variants) in MODE_PARAMS {
        let used: std::collections::HashSet<i64> = preset_values(id)
            .into_iter()
            .map(|v| v.round() as i64)
            .collect();
        for index in 0..variants as i64 {
            assert!(
                used.contains(&index),
                "no factory preset selects {id} = {index}; the mode ships but is \
                 never demonstrated (used: {used:?})"
            );
        }
    }

    for id in TOGGLES {
        let used: std::collections::HashSet<bool> =
            preset_values(id).into_iter().map(|v| v >= 0.5).collect();
        assert!(
            used.contains(&true) && used.contains(&false),
            "the {id} switch is never shown both on and off across the presets"
        );
    }

    // Diffusion is continuous, so "demonstrated" means some preset
    // actually engages the stage (ba todo #1321).
    assert!(
        preset_values("diffusion").iter().any(|&v| v > 0.05),
        "no factory preset engages the diffusion stage"
    );
}

/// A showcase that misbehaves is worse than no showcase: every factory
/// preset must render bounded, finite audio for several seconds under a
/// host tempo (the tempo-locked, over-unity-feedback and frozen patches
/// included).
#[test]
fn every_preset_renders_bounded_finite_audio() {
    const SR: f32 = 48_000.0;
    const BLOCK: usize = 128;
    const BLOCKS: usize = 940; // ~2.5 s

    for entry in PRESETS {
        let mut plugin = ResonanceGranularDelay::new();
        assert!(
            load_preset(&plugin.params, entry.json),
            "loader rejected preset '{}'",
            entry.name
        );
        plugin.initialize(SR, BLOCK as u32);

        let tempo = TempoInfo {
            bpm: 128.0,
            time_sig_num: 4,
            time_sig_den: 4,
            playing: true,
            song_pos_beats: 0.0,
        };
        let mut left = [0.0f32; BLOCK];
        let mut right = [0.0f32; BLOCK];
        let mut peak = 0.0f32;
        let mut n = 0u64;

        for _ in 0..BLOCKS {
            for i in 0..BLOCK {
                let t = (n + i as u64) as f32 / SR;
                let v = 0.5
                    * (0.7 * (196.0 * t * std::f32::consts::TAU).sin()
                        + 0.3 * (523.0 * t * std::f32::consts::TAU).sin());
                left[i] = v;
                right[i] = v;
            }
            {
                let mut outs = [OutputBuffer {
                    left: &mut left[..],
                    right: &mut right[..],
                }];
                let mut ev = EventIterator::empty();
                plugin.process(&mut outs, BLOCK, &mut ev, Some(tempo));
            }
            n += BLOCK as u64;
            for (&l, &r) in left.iter().zip(right.iter()) {
                assert!(
                    l.is_finite() && r.is_finite(),
                    "preset '{}' rendered a non-finite sample",
                    entry.name
                );
                peak = peak.max(l.abs()).max(r.abs());
            }
        }
        assert!(
            peak < 4.0,
            "preset '{}' peaked at {peak} — a factory patch must stay sane",
            entry.name
        );
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
