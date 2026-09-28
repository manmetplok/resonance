//! State saved before the W9 mastering extensions (warmth-width-depth.md
//! §6.3: per-band M/S on the EQ stages, per-band imager width, the
//! clipper and the saturator modes) must load and render exactly as it
//! did before them.
//!
//! The golden this compares against was blessed on the code *before* any
//! of those existed, from blobs that name only the 102 parameters the
//! plugin had then (captured verbatim from that build's `save_state`). A
//! project written by the old build, loaded into this one, must produce
//! the same bits the old build produced. The plugin ships no factory
//! presets, so saved projects are all there is to pin;
//! `the_factory_bank_is_still_empty` fails the day that changes, so a new
//! bank gets pinned here too.
//!
//! The comparison is exact. `dsp_golden.rs` allows an FFT-rounding budget
//! because rustfft picks its kernel from the CPU at runtime; this file is
//! blessed and checked on the canonical machine (CLAUDE.md), where the
//! kernel is fixed, and a transparent change must not move a single bit.
//!
//! Never re-bless this file for a change that is meant to be transparent.
//! It exists to fail when one is not. The narrow switch is
//! `RESONANCE_BLESS_LEGACY_STATE=1`; the workspace-wide `RESONANCE_BLESS`
//! deliberately does not touch it.

use std::path::PathBuf;

use resonance_dsp_test_support as golden;
use resonance_mastering::ResonanceMastering;
use resonance_plugin::{EventIterator, OutputBuffer, ResonancePlugin};

const SR: f32 = 48_000.0;
const MAX_BLOCK: usize = 512;
/// Host block sizes, cycled. Uneven on purpose: the FIR stages land
/// their hops mid-block.
const BLOCK_SIZES: [usize; 3] = [480, 512, 333];
/// Blocks rendered but not stored: the chain's ~20.7k samples of latency
/// is FIR pre-fill, and pinning zeros would pin nothing.
///
/// Re-blessed once, for W12. The de-harsh stage adds one 2048-sample
/// STFT frame of latency even when off, and a project that never engages
/// it spends that latency as a plain delay after dither. Every stream
/// here was shown to be the pre-W12 stream preceded by exactly 2048
/// zero samples, bit for bit, before the re-bless
/// (`docs/design/deharsh-resonance-suppressor.md` §5.2).
const PRIME_BLOCKS: usize = 56;
/// Blocks stored in the golden.
const CAPTURE_BLOCKS: usize = 20;

/// Every stage engaged: three corrective and three tonal bands, glue,
/// the Gritty saturator at a Tube/Tape blend, the multiband on all four
/// bands with moved crossovers, the imager with its side HPF, the
/// limiter against −1 dBTP and 16-bit shaped dither.
const PROJECT_FULL: &str = r#"{"version":1,"params":{
    "bypass":0.0,
    "corr_b0_freq":35.0,"corr_b0_gain":0.0,"corr_b0_on":1.0,"corr_b0_q":0.7070000171661377,"corr_b0_type":3.0,
    "corr_b1_freq":320.0,"corr_b1_gain":-4.5,"corr_b1_on":1.0,"corr_b1_q":6.0,"corr_b1_type":0.0,
    "corr_b2_freq":500.0,"corr_b2_gain":-2.0,"corr_b2_on":0.0,"corr_b2_q":2.0,"corr_b2_type":0.0,
    "corr_b3_freq":3200.0,"corr_b3_gain":-2.0,"corr_b3_on":1.0,"corr_b3_q":3.0,"corr_b3_type":0.0,
    "dith_bits":16.0,"dith_ns":1.0,"dith_on":1.0,
    "glue_attack":20.0,"glue_knee":6.0,"glue_makeup":2.0,"glue_mix":0.800000011920929,"glue_on":1.0,"glue_ratio":3.0,"glue_release":200.0,"glue_threshold":-18.0,
    "img_on":1.0,"img_side_hpf_freq":140.0,"img_side_hpf_on":1.0,"img_width":1.2999999523162842,
    "input_trim_db":6.0,
    "lim_ceiling":-1.0,"lim_on":1.0,"lim_release":60.0,
    "mb_b0_attack":10.0,"mb_b0_gain":-0.5,"mb_b0_knee":6.0,"mb_b0_mix":1.0,"mb_b0_on":1.0,"mb_b0_ratio":2.5,"mb_b0_release":150.0,"mb_b0_thresh":-24.0,
    "mb_b1_attack":15.0,"mb_b1_gain":0.0,"mb_b1_knee":6.0,"mb_b1_mix":0.8999999761581421,"mb_b1_on":1.0,"mb_b1_ratio":2.5,"mb_b1_release":130.0,"mb_b1_thresh":-21.0,
    "mb_b2_attack":20.0,"mb_b2_gain":0.5,"mb_b2_knee":6.0,"mb_b2_mix":0.800000011920929,"mb_b2_on":1.0,"mb_b2_ratio":2.5,"mb_b2_release":110.0,"mb_b2_thresh":-18.0,
    "mb_b3_attack":25.0,"mb_b3_gain":1.0,"mb_b3_knee":6.0,"mb_b3_mix":0.699999988079071,"mb_b3_on":1.0,"mb_b3_ratio":2.5,"mb_b3_release":90.0,"mb_b3_thresh":-15.0,
    "mb_on":1.0,"mb_xo1":150.0,"mb_xo2":1200.0,"mb_xo3":6000.0,
    "sat_character":0.699999988079071,"sat_drive":4.5,"sat_mix":0.6000000238418579,"sat_on":1.0,"sat_shaper":1.0,
    "target_lufs":-11.0,
    "tone_b0_freq":120.0,"tone_b0_gain":3.0,"tone_b0_on":1.0,"tone_b0_q":0.800000011920929,"tone_b0_type":1.0,
    "tone_b1_freq":700.0,"tone_b1_gain":0.0,"tone_b1_on":0.0,"tone_b1_q":0.800000011920929,"tone_b1_type":0.0,
    "tone_b2_freq":2500.0,"tone_b2_gain":1.5,"tone_b2_on":1.0,"tone_b2_q":0.8999999761581421,"tone_b2_type":0.0,
    "tone_b3_freq":8000.0,"tone_b3_gain":2.5,"tone_b3_on":1.0,"tone_b3_q":0.800000011920929,"tone_b3_type":2.0}}"#;

/// A freshly inserted plugin, saved untouched.
const PROJECT_DEFAULTS: &str = r#"{"version":1,"params":{
    "bypass":0.0,
    "corr_b0_freq":30.0,"corr_b0_gain":0.0,"corr_b0_on":0.0,"corr_b0_q":0.7070000171661377,"corr_b0_type":3.0,
    "corr_b1_freq":250.0,"corr_b1_gain":-3.0,"corr_b1_on":0.0,"corr_b1_q":2.0,"corr_b1_type":0.0,
    "corr_b2_freq":500.0,"corr_b2_gain":-2.0,"corr_b2_on":0.0,"corr_b2_q":2.0,"corr_b2_type":0.0,
    "corr_b3_freq":3000.0,"corr_b3_gain":-2.0,"corr_b3_on":0.0,"corr_b3_q":3.0,"corr_b3_type":0.0,
    "dith_bits":16.0,"dith_ns":0.0,"dith_on":0.0,
    "glue_attack":30.0,"glue_knee":6.0,"glue_makeup":0.0,"glue_mix":1.0,"glue_on":0.0,"glue_ratio":2.0,"glue_release":150.0,"glue_threshold":-18.0,
    "img_on":0.0,"img_side_hpf_freq":120.0,"img_side_hpf_on":0.0,"img_width":1.0,
    "input_trim_db":0.0,
    "lim_ceiling":-0.30000001192092896,"lim_on":0.0,"lim_release":50.0,
    "mb_b0_attack":30.0,"mb_b0_gain":0.0,"mb_b0_knee":6.0,"mb_b0_mix":1.0,"mb_b0_on":0.0,"mb_b0_ratio":2.0,"mb_b0_release":150.0,"mb_b0_thresh":-18.0,
    "mb_b1_attack":30.0,"mb_b1_gain":0.0,"mb_b1_knee":6.0,"mb_b1_mix":1.0,"mb_b1_on":0.0,"mb_b1_ratio":2.0,"mb_b1_release":150.0,"mb_b1_thresh":-18.0,
    "mb_b2_attack":30.0,"mb_b2_gain":0.0,"mb_b2_knee":6.0,"mb_b2_mix":1.0,"mb_b2_on":0.0,"mb_b2_ratio":2.0,"mb_b2_release":150.0,"mb_b2_thresh":-18.0,
    "mb_b3_attack":30.0,"mb_b3_gain":0.0,"mb_b3_knee":6.0,"mb_b3_mix":1.0,"mb_b3_on":0.0,"mb_b3_ratio":2.0,"mb_b3_release":150.0,"mb_b3_thresh":-18.0,
    "mb_on":0.0,"mb_xo1":120.0,"mb_xo2":800.0,"mb_xo3":4000.0,
    "sat_character":0.30000001192092896,"sat_drive":3.0,"sat_mix":1.0,"sat_on":0.0,"sat_shaper":0.0,
    "target_lufs":-14.0,
    "tone_b0_freq":100.0,"tone_b0_gain":0.0,"tone_b0_on":0.0,"tone_b0_q":0.7070000171661377,"tone_b0_type":1.0,
    "tone_b1_freq":700.0,"tone_b1_gain":0.0,"tone_b1_on":0.0,"tone_b1_q":0.800000011920929,"tone_b1_type":0.0,
    "tone_b2_freq":2500.0,"tone_b2_gain":0.0,"tone_b2_on":0.0,"tone_b2_q":0.800000011920929,"tone_b2_type":0.0,
    "tone_b3_freq":10000.0,"tone_b3_gain":0.0,"tone_b3_on":0.0,"tone_b3_q":0.7070000171661377,"tone_b3_type":2.0}}"#;

/// The Smooth saturator fully on the Tape side at 9 dB drive, one tonal
/// bell, a narrowed image and a hard limiter.
const PROJECT_SAT_TAPE_IMAGER: &str = r#"{"version":1,"params":{
    "bypass":0.0,
    "corr_b0_freq":30.0,"corr_b0_gain":0.0,"corr_b0_on":0.0,"corr_b0_q":0.7070000171661377,"corr_b0_type":3.0,
    "corr_b1_freq":250.0,"corr_b1_gain":-3.0,"corr_b1_on":0.0,"corr_b1_q":2.0,"corr_b1_type":0.0,
    "corr_b2_freq":500.0,"corr_b2_gain":-2.0,"corr_b2_on":0.0,"corr_b2_q":2.0,"corr_b2_type":0.0,
    "corr_b3_freq":3000.0,"corr_b3_gain":-2.0,"corr_b3_on":0.0,"corr_b3_q":3.0,"corr_b3_type":0.0,
    "dith_bits":16.0,"dith_ns":0.0,"dith_on":0.0,
    "glue_attack":30.0,"glue_knee":6.0,"glue_makeup":0.0,"glue_mix":1.0,"glue_on":0.0,"glue_ratio":2.0,"glue_release":150.0,"glue_threshold":-18.0,
    "img_on":1.0,"img_side_hpf_freq":120.0,"img_side_hpf_on":0.0,"img_width":0.6000000238418579,
    "input_trim_db":0.0,
    "lim_ceiling":-3.0,"lim_on":1.0,"lim_release":20.0,
    "mb_b0_attack":30.0,"mb_b0_gain":0.0,"mb_b0_knee":6.0,"mb_b0_mix":1.0,"mb_b0_on":0.0,"mb_b0_ratio":2.0,"mb_b0_release":150.0,"mb_b0_thresh":-18.0,
    "mb_b1_attack":30.0,"mb_b1_gain":0.0,"mb_b1_knee":6.0,"mb_b1_mix":1.0,"mb_b1_on":0.0,"mb_b1_ratio":2.0,"mb_b1_release":150.0,"mb_b1_thresh":-18.0,
    "mb_b2_attack":30.0,"mb_b2_gain":0.0,"mb_b2_knee":6.0,"mb_b2_mix":1.0,"mb_b2_on":0.0,"mb_b2_ratio":2.0,"mb_b2_release":150.0,"mb_b2_thresh":-18.0,
    "mb_b3_attack":30.0,"mb_b3_gain":0.0,"mb_b3_knee":6.0,"mb_b3_mix":1.0,"mb_b3_on":0.0,"mb_b3_ratio":2.0,"mb_b3_release":150.0,"mb_b3_thresh":-18.0,
    "mb_on":0.0,"mb_xo1":120.0,"mb_xo2":800.0,"mb_xo3":4000.0,
    "sat_character":1.0,"sat_drive":9.0,"sat_mix":1.0,"sat_on":1.0,"sat_shaper":0.0,
    "target_lufs":-14.0,
    "tone_b0_freq":100.0,"tone_b0_gain":0.0,"tone_b0_on":0.0,"tone_b0_q":0.7070000171661377,"tone_b0_type":1.0,
    "tone_b1_freq":700.0,"tone_b1_gain":-2.0,"tone_b1_on":1.0,"tone_b1_q":0.800000011920929,"tone_b1_type":0.0,
    "tone_b2_freq":2500.0,"tone_b2_gain":0.0,"tone_b2_on":0.0,"tone_b2_q":0.800000011920929,"tone_b2_type":0.0,
    "tone_b3_freq":10000.0,"tone_b3_gain":0.0,"tone_b3_on":0.0,"tone_b3_q":0.7070000171661377,"tone_b3_type":2.0}}"#;

/// Written before the state version field existed, naming only a few
/// ids: every other param keeps its default.
const PROJECT_V0_PARTIAL: &str = r#"{"params":{
    "input_trim_db":3.0,
    "glue_on":1.0,"glue_threshold":-24.0,"glue_ratio":4.0,"glue_attack":5.0,"glue_release":80.0,
    "sat_on":1.0,"sat_drive":12.0,"sat_character":0.0,"sat_mix":0.4,"sat_shaper":0.0,
    "lim_on":1.0,"lim_ceiling":-0.5,"lim_release":150.0}}"#;

fn golden_path() -> PathBuf {
    golden::golden_path(env!("CARGO_MANIFEST_DIR"), "legacy_state.f32")
}

fn blessing() -> bool {
    golden::blessed(&["RESONANCE_BLESS_LEGACY_STATE"])
}

const TAU: f32 = std::f32::consts::TAU;

/// Deterministic pseudo-noise from the absolute sample index.
fn noise(n: u64) -> f32 {
    let mut s = n.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1) as u32;
    s ^= s >> 16;
    s = s.wrapping_mul(2_246_822_519);
    s ^= s >> 13;
    (s >> 8) as f32 * (2.0 / (1 << 23) as f32) - 1.0
}

/// Hot drum-like transients over tones in every multiband band and a
/// partly decorrelated noise bed: the limiter works, every crossover
/// band has energy, and the side channel is non-trivial.
fn input(n: u64) -> (f32, f32) {
    let t = n as f32 / SR;
    let env = (-22.0 * (t % (1.0 / 7.0))).exp();
    let hit = 1.4 * env * ((70.0 * t * TAU).sin() + 0.4 * (1600.0 * t * TAU).sin());
    let bed = 0.20 * (60.0 * t * TAU).sin()
        + 0.15 * (400.0 * t * TAU).sin()
        + 0.10 * (2500.0 * t * TAU).sin()
        + 0.06 * (9000.0 * t * TAU).sin();
    let l = hit + bed + 0.08 * noise(n) + 0.10 * (660.0 * t * TAU).sin();
    let r = 0.9 * hit + bed + 0.08 * noise(n + 4_651) - 0.10 * (660.0 * t * TAU).sin();
    (l, r)
}

fn render_state(state: &[u8]) -> Vec<f32> {
    let mut plugin = ResonanceMastering::new();
    assert!(plugin.load_state(state), "state blob failed to load");
    plugin.initialize(SR, MAX_BLOCK as u32);
    plugin.reset();
    let mut out = Vec::new();
    let mut left = vec![0.0f32; MAX_BLOCK];
    let mut right = vec![0.0f32; MAX_BLOCK];
    let mut n = 0u64;
    for block in 0..PRIME_BLOCKS + CAPTURE_BLOCKS {
        let frames = BLOCK_SIZES[block % BLOCK_SIZES.len()];
        for i in 0..frames {
            let (l, r) = input(n + i as u64);
            left[i] = l;
            right[i] = r;
        }
        {
            let mut outs = [OutputBuffer {
                left: &mut left[..frames],
                right: &mut right[..frames],
            }];
            let mut ev = EventIterator::empty();
            plugin.process(&mut outs, frames, &mut ev, None);
        }
        n += frames as u64;
        if block >= PRIME_BLOCKS {
            out.extend_from_slice(&left[..frames]);
            out.extend_from_slice(&right[..frames]);
        }
    }
    out
}

fn states() -> Vec<(&'static str, &'static str)> {
    vec![
        ("project_full", PROJECT_FULL),
        ("project_defaults", PROJECT_DEFAULTS),
        ("project_sat_tape_imager", PROJECT_SAT_TAPE_IMAGER),
        ("project_v0_partial", PROJECT_V0_PARTIAL),
    ]
}

#[test]
fn pre_w9_state_renders_bit_identically() {
    let mut rendered = Vec::new();
    for (name, state) in states() {
        let out = render_state(state.as_bytes());
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
            "state saved before the W9 extensions no longer renders the same: \
             {}/{} samples differ (peak {:.3e}), first at {i} (got {got}, want {want})",
            diff.diff_count,
            rendered.len(),
            diff.max_abs
        );
    }
}

/// The prime window must cover the chain's latency, or the golden would
/// pin FIR pre-fill instead of audio.
#[test]
fn prime_window_covers_the_latency() {
    let mut plugin = ResonanceMastering::new();
    plugin.initialize(SR, MAX_BLOCK as u32);
    let latency = plugin.latency_samples() as usize;
    let primed: usize = (0..PRIME_BLOCKS)
        .map(|b| BLOCK_SIZES[b % BLOCK_SIZES.len()])
        .sum();
    assert!(
        primed > latency + 2 * MAX_BLOCK,
        "prime window {primed} frames vs latency {latency}: raise PRIME_BLOCKS"
    );
}

/// This file pins saved projects only because there are no factory
/// presets to pin. If a bank appears, pin it here before it ships.
#[test]
fn the_factory_bank_is_still_empty() {
    assert!(
        <ResonanceMastering as ResonancePlugin>::FACTORY_PRESETS.is_empty(),
        "resonance-mastering now ships factory presets: add them to legacy_state.rs"
    );
}
