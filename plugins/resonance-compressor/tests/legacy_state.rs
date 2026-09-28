//! State saved before `release_mode` existed (warmth-width-depth.md
//! §6.4) must load and render exactly as it did before it.
//!
//! The golden this compares against was blessed on the code *before*
//! the parameter existed, from blobs that name only the eleven
//! parameters the plugin had then, and covers both detector sources:
//! self-keyed, and keyed from an external sidechain.
//!
//! Never re-bless this file for a change that is meant to be
//! transparent. It exists to fail when one is not.

use std::path::PathBuf;

use resonance_compressor::presets::PRESETS;
use resonance_compressor::ResonanceCompressor;
use resonance_dsp_test_support as golden;
use resonance_plugin::{EventIterator, KeyBuffer, OutputBuffer, ResonancePlugin};

const SR: f32 = 48_000.0;
const BLOCK: usize = 256;
const BLOCKS: usize = 20;

const PROJECT_V1: &str = r#"{"version":1,"params":{
    "threshold":-26.0,"ratio":6.0,"attack":4.0,"release":90.0,"knee":3.0,
    "makeup":3.0,"mix":0.85,"detector_mix":0.6,"sc_hpf_freq":120.0,
    "sc_hpf_on":1.0,"auto_makeup":0.0}}"#;

const PROJECT_V0: &str = r#"{"params":{
    "threshold":-14.0,"ratio":2.5,"attack":25.0,"release":400.0,"knee":9.0,
    "makeup":0.0,"mix":1.0,"detector_mix":0.1,"sc_hpf_freq":80.0,
    "sc_hpf_on":0.0,"auto_makeup":1.0}}"#;

fn golden_path() -> PathBuf {
    golden::golden_path(env!("CARGO_MANIFEST_DIR"), "legacy_state.f32")
}

fn blessing() -> bool {
    golden::blessed(&["RESONANCE_BLESS_LEGACY_STATE"])
}

/// Loud and quiet sections alternating every ~85 ms, with a pluck
/// envelope inside each, so attack, release and the knee all act.
fn input(n: u64) -> (f32, f32) {
    let t = n as f32 / SR;
    let section = (n / 4_096) % 2;
    let amp = if section == 0 { 0.9 } else { 0.08 };
    let env = (-18.0 * (t % 0.25)).exp();
    let v = amp * env * (140.0 * t * std::f32::consts::TAU).sin();
    (v, 0.9 * v)
}

/// A key train that is loud where the input is quiet.
fn key(n: u64) -> f32 {
    let t = n as f32 / SR;
    let on = (n / 3_000) % 2 == 1;
    if on {
        0.7 * (60.0 * t * std::f32::consts::TAU).sin()
    } else {
        0.0
    }
}

fn render_state(state: &[u8], keyed: bool) -> Vec<f32> {
    let mut plugin = ResonanceCompressor::new();
    assert!(plugin.load_state(state), "state blob failed to load");
    plugin.initialize(SR, BLOCK as u32);
    let mut out = Vec::with_capacity(BLOCKS * BLOCK * 2);
    let mut left = vec![0.0f32; BLOCK];
    let mut right = vec![0.0f32; BLOCK];
    let mut kl = vec![0.0f32; BLOCK];
    let mut n = 0u64;
    for _ in 0..BLOCKS {
        for i in 0..BLOCK {
            let (l, r) = input(n + i as u64);
            left[i] = l;
            right[i] = r;
            kl[i] = key(n + i as u64);
        }
        let mut outs = [OutputBuffer {
            left: &mut left,
            right: &mut right,
        }];
        let mut ev = EventIterator::empty();
        if keyed {
            let k = KeyBuffer {
                left: &kl,
                right: &kl,
            };
            plugin.process_with_key(&mut outs, Some(k), BLOCK, &mut ev, None);
        } else {
            plugin.process(&mut outs, BLOCK, &mut ev, None);
        }
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
    "Snare — Slam",
    "Bass — Glue",
    "Vocal — Lead",
    "Guitar — Control",
    "Drum Bus",
    "Mix Bus",
    "Master — Glue",
    "Parallel Smash",
    "Transparent",
];

#[test]
fn pre_w8_state_renders_bit_identically() {
    let mut rendered = Vec::new();
    for (name, state) in states() {
        for keyed in [false, true] {
            let out = render_state(&state, keyed);
            let peak = out.iter().fold(0.0f32, |m, x| m.max(x.abs()));
            assert!(peak > 1e-3, "`{name}` (keyed {keyed}) rendered silence");
            assert!(out.iter().all(|x| x.is_finite()), "`{name}` is non-finite");
            rendered.extend(out);
        }
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
            "state saved before release_mode no longer renders the same: \
             {}/{} samples differ (peak {:.3e}), first at {i} (got {got}, want {want})",
            diff.diff_count,
            rendered.len(),
            diff.max_abs
        );
    }
}
