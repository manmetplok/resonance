//! Output-gain smoothing in linear space: at steady state the linear-space
//! smoother must be exactly equivalent to the old per-sample
//! `db_to_linear(smoothed_db)` path, and during a ramp it must move
//! monotonically and converge onto the new linear target.

use resonance_dsp::db_to_linear;
use resonance_eq::dsp::EqDsp;
use resonance_plugin::{Smoother, SmoothingStyle};

const SR: f32 = 48_000.0;

fn make_smoother(initial_db: f32) -> Smoother {
    // Same style/time as ResonanceEq::output_gain_smoother.
    let mut s = Smoother::new(SmoothingStyle::Logarithmic(20.0));
    s.set_sample_rate(SR);
    s.reset(db_to_linear(initial_db));
    s
}

/// At steady state (target reached / never moved) every sample is scaled by
/// exactly `db_to_linear(gain_db)` — bitwise what the old per-sample
/// `db_to_linear(output_gain.next())` produced once settled.
#[test]
fn steady_state_matches_db_to_linear_of_target() {
    for gain_db in [-12.0f32, -3.5, 0.0, 6.25] {
        let mut dsp = EqDsp::new(SR); // no bands configured -> passthrough
        let mut smoother = make_smoother(gain_db);
        smoother.set_target(db_to_linear(gain_db));

        let mut left = vec![1.0f32; 512];
        let mut right = vec![-0.5f32; 512];
        dsp.process_stereo(&mut left, &mut right, &mut smoother);

        let expected = db_to_linear(gain_db);
        for i in 0..512 {
            assert_eq!(left[i], expected, "left[{i}] at {gain_db} dB");
            assert_eq!(right[i], -0.5 * expected, "right[{i}] at {gain_db} dB");
        }
    }
}

/// A retarget ramps the linear gain monotonically (no zipper steps back)
/// and converges onto the new linear target.
#[test]
fn ramp_is_monotonic_and_converges() {
    let mut dsp = EqDsp::new(SR);
    let mut smoother = make_smoother(0.0);
    smoother.set_target(db_to_linear(-18.0));

    // One second of unity input, processed in blocks like the host would.
    let mut last = f32::INFINITY;
    let mut final_gain = 0.0f32;
    for _ in 0..(SR as usize / 256) {
        let mut left = vec![1.0f32; 256];
        let mut right = vec![1.0f32; 256];
        dsp.process_stereo(&mut left, &mut right, &mut smoother);
        for (i, &v) in left.iter().enumerate() {
            assert!(
                v <= last + 1e-7,
                "gain ramp moved upwards at sample {i}: {last} -> {v}"
            );
            last = v;
        }
        final_gain = *left.last().unwrap();
    }

    let target = db_to_linear(-18.0);
    assert!(
        (final_gain - target).abs() < 1e-4,
        "ramp did not converge: {final_gain} vs target {target}"
    );
}
