//! Mono-sum penalty, sample peak and clipped-sample counting.

mod common;

use common::sine_mono;
use resonance_metering::offline::{
    clipped_samples, mono_penalty_db, sample_peak_db, sample_peak_linear, FLOOR_DBFS,
    MONO_PENALTY_FLOOR_DB,
};

const SR: f32 = 48_000.0;

// ---------------------------------------------------------------- mono sum

#[test]
fn identical_channels_have_no_mono_penalty() {
    let (l, r) = sine_mono(SR, 1_000.0, -20.0, 3.0);
    let penalty = mono_penalty_db(SR, &l, &r);
    assert_eq!(
        penalty, 0.0,
        "L == R folds to itself, so the penalty is exactly zero"
    );
}

#[test]
fn anti_phase_channels_hit_the_floor() {
    let (l, r) = sine_mono(SR, 1_000.0, -20.0, 3.0);
    let inverted: Vec<f32> = r.iter().map(|s| -s).collect();
    let penalty = mono_penalty_db(SR, &l, &inverted);
    assert!(
        penalty < -3.0,
        "anti-phase channels must read clearly negative, got {penalty}"
    );
    assert_eq!(
        penalty, MONO_PENALTY_FLOOR_DB,
        "a fully cancelling mono sum reports the documented floor"
    );
}

#[test]
fn partially_out_of_phase_loses_level_without_cancelling() {
    // A 1 kHz tone against a 1 kHz tone shifted by 120°: the sum survives
    // but is measurably quieter than the stereo pair.
    let (l, _) = sine_mono(SR, 1_000.0, -20.0, 3.0);
    let phase = std::f32::consts::TAU / 3.0;
    let amp = 10.0_f32.powf(-20.0 / 20.0);
    let r: Vec<f32> = (0..l.len())
        .map(|i| ((i as f32 / SR * 1_000.0 * std::f32::consts::TAU) + phase).sin() * amp)
        .collect();

    let penalty = mono_penalty_db(SR, &l, &r);
    assert!(
        penalty < -3.0,
        "120° apart should lose real level when summed, got {penalty}"
    );
    assert!(
        penalty > MONO_PENALTY_FLOOR_DB,
        "…but not cancel to the floor, got {penalty}"
    );
}

#[test]
fn decorrelated_channels_lose_about_three_db() {
    // Two different frequencies are uncorrelated, so summing at 0.5 gain
    // gives half the power: ~-3 dB.
    let (l, _) = sine_mono(SR, 400.0, -20.0, 3.0);
    let (r, _) = sine_mono(SR, 3_100.0, -20.0, 3.0);
    let penalty = mono_penalty_db(SR, &l, &r);
    assert!(
        (-5.0..-1.5).contains(&penalty),
        "decorrelated channels should land near -3 dB, got {penalty}"
    );
}

#[test]
fn silence_has_no_mono_penalty() {
    let silence = vec![0.0_f32; 3 * SR as usize];
    assert_eq!(mono_penalty_db(SR, &silence, &silence), 0.0);
    assert_eq!(mono_penalty_db(SR, &[], &[]), 0.0);
}

#[test]
fn buffer_shorter_than_a_gating_block_has_no_mono_penalty() {
    // 50 ms is below the 400 ms momentary block, so integrated LUFS has no
    // gated block to report and the honest answer is "no penalty".
    let (l, r) = sine_mono(SR, 1_000.0, -20.0, 0.05);
    let inverted: Vec<f32> = r.iter().map(|s| -s).collect();
    assert_eq!(mono_penalty_db(SR, &l, &inverted), 0.0);
}

// -------------------------------------------------------------- sample peak

#[test]
fn half_amplitude_sine_peaks_at_minus_six_dbfs() {
    let (l, r) = sine_mono(SR, 1_000.0, -6.020_6, 1.0);
    let db = sample_peak_db(&l, &r);
    assert!(
        (db - -6.0).abs() < 0.05,
        "0.5 linear should read ~-6 dBFS, got {db}"
    );
    assert!((sample_peak_linear(&l, &r) - 0.5).abs() < 1e-3);
}

#[test]
fn full_scale_reads_zero_dbfs() {
    let l = vec![0.0, -1.0, 0.25];
    let r = vec![0.0, 0.0, 0.0];
    assert!((sample_peak_db(&l, &r) - 0.0).abs() < 1e-5);
}

#[test]
fn over_full_scale_reads_positive() {
    let l = vec![0.0, 2.0];
    let r = vec![0.0, 0.0];
    assert!((sample_peak_db(&l, &r) - 6.0206).abs() < 1e-3);
}

#[test]
fn silence_reads_the_documented_floor() {
    let silence = vec![0.0_f32; 128];
    assert_eq!(sample_peak_db(&silence, &silence), FLOOR_DBFS);
    assert_eq!(sample_peak_db(&[], &[]), FLOOR_DBFS);
}

#[test]
fn sample_peak_sees_both_channels() {
    let l = vec![0.1, 0.2];
    let r = vec![0.1, 0.8];
    assert!((sample_peak_linear(&l, &r) - 0.8).abs() < 1e-6);
}

#[test]
fn sample_peak_ignores_nan() {
    let l = vec![0.25, f32::NAN];
    let r = vec![0.0, 0.0];
    assert!((sample_peak_linear(&l, &r) - 0.25).abs() < 1e-6);
    assert!(sample_peak_db(&l, &r).is_finite());
}

#[test]
fn sample_peak_is_never_above_true_peak() {
    // Sanity check against the existing meter: inter-sample peaks are only
    // ever equal or higher.
    let (l, r) = sine_mono(SR, 997.0, -3.0, 0.5);
    let mut tp = resonance_metering::TruePeakMeter::new();
    tp.push_stereo(&l, &r);
    assert!(
        sample_peak_db(&l, &r) <= tp.peak_dbtp() + 1e-3,
        "sample peak {} exceeded true peak {}",
        sample_peak_db(&l, &r),
        tp.peak_dbtp()
    );
}

// ------------------------------------------------------------------ clipping

#[test]
fn clipping_counts_every_channel_sample_at_or_over_full_scale() {
    let l = vec![0.0, 1.0, 0.999_9, -1.000_1, 0.5];
    let r = vec![0.0, 0.9, -1.0, 2.0, 0.5];
    // left: 1.0, -1.0001 → 2; right: -1.0, 2.0 → 2.
    assert_eq!(clipped_samples(&l, &r), 4);
}

#[test]
fn clean_material_reports_no_clipping() {
    let (l, r) = sine_mono(SR, 1_000.0, -0.1, 0.5);
    assert_eq!(clipped_samples(&l, &r), 0);
}

#[test]
fn clipping_ignores_nan_and_empty_input() {
    let l = vec![f32::NAN, 0.5];
    let r = vec![0.5, 0.5];
    assert_eq!(clipped_samples(&l, &r), 0);
    assert_eq!(clipped_samples(&[], &[]), 0);
}

#[test]
fn clipping_stops_at_the_shorter_channel() {
    let l = vec![1.0, 1.0, 1.0];
    let r = vec![1.0];
    assert_eq!(clipped_samples(&l, &r), 2);
}
