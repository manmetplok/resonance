//! Behaviour of the width tool (warmth-width-depth.md §6.2 / W7 exit
//! criteria): transparent defaults, the Decorrelate mono-sum invariant,
//! the mono-maker's low-band correlation, width, balance, rotation, the
//! auditions, the Haas mono-risk flag, and the latency decision.

mod common;

use common::*;
use resonance_plugin::{Param, ResonancePlugin};
use resonance_stereo::dsp::{
    haas_delay_ms, micro_shift_notch_db, MonoSlope, WidenMode, HAAS_LEVEL_DB,
};
use resonance_stereo::params::{index, StereoParams, PARAM_COUNT};
use resonance_stereo::ResonanceStereo;

/// A wide test mix: a centred tone, partly-correlated noise, and a
/// slightly off-centre bass.
fn mix(n: usize) -> (Vec<f32>, Vec<f32>) {
    let tone = sine(440.0, 0.3, n);
    let bass = sine(55.0, 0.3, n);
    let a = noise(n, 0.3, 1);
    let b = noise(n, 0.3, 2);
    let l = (0..n).map(|i| tone[i] + bass[i] + a[i] + 0.4 * b[i]).collect();
    let r = (0..n).map(|i| tone[i] + 0.8 * bass[i] + b[i] + 0.4 * a[i]).collect();
    (l, r)
}

// ---------------------------------------------------------------------------
// Transparent defaults
// ---------------------------------------------------------------------------

#[test]
fn the_defaults_are_a_bit_exact_passthrough() {
    let (l, r) = mix(24_000);
    let mono = noise(24_000, 0.7, 5);
    let hard = vec![0.0f32; 24_000];
    for (name, a, b) in [("mix", &l, &r), ("mono", &mono, &mono), ("hard left", &mono, &hard)] {
        for blocks in [&[256usize][..], &[1, 63, 512, 100]] {
            let (ol, or, _) = render_with(|_| {}, a, b, blocks);
            for i in 0..a.len() {
                assert_eq!(
                    (ol[i].to_bits(), or[i].to_bits()),
                    (a[i].to_bits(), b[i].to_bits()),
                    "{name}: sample {i} changed at the default settings"
                );
            }
        }
    }
}

#[test]
fn the_declared_defaults_are_the_transparent_values() {
    let p = StereoParams::default();
    assert_eq!(p.width.value(), 1.0);
    assert!(p.mono_below.display(p.mono_below.get_plain()).contains("Off"));
    assert_eq!(p.widen_mode(), WidenMode::Off);
    assert_eq!(p.balance.value(), 0.0);
    assert_eq!(p.rotation.value(), 0.0);
    assert!(!p.solo_side.value() && !p.mono_check.value());
}

#[test]
fn turning_everything_back_to_default_returns_to_exact_passthrough() {
    // Engage every stage, run, then put the defaults back mid-stream: once
    // the 20 ms ramps settle the output is the input again, bit for bit.
    let (l, r) = mix(48_000);
    let mut plugin = ResonanceStereo::new();
    plugin.initialize(SR, MAX_BLOCK as u32);
    let p = plugin.params.clone();
    let (mut ol, mut or) = (l.clone(), r.clone());
    for (k, (cl, cr)) in ol.chunks_mut(256).zip(or.chunks_mut(256)).enumerate() {
        if k == 0 {
            p.width.set_value(1.7);
            p.mono_below.set_value(150.0);
            p.widen_mode.set_value(WidenMode::Decorrelate.index());
            p.balance.set_value(0.3);
            p.rotation.set_value(-20.0);
        }
        if k == 40 {
            for i in 0..PARAM_COUNT {
                let q = p.param_at(i);
                q.set_plain(q.default_plain());
            }
        }
        let frames = cl.len();
        let mut outs = [resonance_plugin::OutputBuffer { left: cl, right: cr }];
        plugin.process(&mut outs, frames, &mut resonance_plugin::EventIterator::empty(), None);
    }
    let settled = 45 * 256;
    assert_ne!(ol[..settled], l[..settled], "the engaged stages did nothing");
    assert_eq!(ol[settled..], l[settled..]);
    assert_eq!(or[settled..], r[settled..]);
}

// ---------------------------------------------------------------------------
// Decorrelate: the mono sum never changes
// ---------------------------------------------------------------------------

#[test]
fn decorrelate_leaves_the_mono_sum_unchanged() {
    let n = 24_000;
    let (ml, mr) = mix(n);
    let mono = noise(n, 0.8, 7);
    let tone = sine(1_000.0, 0.8, n);
    let cases: [(&str, &[f32], &[f32]); 3] =
        [("mix", &ml, &mr), ("mono noise", &mono, &mono), ("tone", &tone, &tone)];
    for (name, l, r) in cases {
        for amount in [0.2f32, 0.6, 1.0] {
            for (lo, hi) in [(20.0f32, 20_000.0f32), (150.0, 20_000.0), (300.0, 8_000.0)] {
                // Width and the mono-maker only touch the side, so they
                // keep the invariant too; they run here alongside.
                for (width, mono_below) in [(1.0f32, 0.0f32), (1.6, 120.0)] {
                    let (ol, or) = render(
                        |p| {
                            p.widen_mode.set_value(WidenMode::Decorrelate.index());
                            p.widen_amount.set_value(amount);
                            p.focus_low.set_value(lo);
                            p.focus_high.set_value(hi);
                            p.width.set_value(width);
                            p.mono_below.set_value(mono_below);
                        },
                        l,
                        r,
                    );
                    // Rounding is relative to the signal's own scale: the
                    // peak of anything in the path, inputs and outputs.
                    let peak = l
                        .iter()
                        .chain(r)
                        .chain(&ol)
                        .chain(&or)
                        .fold(0.0f32, |m, v| m.max(v.abs()));
                    let mut worst = 0.0f32;
                    for i in 0..n {
                        let err = ((ol[i] + or[i]) - (l[i] + r[i])).abs();
                        worst = worst.max(err / peak);
                    }
                    assert!(
                        worst <= 8.0 * f32::EPSILON,
                        "{name}, amount {amount}, focus {lo}..{hi}, width {width}: mono sum \
                         moved by {worst:e} of full scale"
                    );
                    // …and the widener actually did something.
                    let side: f64 = (0..n)
                        .map(|i| ((ol[i] - or[i]) - (l[i] - r[i])) as f64)
                        .map(|d| d * d)
                        .sum();
                    assert!(side > 1e-3 * energy(l), "{name}: no side was added");
                }
            }
        }
    }
}

#[test]
fn decorrelate_widens_a_mono_source_by_the_amount() {
    // For a mono input at amount a the velvet side makes the correlation
    // (1 − a²)/(1 + a²); measured above the focus, where it acts fully.
    let n = 96_000;
    let x = noise(n, 0.5, 11);
    for amount in [0.5f32, 1.0] {
        let (ol, or) = render(
            |p| {
                p.widen_mode.set_value(WidenMode::Decorrelate.index());
                p.widen_amount.set_value(amount);
                p.focus_low.set_value(150.0);
            },
            &x,
            &x,
        );
        let hl = band(&ol, 600.0, 4, false);
        let hr = band(&or, 600.0, 4, false);
        let r = correlation(&hl[4_800..], &hr[4_800..]);
        let want = ((1.0 - amount * amount) / (1.0 + amount * amount)) as f64;
        assert!((r - want).abs() < 0.08, "amount {amount}: correlation {r:.3}, want {want:.3}");
        // Below the focus the bass stays centred.
        let ll = band(&ol, 50.0, 4, true);
        let lr = band(&or, 50.0, 4, true);
        assert!(correlation(&ll[4_800..], &lr[4_800..]) > 0.97);
    }
}

// ---------------------------------------------------------------------------
// Mono-maker
// ---------------------------------------------------------------------------

/// Correlation of the band an octave below `corner` (8th-order low-pass
/// at `corner / 2`), skipping the first 100 ms.
fn low_band_correlation(l: &[f32], r: &[f32], corner: f32) -> f64 {
    let a = band(l, corner * 0.5, 4, true);
    let b = band(r, corner * 0.5, 4, true);
    correlation(&a[4_800..], &b[4_800..])
}

#[test]
fn the_mono_maker_folds_the_band_below_its_corner_to_mono() {
    // Wide, fully decorrelated noise: independent L and R, r ≈ 0.
    let n = 192_000;
    let l = noise(n, 0.5, 21);
    let r = noise(n, 0.5, 22);
    for corner in [80.0f32, 120.0, 200.0] {
        let before = low_band_correlation(&l, &r, corner);
        assert!(before.abs() < 0.1, "the input's low band is not decorrelated: {before}");

        let mut per_slope = Vec::new();
        for slope in [MonoSlope::Db6, MonoSlope::Db12, MonoSlope::Db24] {
            let (ol, or) = render(
                |p| {
                    p.mono_below.set_value(corner);
                    p.mono_slope.set_value(slope.index());
                },
                &l,
                &r,
            );
            per_slope.push(low_band_correlation(&ol, &or, corner));

            // The top stays as wide as it came in.
            let hl = band(&ol, corner * 8.0, 4, false);
            let hr = band(&or, corner * 8.0, 4, false);
            let top = correlation(&hl[4_800..], &hr[4_800..]);
            assert!(top.abs() < 0.1, "{slope:?} at {corner} Hz narrowed the top: r = {top:.3}");
        }
        assert!(
            per_slope[2] >= 0.99,
            "24 dB/oct at {corner} Hz: low-band correlation {:.4} < 0.99",
            per_slope[2]
        );
        assert!(
            per_slope[0] < per_slope[1] && per_slope[1] < per_slope[2],
            "steeper slopes must fold more: {per_slope:?}"
        );
        assert!(per_slope[0] > 0.9, "even 6 dB/oct folds most of it: {per_slope:?}");
    }
}

#[test]
fn the_mono_maker_leaves_the_mid_alone() {
    let (l, r) = mix(48_000);
    let (ol, or) = render(|p| p.mono_below.set_value(200.0), &l, &r);
    for i in 0..l.len() {
        let err = ((ol[i] + or[i]) - (l[i] + r[i])).abs();
        assert!(err <= 4.0 * f32::EPSILON * (l[i].abs() + r[i].abs()).max(1.0));
    }
}

// ---------------------------------------------------------------------------
// Width, balance, rotation
// ---------------------------------------------------------------------------

#[test]
fn width_zero_is_mono_and_width_two_doubles_the_side() {
    let (l, r) = mix(48_000);
    let (ml, mr) = render(|p| p.width.set_value(0.0), &l, &r);
    assert_eq!(ml, mr, "width 0 must be exactly mono");

    let before = side_mid_db(&l, &r);
    let (wl, wr) = render(|p| p.width.set_value(2.0), &l, &r);
    let after = side_mid_db(&wl, &wr);
    assert!(
        (after - before - 6.0206).abs() < 0.01,
        "width 200 % raised S/M by {:.3} dB, want +6.02",
        after - before
    );
    // The mid is untouched at any width.
    for i in 0..l.len() {
        assert!(((wl[i] + wr[i]) - (l[i] + r[i])).abs() <= 1e-6);
    }
}

#[test]
fn balance_fades_the_far_side_and_keeps_the_near_one() {
    let (l, r) = mix(4_800);
    let (bl, br) = render(|p| p.balance.set_value(1.0), &l, &r);
    assert!(bl.iter().all(|&v| v == 0.0), "balance hard right silences the left");
    assert_eq!(br, r, "…and leaves the right at unity");

    let (bl, br) = render(|p| p.balance.set_value(-0.5), &l, &r);
    assert_eq!(bl, l, "balance left keeps the left at unity");
    for i in 0..r.len() {
        assert_eq!(br[i], r[i] * 0.5);
    }
}

#[test]
fn rotation_turns_the_image_and_keeps_its_energy() {
    let x = noise(9_600, 0.5, 31);
    let (rl, rr) = render(|p| p.rotation.set_value(45.0), &x, &x);
    for i in 0..x.len() {
        assert!(rl[i].abs() <= 1e-6, "+45° puts a centred source hard right");
        assert!((rr[i] - x[i] * std::f32::consts::SQRT_2).abs() <= 1e-6);
    }
    let (ll, lr) = render(|p| p.rotation.set_value(-45.0), &x, &x);
    assert!(lr.iter().all(|v| v.abs() <= 1e-6), "−45° puts it hard left");
    assert!(energy(&ll) > 0.0);

    let (l, r) = mix(9_600);
    for deg in [-30.0f32, 10.0, 45.0] {
        let (ol, or) = render(|p| p.rotation.set_value(deg), &l, &r);
        let e_in = energy(&l) + energy(&r);
        let e_out = energy(&ol) + energy(&or);
        assert!(((e_out - e_in) / e_in).abs() < 1e-5, "{deg}°: energy moved");
    }
}

// ---------------------------------------------------------------------------
// Auditions
// ---------------------------------------------------------------------------

#[test]
fn solo_side_and_mono_check_audition_what_they_say() {
    let (l, r) = mix(4_800);
    let (sl, sr) = render(|p| p.solo_side.set_value(true), &l, &r);
    for i in 0..l.len() {
        let s = 0.5 * (l[i] - r[i]);
        assert_eq!((sl[i], sr[i]), (s, -s));
    }
    let (ml, mr) = render(|p| p.mono_check.set_value(true), &l, &r);
    for i in 0..l.len() {
        let m = 0.5 * (l[i] + r[i]);
        assert_eq!((ml[i], mr[i]), (m, m));
    }
    let (zl, zr) = render(
        |p| {
            p.solo_side.set_value(true);
            p.mono_check.set_value(true);
        },
        &l,
        &r,
    );
    assert!(zl.iter().chain(&zr).all(|&v| v == 0.0), "side folded to mono is silence");
}

// ---------------------------------------------------------------------------
// Haas: flagged, and doing what its label says
// ---------------------------------------------------------------------------

#[test]
fn the_haas_mode_is_flagged_as_a_mono_risk_and_only_it() {
    for mode in WidenMode::ALL {
        assert_eq!(mode.is_mono_risk(), mode == WidenMode::Haas, "{mode:?}");
    }
    // The flag is in the choice label every surface shows: the host's
    // automation lane, the control API's param listing, the editor.
    let p = StereoParams::default();
    let label = p.widen_mode.display(WidenMode::Haas.index() as f64);
    assert!(label.to_ascii_lowercase().contains("mono risk"), "label {label:?}");
    for mode in WidenMode::ALL.into_iter().filter(|m| *m != WidenMode::Haas) {
        let other = p.widen_mode.display(mode.index() as f64);
        assert!(!other.to_ascii_lowercase().contains("risk"), "{other:?}");
    }
    assert!(WidenMode::Haas.amount_hint(0.5).contains("MONO RISK"));

    // And the running plugin publishes it for the editor.
    let x = noise(2_048, 0.3, 3);
    let (_, _, plugin) = render_with(|p| p.widen_mode.set_value(WidenMode::Haas.index()), &x, &x, &[256]);
    assert!(plugin.viz().mono_risk());
    let (_, _, plugin) =
        render_with(|p| p.widen_mode.set_value(WidenMode::Decorrelate.index()), &x, &x, &[256]);
    assert!(!plugin.viz().mono_risk());
}

#[test]
fn haas_delays_the_right_side_above_the_exclude_at_a_level_offset() {
    let amount = 0.5;
    let delay = (haas_delay_ms(amount) * 0.001 * SR).round() as usize;
    // A click on the right only, with the low exclude off (20 Hz).
    let mut r = vec![0.0f32; 4_800];
    r[100] = 1.0;
    let l = vec![0.0f32; 4_800];
    let (ol, or) = render(
        |p| {
            p.widen_mode.set_value(WidenMode::Haas.index());
            p.widen_amount.set_value(amount);
            p.focus_low.set_value(20.0);
        },
        &l,
        &r,
    );
    assert!(ol.iter().all(|&v| v == 0.0), "the left is not touched");
    let (peak_at, peak) = or
        .iter()
        .enumerate()
        .fold((0, 0.0f32), |acc, (i, &v)| if v.abs() > acc.1.abs() { (i, v) } else { acc });
    assert_eq!(peak_at, 100 + delay, "the click moved by the Haas delay");
    let want = 10f32.powf(HAAS_LEVEL_DB / 20.0);
    assert!((peak - want).abs() < 0.02, "delayed side at {peak}, want {want}");

    // The low exclude keeps bass in time and at level: a 30 Hz tone under
    // a 600 Hz exclude comes out undelayed (only the LR4 split's small
    // phase shift) and at full level, while with no exclude the same tone
    // is delayed and 3 dB down.
    let bass = sine(30.0, 0.5, 48_000);
    let silent = vec![0.0f32; 48_000];
    let haas = |exclude: f32| {
        render(
            |p| {
                p.widen_mode.set_value(WidenMode::Haas.index());
                p.widen_amount.set_value(amount);
                p.focus_low.set_value(exclude);
            },
            &silent,
            &bass,
        )
        .1
    };
    let kept = haas(600.0);
    assert!(correlation(&kept[9_600..], &bass[9_600..]) > 0.98);
    let level_db = 10.0 * (energy(&kept[9_600..]) / energy(&bass[9_600..])).log10();
    assert!(level_db.abs() < 0.3, "excluded bass moved {level_db:.2} dB");
    let moved = haas(20.0);
    let moved_db = 10.0 * (energy(&moved[9_600..]) / energy(&bass[9_600..])).log10();
    assert!((moved_db - HAAS_LEVEL_DB as f64).abs() < 0.1, "unexcluded bass at {moved_db:.2} dB");
}

// ---------------------------------------------------------------------------
// The other widening modes stay sane in mono
// ---------------------------------------------------------------------------

/// The deepest dip of the mono fold's magnitude response, in dB, for a
/// mono input through `mode` at `amount`: unit impulses 0.25 s apart,
/// each one's fold `(L + R) / 2` read over the next 4096 samples at
/// 1/48-octave steps from 30 Hz to 18 kHz. Several impulses, because
/// Micro-shift's combs move; for the time-invariant Diffuse they agree.
fn worst_fold_notch_db(mode: WidenMode, amount: f32) -> f64 {
    const IMPULSES: usize = 8;
    const SPACING: usize = 12_000;
    const LEN: usize = 4_096;
    let first = 9_600;
    let mut x = vec![0.0f32; first + IMPULSES * SPACING + LEN];
    for k in 0..IMPULSES {
        x[first + k * SPACING] = 1.0;
    }
    let (l, r) = render(
        |p| {
            p.widen_mode.set_value(mode.index());
            p.widen_amount.set_value(amount);
        },
        &x,
        &x,
    );
    let mut worst = f64::INFINITY;
    for k in 0..IMPULSES {
        let at = first + k * SPACING;
        let h: Vec<f64> = (at..at + LEN).map(|i| 0.5 * (l[i] + r[i]) as f64).collect();
        let mut f = 30.0f64;
        while f < 18_000.0 {
            // DFT at `f` by a rotating phasor (no per-sample trig).
            let w = std::f64::consts::TAU * f / SR as f64;
            let (c, sn) = (w.cos(), -w.sin());
            let (mut pr, mut pi) = (1.0f64, 0.0f64);
            let (mut re, mut im) = (0.0f64, 0.0f64);
            for &v in &h {
                re += v * pr;
                im += v * pi;
                (pr, pi) = (pr * c - pi * sn, pr * sn + pi * c);
            }
            worst = worst.min(10.0 * (re * re + im * im).log10());
            f *= 2f64.powf(1.0 / 48.0);
        }
    }
    worst
}

/// Diffuse's spread is capped (`DIFFUSE_MAX_SPREAD`) where the mono
/// fold's deepest notch is still shallow: no worse than 2.5 dB anywhere
/// in the band at any amount. (Uncapped, amount 1 notched −14.6 dB.) It
/// must still widen at the top of the range.
#[test]
fn diffuse_stays_mono_safe_at_every_amount() {
    for amount in [0.25f32, 0.5, 0.75, 1.0] {
        let notch = worst_fold_notch_db(WidenMode::Diffuse, amount);
        eprintln!("Diffuse amount {amount}: worst fold notch {notch:.2} dB");
        assert!(notch > -2.5, "Diffuse at {amount}: the mono fold notches {notch:.2} dB");
    }
    let n = 96_000;
    let x = noise(n, 0.5, 41);
    let (ol, or) = render(
        |p| {
            p.widen_mode.set_value(WidenMode::Diffuse.index());
            p.widen_amount.set_value(1.0);
        },
        &x,
        &x,
    );
    let r = correlation(&ol[4_800..], &or[4_800..]);
    assert!(r < 0.97, "Diffuse at full amount did not widen a mono source (r = {r:.3})");
    assert!(ol.iter().chain(&or).all(|v| v.is_finite()));
}

/// Micro-shift's combs are as deep as documented (`micro_shift_notch_db`:
/// the voices in antiphase with the dry leave `1 − amount`), and no
/// deeper; the hint and the skill quote that figure.
#[test]
fn micro_shift_notches_the_mono_fold_as_deep_as_documented() {
    for amount in [0.2f32, 0.4, 0.6] {
        let notch = worst_fold_notch_db(WidenMode::MicroShift, amount);
        let said = micro_shift_notch_db(amount) as f64;
        eprintln!("Micro-shift amount {amount}: worst fold notch {notch:.2} dB (documented {said:.2})");
        assert!(
            notch > said - 0.5 && notch < said + 1.0,
            "Micro-shift at {amount}: the mono fold notches {notch:.2} dB, documented {said:.2}"
        );
    }
    let n = 96_000;
    let x = noise(n, 0.5, 41);
    let (ol, or) = render(
        |p| {
            p.widen_mode.set_value(WidenMode::MicroShift.index());
            p.widen_amount.set_value(0.4);
        },
        &x,
        &x,
    );
    let r = correlation(&ol[4_800..], &or[4_800..]);
    assert!(r < 0.97, "Micro-shift did not widen a mono source (r = {r:.3})");
    assert!(ol.iter().chain(&or).all(|v| v.is_finite()));
}

// ---------------------------------------------------------------------------
// Latency
// ---------------------------------------------------------------------------

#[test]
fn the_reported_latency_is_zero_in_every_mode() {
    for mode in WidenMode::ALL {
        let mut plugin = ResonanceStereo::new();
        plugin.params.widen_mode.set_value(mode.index());
        plugin.initialize(SR, MAX_BLOCK as u32);
        assert_eq!(plugin.latency_samples(), 0, "{mode:?}");
    }
    let _ = index::WIDEN_MODE;
}
