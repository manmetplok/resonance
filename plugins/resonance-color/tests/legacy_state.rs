//! State saved before slice W6b (warmth-width-depth.md §6.1 / D2: the
//! `tape_quality` switch and its Jiles-Atherton HQ stage) must load and
//! render exactly as it did before.
//!
//! The golden this compares against was blessed on the code *before*
//! `tape_quality` existed, from blobs that name only the 11 parameters
//! the plugin had then. A project or preset written by the old build has
//! no `tape_quality` key, so it must load as Standard and produce the
//! same bits the old build produced.
//!
//! Never re-bless this file for a change that is meant to be
//! transparent. It exists to fail when one is not.

use std::path::PathBuf;

use resonance_color::presets::PRESETS;
use resonance_color::ResonanceColor;
use resonance_dsp_test_support as golden;
use resonance_plugin::{EventIterator, OutputBuffer, ResonancePlugin};

const SR: f32 = 48_000.0;
const BLOCK: usize = 256;
const BLOCKS: usize = 16;

/// Tape at 4x, driven hard, with flutter and a response tilt: every
/// piece of the Standard tape path a quality switch could disturb.
const TAPE_HOT_4X: &str = r#"{"params":{
    "mode":1,"drive":0.9,"bias":0.7,"response":4.0,"tone":-1.5,"mix":1.0,
    "auto_gain":1,"output":-2.0,"oversample":2,"speed":0,"flutter":0.4}}"#;

/// Tape at 1x, parallel, auto-gain off, 30 ips.
const TAPE_PARALLEL_1X: &str = r#"{"params":{
    "mode":1,"drive":0.5,"bias":0.2,"response":-3.0,"tone":1.0,"mix":0.6,
    "auto_gain":0,"output":1.0,"oversample":0,"speed":2,"flutter":0.0}}"#;

/// Tape at the default 2x and 15 ips.
const TAPE_DEFAULT_2X: &str = r#"{"params":{
    "mode":1,"drive":0.35,"bias":0.5,"response":0.0,"tone":0.0,"mix":1.0,
    "auto_gain":1,"output":0.0,"oversample":1,"speed":1,"flutter":0.0}}"#;

/// A non-tape mode, so a change that leaks outside Tape is caught too.
const TRANSFORMER_4X: &str = r#"{"params":{
    "mode":2,"drive":0.8,"bias":0.4,"response":6.0,"tone":1.5,"mix":1.0,
    "auto_gain":1,"output":-1.0,"oversample":2,"speed":1,"flutter":0.0}}"#;

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

/// Broadband, partly decorrelated stereo: noise plus a 110 Hz tone and
/// a 3 kHz one (what the head bump and the HF loss act on).
fn input(n: u64) -> (f32, f32) {
    let t = n as f32 / SR;
    let tau = std::f32::consts::TAU;
    let tone = 0.3 * (110.0 * t * tau).sin() + 0.1 * (3_000.0 * t * tau).sin();
    (0.2 * noise(n) + tone, 0.15 * noise(n + 77_777) + 0.1 * noise(n) + tone)
}

fn render_state(state: &[u8]) -> Vec<f32> {
    let mut plugin = ResonanceColor::new();
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
        ("tape_hot_4x".to_string(), TAPE_HOT_4X.as_bytes().to_vec()),
        ("tape_parallel_1x".to_string(), TAPE_PARALLEL_1X.as_bytes().to_vec()),
        ("tape_default_2x".to_string(), TAPE_DEFAULT_2X.as_bytes().to_vec()),
        ("transformer_4x".to_string(), TRANSFORMER_4X.as_bytes().to_vec()),
    ];
    // The bank as it stood when the golden was blessed. Named rather than
    // enumerated, so a preset added later cannot change what this pins.
    for name in PRE_W6B_PRESETS {
        let p = PRESETS
            .iter()
            .find(|p| p.name == *name)
            .unwrap_or_else(|| panic!("factory preset `{name}` disappeared"));
        v.push((format!("preset {}", p.name), p.json.as_bytes().to_vec()));
    }
    v
}

/// Factory presets that existed before W6b, in bank order.
const PRE_W6B_PRESETS: &[&str] = &[
    "Bus — Warm Glue",
    "Bass — Iron",
    "Vocal — Tube Air",
    "Drums — Tape 15",
    "Master — Subtle Tape",
];

#[test]
fn pre_w6b_state_renders_bit_identically() {
    let mut rendered = Vec::new();
    for (name, state) in states() {
        let out = render_state(&state);
        let peak = out.iter().fold(0.0f32, |m, x| m.max(x.abs()));
        assert!(peak > 1e-2, "`{name}` rendered silence");
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
            "state saved before W6b (tape_quality) no longer renders the same: \
             {}/{} samples differ (peak {:.3e}), first at {i} (got {got}, want {want})",
            diff.diff_count,
            rendered.len(),
            diff.max_abs
        );
    }
}
