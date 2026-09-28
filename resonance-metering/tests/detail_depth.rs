//! `depth` proxies (warmth-width-depth.md §7.6, decision D5): the DRR
//! estimate's combination rule, the layer tertiles and the HF tilt.

mod common;

use common::coloured_noise;
use resonance_metering::detail::analyze_detail;
use resonance_metering::detail::depth::{
    drr_db_estimate, hf_tilt_db, layer_hints, mean_square, Layer, SendTerm,
};

fn post(send: f64, ret: f64) -> SendTerm {
    SendTerm {
        send_level_db: send,
        return_gain_db: ret,
        pre_fader: false,
    }
}

#[test]
fn one_post_fader_send_is_minus_send_plus_return() {
    assert_eq!(drr_db_estimate(0.0, &[post(-10.0, -6.0)]), Some(16.0));
    // A post-fader send moves with the fader, so the fader cancels.
    assert_eq!(drr_db_estimate(-9.0, &[post(-10.0, -6.0)]), Some(16.0));
}

#[test]
fn a_pre_fader_send_adds_the_source_fader() {
    let pre = SendTerm {
        pre_fader: true,
        ..post(-10.0, -6.0)
    };
    // Fader at -6 dB: the dry path is 6 dB quieter, the send is not.
    let drr = drr_db_estimate(-6.0, &[pre]).unwrap();
    assert!((drr - 10.0).abs() < 1e-9, "{drr}");
}

#[test]
fn several_sends_sum_in_power() {
    // Two equal wet paths are 3 dB more wet than one.
    let one = drr_db_estimate(0.0, &[post(-10.0, 0.0)]).unwrap();
    let two = drr_db_estimate(0.0, &[post(-10.0, 0.0), post(-10.0, 0.0)]).unwrap();
    assert!((one - two - 3.0103).abs() < 1e-3, "{one} {two}");
}

#[test]
fn no_sends_is_no_estimate() {
    assert_eq!(drr_db_estimate(0.0, &[]), None);
}

#[test]
fn tertiles_rank_driest_first_and_dry_only_counts_as_front() {
    let drr = [Some(0.0), None, Some(12.0), Some(5.0), Some(-6.0), Some(20.0)];
    // Ranked driest first: dry-only, 20 | 12, 5 | 0, -6.
    assert_eq!(
        layer_hints(&drr),
        vec![
            Layer::Back,   // 0
            Layer::Front,  // dry-only
            Layer::Middle, // 12
            Layer::Middle, // 5
            Layer::Back,   // -6
            Layer::Front,  // 20
        ]
    );
    assert_eq!(layer_hints(&[Some(3.0)]), vec![Layer::Front], "one source alone is front");
    assert!(layer_hints(&[]).is_empty());
}

#[test]
fn hf_tilt_is_flat_for_white_and_darker_for_pink() {
    let white = coloured_noise(1 << 19, 0.0, -20.0, 1);
    let pink = coloured_noise(1 << 19, 1.0, -20.0, 2);
    let tilt = |x: &[f32]| hf_tilt_db(&analyze_detail(48_000.0, x, x)).unwrap();
    // Band widths 10 kHz vs 3 kHz for white; ln(16/6) vs ln(4) for pink.
    let white_want = 10.0 * (10_000.0f32 / 3_000.0).log10();
    let pink_want = 10.0 * ((16.0f32 / 6.0).ln() / 4.0f32.ln()).log10();
    assert!((tilt(&white) - white_want).abs() < 0.1, "{}", tilt(&white));
    assert!((tilt(&pink) - pink_want).abs() < 0.1, "{}", tilt(&pink));
    let silence = vec![0.0f32; 48_000];
    assert_eq!(hf_tilt_db(&analyze_detail(48_000.0, &silence, &silence)), None);
}

#[test]
fn mean_square_covers_both_channels() {
    assert_eq!(mean_square(&[1.0, 1.0], &[0.0, 0.0]), 0.5);
    assert_eq!(mean_square(&[], &[]), 0.0);
}
