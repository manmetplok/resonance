use resonance_dsp::DelayLine;

#[test]
fn tap_reads_sample_from_delay_plus_one_pushes_ago() {
    let mut dl = DelayLine::new(8);
    for i in 0..8 {
        dl.push(i as f32);
    }
    // tap(d) reads the sample written d + 1 pushes ago.
    assert_eq!(dl.tap(0), 7.0);
    assert_eq!(dl.tap(3), 4.0);
    assert_eq!(dl.tap(7), 0.0);
}

#[test]
fn tap_linear_interpolates_between_integer_taps() {
    let mut dl = DelayLine::new(8);
    for s in [0.0_f32, 1.0, 2.0, 3.0] {
        dl.push(s);
    }
    // tap(1) = 2.0, tap(2) = 1.0 → halfway is 1.5.
    let v = dl.tap_linear(1.5);
    assert!((v - 1.5).abs() < 1e-6, "got {v}");
}

#[test]
fn max_valid_tap_reads_oldest_sample() {
    // Capacity is the power-of-two size; the largest non-aliasing tap is
    // size - 1 and reads the oldest retained sample.
    let mut dl = DelayLine::new(8);
    for i in 0..8 {
        dl.push(100.0 + i as f32);
    }
    assert_eq!(dl.tap(7), 100.0);
}

#[cfg(debug_assertions)]
#[test]
#[should_panic(expected = "exceeds capacity")]
fn tap_beyond_capacity_asserts_in_debug() {
    let dl = DelayLine::new(8); // size 8, valid delays 0..=7
    let _ = dl.tap(8); // would silently alias to tap(0)
}
