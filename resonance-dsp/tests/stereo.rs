//! Mid/side helpers, width, balance and rotation.

mod common;

use common::noise;
use resonance_dsp::{apply_balance, apply_width, ms_decode, ms_encode, StereoRotation};
use std::f32::consts::FRAC_PI_4;

#[test]
fn ms_round_trip_is_identity_up_to_rounding() {
    let l = noise(4_800, 0.9, 1);
    let r = noise(4_800, 0.9, 2);
    for i in 0..l.len() {
        let (m, s) = ms_encode(l[i], r[i]);
        let (a, b) = ms_decode(m, s);
        assert!((a - l[i]).abs() <= 2.0 * f32::EPSILON && (b - r[i]).abs() <= 2.0 * f32::EPSILON);
    }
    // A centred source is all mid, equal to the source.
    assert_eq!(ms_encode(0.5, 0.5), (0.5, 0.0));
}

#[test]
fn width_keeps_the_mid_and_width_one_is_exact() {
    let l = noise(4_800, 0.9, 3);
    let r = noise(4_800, 0.9, 4);
    for i in 0..l.len() {
        assert_eq!(apply_width(l[i], r[i], 1.0), (l[i], r[i]));
        let (a, b) = apply_width(l[i], r[i], 0.0);
        assert_eq!(a, b, "width 0 is mono");
        for w in [0.0f32, 0.5, 2.0] {
            let (a, b) = apply_width(l[i], r[i], w);
            assert!(((a + b) - (l[i] + r[i])).abs() <= 4.0 * f32::EPSILON);
        }
    }
}

#[test]
fn balance_is_unity_at_centre_and_fades_the_far_side() {
    assert_eq!(apply_balance(0.7, -0.2, 0.0), (0.7, -0.2));
    assert_eq!(apply_balance(1.0, 1.0, 1.0), (0.0, 1.0));
    assert_eq!(apply_balance(1.0, 1.0, -0.5), (1.0, 0.5));
}

#[test]
fn rotation_preserves_energy_and_moves_the_image() {
    let rot = StereoRotation::new(FRAC_PI_4);
    let (l, r) = rot.process(1.0, 1.0);
    assert!(l.abs() < 1e-6 && (r - 2f32.sqrt()).abs() < 1e-6, "centre → hard right: ({l}, {r})");
    let a = noise(1_000, 0.9, 5);
    let b = noise(1_000, 0.9, 6);
    let rot = StereoRotation::new(0.3);
    for i in 0..a.len() {
        let (x, y) = rot.process(a[i], b[i]);
        let before = a[i] * a[i] + b[i] * b[i];
        assert!((x * x + y * y - before).abs() <= 1e-6);
    }
    let id = StereoRotation::new(0.0);
    assert_eq!(id.process(0.25, -0.75), (0.25, -0.75));
    let nan = StereoRotation::new(f32::NAN);
    assert_eq!(nan.process(0.25, -0.75), (0.25, -0.75));
}
