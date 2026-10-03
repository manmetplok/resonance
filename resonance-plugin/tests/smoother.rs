//! Unit tests for the per-sample parameter smoother (`src/smoother.rs`).
//!
//! The smoother is what stops host automation from zippering: plugins feed
//! it the raw param value once per block and read `next()` per sample. Its
//! two contracts are that a ramp reaches the target *exactly* (never
//! asymptotically close, which would leave gain trims permanently off) and
//! that `skip(n)` is an exact stand-in for calling `next()` n times.

use resonance_plugin::{Smoother, SmoothingStyle};

const SR: f32 = 48_000.0;
/// 10 ms at 48 kHz.
const RAMP: u32 = 480;

fn linear_smoother() -> Smoother {
    let mut s = Smoother::new(SmoothingStyle::Linear(10.0));
    s.set_sample_rate(SR);
    s
}

fn log_smoother() -> Smoother {
    let mut s = Smoother::new(SmoothingStyle::Logarithmic(10.0));
    s.set_sample_rate(SR);
    s
}

// ---------------------------------------------------------------------------
// SmoothingStyle::None
// ---------------------------------------------------------------------------

#[test]
fn no_smoothing_jumps_to_the_target_immediately() {
    let mut s = Smoother::new(SmoothingStyle::None);
    s.set_sample_rate(SR);

    s.set_target(0.75);
    assert_eq!(s.current(), 0.75, "set_target must land instantly");
    assert_eq!(s.next(), 0.75);
    assert_eq!(s.next(), 0.75);

    s.set_target(-1.5);
    assert_eq!(s.current(), -1.5);
}

#[test]
fn an_unconfigured_smoother_still_smooths_at_the_default_rate() {
    // `set_sample_rate` has not been called. The smoother must neither
    // stall at 0 nor — the old behaviour (code review HOST-09) — jump
    // straight to the target with no smoothing at all: it ramps as if
    // at 44.1 kHz, and lands.
    for style in [
        SmoothingStyle::Linear(10.0),
        SmoothingStyle::Logarithmic(10.0),
    ] {
        let mut s = Smoother::new(style);
        s.set_target(0.5);
        let first = s.next();
        assert!(
            first > 0.0 && first < 0.5,
            "an unconfigured smoother must ramp, got {first}"
        );
        // 10 ms at 44.1 kHz is 441 samples.
        for _ in 1..441 {
            s.next();
        }
        assert_eq!(s.current(), 0.5, "the default-rate ramp must land");
    }
}

// ---------------------------------------------------------------------------
// Linear ramps
// ---------------------------------------------------------------------------

#[test]
fn a_linear_ramp_rises_monotonically_and_lands_exactly_on_the_target() {
    let mut s = linear_smoother();
    s.reset(0.0);
    s.set_target(1.0);

    let mut previous = 0.0_f32;
    for sample in 1..RAMP {
        let v = s.next();
        assert!(
            v > previous,
            "sample {sample}: {v} did not advance past {previous}"
        );
        assert!(v < 1.0, "sample {sample}: {v} overshot the target early");
        previous = v;
    }

    // The final sample of the ramp lands exactly on the target.
    assert_eq!(s.next(), 1.0, "the ramp must land exactly on the target");
    // …and stays there.
    assert_eq!(s.next(), 1.0);
    assert_eq!(s.current(), 1.0);
}

#[test]
fn a_linear_ramp_falls_monotonically() {
    let mut s = linear_smoother();
    s.reset(1.0);
    s.set_target(0.0);

    let mut previous = 1.0_f32;
    for _ in 1..RAMP {
        let v = s.next();
        assert!(v < previous, "{v} did not descend past {previous}");
        assert!(v > -1e-6);
        previous = v;
    }
    assert_eq!(s.next(), 0.0);
}

#[test]
fn a_linear_ramp_takes_the_full_configured_duration() {
    let mut s = linear_smoother();
    s.reset(0.0);
    s.set_target(1.0);

    // Halfway through the ramp we are halfway to the target.
    for _ in 0..RAMP / 2 {
        s.next();
    }
    assert!(
        (s.current() - 0.5).abs() < 1e-3,
        "midway through a 10 ms ramp the value should be ~0.5, got {}",
        s.current()
    );
}

#[test]
fn the_ramp_length_follows_the_sample_rate() {
    let mut s = Smoother::new(SmoothingStyle::Linear(10.0));

    // 10 ms at 96 kHz is 960 samples: still not there at 959.
    s.set_sample_rate(96_000.0);
    s.reset(0.0);
    s.set_target(1.0);
    for _ in 0..959 {
        s.next();
    }
    assert!(s.current() < 1.0, "the ramp finished too early at 96 kHz");
    assert_eq!(s.next(), 1.0);

    // Re-rating shortens it to 480 samples.
    s.set_sample_rate(48_000.0);
    s.reset(0.0);
    s.set_target(1.0);
    for _ in 0..479 {
        s.next();
    }
    assert!(s.current() < 1.0);
    assert_eq!(s.next(), 1.0);
}

#[test]
fn retargeting_mid_ramp_starts_from_the_current_value() {
    let mut s = linear_smoother();
    s.reset(0.0);
    s.set_target(1.0);
    for _ in 0..RAMP / 2 {
        s.next();
    }
    let midpoint = s.current();

    // A new target picks up from where the old ramp got to — no jump.
    s.set_target(0.0);
    let first = s.next();
    assert!(
        first < midpoint && (midpoint - first) < 0.01,
        "retargeting jumped from {midpoint} to {first}"
    );

    for _ in 1..RAMP {
        s.next();
    }
    assert_eq!(s.current(), 0.0);
}

/// The per-block calling pattern: the owner re-sends the unchanged param
/// value every block. The ramp must still land on the target exactly, on
/// the sample it would have without the repeats.
#[test]
fn re_sending_the_same_target_every_block_still_lands_on_time() {
    for make in [linear_smoother as fn() -> Smoother, log_smoother] {
        let mut once = make();
        once.reset(0.0);
        once.set_target(1.0);
        let mut every_block = make();
        every_block.reset(0.0);
        for i in 0..RAMP + 64 {
            if i % 64 == 0 {
                every_block.set_target(1.0);
            }
            assert_eq!(
                every_block.next().to_bits(),
                once.next().to_bits(),
                "sample {i}: a repeated target restarted the ramp"
            );
        }
        assert_eq!(every_block.current(), 1.0, "the ramp never landed");
    }
}

// ---------------------------------------------------------------------------
// Logarithmic ramps
// ---------------------------------------------------------------------------

#[test]
fn a_logarithmic_ramp_rises_monotonically_and_lands_exactly_on_the_target() {
    let mut s = log_smoother();
    s.reset(0.0);
    s.set_target(1.0);

    let mut previous = 0.0_f32;
    for sample in 1..RAMP {
        let v = s.next();
        assert!(
            v > previous,
            "sample {sample}: {v} did not advance past {previous}"
        );
        assert!(v < 1.0, "sample {sample}: {v} overshot the target early");
        previous = v;
    }

    // The ramp converges to within 1e-4 of the distance by the end of
    // the nominal ramp, so the final snap onto the target is inaudible.
    assert!(
        1.0 - previous < 2e-4,
        "expected convergence to within 1e-4 one sample before the end, got {previous}"
    );
    assert_eq!(s.next(), 1.0);
    assert_eq!(s.next(), 1.0);
}

/// The final snap onto the target is inaudible (code review HOST-09):
/// the old coefficient reached only ~95 % of the way, then jumped the
/// last 5 % in one sample — a −26 dB click on a 0 → 1 gain step, larger
/// than any step of the ramp's tail.
#[test]
fn a_logarithmic_ramp_never_ends_with_a_jump() {
    let mut s = log_smoother();
    s.reset(0.0);
    s.set_target(1.0);
    let mut previous = 0.0_f32;
    let mut steps = Vec::with_capacity(RAMP as usize);
    for _ in 0..RAMP {
        let v = s.next();
        steps.push(v - previous);
        previous = v;
    }
    assert_eq!(previous, 1.0);
    let last = *steps.last().unwrap();
    // 1e-4 of the distance (−80 dB), plus f32 rounding.
    assert!(last <= 2e-4, "final snap of {last} onto the target");
    // The largest step is the first, bounded by the coefficient.
    let max = steps.iter().cloned().fold(0.0f32, f32::max);
    assert!(max < 10.0 / RAMP as f32, "max per-sample step {max}");
}

#[test]
fn a_logarithmic_ramp_falls_monotonically() {
    let mut s = log_smoother();
    s.reset(2.0);
    s.set_target(0.5);

    let mut previous = 2.0_f32;
    for _ in 1..RAMP {
        let v = s.next();
        assert!(v < previous, "{v} did not descend past {previous}");
        assert!(v > 0.5 - 1e-6);
        previous = v;
    }
    assert_eq!(s.next(), 0.5);
}

// ---------------------------------------------------------------------------
// reset
// ---------------------------------------------------------------------------

#[test]
fn reset_cancels_any_ramp_in_flight() {
    for mut s in [linear_smoother(), log_smoother()] {
        s.reset(0.0);
        s.set_target(1.0);
        s.next();
        assert!(s.current() > 0.0 && s.current() < 1.0);

        s.reset(0.25);
        assert_eq!(s.current(), 0.25, "reset must land immediately");
        // The target moved too, so nothing ramps afterwards.
        assert_eq!(s.next(), 0.25);
        assert_eq!(s.next(), 0.25);
    }
}

#[test]
fn current_does_not_advance_the_ramp() {
    let mut s = linear_smoother();
    s.reset(0.0);
    s.set_target(1.0);

    s.next();
    let v = s.current();
    assert_eq!(s.current(), v);
    assert_eq!(s.current(), v);
    // Only next() moves it.
    assert!(s.next() > v);
}

// ---------------------------------------------------------------------------
// skip: the analytic fast-forward must equal the per-sample loop
// ---------------------------------------------------------------------------

/// Tolerance for skip-vs-loop: the loop accumulates f32 rounding over
/// hundreds of iterations, the closed form does not.
const SKIP_EPS: f32 = 5e-4;

fn assert_skip_matches_loop(make: fn() -> Smoother, from: f32, to: f32, n: u32) {
    let mut looped = make();
    looped.reset(from);
    looped.set_target(to);
    for _ in 0..n {
        looped.next();
    }

    let mut skipped = make();
    skipped.reset(from);
    skipped.set_target(to);
    skipped.skip(n);

    assert!(
        (looped.current() - skipped.current()).abs() <= SKIP_EPS,
        "skip({n}) from {from} to {to} gave {} but {n} x next() gave {}",
        skipped.current(),
        looped.current()
    );
}

#[test]
fn skip_matches_the_per_sample_loop_for_linear_ramps() {
    for n in [1, 7, 100, RAMP / 2, RAMP - 1, RAMP, RAMP + 1, RAMP * 3] {
        assert_skip_matches_loop(linear_smoother, 0.0, 1.0, n);
        assert_skip_matches_loop(linear_smoother, 1.0, -1.0, n);
    }
}

#[test]
fn skip_matches_the_per_sample_loop_for_logarithmic_ramps() {
    for n in [1, 7, 100, RAMP / 2, RAMP - 1, RAMP, RAMP + 1, RAMP * 3] {
        assert_skip_matches_loop(log_smoother, 0.0, 1.0, n);
        assert_skip_matches_loop(log_smoother, 2.0, 0.5, n);
    }
}

#[test]
fn skipping_the_whole_ramp_lands_exactly_on_the_target() {
    for make in [linear_smoother as fn() -> Smoother, log_smoother] {
        let mut s = make();
        s.reset(0.0);
        s.set_target(0.8);
        s.skip(RAMP);
        assert_eq!(s.current(), 0.8);

        // Skipping past the end stays pinned.
        s.skip(10_000);
        assert_eq!(s.current(), 0.8);
    }
}

#[test]
fn skipping_zero_samples_is_a_no_op() {
    let mut s = linear_smoother();
    s.reset(0.0);
    s.set_target(1.0);
    s.skip(0);
    assert_eq!(s.current(), 0.0, "skip(0) must not advance the ramp");

    // …and the ramp still takes its full length afterwards.
    s.skip(RAMP - 1);
    assert!(s.current() < 1.0);
    s.skip(1);
    assert_eq!(s.current(), 1.0);
}

#[test]
fn a_partial_skip_leaves_the_rest_of_the_ramp_intact() {
    let mut s = linear_smoother();
    s.reset(0.0);
    s.set_target(1.0);

    s.skip(RAMP / 4);
    assert!((s.current() - 0.25).abs() < 1e-3, "got {}", s.current());
    s.skip(RAMP / 4);
    assert!((s.current() - 0.5).abs() < 1e-3, "got {}", s.current());

    // The remaining half still needs exactly RAMP/2 samples.
    for _ in 0..RAMP / 2 - 1 {
        s.next();
    }
    assert!(s.current() < 1.0);
    assert_eq!(s.next(), 1.0);
}

#[test]
fn skipping_an_idle_smoother_pins_it_to_the_target() {
    let mut s = linear_smoother();
    s.reset(0.3);
    // No ramp in flight.
    s.skip(64);
    assert_eq!(s.current(), 0.3);
}
