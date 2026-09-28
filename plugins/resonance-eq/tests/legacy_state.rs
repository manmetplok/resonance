//! State saved before the W8 extensions (warmth-width-depth.md §6.4:
//! per-band M/S, the Tilt / LF Lift+Dip / Air band types, auto-gain and
//! dynamic bands) must load and render exactly as it did before them.
//!
//! The golden this compares against was blessed on the code *before*
//! any of those existed, from blobs that name only the 49 parameters the
//! plugin had then. A project or preset written by the old build, loaded
//! into this one, must produce the same bits the old build produced.
//!
//! Never re-bless this file for a change that is meant to be
//! transparent. It exists to fail when one is not.

use std::path::PathBuf;

use resonance_dsp_test_support as golden;
use resonance_eq::presets::PRESETS;
use resonance_eq::ResonanceEq;
use resonance_plugin::{EventIterator, OutputBuffer, ResonancePlugin};

const SR: f32 = 48_000.0;
const BLOCK: usize = 256;
const BLOCKS: usize = 16;

/// A project saved by the pre-W8 build: every band enabled and every
/// kind the plugin had (bell, both shelves, both cuts at all three
/// slopes), plus a non-zero output trim.
const PROJECT_V1: &str = r#"{"version":1,"params":{
    "band0_enabled":1,"band0_freq":35.0,"band0_gain":0.0,"band0_q":0.707,"band0_kind":3,"band0_slope":2,
    "band1_enabled":1,"band1_freq":110.0,"band1_gain":4.0,"band1_q":0.8,"band1_kind":1,"band1_slope":1,
    "band2_enabled":1,"band2_freq":320.0,"band2_gain":-3.5,"band2_q":1.4,"band2_kind":0,"band2_slope":1,
    "band3_enabled":1,"band3_freq":900.0,"band3_gain":2.0,"band3_q":0.6,"band3_kind":0,"band3_slope":1,
    "band4_enabled":1,"band4_freq":2500.0,"band4_gain":-6.0,"band4_q":4.0,"band4_kind":0,"band4_slope":1,
    "band5_enabled":1,"band5_freq":5200.0,"band5_gain":3.0,"band5_q":2.0,"band5_kind":0,"band5_slope":1,
    "band6_enabled":1,"band6_freq":10000.0,"band6_gain":-2.5,"band6_q":0.707,"band6_kind":2,"band6_slope":1,
    "band7_enabled":1,"band7_freq":17000.0,"band7_gain":0.0,"band7_q":0.707,"band7_kind":4,"band7_slope":0,
    "output_gain":-1.5}}"#;

/// Before the version field existed; only some bands on.
const PROJECT_V0: &str = r#"{"params":{
    "band0_enabled":1,"band0_freq":60.0,"band0_gain":0.0,"band0_q":0.707,"band0_kind":3,"band0_slope":0,
    "band1_enabled":0,"band1_freq":120.0,"band1_gain":0.0,"band1_q":0.707,"band1_kind":1,"band1_slope":1,
    "band2_enabled":1,"band2_freq":450.0,"band2_gain":5.0,"band2_q":0.9,"band2_kind":0,"band2_slope":1,
    "band3_enabled":0,"band3_freq":600.0,"band3_gain":0.0,"band3_q":0.707,"band3_kind":0,"band3_slope":1,
    "band4_enabled":0,"band4_freq":1500.0,"band4_gain":0.0,"band4_q":0.707,"band4_kind":0,"band4_slope":1,
    "band5_enabled":1,"band5_freq":3000.0,"band5_gain":-4.0,"band5_q":3.0,"band5_kind":0,"band5_slope":1,
    "band6_enabled":1,"band6_freq":8000.0,"band6_gain":6.0,"band6_q":0.5,"band6_kind":2,"band6_slope":1,
    "band7_enabled":1,"band7_freq":12000.0,"band7_gain":0.0,"band7_q":0.707,"band7_kind":4,"band7_slope":2,
    "output_gain":2.0}}"#;

fn golden_path() -> PathBuf {
    golden::golden_path(env!("CARGO_MANIFEST_DIR"), "legacy_state.f32")
}

fn blessing() -> bool {
    golden::blessed(&["RESONANCE_BLESS_LEGACY_STATE"])
}

/// Deterministic pseudo-noise from the absolute sample index.
fn noise(n: u64) -> f32 {
    let mut s = n.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1) as u32;
    s ^= s >> 16;
    s = s.wrapping_mul(2_246_822_519);
    s ^= s >> 13;
    (s >> 8) as f32 * (2.0 / (1 << 23) as f32) - 1.0
}

/// Broadband, partly decorrelated stereo: noise plus a 200 Hz tone.
fn input(n: u64) -> (f32, f32) {
    let t = n as f32 / SR;
    let tone = 0.2 * (200.0 * t * std::f32::consts::TAU).sin();
    (0.3 * noise(n) + tone, 0.2 * noise(n + 77_777) + 0.2 * noise(n) + tone)
}

fn render_state(state: &[u8]) -> Vec<f32> {
    let mut plugin = ResonanceEq::new();
    assert!(plugin.load_state(state), "state blob failed to load");
    plugin.initialize(SR, BLOCK as u32);
    let mut out = Vec::with_capacity(BLOCKS * BLOCK * 2);
    let mut left = vec![0.0f32; BLOCK];
    let mut right = vec![0.0f32; BLOCK];
    let mut n = 0u64;
    for _ in 0..BLOCKS {
        for i in 0..BLOCK {
            let (l, r) = input(n + i as u64);
            left[i] = l;
            right[i] = r;
        }
        let mut outs = [OutputBuffer {
            left: &mut left,
            right: &mut right,
        }];
        let mut ev = EventIterator::empty();
        plugin.process(&mut outs, BLOCK, &mut ev, None);
        n += BLOCK as u64;
        out.extend_from_slice(&left);
        out.extend_from_slice(&right);
    }
    out
}

fn states() -> Vec<(String, Vec<u8>)> {
    let mut v = vec![
        ("project_v1".to_string(), PROJECT_V1.as_bytes().to_vec()),
        ("project_v0".to_string(), PROJECT_V0.as_bytes().to_vec()),
    ];
    // The bank as it stood when the golden was blessed. Named rather than
    // enumerated, so a preset added later cannot change what this pins.
    for name in PRE_W8_PRESETS {
        let p = PRESETS
            .iter()
            .find(|p| p.name == *name)
            .unwrap_or_else(|| panic!("factory preset `{name}` disappeared"));
        v.push((format!("preset {}", p.name), p.json.as_bytes().to_vec()));
    }
    v
}

/// Factory presets that existed before W8, in bank order.
const PRE_W8_PRESETS: &[&str] = &[
    "Kick — Punch",
    "Kick — Sub",
    "Snare — Crack",
    "Snare — Body",
    "Bass — Tight",
    "Bass — Warm",
    "Guitar — Body",
    "Guitar — Air",
    "Vocal — Clarity",
    "Synth — Wide",
    "Master — Polish",
];

#[test]
fn pre_w8_state_renders_bit_identically() {
    let mut rendered = Vec::new();
    for (name, state) in states() {
        let out = render_state(&state);
        let peak = out.iter().fold(0.0f32, |m, x| m.max(x.abs()));
        assert!(peak > 1e-3, "`{name}` rendered silence");
        assert!(out.iter().all(|x| x.is_finite()), "`{name}` is non-finite");
        rendered.extend(out);
    }
    let path = golden_path();
    if blessing() {
        golden::bless_f32(&path, &rendered);
        return;
    }
    let want = golden::load_golden_f32(&path, rendered.len(), "RESONANCE_BLESS_LEGACY_STATE=1");
    let diff = golden::compare_f32(&rendered, &want);
    if let Some((i, got, want)) = diff.first_diff {
        panic!(
            "state saved before the W8 extensions no longer renders the same: \
             {}/{} samples differ (peak {:.3e}), first at {i} (got {got}, want {want})",
            diff.diff_count,
            rendered.len(),
            diff.max_abs
        );
    }
}
