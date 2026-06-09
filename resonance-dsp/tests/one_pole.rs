use resonance_dsp::OnePole;

/// Cutoffs at or above Nyquist clamp `w` at PI, so any super-Nyquist value
/// produces the identical coefficient — there is no aliased/wrapped regime.
#[test]
fn cutoff_above_nyquist_clamps_to_same_response() {
    let sr = 48_000.0;
    let mut at_nyquist = OnePole::new();
    let mut way_above = OnePole::new();
    at_nyquist.set_cutoff(sr / 2.0, sr);
    way_above.set_cutoff(sr * 100.0, sr);

    for i in 0..64 {
        let x = ((i as f32) * 0.37).sin();
        assert_eq!(at_nyquist.process(x), way_above.process(x));
    }
}

/// At the Nyquist clamp the filter is a near-identity passthrough: a unit
/// step settles to within e^-PI (~4.3%) of the input after one sample and
/// never overshoots or blows up.
#[test]
fn nyquist_cutoff_is_stable_near_identity() {
    let sr = 48_000.0;
    let mut f = OnePole::new();
    f.set_cutoff(sr / 2.0, sr);

    let first = f.process(1.0);
    assert!(
        (1.0 - first) <= (-std::f32::consts::PI).exp() + 1e-6,
        "first step output {first} further from input than e^-PI allows"
    );
    for _ in 0..1024 {
        let y = f.process(1.0);
        assert!((0.0..=1.0).contains(&y), "unstable output {y}");
    }

    // Alternating full-scale input stays bounded.
    let mut sign = 1.0f32;
    for _ in 0..1024 {
        let y = f.process(sign);
        assert!(y.abs() <= 1.0, "unstable output {y}");
        sign = -sign;
    }
}
