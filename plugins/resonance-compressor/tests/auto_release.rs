//! `release_mode`: Manual (the default, the release knob) and Auto (the
//! program-dependent dual-envelope release, warmth-width-depth.md §6.4).
//!
//! DC input with the sidechain HPF off makes `output / input` the exact
//! per-sample gain, as in `release_time.rs`, so every GR trajectory here
//! is read straight off the audio.

use std::path::PathBuf;

use resonance_compressor::dsp::{
    CompressorDsp, AUTO_FAST_RELEASE_MS, AUTO_SLOW_RELEASE_MS,
};
use resonance_compressor::params::CompressorParams;
use resonance_compressor::viz::CompressorViz;
use resonance_compressor::ResonanceCompressor;
use resonance_dsp_test_support as golden;
use resonance_plugin::{EventIterator, OutputBuffer, Param, ResonancePlugin};

const SR: f32 = 48_000.0;
const BLOCK: usize = 64;
const LOUD: f32 = 1.0; // 0 dBFS
const QUIET: f32 = 0.001; // -60 dBFS

fn params(auto: bool) -> CompressorParams {
    let p = CompressorParams::default();
    p.threshold.set_value(-20.0);
    p.ratio.set_value(4.0);
    p.knee.set_value(0.0);
    p.attack.set_value(1.0);
    p.release.set_value(120.0);
    p.makeup.set_value(0.0);
    p.mix.set_value(1.0);
    p.detector_mix.set_value(0.0); // peak
    p.auto_makeup.set_value(false);
    p.sc_hpf_on.set_value(false);
    p.release_mode.set_value(auto as i32);
    p
}

/// GR a recovery is timed down to, dB: 80 % of the way back from the
/// 15 dB these tests compress by. (After a short hit Auto leaves a small
/// residue on the slow envelope — about 1 dB — that fades on the slow
/// constant; that tail is the character, not the recovery.)
const RECOVERED_DB: f32 = 3.0;

/// Hold 0 dBFS for `loud_ms`, then drop to -60 dBFS. Returns how long GR
/// takes to fall from its value at the step to below [`RECOVERED_DB`],
/// in ms, and that value.
fn recovery_ms(auto: bool, loud_ms: f32) -> (f32, f32) {
    let params = params(auto);
    let viz = CompressorViz::new();
    let mut dsp = CompressorDsp::new(SR, &params);
    let loud_blocks = ((loud_ms * 0.001 * SR) as usize / BLOCK).max(1);
    let mut gr_at_step = 0.0;
    for _ in 0..loud_blocks {
        let mut l = vec![LOUD; BLOCK];
        let mut r = vec![LOUD; BLOCK];
        dsp.process_stereo(&mut l, &mut r, None, &params, &viz);
        gr_at_step = -20.0 * (l[BLOCK - 1] / LOUD).log10();
    }
    let mut n = 0usize;
    for _ in 0..(10 * SR as usize / BLOCK) {
        let mut l = vec![QUIET; BLOCK];
        let mut r = vec![QUIET; BLOCK];
        dsp.process_stereo(&mut l, &mut r, None, &params, &viz);
        for &y in &l {
            if -20.0 * (y / QUIET).log10() < RECOVERED_DB {
                return (n as f32 / SR * 1000.0, gr_at_step);
            }
            n += 1;
        }
    }
    panic!("GR never recovered (auto {auto}, loud {loud_ms} ms)");
}

#[test]
fn release_mode_defaults_to_manual() {
    assert_eq!(CompressorParams::default().release_mode.value(), 0);
    assert_eq!(CompressorParams::default().release_mode.display(0.0), "Manual");
}

#[test]
fn manual_release_ignores_how_long_the_compression_lasted() {
    let (short, gr_short) = recovery_ms(false, 20.0);
    let (long, gr_long) = recovery_ms(false, 2_000.0);
    // Both reached the full 15 dB at 1 ms attack, so one time constant
    // governs both recoveries equally.
    assert!((gr_short - 15.0).abs() < 0.5 && (gr_long - 15.0).abs() < 0.5);
    assert!((short - long).abs() < 2.0, "manual: {short:.1} vs {long:.1} ms");
}

#[test]
fn auto_release_recovers_fast_after_a_hit_and_slowly_after_sustained_compression() {
    let (short, gr_short) = recovery_ms(true, 20.0);
    let (long, gr_long) = recovery_ms(true, 2_000.0);
    assert!((gr_short - 15.0).abs() < 0.5, "the fast envelope must still clamp: {gr_short}");
    assert!((gr_long - 15.0).abs() < 0.5);
    // After a 20 ms hit the slow envelope has barely charged: recovery is
    // on the fast constant, ln(15 / 3) ≈ 1.6 time constants.
    let fast_bound = 2.5 * AUTO_FAST_RELEASE_MS;
    assert!(short < fast_bound, "auto after a hit: {short:.0} ms (bound {fast_bound:.0})");
    // After two seconds held in compression the slow envelope is fully
    // charged and governs the recovery (1.6 slow time constants).
    let slow_floor = 1.2 * AUTO_SLOW_RELEASE_MS;
    assert!(long > slow_floor, "auto after sustained GR: {long:.0} ms (floor {slow_floor:.0})");
    assert!(long > 5.0 * short);
}

#[test]
fn switching_modes_mid_compression_does_not_jump() {
    let p = params(false);
    let viz = CompressorViz::new();
    let mut dsp = CompressorDsp::new(SR, &p);
    let mut prev = None;
    for block in 0..400 {
        if block == 150 {
            p.release_mode.set_value(1);
        }
        if block == 300 {
            p.release_mode.set_value(0);
        }
        let x = if block < 200 { LOUD } else { QUIET };
        let mut l = vec![x; BLOCK];
        let mut r = vec![x; BLOCK];
        dsp.process_stereo(&mut l, &mut r, None, &p, &viz);
        for &y in &l {
            let gr = -20.0 * (y / x).log10();
            if let Some((px, pgr)) = prev {
                // Only compare within a steady input level.
                if px == x {
                    let step: f32 = gr - pgr;
                    assert!(step.abs() < 0.5, "GR jumped {step:.2} dB at block {block}");
                }
            }
            prev = Some((x, gr));
        }
    }
}

// ---------------------------------------------------------------------------
// Golden: Auto release on a programme-like signal
// ---------------------------------------------------------------------------

fn golden_input(n: u64) -> (f32, f32) {
    let t = n as f32 / SR;
    // Hits every 150 ms, over a bed that swells in for the second half.
    let hit = 0.9 * (-30.0 * (t % 0.15)).exp() * (std::f32::consts::TAU * 110.0 * t).sin();
    let bed = if n > 12_000 { 0.5 } else { 0.05 } * (std::f32::consts::TAU * 330.0 * t).sin();
    (hit + bed, 0.9 * hit + bed)
}

#[test]
fn auto_release_golden() {
    let mut plugin = ResonanceCompressor::new();
    let p = &plugin.params;
    p.threshold.set_value(-18.0);
    p.ratio.set_value(3.0);
    p.attack.set_value(10.0);
    p.release.set_value(120.0);
    p.knee.set_value(6.0);
    p.makeup.set_value(0.0);
    p.mix.set_value(1.0);
    p.detector_mix.set_value(0.3);
    p.sc_hpf_on.set_value(false);
    p.auto_makeup.set_value(false);
    p.release_mode.set_value(1);
    plugin.initialize(SR, 256);
    let mut all = Vec::new();
    let mut l = vec![0.0f32; 256];
    let mut r = vec![0.0f32; 256];
    let mut n = 0u64;
    for _ in 0..96 {
        for i in 0..256 {
            (l[i], r[i]) = golden_input(n + i as u64);
        }
        let mut outs = [OutputBuffer {
            left: &mut l,
            right: &mut r,
        }];
        plugin.process(&mut outs, 256, &mut EventIterator::empty(), None);
        n += 256;
        all.extend_from_slice(&l);
        all.extend_from_slice(&r);
    }
    let peak = all.iter().fold(0.0f32, |m, x| m.max(x.abs()));
    assert!(peak > 0.05, "the golden rendered silence");
    // It must actually be compressing, or it pins the input.
    let raw_peak = (0..n).fold(0.0f32, |m, i| m.max(golden_input(i).0.abs()));
    assert!(peak < 0.9 * raw_peak, "no gain reduction in the golden ({peak} vs {raw_peak})");

    let path: PathBuf = golden::golden_path(env!("CARGO_MANIFEST_DIR"), "auto_release.f32");
    if golden::blessed(&["RESONANCE_BLESS", "RESONANCE_BLESS_AUTO_RELEASE"]) {
        golden::bless_f32(&path, &all);
        return;
    }
    let want = golden::load_golden_f32(&path, all.len(), "RESONANCE_BLESS=1");
    let diff = golden::compare_f32(&all, &want);
    if let Some((i, got, want)) = diff.first_diff {
        panic!(
            "auto-release render changed: {}/{} samples differ (peak {:.3e}), \
             first at {i} (got {got}, want {want})",
            diff.diff_count,
            all.len(),
            diff.max_abs
        );
    }
}
