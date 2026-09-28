//! Tape HQ (slice W6b, decision D2): the Jiles-Atherton hysteresis stage
//! behind `tape_quality`.
//!
//! - its harmonic signature, pinned beside Standard's, so the HQ vs
//!   Standard delta is a decision and not a drift;
//! - auto-gain holds ±0.5 LU in HQ as it does in Standard;
//! - no NaN, runaway or stuck output at max drive on full-scale noise, at
//!   every sample rate from 44.1 to 192 kHz, every solver and every
//!   oversampling setting;
//! - oversampling is forced to at least 2x and the plugin still reports
//!   no latency;
//! - a bit-exact golden over HQ scenarios with pinned params and a
//!   non-silence guard;
//! - a CPU sanity check (`#[ignore]`d; run it by hand in release).

mod common;

use std::path::PathBuf;

use common::*;
use resonance_color::dsp::voicing::transfer_for;
use resonance_color::dsp::{ColorDsp, Settings};
use resonance_color::params::{ColorParams, Mode, TapeQuality};
use resonance_color::probe::{probe, HarmonicSignature, PROBE_LEVEL_DBFS};
use resonance_color::ResonanceColor;
use resonance_dsp::{HysteresisSolver, OversampleFactor, SimpleRng};
use resonance_dsp_test_support as golden;
use resonance_metering::LufsMeter;
use resonance_plugin::{EventIterator, OutputBuffer, Param, ResonancePlugin};

const TAU: f32 = std::f32::consts::TAU;

fn hq(drive: f32, bias: f32) -> Settings {
    Settings {
        mode: Mode::Tape,
        drive,
        bias,
        tape_quality: TapeQuality::Hq,
        ..Settings::default()
    }
}

fn standard(drive: f32, bias: f32) -> Settings {
    Settings {
        tape_quality: TapeQuality::Standard,
        ..hq(drive, bias)
    }
}

fn show(tag: &str, sig: &HarmonicSignature) {
    eprintln!(
        "{tag:24} THD {:.4} %  H2 {:.2}  H3 {:.2}  H4 {:.2}  H5 {:.2}  H7 {:.2}  gain {:+.2} dB",
        sig.thd_pct, sig.h_dbc[2], sig.h_dbc[3], sig.h_dbc[4], sig.h_dbc[5], sig.h_dbc[7], sig.gain_db
    );
}

// ---------------------------------------------------------------------------
// Harmonics
// ---------------------------------------------------------------------------

/// `(THD %, H3 dBc, H5 dBc)` pins.
fn assert_pin(tag: &str, sig: &HarmonicSignature, thd: f64, h3: f64, h5: f64) {
    assert!((sig.thd_pct / thd - 1.0).abs() < 0.02, "{tag}: THD {:.4} %, pinned {thd} %", sig.thd_pct);
    assert!((sig.h_dbc[3] - h3).abs() < 0.3, "{tag}: H3 {:.2} dBc, pinned {h3}", sig.h_dbc[3]);
    assert!((sig.h_dbc[5] - h5).abs() < 0.5, "{tag}: H5 {:.2} dBc, pinned {h5}", sig.h_dbc[5]);
}

/// §11's stimulus at the harmonics suite's operating point (drive and
/// bias 50 %). Standard Tape is the asymmetric ADAA curve, H2-led; HQ is
/// the symmetric loop, so its even orders vanish and H3 leads, about 15
/// dB hotter than Standard's.
#[test]
fn hq_vs_standard_harmonic_delta_is_pinned() {
    let std_sig = probe(&standard(0.5, 0.5), PROBE_LEVEL_DBFS);
    let hq_sig = probe(&hq(0.5, 0.5), PROBE_LEVEL_DBFS);
    show("Standard 0.5/0.5", &std_sig);
    show("HQ 0.5/0.5", &hq_sig);

    // Standard is what tests/harmonics.rs pins; restated so the delta
    // below reads against the same numbers.
    assert_pin("Standard", &std_sig, 2.167, -46.60, -92.2);
    assert!(std_sig.h2_h3_db() > 0.0, "Standard Tape is H2-led");

    // HQ: odd-only, H3-led.
    for k in [2, 4, 6, 8] {
        assert!(hq_sig.h_dbc[k] < -90.0, "HQ H{k} {:.1} dBc from a symmetric loop", hq_sig.h_dbc[k]);
    }
    assert!(hq_sig.h_dbc[3] > hq_sig.h_dbc[5] + 6.0, "HQ series falls");
    assert_pin("HQ", &hq_sig, HQ_PIN.0, HQ_PIN.1, HQ_PIN.2);

    // The delta itself.
    let d_h3 = hq_sig.h_dbc[3] - std_sig.h_dbc[3];
    eprintln!("HQ − Standard: H3 {d_h3:+.2} dB, THD ×{:.3}", hq_sig.thd_pct / std_sig.thd_pct);
    assert!((d_h3 - HQ_H3_DELTA_DB).abs() < 0.5, "H3 delta {d_h3:+.2} dB, pinned {HQ_H3_DELTA_DB:+}");
}

/// HQ at 50 % drive, 50 % bias: THD %, H3 and H5 in dBc.
const HQ_PIN: (f64, f64, f64) = (2.722, -31.39, -48.60);
/// HQ's H3 over Standard's at the same point.
const HQ_H3_DELTA_DB: f64 = 15.18;

/// Per-bias pins: the bias knob is tape bias in HQ, and the loop's
/// distortion falls as it rises.
#[test]
fn hq_bias_sets_the_loop_distortion() {
    let mut last = f64::INFINITY;
    for (bias, thd) in HQ_BIAS_PINS {
        let sig = probe(&hq(0.35, bias), PROBE_LEVEL_DBFS);
        show(&format!("HQ drive 0.35 bias {bias}"), &sig);
        assert!(sig.thd_pct < last, "bias {bias}: THD {:.3} % did not fall", sig.thd_pct);
        assert!((sig.thd_pct / thd - 1.0).abs() < 0.02, "bias {bias}: THD {:.4} %, pinned {thd}", sig.thd_pct);
        last = sig.thd_pct;
    }
}

/// `(bias, THD %)` at 35 % drive (the default).
const HQ_BIAS_PINS: [(f32, f64); 3] = [(0.0, 4.488), (0.5, 2.339), (1.0, 0.5499)];

#[test]
fn hq_thd_rises_monotonically_with_drive() {
    let mut last = -1.0;
    for i in 0..=8 {
        let drive = i as f32 / 8.0;
        let thd = probe(&hq(drive, 0.5), PROBE_LEVEL_DBFS).thd_pct;
        assert!(thd > last, "HQ: THD fell to {thd:.4} % at drive {drive}");
        last = thd;
    }
    assert!(last > 10.0, "HQ cannot reach track levels: {last:.2} %");
    let clean = probe(&hq(0.0, 1.0), PROBE_LEVEL_DBFS).thd_pct;
    assert!(clean < 1.0, "HQ over-biased at drive 0 is still {clean:.2} % THD");
}

/// The level stays put: HQ's normalisation divides by the loop's
/// anhysteretic slope, so the probe's fundamental gain is near unity at
/// low and moderate drive, like Standard's.
#[test]
fn hq_level_is_normalised_across_drive() {
    for drive in [0.0, 0.25, 0.5] {
        for bias in [0.0, 0.5, 1.0] {
            let sig = probe(&hq(drive, bias), PROBE_LEVEL_DBFS);
            assert!(sig.gain_db.abs() < 2.0, "drive {drive} bias {bias}: gain {:+.2} dB", sig.gain_db);
        }
    }
}

/// The three solvers draw the same loop at the default 2x.
#[test]
fn every_solver_gives_the_same_signature() {
    let base = probe(&hq(0.5, 0.5), PROBE_LEVEL_DBFS);
    for solver in HysteresisSolver::ALL {
        let sig = probe(&Settings { tape_solver: solver, ..hq(0.5, 0.5) }, PROBE_LEVEL_DBFS);
        show(&format!("HQ {solver:?}"), &sig);
        assert!((sig.thd_pct / base.thd_pct - 1.0).abs() < 0.02, "{solver:?}: THD {}", sig.thd_pct);
        assert!((sig.h_dbc[3] - base.h_dbc[3]).abs() < 0.2, "{solver:?}: H3 {}", sig.h_dbc[3]);
    }
}

// ---------------------------------------------------------------------------
// Oversampling and latency
// ---------------------------------------------------------------------------

/// HQ runs at max(user factor, 2x): Off renders exactly what 2x does,
/// and 4x stays 4x.
#[test]
fn hq_forces_at_least_2x() {
    assert_eq!(
        Settings { oversample: OversampleFactor::Off, ..hq(0.5, 0.5) }.stage_factor(),
        OversampleFactor::X2
    );
    assert_eq!(
        Settings { oversample: OversampleFactor::X4, ..hq(0.5, 0.5) }.stage_factor(),
        OversampleFactor::X4
    );
    assert_eq!(
        Settings { oversample: OversampleFactor::Off, ..standard(0.5, 0.5) }.stage_factor(),
        OversampleFactor::Off
    );
    let (l, r) = pink_noise(9_600, -12.0, 5);
    let off = render(&Settings { oversample: OversampleFactor::Off, ..hq(0.6, 0.4) }, &l, &r);
    let x2 = render(&Settings { oversample: OversampleFactor::X2, ..hq(0.6, 0.4) }, &l, &r);
    assert_eq!(off, x2, "HQ at Off must run the 2x stage");
    let x4 = render(&Settings { oversample: OversampleFactor::X4, ..hq(0.6, 0.4) }, &l, &r);
    assert_ne!(x4, x2);
}

#[test]
fn hq_reports_no_latency() {
    for factor in factors() {
        let mut plugin = ResonanceColor::new();
        apply_settings(&plugin.params, &Settings { oversample: factor, ..hq(0.8, 0.5) });
        plugin.initialize(SR, BLOCK as u32);
        assert_eq!(plugin.latency_samples(), 0, "{factor:?}");
    }
}

/// Switching quality (and solver) mid-stream is finite and audible, and
/// switching back renders Standard again.
#[test]
fn switching_quality_mid_stream_is_clean() {
    let (l, r) = pink_noise(48_000, -12.0, 17);
    let mut plugin = ResonanceColor::new();
    apply_settings(&plugin.params, &standard(0.6, 0.5));
    plugin.initialize(SR, BLOCK as u32);
    let mut out = Vec::new();
    for (i, chunk) in (0..l.len()).step_by(4_800).enumerate() {
        let q = if i % 2 == 1 { TapeQuality::Hq } else { TapeQuality::Standard };
        plugin.params.tape_quality.set_plain(q as i32 as f64);
        plugin.params.tape_solver.set_plain((i % 3) as f64);
        let end = (chunk + 4_800).min(l.len());
        let (ol, or) = render_with(&mut plugin, &l[chunk..end], &r[chunk..end]);
        out.extend(ol.into_iter().chain(or));
    }
    assert!(out.iter().all(|v| v.is_finite()));
    let peak = out.iter().fold(0.0f32, |m, v| m.max(v.abs()));
    assert!(peak > 0.05 && peak < 4.0, "peak {peak}");
}

/// The editor's curve in HQ is the loop's centre line: odd, monotone,
/// unity slope at the origin, compressing toward full scale.
#[test]
fn hq_transfer_curve_is_the_loop_centre_line() {
    let s = hq(0.6, 0.5);
    let f = |x: f32| transfer_for(&s, x);
    assert_eq!(f(0.0), 0.0);
    assert!(((f(1e-3) / 1e-3) - 1.0).abs() < 0.01, "slope {}", f(1e-3) / 1e-3);
    let mut last = f(-1.0);
    for i in 1..=200 {
        let x = -1.0 + i as f32 / 100.0;
        assert!(f(x) > last, "not monotone at {x}");
        assert!((f(-x) + f(x)).abs() < 1e-6, "not odd at {x}");
        last = f(x);
    }
    assert!(f(1.0) < 1.0, "no compression at full scale");
    // Standard Tape still draws its own curve.
    let st = standard(0.6, 0.5);
    assert_ne!(transfer_for(&st, 0.5), f(0.5));
}

// ---------------------------------------------------------------------------
// Auto-gain
// ---------------------------------------------------------------------------

fn lufs(l: &[f32], r: &[f32]) -> f32 {
    let skip = SR as usize;
    LufsMeter::analyze_offline(SR, &l[skip..], &r[skip..]).integrated
}

/// HQ keeps auto-gain's promise: ±0.5 LU on pink noise and a drum loop,
/// across the drive range and at both ends of the bias knob.
#[test]
fn hq_auto_gain_holds_half_an_lu() {
    let n = (9.0 * SR) as usize;
    for (name, (l, r)) in [("pink", pink_noise(n, -18.0, 11)), ("drums", drum_loop(n, -6.0, 23))] {
        let want = lufs(&l, &r);
        for drive in [0.0, 0.35, 0.7, 1.0] {
            for bias in [0.0, 1.0] {
                let s = Settings { mix: 1.0, auto_gain: true, ..hq(drive, bias) };
                let (ol, or) = render(&s, &l, &r);
                let delta = lufs(&ol, &or) - want;
                eprintln!("HQ {name:6} drive {drive:.2} bias {bias}: Δ {delta:+.2} LU");
                assert!(delta.abs() <= 0.5, "HQ {name} drive {drive} bias {bias}: Δ {delta:+.2} LU");
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Stability
// ---------------------------------------------------------------------------

const SAMPLE_RATES: [f32; 6] = [44_100.0, 48_000.0, 88_200.0, 96_000.0, 176_400.0, 192_000.0];

fn render_at(sr: f32, s: &Settings, l: &[f32], r: &[f32]) -> (Vec<f32>, Vec<f32>) {
    let mut plugin = ResonanceColor::new();
    apply_settings(&plugin.params, s);
    plugin.initialize(sr, BLOCK as u32);
    render_with(&mut plugin, l, r)
}

/// Max drive on full-scale white noise, every other control at an end
/// of its range, at every rate, solver and oversampling setting: finite,
/// bounded, and still moving with the input (not stuck on a rail).
#[test]
fn no_nan_or_instability_at_max_drive_on_full_scale_noise_at_every_rate() {
    let (l, r) = white_noise(9_600, 1.0, 3);
    for sr in SAMPLE_RATES {
        for solver in HysteresisSolver::ALL {
            for factor in factors() {
                for (bias, sign) in [(0.0, 1.0), (1.0, -1.0)] {
                    let s = Settings {
                        mode: Mode::Tape,
                        drive: 1.0,
                        bias,
                        response_db: 12.0 * sign,
                        tone_db: 6.0 * sign,
                        mix: 1.0,
                        auto_gain: sign > 0.0,
                        output_db: 12.0,
                        oversample: factor,
                        speed_ips: if sign > 0.0 { 30.0 } else { 7.5 },
                        flutter: 1.0,
                        tape_quality: TapeQuality::Hq,
                        tape_solver: solver,
                    };
                    let (ol, or) = render_at(sr, &s, &l, &r);
                    let tag = format!("{sr} Hz {solver:?} {factor:?} bias {bias}");
                    assert!(ol.iter().chain(&or).all(|v| v.is_finite()), "{tag}: non-finite");
                    let peak = ol.iter().chain(&or).fold(0.0f32, |m, v| m.max(v.abs()));
                    assert!(peak < 100.0, "{tag}: runaway peak {peak}");
                    let tail = &ol[ol.len() / 2..];
                    let rms = (tail.iter().map(|v| v * v).sum::<f32>() / tail.len() as f32).sqrt();
                    assert!(rms > 0.01, "{tag}: output died (rms {rms})");
                }
            }
        }
    }
}

/// Hostile input into HQ: NaN, ±inf and huge values read as silence or
/// clamp, and the stage recovers.
#[test]
fn hq_survives_hostile_input() {
    for solver in HysteresisSolver::ALL {
        let s = Settings { tape_solver: solver, ..hq(1.0, 0.0) };
        let mut dsp = ColorDsp::new(SR, &s);
        let mut l = vec![0.0f32; 512];
        let mut r = vec![0.0f32; 512];
        for (i, v) in l.iter_mut().enumerate() {
            *v = match i % 5 {
                0 => f32::NAN,
                1 => f32::INFINITY,
                2 => -1.0e30,
                3 => 1.0e30,
                _ => 0.5,
            };
        }
        dsp.process(&mut l, &mut r, &s, None);
        assert!(l.iter().chain(&r).all(|v| v.is_finite()), "{solver:?}");
        let (sl, sr) = sine(4_800, 440.0, 0.5);
        let (mut a, mut b) = (sl.clone(), sr.clone());
        dsp.process(&mut a, &mut b, &s, None);
        let tail = &a[2_400..];
        let peak = tail.iter().fold(0.0f32, |m, v| m.max(v.abs()));
        assert!(peak > 0.05 && peak < 10.0, "{solver:?}: did not recover (peak {peak})");
    }
}

// ---------------------------------------------------------------------------
// Golden
// ---------------------------------------------------------------------------

const G_MAX_BLOCK: usize = 320;
const G_BLOCKS: usize = 24;

fn golden_path() -> PathBuf {
    golden::golden_path(env!("CARGO_MANIFEST_DIR"), "tape_hq.f32")
}

fn blessing() -> bool {
    golden::blessed(&["RESONANCE_BLESS", "RESONANCE_BLESS_TAPE_HQ"])
}

/// A bass note and a mid tone under decaying noise hits (the
/// `dsp_golden` program signal).
fn program(n0: u64, l: &mut [f32], r: &mut [f32], rng: &mut SimpleRng) {
    for i in 0..l.len() {
        let t = (n0 + i as u64) as f32 / SR;
        let hit = (-30.0 * (t % 0.125)).exp();
        let u = (rng.next_u32() >> 8) as f32 / (1u32 << 24) as f32;
        let v = 0.3 * (TAU * 110.0 * t).sin() + 0.15 * (TAU * 1_300.0 * t).sin() + 0.4 * hit * (2.0 * u - 1.0);
        l[i] = v;
        r[i] = 0.8 * v + 0.1 * (TAU * 330.0 * t).sin();
    }
}

type Edit = fn(&ColorParams, usize);

struct Scenario {
    name: &'static str,
    blocks: &'static [usize],
    settings: Settings,
    edit: Option<Edit>,
}

/// Every scenario is a full `Settings` literal: nothing rides a default.
fn scenarios() -> Vec<Scenario> {
    vec![
        // 1. The default solver at the default 2x, on a bus-level drive.
        Scenario {
            name: "hq_rk4_2x",
            blocks: &[256],
            settings: Settings {
                mode: Mode::Tape,
                drive: 0.6,
                bias: 0.4,
                response_db: 0.0,
                tone_db: -1.0,
                mix: 1.0,
                auto_gain: true,
                output_db: 0.0,
                oversample: OversampleFactor::X2,
                speed_ips: 15.0,
                flutter: 0.0,
                tape_quality: TapeQuality::Hq,
                tape_solver: HysteresisSolver::Rk4,
            },
            edit: None,
        },
        // 2. Oversampling Off, forced up to 2x; NR; parallel, no
        //    auto-gain, 30 ips, under-biased and hot.
        Scenario {
            name: "hq_nr_forced_2x_parallel",
            blocks: &[64, 197, 320],
            settings: Settings {
                mode: Mode::Tape,
                drive: 0.8,
                bias: 0.1,
                response_db: -3.0,
                tone_db: 0.0,
                mix: 0.7,
                auto_gain: false,
                output_db: -3.0,
                oversample: OversampleFactor::Off,
                speed_ips: 30.0,
                flutter: 0.0,
                tape_quality: TapeQuality::Hq,
                tape_solver: HysteresisSolver::NewtonRaphson,
            },
            edit: None,
        },
        // 3. RK2 at 4x with flutter and a low-weighted drive, 7.5 ips.
        Scenario {
            name: "hq_rk2_4x_flutter",
            blocks: &[128, 96],
            settings: Settings {
                mode: Mode::Tape,
                drive: 0.45,
                bias: 0.8,
                response_db: 4.0,
                tone_db: 1.5,
                mix: 1.0,
                auto_gain: true,
                output_db: 0.0,
                oversample: OversampleFactor::X4,
                speed_ips: 7.5,
                flutter: 0.4,
                tape_quality: TapeQuality::Hq,
                tape_solver: HysteresisSolver::Rk2,
            },
            edit: None,
        },
        // 4. Standard → HQ → solver change → back, with drive and bias
        //    riding the smoothers across the switches.
        Scenario {
            name: "hq_switches_between_blocks",
            blocks: &[192, 320],
            settings: Settings {
                mode: Mode::Tape,
                drive: 0.3,
                bias: 0.5,
                response_db: 0.0,
                tone_db: 0.0,
                mix: 1.0,
                auto_gain: true,
                output_db: 0.0,
                oversample: OversampleFactor::Off,
                speed_ips: 15.0,
                flutter: 0.0,
                tape_quality: TapeQuality::Standard,
                tape_solver: HysteresisSolver::Rk4,
            },
            edit: Some(|p, block| {
                let t = block as f32 / G_BLOCKS as f32;
                p.drive.set_value(0.3 + 0.6 * t);
                p.bias.set_value(1.0 - t);
                if block == 6 {
                    p.tape_quality.set_plain(TapeQuality::Hq as i32 as f64);
                }
                if block == 12 {
                    p.tape_solver.set_plain(HysteresisSolver::NewtonRaphson as i32 as f64);
                    p.oversample.set_plain(OversampleFactor::X4 as i32 as f64);
                }
                if block == 18 {
                    p.tape_quality.set_plain(TapeQuality::Standard as i32 as f64);
                }
            }),
        },
    ]
}

fn render_scenario(s: &Scenario, dry_path: bool) -> Vec<f32> {
    let mut plugin = ResonanceColor::new();
    apply_settings(&plugin.params, &s.settings);
    if dry_path {
        plugin.params.mix.set_value(0.0);
    }
    plugin.initialize(SR, G_MAX_BLOCK as u32);
    plugin.reset();
    let mut rng = SimpleRng::new(0xC0102);
    let mut out = Vec::new();
    let mut left = vec![0.0f32; G_MAX_BLOCK];
    let mut right = vec![0.0f32; G_MAX_BLOCK];
    let mut n = 0u64;
    for block in 0..G_BLOCKS {
        if let Some(edit) = s.edit {
            edit(&plugin.params, block);
            if dry_path {
                plugin.params.mix.set_value(0.0);
            }
        }
        let frames = s.blocks[block % s.blocks.len()];
        program(n, &mut left[..frames], &mut right[..frames], &mut rng);
        let mut outs = [OutputBuffer {
            left: &mut left[..frames],
            right: &mut right[..frames],
        }];
        plugin.process(&mut outs, frames, &mut EventIterator::empty(), None);
        n += frames as u64;
        out.extend_from_slice(&left[..frames]);
        out.extend_from_slice(&right[..frames]);
    }
    out
}

#[test]
fn hq_output_is_bit_exact() {
    let rendered: Vec<f32> = scenarios().iter().flat_map(|s| render_scenario(s, false)).collect();
    assert!(rendered.iter().all(|s| s.is_finite()), "non-finite samples");
    let path = golden_path();
    if blessing() {
        golden::bless_f32(&path, &rendered);
        return;
    }
    let want = golden::load_golden_f32(&path, rendered.len(), "RESONANCE_BLESS=1");
    let diff = golden::compare_f32(&rendered, &want);
    if let Some((i, got, want)) = diff.first_diff {
        panic!(
            "Tape HQ output changed: {}/{} samples differ, peak delta {:.3e}; first at \
             sample {i} (got {got:?}, want {want:?}). If intended, re-bless with \
             RESONANCE_BLESS=1.",
            diff.diff_count,
            rendered.len(),
            diff.max_abs,
        );
    }
}

/// Every HQ scenario renders real audio that is not a plain gain of its
/// own dry path.
#[test]
fn every_hq_scenario_is_audible_and_not_a_bypass() {
    for s in scenarios() {
        let out = render_scenario(&s, false);
        let dry = render_scenario(&s, true);
        let peak = out.iter().fold(0.0f32, |m, v| m.max(v.abs()));
        assert!(peak > 0.05, "scenario `{}` rendered (near) silence: peak {peak}", s.name);
        let resid = golden::residual_fraction(&out, &dry);
        eprintln!("{:28} peak {peak:.3} residual {resid:.2e}", s.name);
        assert!(resid > 1e-4, "scenario `{}` is a plain gain of its input ({resid:.2e})", s.name);
    }
}

// ---------------------------------------------------------------------------
// CPU
// ---------------------------------------------------------------------------

/// Realtime factor of HQ against Standard, per solver and factor, on
/// dense program material (pink noise at −12 dBFS RMS). Run by hand in
/// release:
///
///     cargo test --release -p resonance-color --test tape_hq -- --ignored --nocapture
///
/// The bounds are loose, so they catch a regression by a multiple, not a
/// percent: HQ at any solver and factor costs at most 10x Standard at 2x
/// and under a quarter of a core per stereo instance. Measured on the
/// canonical machine (release, under load): Standard ≈ 1 % of a core;
/// HQ RK2 / RK4 / NR ≈ 2.2x / 3.1x / 4.1x that at 2x and 3.3x / 5.2x /
/// 7.0x at 4x.
#[test]
#[ignore]
fn cpu_sanity() {
    let seconds = 10.0;
    let n = (seconds * SR) as usize;
    let (l, r) = pink_noise(n, -12.0, 99);
    // Best of three runs: the minimum is the cost, the rest is whatever
    // else the machine was doing.
    let time = |s: &Settings| {
        (0..3)
            .map(|_| {
                let mut plugin = ResonanceColor::new();
                apply_settings(&plugin.params, s);
                plugin.initialize(SR, BLOCK as u32);
                let t0 = std::time::Instant::now();
                let out = render_with(&mut plugin, &l, &r);
                let dt = t0.elapsed().as_secs_f64();
                std::hint::black_box(out);
                dt / seconds as f64
            })
            .fold(f64::INFINITY, f64::min)
    };
    let base = time(&standard(0.6, 0.5));
    eprintln!("Standard 2x: {:.4} of realtime", base);
    for factor in [OversampleFactor::X2, OversampleFactor::X4] {
        for solver in HysteresisSolver::ALL {
            let load = time(&Settings { oversample: factor, tape_solver: solver, ..hq(0.6, 0.5) });
            eprintln!("HQ {factor:?} {solver:?}: {load:.4} of realtime ({:.1}x Standard)", load / base);
            assert!(load < 10.0 * base, "HQ {factor:?} {solver:?} costs {:.1}x Standard", load / base);
            assert!(load < 0.25, "HQ {factor:?} {solver:?} costs {load:.3} of a core");
        }
    }
}
