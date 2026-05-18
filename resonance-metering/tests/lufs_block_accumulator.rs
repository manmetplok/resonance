use resonance_metering::lufs::block_accumulator::BlockAccumulator;

#[test]
fn momentary_sum_tracks_constant_signal() {
    let sr = 48_000.0_f32;
    let mut acc = BlockAccumulator::new(sr);
    // Feed 500 ms of squared=1.0. After 400 ms the momentary mean-square
    // must settle at exactly 1.0.
    let n = (0.5 * sr) as usize;
    for _ in 0..n {
        acc.push_sample(1.0);
    }
    let m = acc.momentary_mean_square().unwrap();
    assert!((m - 1.0).abs() < 1e-9, "momentary MS = {m}");
}

#[test]
fn short_term_takes_three_seconds_to_settle() {
    let sr = 48_000.0_f32;
    let mut acc = BlockAccumulator::new(sr);
    let n = (3.5 * sr) as usize;
    for _ in 0..n {
        acc.push_sample(4.0);
    }
    let s = acc.short_term_mean_square().unwrap();
    assert!((s - 4.0).abs() < 1e-9, "short-term MS = {s}");
}

#[test]
fn emits_blocks_after_first_momentary_window() {
    let sr = 48_000.0_f32;
    let mut acc = BlockAccumulator::new(sr);
    let mut blocks = 0usize;
    // 1.0 seconds of audio. Expect: first block at 400 ms, then 500,
    // 600, 700, 800, 900, 1000 → 7 blocks total.
    let n = sr as usize;
    for _ in 0..n {
        if acc.push_sample(0.5).is_some() {
            blocks += 1;
        }
    }
    assert_eq!(blocks, 7);
}

#[test]
fn silence_produces_zero_mean_square() {
    let sr = 48_000.0_f32;
    let mut acc = BlockAccumulator::new(sr);
    for _ in 0..48_000 {
        acc.push_sample(0.0);
    }
    assert_eq!(acc.momentary_mean_square().unwrap(), 0.0);
    assert_eq!(acc.short_term_mean_square().unwrap(), 0.0);
}
