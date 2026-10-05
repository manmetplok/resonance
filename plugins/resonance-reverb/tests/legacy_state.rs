//! State saved before the W8 extensions (warmth-width-depth.md §6.4:
//! wet HPF/LPF, ducking, ER/tail balance) must load and render exactly
//! as it did before them.
//!
//! The golden this compares against was blessed on the code *before*
//! any of those parameters existed, from blobs that name only the twelve
//! parameters the plugin had then. So it proves more than "the defaults
//! look transparent": a project or preset written by the old build,
//! loaded into this one, produces the same bits the old build produced.
//!
//! The renders go through `load_state` (the path a project reload takes)
//! for a hand-written version-1 project blob and a version-0 blob (no
//! `"version"` key, as every project saved before ba todo #1332 is), and
//! through every factory preset.
//!
//! Never re-bless this file for a change that is meant to be
//! transparent. It exists to fail when one is not.

use std::path::PathBuf;

use resonance_dsp_test_support as golden;
use resonance_plugin::{EventIterator, OutputBuffer, ResonancePlugin};
use resonance_reverb::ResonanceReverb;

const SR: f32 = 48_000.0;
const BLOCK: usize = 256;
const BLOCKS: usize = 24;

/// A project saved by the pre-W8 build: every one of the twelve
/// parameters it knew, off its default.
const PROJECT_V1: &str = r#"{"version":1,"params":{
    "predelay":22.0,"er_level":0.55,"er_time":0.35,"size":0.42,"decay":3.1,
    "damping":6500.0,"diffusion":0.7,"mod_rate":1.6,"mod_depth":0.25,
    "width":0.8,"mix":0.45,"freeze":0.0}}"#;

/// The same kind of project from before the version field existed.
const PROJECT_V0: &str = r#"{"params":{
    "predelay":8.0,"er_level":0.2,"er_time":0.8,"size":0.7,"decay":5.5,
    "damping":12000.0,"diffusion":0.9,"mod_rate":0.5,"mod_depth":0.6,
    "width":1.0,"mix":0.6,"freeze":0.0}}"#;

fn golden_path() -> PathBuf {
    golden::golden_path(env!("CARGO_MANIFEST_DIR"), "legacy_state.f32")
}

fn blessing() -> bool {
    golden::blessed(&["RESONANCE_BLESS_LEGACY_STATE"])
}

/// An impulse on L, one on R 37 samples later, then an 8 Hz pluck train:
/// the response and a steady wash in one render.
fn input(n: u64) -> (f32, f32) {
    let t = n as f32 / SR;
    let pluck = if n > 4_800 {
        let x = t % 0.125;
        0.5 * (-240.0 * x).exp() * (330.0 * t * std::f32::consts::TAU).sin()
    } else {
        0.0
    };
    (
        pluck + if n == 0 { 1.0 } else { 0.0 },
        0.8 * pluck + if n == 37 { 1.0 } else { 0.0 },
    )
}

fn render_state(state: &[u8]) -> Vec<f32> {
    let mut plugin = ResonanceReverb::new();
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
    // The bank as it stood when the golden was blessed, frozen as
    // fixtures: the live presets have since moved onto the new
    // algorithms (reverb-algorithms.md D1), which is a deliberate sound
    // change this file must not see.
    for (name, json) in PRE_W8_PRESETS {
        v.push((format!("preset {name}"), json.as_bytes().to_vec()));
    }
    v
}

/// Factory presets that existed before W8, in bank order, as they were
/// before the algorithm re-voice (copied from 30504ced).
const PRE_W8_PRESETS: &[(&str, &str)] = &[
    ("Tight Room", include_str!("fixtures/pre_revoice_presets/tight_room.json")),
    ("Vocal Plate", include_str!("fixtures/pre_revoice_presets/vocal_plate.json")),
    ("Warm Hall", include_str!("fixtures/pre_revoice_presets/warm_hall.json")),
    ("Cathedral", include_str!("fixtures/pre_revoice_presets/cathedral.json")),
    ("Ambient Bloom", include_str!("fixtures/pre_revoice_presets/ambient_bloom.json")),
    ("Shimmer Drone", include_str!("fixtures/pre_revoice_presets/shimmer_drone.json")),
    ("Snare Plate", include_str!("fixtures/pre_revoice_presets/snare_plate.json")),
    ("Snare Tight", include_str!("fixtures/pre_revoice_presets/snare_tight.json")),
    ("Snare Ambient", include_str!("fixtures/pre_revoice_presets/snare_ambient.json")),
    ("Snare Gated", include_str!("fixtures/pre_revoice_presets/snare_gated.json")),
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
