#![cfg(feature = "editor")]

use resonance_amp::editor::tuner_view::hz_to_note_cents;

#[test]
fn a4_is_a4_zero_cents() {
    let (n, o, c) = hz_to_note_cents(440.0);
    assert_eq!(n, "A");
    assert_eq!(o, 4);
    assert!(c.abs() < 0.01, "cents should be ~0, got {c}");
}

#[test]
fn low_e_is_e2() {
    let (n, o, c) = hz_to_note_cents(82.407);
    assert_eq!(n, "E");
    assert_eq!(o, 2);
    assert!(c.abs() < 1.0);
}

#[test]
fn high_e_is_e4() {
    let (n, o, c) = hz_to_note_cents(329.628);
    assert_eq!(n, "E");
    assert_eq!(o, 4);
    assert!(c.abs() < 1.0);
}

#[test]
fn plus_thirty_cents_sharp() {
    // 440 * 2^(0.3/12) ≈ 447.70 — unambiguously +30 cents above A4.
    let hz = 440.0 * 2f32.powf(0.3 / 12.0);
    let (n, o, c) = hz_to_note_cents(hz);
    assert_eq!(n, "A");
    assert_eq!(o, 4);
    assert!((c - 30.0).abs() < 0.5, "expected +30 cents, got {c}");
}

#[test]
fn minus_thirty_cents_flat() {
    // 440 * 2^(-0.3/12) ≈ 432.47 — unambiguously -30 cents below A4.
    let hz = 440.0 * 2f32.powf(-0.3 / 12.0);
    let (n, o, c) = hz_to_note_cents(hz);
    assert_eq!(n, "A");
    assert_eq!(o, 4);
    assert!((c + 30.0).abs() < 0.5, "expected -30 cents, got {c}");
}
