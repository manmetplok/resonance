//! `mix_into_timeline` placement semantics, in particular negative
//! segment offsets (audio starting before the timeline origin).

use resonance_svs::mix_into_timeline;

#[test]
fn positive_offsets_place_and_sum() {
    let out = mix_into_timeline(&[(0, vec![1.0, 1.0, 1.0]), (2, vec![0.5, 0.5])]);
    assert_eq!(out, vec![1.0, 1.0, 1.5, 0.5]);
}

#[test]
fn negative_offset_trims_leading_samples() {
    // Two samples fall before the origin; the rest must land at sample 0
    // (not be shifted late by two samples, as offset-clamping would do).
    let out = mix_into_timeline(&[(-2, vec![0.1, 0.2, 0.3, 0.4])]);
    assert_eq!(out, vec![0.3, 0.4]);
}

#[test]
fn negative_offset_mixes_against_other_segments_in_time() {
    let out = mix_into_timeline(&[(-1, vec![1.0, 1.0, 1.0]), (1, vec![0.5, 0.5])]);
    // Trimmed segment contributes samples 0..2; second segment 1..3.
    assert_eq!(out, vec![1.0, 1.5, 0.5]);
}

#[test]
fn segment_entirely_before_origin_is_dropped() {
    let out = mix_into_timeline(&[(-4, vec![1.0, 1.0]), (0, vec![0.5])]);
    assert_eq!(out, vec![0.5]);

    // Boundary: last sample ends exactly at the origin.
    let out = mix_into_timeline(&[(-2, vec![1.0, 1.0])]);
    assert!(out.is_empty());
}

#[test]
fn empty_input_yields_empty_timeline() {
    assert!(mix_into_timeline(&[]).is_empty());
}
