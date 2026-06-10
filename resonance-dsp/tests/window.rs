//! Tests for the shared Hann window.

use resonance_dsp::{fill_hann_window, hann_window};

#[test]
fn hann_endpoints_are_zero_and_centre_is_one() {
    let w = hann_window(4097);
    assert!(w[0].abs() < 1e-7);
    assert!(w[4096].abs() < 1e-6);
    assert!((w[2048] - 1.0).abs() < 1e-7);
}

#[test]
fn hann_is_symmetric() {
    let w = hann_window(2048);
    for i in 0..1024 {
        let a = w[i];
        let b = w[2047 - i];
        assert!(
            (a - b).abs() < 1e-6,
            "asymmetry at {i}: {a} vs {b}"
        );
    }
}

#[test]
fn fill_matches_allocating_variant_bitwise() {
    let alloc = hann_window(1024);
    let mut filled = vec![0.0_f32; 1024];
    fill_hann_window(&mut filled);
    for (i, (a, b)) in alloc.iter().zip(filled.iter()).enumerate() {
        assert!(a.to_bits() == b.to_bits(), "coefficient {i} differs");
    }
}

#[test]
fn hann_matches_closed_form() {
    let w = hann_window(192);
    for (i, &v) in w.iter().enumerate() {
        let x = i as f32 / 191.0;
        let expected = 0.5 - 0.5 * (std::f32::consts::TAU * x).cos();
        assert!(v.to_bits() == expected.to_bits(), "coefficient {i} differs");
    }
}
