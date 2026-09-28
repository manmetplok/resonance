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

// The factory presets as they stood before W6b, verbatim from the W6
// merge (`git show 5ef6499e:plugins/resonance-color/presets/<file>`).
// Copies, not the live bank: today's preset files carry `tape_quality`
// and `tape_solver`, so loading them would never exercise the missing
// keys this test exists for. A preset added or re-voiced later cannot
// change what this pins either.

/// `presets/bus_warm_glue.json` (Bus — Warm Glue) as W6 shipped it.
const PRESET_BUS_WARM_GLUE: &str = r#"{
  "params": {
    "mode": 4,
    "drive": 0.55,
    "bias": 0.7,
    "response": 3.0,
    "tone": -1.0,
    "mix": 0.6,
    "auto_gain": 1,
    "output": 0.0,
    "oversample": 1,
    "speed": 1,
    "flutter": 0.0
  }
}
"#;

/// `presets/bass_iron.json` (Bass — Iron) as W6 shipped it.
const PRESET_BASS_IRON: &str = r#"{
  "params": {
    "mode": 2,
    "drive": 0.75,
    "bias": 0.5,
    "response": 4.0,
    "tone": 0.0,
    "mix": 1.0,
    "auto_gain": 1,
    "output": 0.0,
    "oversample": 1,
    "speed": 1,
    "flutter": 0.0
  }
}
"#;

/// `presets/vocal_tube_air.json` (Vocal — Tube Air) as W6 shipped it.
const PRESET_VOCAL_TUBE_AIR: &str = r#"{
  "params": {
    "mode": 0,
    "drive": 0.52,
    "bias": 0.5,
    "response": -3.0,
    "tone": 2.0,
    "mix": 0.85,
    "auto_gain": 1,
    "output": 0.0,
    "oversample": 1,
    "speed": 1,
    "flutter": 0.0
  }
}
"#;

/// `presets/drums_tape_15.json` (Drums — Tape 15) as W6 shipped it.
const PRESET_DRUMS_TAPE_15: &str = r#"{
  "params": {
    "mode": 1,
    "drive": 0.55,
    "bias": 0.4,
    "response": 0.0,
    "tone": -0.5,
    "mix": 0.8,
    "auto_gain": 1,
    "output": 0.0,
    "oversample": 1,
    "speed": 1,
    "flutter": 0.0
  }
}
"#;

/// `presets/master_subtle_tape.json` (Master — Subtle Tape) as W6 shipped it.
const PRESET_MASTER_SUBTLE_TAPE: &str = r#"{
  "params": {
    "mode": 1,
    "drive": 0.25,
    "bias": 0.3,
    "response": 0.0,
    "tone": 0.0,
    "mix": 0.8,
    "auto_gain": 1,
    "output": 0.0,
    "oversample": 2,
    "speed": 2,
    "flutter": 0.0
  }
}
"#;

fn states() -> Vec<(String, Vec<u8>)> {
    let blobs = [
        ("tape_hot_4x", TAPE_HOT_4X),
        ("tape_parallel_1x", TAPE_PARALLEL_1X),
        ("tape_default_2x", TAPE_DEFAULT_2X),
        ("transformer_4x", TRANSFORMER_4X),
        ("preset Bus — Warm Glue", PRESET_BUS_WARM_GLUE),
        ("preset Bass — Iron", PRESET_BASS_IRON),
        ("preset Vocal — Tube Air", PRESET_VOCAL_TUBE_AIR),
        ("preset Drums — Tape 15", PRESET_DRUMS_TAPE_15),
        ("preset Master — Subtle Tape", PRESET_MASTER_SUBTLE_TAPE),
    ];
    for (name, json) in blobs {
        assert!(
            !json.contains("tape_quality") && !json.contains("tape_solver"),
            "`{name}` is not a pre-W6b blob"
        );
    }
    blobs
        .iter()
        .map(|(name, json)| (name.to_string(), json.as_bytes().to_vec()))
        .collect()
}

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
