use resonance_mastering::stages::linear_phase_eq::convolver::OverlapSaveConvolver;
use resonance_mastering::stages::linear_phase_eq::HOP_SIZE;

fn delta_signal(len: usize) -> Vec<f32> {
    let mut v = vec![0.0_f32; len];
    v[0] = 1.0;
    v
}

#[test]
fn delta_through_identity_filter_appears_at_reported_latency() {
    let mut c = OverlapSaveConvolver::new();
    let latency = c.latency();
    // Feed enough samples to flush the delta through.
    let n = latency + HOP_SIZE;
    let mut buf = delta_signal(n);
    c.process_in_place(&mut buf);

    // The delta must land at index `latency` with near-unit magnitude.
    assert!(
        (buf[latency] - 1.0).abs() < 1e-4,
        "expected delta at index {latency}, got {}",
        buf[latency]
    );
    // Surrounding samples must be near zero.
    let mut max_other = 0.0_f32;
    for (i, &v) in buf.iter().enumerate() {
        if i != latency {
            max_other = max_other.max(v.abs());
        }
    }
    assert!(max_other < 1e-4, "non-delta ringing = {max_other}");
}

#[test]
fn sine_through_identity_is_delayed_copy() {
    let mut c = OverlapSaveConvolver::new();
    let latency = c.latency();
    let n = latency + 2048;
    let mut buf = vec![0.0_f32; n];
    for (i, v) in buf.iter_mut().enumerate() {
        *v = (i as f32 * 0.1).sin() * 0.5;
    }
    let input = buf.clone();
    c.process_in_place(&mut buf);

    // After `latency`, the output equals input shifted by `latency`.
    let mut max_err = 0.0_f32;
    for i in latency..n {
        let expected = input[i - latency];
        let err = (buf[i] - expected).abs();
        if err > max_err {
            max_err = err;
        }
    }
    assert!(max_err < 1e-4, "identity filter error = {max_err}");
}
