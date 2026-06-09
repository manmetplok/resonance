use resonance_metering::CrestMeter;

#[test]
fn sine_wave_crest_is_about_three_db() {
    // A unit sine has peak = 1, RMS = 1/sqrt(2) → crest = 3.01 dB.
    let sr = 48_000.0;
    let mut m = CrestMeter::new(sr);
    let mut l = vec![0.0_f32; (sr * 0.2) as usize];
    for (i, s) in l.iter_mut().enumerate() {
        *s = (i as f32 / sr * 1000.0 * std::f32::consts::TAU).sin();
    }
    let r = l.clone();
    m.push_stereo(&l, &r);
    let crest = m.crest_db();
    assert!((crest - 3.01).abs() < 0.5, "crest = {crest}");
}

#[test]
fn silence_is_zero_crest() {
    let m = CrestMeter::new(48_000.0);
    assert_eq!(m.crest_db(), 0.0);
}

#[test]
fn spike_leaves_the_window() {
    // One full-scale spike, then 200 ms of a quiet sine. Once the spike
    // has slid out of the 100 ms window, the crest must reflect only the
    // sine (~3 dB), not the stale spike (~23 dB).
    let sr = 48_000.0;
    let mut m = CrestMeter::new(sr);
    m.push_stereo(&[1.0], &[1.0]);
    let n = (sr * 0.2) as usize;
    let mut l = vec![0.0_f32; n];
    for (i, s) in l.iter_mut().enumerate() {
        *s = 0.1 * (i as f32 / sr * 1000.0 * std::f32::consts::TAU).sin();
    }
    let r = l.clone();
    m.push_stereo(&l, &r);
    let crest = m.crest_db();
    assert!((crest - 3.01).abs() < 0.5, "crest = {crest}");
}

#[test]
fn matches_naive_window_scan() {
    // The monotonic-deque peak must agree with a brute-force max over
    // the last `window` samples at every probe point, across blocks of
    // awkward sizes that straddle the ring wrap.
    let sr = 1_000.0; // window = 100 samples
    let window = 100usize;
    let mut m = CrestMeter::new(sr);

    // Deterministic LCG noise with occasional spikes.
    let mut state = 0x2545_f491_u32;
    let mut next = || {
        state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        let v = (state >> 8) as f32 / (1 << 24) as f32; // [0, 1)
        if state % 97 == 0 { v * 4.0 } else { v }
    };

    let mut history: Vec<f32> = Vec::new();
    for &block in &[1usize, 7, 64, 100, 101, 3, 255, 19] {
        let l: Vec<f32> = (0..block).map(|_| next() - 0.5).collect();
        let r: Vec<f32> = (0..block).map(|_| next() - 0.5).collect();
        for i in 0..block {
            history.push(l[i].abs().max(r[i].abs()));
        }
        m.push_stereo(&l, &r);

        let tail = &history[history.len().saturating_sub(window)..];
        let peak = tail.iter().copied().fold(0.0_f32, f32::max);
        let rms =
            (tail.iter().map(|&s| (s as f64) * (s as f64)).sum::<f64>() / tail.len() as f64)
                .sqrt() as f32;
        let expected = 20.0 * (peak / rms).log10();
        let got = m.crest_db();
        assert!(
            (got - expected).abs() < 1e-3,
            "after {} samples: got {got}, expected {expected}",
            history.len()
        );
    }
}

#[test]
fn reset_clears_peak_state() {
    let mut m = CrestMeter::new(1_000.0);
    m.push_stereo(&[1.0; 50], &[1.0; 50]);
    m.reset();
    assert_eq!(m.crest_db(), 0.0);
    // A clean post-reset signal must not see the pre-reset peak.
    let l = vec![0.25_f32; 200];
    m.push_stereo(&l, &l);
    let crest = m.crest_db();
    assert!(crest.abs() < 0.01, "constant signal crest = {crest}");
}
