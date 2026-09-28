//! The EQ's W8 additions (warmth-width-depth.md §6.4): per-band
//! Stereo/Mid/Side, the Tilt / LF Lift+Dip / Air band types, and
//! auto-gain.
//!
//! Every feature gets a measured assertion, and the set gets its own
//! pinned golden (`tests/golden/warmth_bands.f32`) with a non-silence
//! guard per scenario. `dsp_golden.f32` is untouched by all of this, and
//! `tests/legacy_state.rs` proves pre-W8 state renders the same bits.

use std::path::PathBuf;

use resonance_dsp::Biquad;
use resonance_dsp_test_support as golden;
use resonance_eq::band::{
    configure_stages, BandKind, BandMs, BandSlope, LF_DIP_RATIO, MAX_STAGES_PER_BAND,
};
use resonance_eq::dsp::static_gain_db;
use resonance_eq::params::{BandSnapshot, EqParams};
use resonance_eq::ResonanceEq;
use resonance_plugin::{EventIterator, OutputBuffer, ResonancePlugin};

const SR: f32 = 48_000.0;
const BLOCK: usize = 256;
const TAU: f32 = std::f32::consts::TAU;

fn snap(kind: BandKind, freq: f32, gain_db: f32, ms: BandMs) -> BandSnapshot {
    BandSnapshot {
        enabled: true,
        freq,
        gain_db,
        q: 0.707,
        kind,
        slope: BandSlope::Db24,
        ms,
    }
}

/// Magnitude of one band's whole cascade, dB.
fn band_db(s: &BandSnapshot, sr: f32, freq: f32) -> f32 {
    let mut stages = [Biquad::identity(); MAX_STAGES_PER_BAND];
    let n = configure_stages(s, sr, &mut stages);
    let lin: f32 = stages[..n].iter().map(|b| b.magnitude(freq, sr)).product();
    20.0 * lin.max(1e-12).log10()
}

fn noise(n: u64) -> f32 {
    let mut s = n.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1) as u32;
    s ^= s >> 16;
    s = s.wrapping_mul(2_246_822_519);
    s ^= s >> 13;
    (s >> 8) as f32 * (2.0 / (1 << 23) as f32) - 1.0
}

/// Run `blocks` blocks of stereo `input` through a fresh EQ.
fn render(
    setup: impl Fn(&EqParams),
    input: impl Fn(u64) -> (f32, f32),
    blocks: usize,
) -> (Vec<f32>, Vec<f32>) {
    let mut plugin = ResonanceEq::new();
    setup(&plugin.params);
    plugin.initialize(SR, BLOCK as u32);
    let (mut out_l, mut out_r) = (Vec::new(), Vec::new());
    let mut l = vec![0.0f32; BLOCK];
    let mut r = vec![0.0f32; BLOCK];
    let mut n = 0u64;
    for _ in 0..blocks {
        for i in 0..BLOCK {
            (l[i], r[i]) = input(n + i as u64);
        }
        let mut outs = [OutputBuffer {
            left: &mut l,
            right: &mut r,
        }];
        let mut ev = EventIterator::empty();
        plugin.process(&mut outs, BLOCK, &mut ev, None);
        n += BLOCK as u64;
        out_l.extend_from_slice(&l);
        out_r.extend_from_slice(&r);
    }
    (out_l, out_r)
}

fn set_band(p: &EqParams, i: usize, kind: BandKind, freq: f32, gain: f32, q: f32, ms: BandMs) {
    let b = &p.bands[i];
    b.enabled.set_value(true);
    b.kind.set_value(kind.to_index());
    b.freq.set_value(freq);
    b.gain.set_value(gain);
    b.q.set_value(q);
    b.ms.set_value(ms.to_index());
}

/// Decorrelated stereo: independent noise on each side plus a shared
/// component, so mid and side are both substantial.
fn stereo_noise(n: u64) -> (f32, f32) {
    let c = 0.3 * noise(n + 5_000_000);
    (0.4 * noise(n) + c, 0.4 * noise(n + 9_999_991) + c)
}

fn rms(x: &[f32]) -> f32 {
    (x.iter().map(|v| (*v as f64).powi(2)).sum::<f64>() / x.len().max(1) as f64).sqrt() as f32
}

fn db(x: f32) -> f32 {
    20.0 * x.max(1e-12).log10()
}

// ---------------------------------------------------------------------------
// M/S
// ---------------------------------------------------------------------------

#[test]
fn side_bands_leave_the_mono_sum_unchanged() {
    let setup = |p: &EqParams| {
        set_band(p, 0, BandKind::LowCut, 250.0, 0.0, 0.707, BandMs::Side);
        set_band(p, 1, BandKind::Bell, 2_500.0, 9.0, 1.2, BandMs::Side);
        set_band(p, 2, BandKind::Air, 12_000.0, 6.0, 0.707, BandMs::Side);
    };
    let (l, r) = render(setup, stereo_noise, 40);
    let mut worst = 0.0f32;
    let mut side_moved = 0.0f32;
    for (i, (yl, yr)) in l.iter().zip(&r).enumerate() {
        let (xl, xr) = stereo_noise(i as u64);
        worst = worst.max(((yl + yr) * 0.5 - (xl + xr) * 0.5).abs());
        side_moved = side_moved.max(((yl - yr) - (xl - xr)).abs());
    }
    assert!(worst < 2e-6, "a Side band changed the mono sum by {worst:.3e}");
    assert!(side_moved > 0.05, "the Side bands did nothing to the side ({side_moved})");
}

#[test]
fn mid_bands_leave_the_side_unchanged() {
    let setup = |p: &EqParams| {
        set_band(p, 3, BandKind::LowShelf, 120.0, 6.0, 0.707, BandMs::Mid);
        set_band(p, 4, BandKind::Tilt, 1_000.0, -3.0, 0.707, BandMs::Mid);
    };
    let (l, r) = render(setup, stereo_noise, 40);
    let mut worst = 0.0f32;
    for (i, (yl, yr)) in l.iter().zip(&r).enumerate() {
        let (xl, xr) = stereo_noise(i as u64);
        worst = worst.max(((yl - yr) - (xl - xr)).abs());
    }
    assert!(worst < 4e-6, "a Mid band changed the side by {worst:.3e}");
}

// ---------------------------------------------------------------------------
// Tilt
// ---------------------------------------------------------------------------

/// The analog first-order tilt the section is designed from.
fn analog_tilt_db(pivot: f32, gain_db: f32, f: f32) -> f32 {
    let a = 10f32.powf(gain_db / 20.0);
    let (w, w0) = (f, pivot);
    let num = a * a * (w * w + (w0 / a).powi(2));
    let den = w * w + (w0 * a).powi(2);
    10.0 * (num / den).log10()
}

#[test]
fn tilt_plus_3_db_pivots_at_its_frequency_with_the_first_order_slope() {
    let pivot = 1_000.0;
    let s = snap(BandKind::Tilt, pivot, 3.0, BandMs::Stereo);
    // Unity at the pivot.
    assert!(band_db(&s, SR, pivot).abs() < 0.01);
    // Asymptotes: +3 dB at the top, -3 dB at the bottom.
    assert!((band_db(&s, SR, 20.0) + 3.0).abs() < 0.1);
    assert!((band_db(&s, SR, 16_000.0) - 3.0).abs() < 0.15);
    // Point-symmetric around the pivot on a log axis, and on the analog
    // curve, through the band the slope is heard in.
    for k in [1.5f32, 2.0, 4.0, 8.0] {
        let up = band_db(&s, SR, pivot * k);
        let down = band_db(&s, SR, pivot / k);
        assert!((up + down).abs() < 0.1, "not symmetric at x{k}: {up} / {down}");
        let want = analog_tilt_db(pivot, 3.0, pivot * k);
        assert!((up - want).abs() < 0.1, "x{k}: {up} dB, analog {want} dB");
    }
    // The slope through the pivot, one octave wide: the first-order curve
    // gives ~1.8 dB/oct for a ±3 dB tilt.
    let slope = band_db(&s, SR, pivot * 2f32.sqrt()) - band_db(&s, SR, pivot / 2f32.sqrt());
    let want = analog_tilt_db(pivot, 3.0, pivot * 2f32.sqrt())
        - analog_tilt_db(pivot, 3.0, pivot / 2f32.sqrt());
    assert!((slope - want).abs() < 0.05, "slope {slope} dB/oct, analog {want}");
    assert!(slope > 1.5 && slope < 2.2, "slope {slope} dB/oct");
}

#[test]
fn a_rendered_tilt_moves_lows_down_and_highs_up() {
    let level = |freq: f32, tilted: bool| {
        let setup = move |p: &EqParams| {
            if tilted {
                set_band(p, 2, BandKind::Tilt, 1_000.0, 3.0, 0.707, BandMs::Stereo);
            }
        };
        let tone = move |n: u64| {
            let v = 0.25 * (TAU * freq * n as f32 / SR).sin();
            (v, v)
        };
        let (l, _) = render(setup, tone, 40);
        db(rms(&l[l.len() / 2..]))
    };
    let low = level(100.0, true) - level(100.0, false);
    let high = level(10_000.0, true) - level(10_000.0, false);
    assert!((low - analog_tilt_db(1_000.0, 3.0, 100.0)).abs() < 0.1, "100 Hz moved {low}");
    assert!((high - analog_tilt_db(1_000.0, 3.0, 10_000.0)).abs() < 0.15, "10 kHz moved {high}");
}

// ---------------------------------------------------------------------------
// LF Lift + Dip
// ---------------------------------------------------------------------------

#[test]
fn lf_lift_dip_lifts_the_lows_and_dips_the_low_mids_with_one_knob() {
    let f = 80.0;
    let g = 6.0;
    let s = snap(BandKind::LfLiftDip, f, g, BandMs::Stereo);
    let lift = band_db(&s, SR, f * 0.5);
    let dip = band_db(&s, SR, f * LF_DIP_RATIO);
    assert!(lift > 0.6 * g, "lift only {lift:.2} dB at {} Hz", f * 0.5);
    assert!(dip < -0.25 * g, "dip only {dip:.2} dB at {} Hz", f * LF_DIP_RATIO);
    assert!((200.0..=350.0).contains(&(f * LF_DIP_RATIO)));
    // Out of the way above the low mids.
    assert!(band_db(&s, SR, 5_000.0).abs() < 0.2);
    // The knob is bipolar and symmetric in dB.
    let inv = snap(BandKind::LfLiftDip, f, -g, BandMs::Stereo);
    assert!((band_db(&inv, SR, f * 0.5) + lift).abs() < 0.3);
}

// ---------------------------------------------------------------------------
// Air
// ---------------------------------------------------------------------------

#[test]
fn air_shelf_is_stable_and_well_behaved_up_to_nyquist() {
    for sr in [44_100.0f32, 48_000.0] {
        for (freq, gain) in [(20_000.0f32, 12.0f32), (20_000.0, 24.0), (8_000.0, 6.0), (20_000.0, -12.0)] {
            let s = snap(BandKind::Air, freq, gain, BandMs::Stereo);
            let mut stages = [Biquad::identity(); MAX_STAGES_PER_BAND];
            let n = configure_stages(&s, sr, &mut stages);
            assert_eq!(n, 1);
            let b = stages[0];
            assert_eq!((b.b2, b.a2), (0.0, 0.0), "Air must be first order");
            assert!(b.a1.abs() < 1.0, "pole outside the unit circle ({sr} Hz, {freq}, {gain})");
            // Unity at DC-ish, monotonic toward Nyquist, never past `gain`.
            assert!(band_db(&s, sr, 1_000.0).abs() < 0.6);
            let mut prev = band_db(&s, sr, 20.0);
            let mut f = 20.0f32;
            while f < sr * 0.499 {
                let now = band_db(&s, sr, f);
                assert!(now.is_finite());
                if gain > 0.0 {
                    assert!(now >= prev - 1e-4, "not monotonic at {f} Hz");
                    assert!(now <= gain + 0.01, "overshoots at {f} Hz: {now}");
                } else {
                    assert!(now <= prev + 1e-4, "not monotonic at {f} Hz");
                }
                prev = now;
                f *= 1.05;
            }
        }
    }
    // A full-scale white-noise render at the steepest setting stays
    // bounded and stationary. White noise carries full energy up to
    // Nyquist, where +24 dB is a gain of ~16, so peaks in the 20s are the
    // filter doing its job; growth over time would be instability.
    let setup = |p: &EqParams| set_band(p, 7, BandKind::Air, 20_000.0, 24.0, 0.707, BandMs::Stereo);
    let (l, r) = render(setup, |n| (noise(n), noise(n + 3)), 200);
    assert!(l.iter().chain(&r).all(|x| x.is_finite()));
    let peak = l.iter().chain(&r).fold(0.0f32, |m, x| m.max(x.abs()));
    assert!(peak < 2.0 * 10f32.powf(24.0 / 20.0), "Air ran away: {peak}");
    let q = l.len() / 4;
    let drift = db(rms(&l[3 * q..])) - db(rms(&l[q..2 * q]));
    assert!(drift.abs() < 0.5, "Air output level drifted {drift:.2} dB over the render");
}

#[test]
fn the_air_shelf_is_gentle_in_the_audible_band() {
    // +12 dB with its midpoint at 20 kHz: only the skirt is audible.
    let s = snap(BandKind::Air, 20_000.0, 12.0, BandMs::Stereo);
    let at_5k = band_db(&s, SR, 5_000.0);
    let at_10k = band_db(&s, SR, 10_000.0);
    assert!(at_5k > 0.5 && at_5k < 3.0, "5 kHz: {at_5k}");
    assert!(at_10k > at_5k && at_10k < 6.0, "10 kHz: {at_10k}");
}

// ---------------------------------------------------------------------------
// Auto-gain
// ---------------------------------------------------------------------------

/// Pink-ish noise (Paul Kellet's economy filter), both channels.
fn pink(len: usize) -> Vec<f32> {
    let (mut b0, mut b1, mut b2) = (0.0f32, 0.0f32, 0.0f32);
    (0..len as u64)
        .map(|n| {
            let w = noise(n);
            b0 = 0.99765 * b0 + w * 0.099_046;
            b1 = 0.96300 * b1 + w * 0.296_516_4;
            b2 = 0.57000 * b2 + w * 1.052_691_3;
            0.05 * (b0 + b1 + b2 + w * 0.1848)
        })
        .collect()
}

#[test]
fn auto_gain_is_off_by_default_and_a_flat_eq_estimates_exactly_zero() {
    assert!(!EqParams::default().auto_gain.value());
    let off: Vec<BandSnapshot> = EqParams::default().bands.iter().map(|b| b.snapshot()).collect();
    assert_eq!(static_gain_db(&off, SR), 0.0);
    // Auto-gain on over a flat EQ changes nothing at all.
    let input = |n: u64| (noise(n) * 0.3, noise(n + 1) * 0.3);
    let (a, _) = render(|_| {}, input, 20);
    let (b, _) = render(|p: &EqParams| p.auto_gain.set_value(true), input, 20);
    assert!(a.iter().zip(&b).all(|(x, y)| x.to_bits() == y.to_bits()));
}

#[test]
fn auto_gain_holds_pink_noise_level_through_a_broad_boost() {
    let signal = pink(BLOCK * 400);
    let input = |n: u64| {
        let v = signal[n as usize];
        (v, v)
    };
    let boost = |auto: bool| {
        move |p: &EqParams| {
            set_band(p, 6, BandKind::HighShelf, 2_000.0, 6.0, 0.707, BandMs::Stereo);
            set_band(p, 1, BandKind::LfLiftDip, 80.0, 4.0, 0.707, BandMs::Stereo);
            p.auto_gain.set_value(auto);
        }
    };
    let dry = db(rms(&signal[signal.len() / 4..]));
    let (plain, _) = render(boost(false), input, 400);
    let (matched, _) = render(boost(true), input, 400);
    let plain_db = db(rms(&plain[plain.len() / 4..])) - dry;
    let matched_db = db(rms(&matched[matched.len() / 4..])) - dry;
    assert!(plain_db > 2.0, "the boost itself only added {plain_db:.2} dB");
    assert!(matched_db.abs() < 1.0, "auto-gain left {matched_db:.2} dB on pink noise");
}

// ---------------------------------------------------------------------------
// Golden
// ---------------------------------------------------------------------------

struct Scenario {
    name: &'static str,
    setup: fn(&EqParams),
}

/// Every scenario pins each band it enables from the defaults up.
fn scenarios() -> Vec<Scenario> {
    vec![
        Scenario {
            name: "tilt_warm",
            setup: |p| set_band(p, 2, BandKind::Tilt, 800.0, -2.5, 0.707, BandMs::Stereo),
        },
        Scenario {
            name: "lf_lift_dip",
            setup: |p| set_band(p, 1, BandKind::LfLiftDip, 70.0, 5.0, 0.707, BandMs::Stereo),
        },
        Scenario {
            name: "air_48k",
            setup: |p| set_band(p, 7, BandKind::Air, 18_000.0, 8.0, 0.707, BandMs::Stereo),
        },
        Scenario {
            name: "ms_side_air_mid_cut",
            setup: |p| {
                set_band(p, 0, BandKind::LowCut, 150.0, 0.0, 0.707, BandMs::Side);
                set_band(p, 3, BandKind::Bell, 400.0, -3.0, 1.0, BandMs::Side);
                set_band(p, 6, BandKind::Air, 15_000.0, 4.0, 0.707, BandMs::Side);
                set_band(p, 4, BandKind::Bell, 2_500.0, -2.0, 2.0, BandMs::Mid);
            },
        },
        Scenario {
            name: "auto_gain_shelves",
            setup: |p| {
                set_band(p, 1, BandKind::LowShelf, 120.0, 4.0, 0.707, BandMs::Stereo);
                set_band(p, 6, BandKind::HighShelf, 9_000.0, 3.0, 0.707, BandMs::Stereo);
                p.auto_gain.set_value(true);
            },
        },
    ]
}

fn golden_input(n: u64) -> (f32, f32) {
    let t = n as f32 / SR;
    let tone = 0.15 * (TAU * 90.0 * t).sin() + 0.1 * (TAU * 3_100.0 * t).sin();
    let (a, b) = stereo_noise(n);
    (0.5 * a + tone, 0.5 * b + tone)
}

#[test]
fn warmth_bands_golden() {
    let mut all = Vec::new();
    for s in scenarios() {
        let (l, r) = render(s.setup, golden_input, 16);
        let peak = l.iter().chain(&r).fold(0.0f32, |m, x| m.max(x.abs()));
        assert!(peak > 1e-2, "scenario `{}` rendered silence", s.name);
        assert!(l.iter().chain(&r).all(|x| x.is_finite()), "`{}` non-finite", s.name);
        // The scenario must actually change the signal, or its golden is
        // just the input.
        let moved = l
            .iter()
            .enumerate()
            .fold(0.0f32, |m, (i, y)| m.max((y - golden_input(i as u64).0).abs()));
        assert!(moved > 1e-3, "scenario `{}` is a no-op", s.name);
        all.extend(l);
        all.extend(r);
    }
    let path: PathBuf = golden::golden_path(env!("CARGO_MANIFEST_DIR"), "warmth_bands.f32");
    if golden::blessed(&["RESONANCE_BLESS", "RESONANCE_BLESS_WARMTH_BANDS"]) {
        golden::bless_f32(&path, &all);
        return;
    }
    let want = golden::load_golden_f32(&path, all.len(), "RESONANCE_BLESS=1");
    let diff = golden::compare_f32(&all, &want);
    if let Some((i, got, want)) = diff.first_diff {
        panic!(
            "EQ warmth-band render changed: {}/{} samples differ (peak {:.3e}), \
             first at {i} (got {got}, want {want})",
            diff.diff_count,
            all.len(),
            diff.max_abs
        );
    }
}
