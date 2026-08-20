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

#[test]
fn non_finite_or_negative_rate_freezes_instead_of_poisoning() {
    // NaN/inf/negative rates and a zero sample rate must all sanitize to
    // a zero phase increment: output stays finite and the phase freezes.
    for (rate, sr) in [
        (f32::NAN, 48_000.0),
        (f32::INFINITY, 48_000.0),
        (f32::NEG_INFINITY, 48_000.0),
        (-5.0, 48_000.0),
        (1.0, 0.0),
        (1.0, f32::NAN),
        (1.0, -48_000.0),
    ] {
        let mut lfo = Lfo::new(rate, sr, 0.25);
        let first = lfo.next();
        for _ in 0..64 {
            let v = lfo.next();
            assert!(v.is_finite(), "rate={rate}, sr={sr}: non-finite output {v}");
            assert_eq!(v, first, "rate={rate}, sr={sr}: phase should freeze");
        }
    }
}

#[test]
fn set_rate_sanitizes_like_new() {
    let mut lfo = Lfo::new(2.0, 48_000.0, 0.0);
    lfo.next();
    lfo.set_rate(f32::NAN, 48_000.0);
    let a = lfo.next();
    let b = lfo.next();
    assert!(a.is_finite() && b.is_finite());
    assert_eq!(a, b, "NaN rate should freeze the phase");

    // Recovering with a valid rate resumes advancing.
    lfo.set_rate(2.0, 48_000.0);
    let c = lfo.next();
    let d = lfo.next();
    assert!(c.is_finite() && d.is_finite());
    assert_ne!(c, d);
}

#[test]
fn non_finite_initial_phase_is_sanitized() {
    for phase in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, 2.75, -0.25] {
        let mut lfo = Lfo::new(1.0, 48_000.0, phase);
        let v = lfo.next();
        assert!(v.is_finite(), "phase={phase}: non-finite output {v}");
    }
    // Out-of-range finite phases wrap into [0, 1).
    let mut wrapped = Lfo::new(1.0, 48_000.0, 2.75);
    let mut direct = Lfo::new(1.0, 48_000.0, 0.75);
    assert_eq!(wrapped.next(), direct.next());
}

#[test]
fn reset_rewinds_to_the_constructed_phase() {
    // Not to zero: LFO banks are built with a per-channel phase stagger
    // to keep them decorrelated, so resetting to zero would collapse the
    // bank into unison rather than clear it (ba todo #1378).
    let mut lfo = Lfo::new(3.0, 48_000.0, 0.25);
    let first = lfo.next();
    for _ in 0..1_000 {
        let _ = lfo.next();
    }
    lfo.reset();
    assert_eq!(
        lfo.next().to_bits(),
        first.to_bits(),
        "reset must resume the exact sample sequence a fresh LFO produces"
    );
}

#[test]
fn reset_leaves_the_rate_alone() {
    // `clear()` on a DSP block means "forget the audio", not "forget how
    // you were configured" — the host sets rates from parameters, which
    // a reset must not silently revert.
    let mut lfo = Lfo::new(1.0, 48_000.0, 0.0);
    lfo.set_rate(200.0, 48_000.0);
    let fast: Vec<u32> = (0..8).map(|_| lfo.next().to_bits()).collect();

    lfo.reset();
    let after: Vec<u32> = (0..8).map(|_| lfo.next().to_bits()).collect();
    assert_eq!(fast, after, "reset changed the rate as well as the phase");
}

#[test]
fn reset_on_an_untouched_lfo_changes_nothing() {
    let mut fresh = Lfo::new(2.0, 48_000.0, 0.6);
    let mut reset_first = Lfo::new(2.0, 48_000.0, 0.6);
    reset_first.reset();
    for i in 0..16 {
        assert_eq!(
            fresh.next().to_bits(),
            reset_first.next().to_bits(),
            "sample {i} diverged"
        );
    }
}
