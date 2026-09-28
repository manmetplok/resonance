//! Pinned harmonic signatures per mode (warmth-width-depth.md §11):
//! "harmonic assertions replace 'sounds warm'".
//!
//! Measured with the crate's own probe — a 1 kHz sine at −18 dBFS peak,
//! §2.1's stimulus — at a fixed drive of 50 % and bias of 50 %, every
//! other setting at its default. Two kinds of assertion per mode:
//!
//! - **structure** that is the mode's reason to exist and must survive
//!   any re-voicing: Warm has no odd harmonics, Console no even ones,
//!   Tube is even-dominant, the series falls with order, Transformer
//!   distorts bass harder than mids;
//! - **pinned numbers** (H2..H5 in dBc, THD), so a voicing change is a
//!   decision someone makes on purpose and not a drift nobody saw.
//!
//! And the §11 aliasing floor: a 5 kHz sine at 0 dBFS with +12 dB of
//! drive stays ≤ −90 dBc at 4× for the memoryless voicings.

use resonance_color::dsp::voicing::{DRIVE_MAX_DB, DRIVE_MIN_DB};
use resonance_color::dsp::Settings;
use resonance_color::params::Mode;
use resonance_color::probe::{probe, render_tone, HarmonicSignature, PROBE_LEVEL_DBFS};
use resonance_dsp::OversampleFactor;
use rustfft::{num_complex::Complex, FftPlanner};

const DRIVE: f32 = 0.5;
const BIAS: f32 = 0.5;

fn signature(mode: Mode) -> HarmonicSignature {
    let s = Settings {
        mode,
        drive: DRIVE,
        bias: BIAS,
        ..Settings::default()
    };
    let sig = probe(&s, PROBE_LEVEL_DBFS);
    eprintln!(
        "{:12} THD {:.4} %  H2 {:.2}  H3 {:.2}  H4 {:.2}  H5 {:.2}  H6 {:.2}  H7 {:.2}",
        mode.label(),
        sig.thd_pct,
        sig.h_dbc[2],
        sig.h_dbc[3],
        sig.h_dbc[4],
        sig.h_dbc[5],
        sig.h_dbc[6],
        sig.h_dbc[7]
    );
    sig
}

/// `(thd_pct, [H2, H3, H4, H5] dBc)`; `None` for an order that must be
/// absent (checked structurally instead).
type Pin = (f64, [Option<f64>; 4]);

fn assert_pinned(mode: Mode, sig: &HarmonicSignature, pin: Pin) {
    let (thd, h) = pin;
    assert!(
        (sig.thd_pct / thd - 1.0).abs() < 0.02,
        "{}: THD {:.4} %, pinned {thd} %",
        mode.label(),
        sig.thd_pct
    );
    for (i, want) in h.iter().enumerate() {
        let k = i + 2;
        if let Some(want) = want {
            assert!(
                (sig.h_dbc[k] - want).abs() < 0.3,
                "{}: H{k} {:.2} dBc, pinned {want}",
                mode.label(),
                sig.h_dbc[k]
            );
        }
    }
}

fn falls_by_at_least(sig: &HarmonicSignature, orders: &[usize], db_per_order: f64) {
    let slope = sig
        .decay_db_per_order(orders, -140.0)
        .expect("two harmonics above the floor");
    assert!(slope <= -db_per_order, "series falls only {slope:.1} dB/order over {orders:?}");
}

#[test]
fn tube_is_even_dominant_and_falls_off() {
    let sig = signature(Mode::Tube);
    assert!(sig.h2_h3_db() > 20.0, "Tube H2−H3 {:.1} dB", sig.h2_h3_db());
    falls_by_at_least(&sig, &[2, 3, 4, 5], 6.0);
    assert_pinned(Mode::Tube, &sig, (5.672, [Some(-24.93), Some(-54.30), Some(-67.87), Some(-102.3)]));
}

#[test]
fn tape_is_a_soft_low_order_curve() {
    let sig = signature(Mode::Tape);
    // A little asymmetry: H2 leads at moderate drive, H3 close behind.
    assert!(sig.h2_h3_db() > 0.0 && sig.h2_h3_db() < 20.0, "Tape H2−H3 {:.1} dB", sig.h2_h3_db());
    falls_by_at_least(&sig, &[2, 3, 4, 5], 6.0);
    assert_pinned(Mode::Tape, &sig, (2.167, [Some(-33.49), Some(-46.60), Some(-73.60), Some(-92.2)]));
}

#[test]
fn transformer_is_a_soft_low_order_curve() {
    let sig = signature(Mode::Transformer);
    falls_by_at_least(&sig, &[2, 3, 4, 5], 6.0);
    assert_pinned(
        Mode::Transformer,
        &sig,
        (2.180, [Some(-33.42), Some(-46.50), Some(-73.40), Some(-91.5)]),
    );
}

/// The transformer's point: flux ∝ V/f, so bass saturates first. The
/// same tone an octave-and-a-bit below its LF corner comes out far
/// dirtier than at 1 kHz; Tape, the same curve without the LF
/// weighting, does not show the gap.
#[test]
fn transformer_distorts_bass_harder_than_mids() {
    let thd_at = |mode: Mode, freq: f64| {
        let s = resonance_color::probe::probe_settings(&Settings {
            mode,
            drive: DRIVE,
            bias: BIAS,
            ..Settings::default()
        });
        // 60 Hz: 800 samples per cycle; 0.5 s window, coherent.
        let x = render_tone(&s, 48_000.0, freq, PROBE_LEVEL_DBFS, 24_000, 24_000);
        let a = |f: f64| resonance_color::probe::bin_amplitude(&x, 48_000.0, f);
        let fund = a(freq);
        let harm: f64 = (2..=9).map(|k| a(freq * k as f64).powi(2)).sum::<f64>().sqrt();
        100.0 * harm / fund
    };
    let xfmr_ratio = thd_at(Mode::Transformer, 60.0) / thd_at(Mode::Transformer, 1_000.0);
    let tape_ratio = thd_at(Mode::Tape, 60.0) / thd_at(Mode::Tape, 1_000.0);
    assert!(xfmr_ratio > 2.0, "Transformer 60 Hz / 1 kHz THD ratio {xfmr_ratio:.2}");
    assert!(xfmr_ratio > 1.8 * tape_ratio, "LF weighting: {xfmr_ratio:.2} vs Tape {tape_ratio:.2}");
}

#[test]
fn console_is_odd_only() {
    let sig = signature(Mode::Console);
    for k in [2, 4, 6, 8] {
        assert!(sig.h_dbc[k] < -120.0, "Console H{k} {:.1} dBc", sig.h_dbc[k]);
    }
    falls_by_at_least(&sig, &[3, 5], 6.0);
    assert_pinned(Mode::Console, &sig, (0.2653, [None, Some(-51.53), None, Some(-113.6)]));
}

#[test]
fn warm_is_even_only() {
    let sig = signature(Mode::Warm);
    for k in [3, 5, 7, 9] {
        assert!(sig.h_dbc[k] < -120.0, "Warm H{k} {:.1} dBc", sig.h_dbc[k]);
    }
    falls_by_at_least(&sig, &[2, 4, 6], 6.0);
    assert_pinned(Mode::Warm, &sig, (1.545, [Some(-36.23), None, Some(-84.60), None]));
}

/// More drive is more distortion, in every mode, over the whole knob.
#[test]
fn thd_rises_monotonically_with_drive() {
    for mode in Mode::ALL {
        let mut last = -1.0;
        for i in 0..=8 {
            let s = Settings {
                mode,
                drive: i as f32 / 8.0,
                bias: BIAS,
                ..Settings::default()
            };
            let thd = probe(&s, PROBE_LEVEL_DBFS).thd_pct;
            assert!(thd > last, "{}: THD fell to {thd:.4} % at drive {}", mode.label(), i as f32 / 8.0);
            last = thd;
        }
    }
}

/// The THD span every §2.1 placement needs is reachable: master
/// (0.1–1 %) and bus (0.5–3 %) in every gain-driven mode, and track
/// (3–10 %) in Tube, Tape and Transformer. Console tops out near 1 %,
/// by design ("very low drive").
#[test]
fn the_drive_range_covers_the_placement_targets() {
    let thd = |mode, drive| {
        probe(
            &Settings {
                mode,
                drive,
                bias: BIAS,
                ..Settings::default()
            },
            PROBE_LEVEL_DBFS,
        )
        .thd_pct
    };
    for mode in [Mode::Tape, Mode::Transformer, Mode::Warm] {
        assert!(thd(mode, 0.0) < 0.5, "{} cannot get down to master levels", mode.label());
    }
    for mode in [Mode::Tube, Mode::Tape, Mode::Transformer] {
        assert!(thd(mode, 1.0) > 10.0, "{} cannot reach track levels", mode.label());
    }
    assert!(thd(Mode::Warm, 1.0) > 3.0);
    assert!(thd(Mode::Console, 1.0) > 0.5 && thd(Mode::Console, 1.0) < 2.0);
}

/// Strongest non-harmonic bin relative to the fundamental.
fn alias_floor_dbc(mode: Mode, factor: OversampleFactor) -> f64 {
    const N: usize = 9_600; // 5 Hz bins: 5 kHz and all its aliases are coherent
    // +12 dB of curve drive: the drive knob position that maps to it.
    let drive = (12.0 - DRIVE_MIN_DB) / (DRIVE_MAX_DB - DRIVE_MIN_DB);
    let s = resonance_color::probe::probe_settings(&Settings {
        mode,
        drive,
        bias: BIAS,
        oversample: factor,
        ..Settings::default()
    });
    // Half a second of settle: the DC blocker after the asymmetric curves
    // (5 Hz) must have swallowed the start-up DC step, or its decaying
    // tail leaks into the low bins.
    let x = render_tone(&s, 48_000.0, 5_000.0, 0.0, 24_000, N);
    let mut buf: Vec<Complex<f64>> = x.iter().map(|&v| Complex::new(v as f64, 0.0)).collect();
    FftPlanner::new().plan_fft_forward(N).process(&mut buf);
    let spec: Vec<f64> = buf[..=N / 2].iter().map(|c| c.norm()).collect();
    let f0 = 1_000; // bin of 5 kHz
    let fund = spec[f0];
    let mut worst = 0.0f64;
    // From 500 Hz up: every alias of a 5 kHz tone at 48 kHz lands on a
    // multiple of 1 kHz, so nothing below is an alias, only the tail of
    // the DC blocker's settling.
    for (b, &a) in spec.iter().enumerate().skip(100) {
        let r = b % f0;
        if r <= 2 || r + 2 >= f0 {
            continue;
        }
        worst = worst.max(a);
    }
    20.0 * (worst / fund).log10()
}

#[test]
fn aliasing_floor_at_4x_is_below_minus_90_dbc() {
    for mode in [Mode::Tube, Mode::Tape, Mode::Transformer, Mode::Console, Mode::Warm] {
        let off = alias_floor_dbc(mode, OversampleFactor::Off);
        let x4 = alias_floor_dbc(mode, OversampleFactor::X4);
        eprintln!("{:12} alias floor: Off {off:.1} dBc, 4x {x4:.1} dBc", mode.label());
        assert!(x4 <= -90.0, "{} at 4x: alias floor {x4:.1} dBc", mode.label());
        assert!(x4 < off, "{}: 4x ({x4:.1}) is no cleaner than Off ({off:.1})", mode.label());
    }
}

/// The editor's worker probes on one reused `Prober` (its DSP reset per
/// probe) instead of building a fresh DSP each time. Across a run of
/// unlike settings — every mode, HQ in and out, every factor — each
/// probe's window must be the fresh render's, bit for bit.
#[test]
fn a_reused_prober_renders_exactly_what_a_fresh_dsp_does() {
    use resonance_color::params::TapeQuality;
    use resonance_color::probe::{probe_settings, Prober, SETTLE_SAMPLES, WINDOW_SAMPLES};
    let mut prober = Prober::new();
    let mut run = Vec::new();
    for (k, mode) in Mode::ALL.into_iter().enumerate() {
        for factor in [OversampleFactor::Off, OversampleFactor::X4, OversampleFactor::X2] {
            run.push(Settings {
                mode,
                drive: 0.3 + 0.1 * k as f32,
                bias: 0.8 - 0.1 * k as f32,
                tone_db: 1.5 - k as f32,
                response_db: -3.0 + 2.0 * k as f32,
                mix: if k % 2 == 0 { 1.0 } else { 0.6 },
                oversample: factor,
                flutter: 0.4,
                auto_gain: true,
                tape_quality: if k == 1 && factor != OversampleFactor::X2 {
                    TapeQuality::Hq
                } else {
                    TapeQuality::Standard
                },
                ..Settings::default()
            });
        }
    }
    for s in &run {
        let sig = prober.probe(s, PROBE_LEVEL_DBFS);
        let fresh = render_tone(
            &probe_settings(s),
            48_000.0,
            1_000.0,
            PROBE_LEVEL_DBFS,
            SETTLE_SAMPLES,
            WINDOW_SAMPLES,
        );
        let same = prober.window().iter().zip(&fresh).all(|(a, b)| a.to_bits() == b.to_bits());
        assert!(
            same && prober.window().len() == fresh.len(),
            "reused prober diverged from a fresh DSP for {s:?}"
        );
        assert_eq!(sig, probe(s, PROBE_LEVEL_DBFS));
    }
}
