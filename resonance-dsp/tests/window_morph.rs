//! Tests for the boxcar → trapezoid → Hann morphable grain window.

use resonance_dsp::{hann_window, WindowMorph};

#[test]
fn endpoints_are_zero_for_any_positive_texture() {
    let morph = WindowMorph::new();
    for &texture in &[1e-3_f32, 0.05, 0.25, 0.5, 0.75, 1.0] {
        let start = morph.evaluate(0.0, texture);
        let end = morph.evaluate(1.0, texture);
        assert!(
            start.abs() < 1e-6 && end.abs() < 1e-6,
            "texture {texture}: endpoints {start} / {end} must be ~0"
        );
    }
}

#[test]
fn texture_zero_is_boxcar() {
    let morph = WindowMorph::new();
    for i in 0..=64 {
        let phase = i as f32 / 64.0;
        assert!(
            morph.evaluate(phase, 0.0) == 1.0,
            "boxcar must be 1 at phase {phase}"
        );
    }
}

#[test]
fn texture_one_matches_hann_window() {
    let morph = WindowMorph::new();
    let len = 1024;
    let hann = hann_window(len);
    for (i, &expected) in hann.iter().enumerate() {
        let phase = i as f32 / (len - 1) as f32;
        let got = morph.evaluate(phase, 1.0);
        assert!(
            (got - expected).abs() < 1e-4,
            "phase {phase}: morph {got} vs hann {expected}"
        );
    }
}

#[test]
fn intermediate_texture_is_a_tapered_trapezoid() {
    let morph = WindowMorph::new();
    // texture = 0.5: half-Hann ramps over [0, 0.25] and [0.75, 1], flat
    // plateau at 1.0 in between.
    for &phase in &[0.25_f32, 0.3, 0.5, 0.7, 0.75] {
        let v = morph.evaluate(phase, 0.5);
        assert!(
            (v - 1.0).abs() < 1e-6,
            "phase {phase} should sit on the plateau, got {v}"
        );
    }
    // Ramp mid-point of a raised cosine is exactly 0.5.
    let mid_rise = morph.evaluate(0.125, 0.5);
    let mid_fall = morph.evaluate(0.875, 0.5);
    assert!((mid_rise - 0.5).abs() < 1e-4, "rise mid {mid_rise}");
    assert!((mid_fall - 0.5).abs() < 1e-4, "fall mid {mid_fall}");
}

#[test]
fn plateau_widens_monotonically_as_texture_decreases() {
    let morph = WindowMorph::new();
    // At any fixed phase inside the taper region, lowering the texture
    // narrows the ramps, so the window value must not decrease; the flat
    // top (plateau) widens until the shape reaches the boxcar.
    for i in 1..64 {
        let phase = i as f32 / 128.0; // left half, (0, 0.5)
        let mut prev = morph.evaluate(phase, 1.0);
        for step in (0..=19).rev() {
            let texture = step as f32 / 20.0;
            let v = morph.evaluate(phase, texture);
            assert!(
                v >= prev - 1e-6,
                "phase {phase}, texture {texture}: {v} < previous {prev}"
            );
            prev = v;
        }
        // And the boxcar limit is reached.
        assert!(morph.evaluate(phase, 0.0) == 1.0);
    }
}

#[test]
fn window_is_symmetric_for_all_textures() {
    let morph = WindowMorph::new();
    for &texture in &[0.1_f32, 0.37, 0.5, 0.83, 1.0] {
        for i in 0..=256 {
            let phase = i as f32 / 256.0;
            let a = morph.evaluate(phase, texture);
            let b = morph.evaluate(1.0 - phase, texture);
            assert!(
                (a - b).abs() < 1e-5,
                "texture {texture}, phase {phase}: {a} vs {b}"
            );
        }
    }
}

#[test]
fn out_of_range_phase_is_silent() {
    let morph = WindowMorph::new();
    for &texture in &[0.0_f32, 0.5, 1.0] {
        assert!(morph.evaluate(-0.01, texture) == 0.0);
        assert!(morph.evaluate(1.01, texture) == 0.0);
        assert!(morph.evaluate(f32::NAN, texture) == 0.0);
    }
    assert!(morph.evaluate(0.5, f32::NAN) == 0.0);
}
