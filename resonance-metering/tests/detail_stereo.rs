//! `stereo` detail (warmth-width-depth.md §7.1, slice W1): per-band
//! correlation, side/mid and mono loss, the windowed correlation summary,
//! balance, the one-sided flag and the Haas detector, on synthetic cases
//! whose answers are exact.

mod common;

use common::{coloured_noise, sine_mono};
use resonance_metering::detail::stereo::{RATIO_LIMIT_DB, STEREO_BAND_EDGES_HZ};
use resonance_metering::detail::stereo_detail;

const SR: f32 = 48_000.0;
/// 2^19 samples ≈ 10.9 s at 48 kHz.
const LEN: usize = 1 << 19;

fn white(seed: u64) -> Vec<f32> {
    coloured_noise(LEN, 0.0, -20.0, seed)
}

/// `x` delayed by `d` samples (periodic, as the noise is periodic).
fn delayed(x: &[f32], d: usize) -> Vec<f32> {
    (0..x.len()).map(|i| x[(i + x.len() - d) % x.len()]).collect()
}

/// A sine with its phase computed in f64: `sine_mono`'s f32 phase
/// accumulates enough error over seconds to smear a noise floor across
/// every band, which would make an "empty" band hold content.
fn clean_sine(freq: f64, dbfs: f64, secs: f64) -> Vec<f32> {
    let amp = 10f64.powf(dbfs / 20.0);
    let n = (SR as f64 * secs) as usize;
    (0..n)
        .map(|i| (amp * (std::f64::consts::TAU * freq * i as f64 / SR as f64).sin()) as f32)
        .collect()
}

#[test]
fn mono_is_fully_correlated_in_every_band() {
    let x = white(1);
    let d = stereo_detail(SR, &x, &x);
    assert_eq!(d.bands.len(), 8);
    for (b, edge) in d.bands.iter().zip(STEREO_BAND_EDGES_HZ.windows(2)) {
        assert_eq!((b.lo_hz, b.hi_hz), (edge[0] as f32, edge[1] as f32));
        assert!(b.correlation.unwrap() > 0.9999, "{b:?}");
        assert_eq!(b.side_mid_db, Some(-RATIO_LIMIT_DB), "no side at all: {b:?}");
        assert!(b.mono_loss_db.unwrap().abs() < 1e-3, "{b:?}");
    }
    assert!(!d.one_sided);
    assert!(d.balance_db.unwrap().abs() < 1e-3);
    let w = d.correlation_windows.expect("windows");
    assert!(w.worst > 0.9999 && w.pct_below_0_3 == 0.0, "{w:?}");
    assert_eq!(w.windows, (LEN as f64 / (0.4 * SR as f64)) as u32);
    assert_eq!(d.haas_lag_ms, None, "mono is not a delay");
}

#[test]
fn hard_panned_mono_is_one_sided_not_zero_or_one() {
    let x = white(2);
    let silent = vec![0.0f32; LEN];
    for (l, r, balance) in [(&x, &silent, RATIO_LIMIT_DB), (&silent, &x, -RATIO_LIMIT_DB)] {
        let d = stereo_detail(SR, l, r);
        assert!(d.one_sided, "one channel is silent");
        assert_eq!(d.balance_db, Some(balance), "balance names the side");
        for b in &d.bands {
            assert_eq!(b.correlation, None, "0/0 is reported as absent: {b:?}");
            // Hard-panned: M = S = L/2, and mono keeps half the power.
            assert!(b.side_mid_db.unwrap().abs() < 1e-3, "{b:?}");
            assert!((b.mono_loss_db.unwrap() - -3.0103).abs() < 1e-3, "{b:?}");
        }
        assert_eq!(d.correlation_windows, None, "no countable window");
        assert_eq!(d.haas_lag_ms, None);
    }
}

#[test]
fn a_faint_bleed_below_forty_db_is_still_one_sided() {
    let x = white(3);
    let bleed: Vec<f32> = white(4).iter().map(|s| s * 10f32.powf(-50.0 / 20.0)).collect();
    let d = stereo_detail(SR, &x, &bleed);
    assert!(d.one_sided);
    assert!((d.balance_db.unwrap() - 50.0).abs() < 0.1, "{:?}", d.balance_db);
}

#[test]
fn a_ten_ms_haas_delay_is_found_to_a_tenth_of_a_ms() {
    let x = white(5);
    for (rate, d_samples) in [(48_000.0f32, 480usize), (44_100.0, 441)] {
        let late = delayed(&x, d_samples);
        let right_late = stereo_detail(rate, &x, &late);
        let lag = right_late.haas_lag_ms.expect("a static delay is detected");
        assert!((lag - 10.0).abs() <= 0.1, "right-late lag {lag} ms at {rate}");
        let left_late = stereo_detail(rate, &late, &x);
        let lag = left_late.haas_lag_ms.expect("either direction");
        assert!((lag + 10.0).abs() <= 0.1, "left-late lag {lag} ms at {rate}");
    }
}

#[test]
fn a_haas_pair_combs_in_mono_and_decorrelates_at_zero_lag() {
    let x = white(6);
    let d = stereo_detail(SR, &x, &delayed(&x, 480));
    // White noise against its own 10 ms delay is uncorrelated at lag 0
    // wherever a band spans many cycles of the comb. A narrow low band
    // does not: its correlation is the mean of cos(2π·f·10 ms) across it,
    // which is exactly the per-band story a mono fold-down tells.
    for b in &d.bands[4..] {
        assert!(b.correlation.unwrap().abs() < 0.1, "zero-lag correlation {b:?}");
    }
    assert!(d.bands[0].correlation.unwrap().abs() > 0.3, "{:?}", d.bands[0]);
    let w = d.correlation_windows.unwrap();
    assert!(w.worst.abs() < 0.2 && w.pct_below_0_3 == 100.0, "{w:?}");
    assert!(!d.one_sided);
}

#[test]
fn a_delay_outside_the_haas_window_is_not_reported() {
    let x = white(7);
    let d = stereo_detail(SR, &x, &delayed(&x, 48 * 50));
    assert_eq!(d.haas_lag_ms, None, "50 ms is an echo, not a Haas widening");
}

#[test]
fn a_periodic_mono_signal_is_not_mistaken_for_a_delay() {
    // A 100 Hz sine correlates with itself 10 ms later; L = R is still
    // not a delay between the channels.
    let (l, r) = sine_mono(SR, 100.0, -12.0, 4.0);
    assert_eq!(stereo_detail(SR, &l, &r).haas_lag_ms, None);
}

#[test]
fn antiphase_is_minus_one_with_no_mid() {
    let x = white(8);
    let inverted: Vec<f32> = x.iter().map(|s| -s).collect();
    let d = stereo_detail(SR, &x, &inverted);
    for b in &d.bands {
        assert!(b.correlation.unwrap() < -0.9999, "{b:?}");
        assert_eq!(b.side_mid_db, Some(RATIO_LIMIT_DB), "no mid at all: {b:?}");
        assert_eq!(b.mono_loss_db, Some(-RATIO_LIMIT_DB), "mono cancels: {b:?}");
    }
    let w = d.correlation_windows.unwrap();
    assert!(w.worst < -0.9999 && w.pct_below_0_3 == 100.0, "{w:?}");
    assert!(!d.one_sided);
    assert_eq!(d.haas_lag_ms, None);
}

#[test]
fn uncorrelated_equal_channels_lose_three_db_in_mono() {
    let d = stereo_detail(SR, &white(9), &white(10));
    for b in &d.bands {
        assert!(b.correlation.unwrap().abs() < 0.05, "{b:?}");
        assert!(b.side_mid_db.unwrap().abs() < 0.5, "{b:?}");
        assert!((b.mono_loss_db.unwrap() - -3.01).abs() < 0.25, "{b:?}");
    }
}

#[test]
fn side_mid_and_correlation_obey_r_equals_one_minus_rho_over_one_plus_rho() {
    // Equal-energy channels, partly correlated: L = s + k·a, R = s + k·b.
    let (s, a, b) = (white(11), white(12), white(13));
    for k in [0.3f32, 0.7, 1.5] {
        let l: Vec<f32> = s.iter().zip(&a).map(|(s, a)| s + k * a).collect();
        let r: Vec<f32> = s.iter().zip(&b).map(|(s, b)| s + k * b).collect();
        let d = stereo_detail(SR, &l, &r);
        for band in &d.bands[2..] {
            let rho = 10f32.powf(band.side_mid_db.unwrap() / 10.0);
            let predicted = (1.0 - rho) / (1.0 + rho);
            let r = band.correlation.unwrap();
            assert!((predicted - r).abs() < 0.03, "k {k}: r {r} vs (1-ρ)/(1+ρ) {predicted}");
        }
    }
}

#[test]
fn bands_separate_mono_lows_from_antiphase_highs() {
    let low = clean_sine(100.0, -12.0, 4.0);
    let high = clean_sine(3_000.0, -12.0, 4.0);
    let l: Vec<f32> = low.iter().zip(&high).map(|(a, b)| a + b).collect();
    let r: Vec<f32> = low.iter().zip(&high).map(|(a, b)| a - b).collect();
    let d = stereo_detail(SR, &l, &r);
    let band = |hz: f32| {
        d.bands
            .iter()
            .find(|b| b.lo_hz <= hz && hz < b.hi_hz)
            .unwrap()
    };
    assert!(band(100.0).correlation.unwrap() > 0.999);
    assert!(band(3_000.0).correlation.unwrap() < -0.999);
    // Bands with neither sine hold only leakage and report nothing.
    let empty = band(15_000.0);
    assert_eq!((empty.correlation, empty.side_mid_db, empty.mono_loss_db), (None, None, None));
}

#[test]
fn balance_reads_the_level_difference() {
    let x = white(14);
    let quieter: Vec<f32> = x.iter().map(|s| s * 0.5).collect();
    let d = stereo_detail(SR, &x, &quieter);
    assert!((d.balance_db.unwrap() - 6.0206).abs() < 1e-3, "{:?}", d.balance_db);
    assert!(!d.one_sided);
}

#[test]
fn windows_find_the_worst_passage_and_when_it_happens() {
    // 4 s mono, then 4 s anti-phase, then 2 s of silence.
    let (m, _) = sine_mono(SR, 440.0, -12.0, 4.0);
    let mut l = m.clone();
    let mut r = m.clone();
    l.extend(m.iter());
    r.extend(m.iter().map(|s| -s));
    l.extend(vec![0.0; 2 * SR as usize]);
    r.extend(vec![0.0; 2 * SR as usize]);
    let w = stereo_detail(SR, &l, &r).correlation_windows.unwrap();
    assert_eq!(w.windows, 20, "silent windows are not counted: {w:?}");
    assert_eq!(w.pct_below_0_3, 50.0);
    assert!(w.worst < -0.99);
    assert!((4.0..8.0).contains(&w.worst_at_seconds), "{w:?}");
}

#[test]
fn silence_reports_nothing() {
    let silent = vec![0.0f32; 48_000];
    let d = stereo_detail(SR, &silent, &silent);
    assert!(!d.one_sided, "silence is not one-sided, it is nothing");
    assert_eq!(d.balance_db, None);
    assert_eq!(d.correlation_windows, None);
    assert_eq!(d.haas_lag_ms, None);
    assert!(d.bands.iter().all(|b| b.correlation.is_none() && b.side_mid_db.is_none()));
    let empty: Vec<f32> = Vec::new();
    assert_eq!(stereo_detail(SR, &empty, &empty).balance_db, None);
}
