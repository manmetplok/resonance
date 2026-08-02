//! Band-share behaviour of the offline mix-analysis primitives.

mod common;

use common::sine_mono;
use resonance_metering::offline::{band_shares, BandShares};

const SR: f32 = 48_000.0;

fn sum(shares: BandShares) -> f32 {
    shares.low + shares.mid + shares.high + shares.air
}

#[test]
fn hundred_hz_sine_is_almost_all_low() {
    let (l, r) = sine_mono(SR, 100.0, -12.0, 2.0);
    let shares = band_shares(SR, &l, &r);
    assert!(
        shares.low > 0.95,
        "100 Hz sine should sit in the low band, got {shares:?}"
    );
    assert!((sum(shares) - 1.0).abs() < 1e-4, "shares must sum to 1");
}

#[test]
fn one_khz_sine_is_almost_all_mid() {
    let (l, r) = sine_mono(SR, 1_000.0, -12.0, 2.0);
    let shares = band_shares(SR, &l, &r);
    assert!(
        shares.mid > 0.95,
        "1 kHz sine should sit in the mid band, got {shares:?}"
    );
}

#[test]
fn four_khz_sine_is_almost_all_high() {
    let (l, r) = sine_mono(SR, 4_000.0, -12.0, 2.0);
    let shares = band_shares(SR, &l, &r);
    assert!(
        shares.high > 0.95,
        "4 kHz sine should sit in the high band, got {shares:?}"
    );
}

#[test]
fn ten_khz_sine_is_almost_all_air() {
    let (l, r) = sine_mono(SR, 10_000.0, -12.0, 2.0);
    let shares = band_shares(SR, &l, &r);
    assert!(
        shares.air > 0.95,
        "10 kHz sine should sit in the air band, got {shares:?}"
    );
    assert!((sum(shares) - 1.0).abs() < 1e-4, "shares must sum to 1");
}

#[test]
fn pink_noise_spreads_across_all_four_bands() {
    let (l, r) = pink_noise(SR, 4.0, 0x5EED_1217);
    let shares = band_shares(SR, &l, &r);
    for (name, v) in [
        ("low", shares.low),
        ("mid", shares.mid),
        ("high", shares.high),
        ("air", shares.air),
    ] {
        assert!(
            v > 0.05,
            "pink noise should populate every band; {name} = {v} of {shares:?}"
        );
    }
    assert!((sum(shares) - 1.0).abs() < 1e-4, "shares must sum to 1");
    // Pink noise carries roughly equal energy per octave, and the low band
    // spans the most octaves (20 Hz–250 Hz ≈ 3.6), so it should lead.
    assert!(
        shares.low > shares.air,
        "pink noise low band should exceed air, got {shares:?}"
    );
}

#[test]
fn silence_reports_zero_not_nan() {
    let silence = vec![0.0_f32; 4 * SR as usize];
    let shares = band_shares(SR, &silence, &silence);
    assert_eq!(shares, BandShares::SILENT);
    assert!(shares.low.is_finite() && shares.air.is_finite());
}

#[test]
fn empty_buffer_reports_silent() {
    assert_eq!(band_shares(SR, &[], &[]), BandShares::SILENT);
}

#[test]
fn buffer_shorter_than_one_fft_frame_still_measures() {
    // 1000 samples ≈ 21 ms, far below the 8192-sample FFT window: the
    // offline analyser must zero-pad rather than report silence.
    let (l, r) = sine_mono(SR, 100.0, -12.0, 1_000.0 / SR);
    assert!(
        l.len() < 1_100,
        "expected a sub-frame buffer, got {}",
        l.len()
    );
    let shares = band_shares(SR, &l, &r);
    assert!(
        shares.low > 0.8,
        "short 100 Hz burst should still read as low, got {shares:?}"
    );
    assert!((sum(shares) - 1.0).abs() < 1e-4, "shares must sum to 1");
}

#[test]
fn shares_ignore_absolute_level() {
    let (loud_l, loud_r) = sine_mono(SR, 100.0, -6.0, 2.0);
    let (quiet_l, quiet_r) = sine_mono(SR, 100.0, -48.0, 2.0);
    let loud = band_shares(SR, &loud_l, &loud_r);
    let quiet = band_shares(SR, &quiet_l, &quiet_r);
    assert!(
        (loud.low - quiet.low).abs() < 1e-3,
        "band shares are ratios: {loud:?} vs {quiet:?}"
    );
}

/// Deterministic pink-ish noise (Paul Kellett's economy filter over an LCG
/// white source), identical on both channels.
fn pink_noise(sr: f32, secs: f32, seed: u32) -> (Vec<f32>, Vec<f32>) {
    let n = (sr * secs) as usize;
    let mut out = vec![0.0_f32; n];
    let mut state = seed | 1;
    let (mut b0, mut b1, mut b2) = (0.0_f32, 0.0_f32, 0.0_f32);
    for slot in out.iter_mut() {
        state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        let white = (state >> 8) as f32 / 8_388_608.0 - 1.0; // [-1, 1)
        b0 = 0.99765 * b0 + white * 0.0990460;
        b1 = 0.96300 * b1 + white * 0.2965164;
        b2 = 0.57000 * b2 + white * 1.0526913;
        *slot = (b0 + b1 + b2 + white * 0.1848) * 0.2;
    }
    (out.clone(), out)
}
