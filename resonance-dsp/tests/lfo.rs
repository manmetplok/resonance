use resonance_dsp::Lfo;

#[test]
fn table_matches_sin_within_1e_3() {
    // Spot-check at 200 phases across one cycle. The real `sin`
    // comparison guards against regressions in the const table
    // builder, which can't use `f32::sin` directly.
    //
    // We drive the public Lfo API by setting initial_phase = i/200 and
    // taking the first sample (`next()` returns the value at the current
    // phase before advancing).
    for i in 0..200 {
        let phase = i as f32 / 200.0;
        let mut lfo = Lfo::new(1.0, 48_000.0, phase);
        let table = lfo.next();
        let expected = (phase * 2.0 * std::f32::consts::PI).sin();
        let err = (table - expected).abs();
        assert!(
            err < 1e-3,
            "phase={phase}: table={table}, sin={expected}, err={err}"
        );
    }
}

#[test]
fn next_advances_phase() {
    let mut lfo = Lfo::new(1.0, 48_000.0, 0.0);
    let a = lfo.next();
    let b = lfo.next();
    assert!(a.is_finite() && b.is_finite());
    assert!(a != b);
}
