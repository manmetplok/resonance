use resonance_audio::decode::{linear_resample, StreamingLinearResampler};

fn sine_44_1k(frames: usize) -> Vec<f32> {
    let mut v = Vec::with_capacity(frames * 2);
    for i in 0..frames {
        let t = i as f32 / 44_100.0;
        let s = (2.0 * std::f32::consts::PI * 440.0 * t).sin();
        v.push(s);
        v.push(s);
    }
    v
}

#[test]
fn streaming_matches_oneshot_within_f64_phase() {
    // Long enough to exercise many chunk boundaries.
    let input = sine_44_1k(20_000);
    let oneshot = linear_resample(&input, 44_100, 48_000);

    let mut streamed = Vec::new();
    let mut r = StreamingLinearResampler::new(44_100, 48_000);
    for chunk in input.chunks(512 * 2) {
        r.process(chunk, &mut streamed);
    }
    r.flush(&mut streamed);

    // Output lengths may differ by at most one frame because the
    // one-shot floors the target frame count up front; compare
    // over the shared prefix.
    let common = oneshot.len().min(streamed.len());
    assert!(common > 0);
    let mut max_err = 0.0f32;
    for i in 0..common {
        let d = (oneshot[i] - streamed[i]).abs();
        if d > max_err {
            max_err = d;
        }
    }
    assert!(max_err < 1e-4, "max_err = {}", max_err);
}

#[test]
fn streaming_no_chunk_boundary_clicks() {
    // The derivative between consecutive samples should be
    // bounded by what the continuous sine yields; a chunk-
    // boundary discontinuity would show up as an outlier.
    let input = sine_44_1k(10_000);
    let mut streamed = Vec::new();
    let mut r = StreamingLinearResampler::new(44_100, 48_000);
    for chunk in input.chunks(512 * 2) {
        r.process(chunk, &mut streamed);
    }
    r.flush(&mut streamed);

    let mut max_delta = 0.0f32;
    for w in streamed.chunks(2).collect::<Vec<_>>().windows(2) {
        let d = (w[1][0] - w[0][0]).abs();
        if d > max_delta {
            max_delta = d;
        }
    }
    // One sample of a 440 Hz sine at 48 kHz is at most
    // ~0.0577 in amplitude delta; allow 2x for interpolation.
    assert!(max_delta < 0.12, "max_delta = {}", max_delta);
}

#[test]
fn passthrough_when_rates_match() {
    let input = sine_44_1k(1_000);
    let mut out = Vec::new();
    let mut r = StreamingLinearResampler::new(44_100, 44_100);
    r.process(&input, &mut out);
    r.flush(&mut out);
    // Passthrough emits all but the final frame from process(),
    // then the final frame from flush().
    assert_eq!(out.len(), input.len());
    for (a, b) in out.iter().zip(input.iter()) {
        assert!((a - b).abs() < 1e-6);
    }
}
