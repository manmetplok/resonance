//! Tests for the external-instrument round-trip latency auto-detect ("ping")
//! math (todo #453, doc #169).
//!
//! The hardware-touching part of the ping — opening the capture stream, firing
//! the MIDI impulse, draining the ring on the engine thread — can't run
//! headless. But the *measurement* itself is a set of small pure functions
//! (`estimate_noise_floor`, `detect_impulse_onset`, `onset_to_engine_samples`,
//! `onset_to_ms`), exposed via `#[doc(hidden)]` re-exports. Those are the heart
//! of the auto-detect: given a captured buffer they decide where the returned
//! impulse starts and convert that onset into the sample/ms result the engine
//! applies and reports. Exercising them directly here proves a clean return is
//! found at a plausible offset, a silent/absent return is rejected (the clean
//! failure path, not a hang), and the rate conversions are correct.

use resonance_audio::{
    detect_impulse_onset, estimate_noise_floor, onset_to_engine_samples, onset_to_ms, OnsetOutcome,
};

/// Build a captured buffer: `lead` frames of low-level noise (amplitude
/// `noise`), then a sharp impulse of amplitude `impulse` starting at
/// `onset`, the rest silence. Models "synth return arrives at frame `onset`".
fn buffer_with_impulse(
    len: usize,
    lead: usize,
    onset: usize,
    noise: f32,
    impulse: f32,
) -> Vec<f32> {
    let mut buf = vec![0.0f32; len];
    // Steady low-level noise across the whole lead-in (alternating sign so the
    // mean-abs floor estimate is exactly `noise`).
    for (i, s) in buf.iter_mut().enumerate().take(lead) {
        *s = if i % 2 == 0 { noise } else { -noise };
    }
    if onset < len {
        buf[onset] = impulse;
    }
    buf
}

#[test]
fn noise_floor_is_mean_abs_over_lead_in() {
    // Lead-in of 4 frames, all magnitude 0.01 -> floor 0.01.
    let buf = buffer_with_impulse(100, 4, 50, 0.01, 0.9);
    assert_eq!(estimate_noise_floor(&buf, 4), Some(0.01));
}

#[test]
fn noise_floor_needs_enough_frames() {
    // Fewer than `lead_in` samples -> can't estimate yet.
    let buf = vec![0.5f32; 3];
    assert_eq!(estimate_noise_floor(&buf, 4), None);
    // Zero lead-in is meaningless -> None (guards a div-by-zero).
    assert_eq!(estimate_noise_floor(&buf, 0), None);
}

#[test]
fn detects_clean_impulse_at_its_onset() {
    // Quiet lead-in then a loud impulse at frame 200: found exactly there.
    let buf = buffer_with_impulse(1000, 16, 200, 0.001, 0.8);
    match detect_impulse_onset(&buf, 16) {
        OnsetOutcome::Found(idx) => assert_eq!(idx, 200, "onset is the impulse frame"),
        other => panic!("expected Found(200), got {other:?}"),
    }
}

#[test]
fn detection_ignores_steady_input_hum_below_trigger() {
    // A steady hum at 0.01 across the lead-in raises the floor; a return that
    // never rises 8x above it (max 0.05) must NOT trigger -> NotFound, which
    // at the deadline is the clean "no detectable return" failure.
    let mut buf = vec![0.0f32; 500];
    for (i, s) in buf.iter_mut().enumerate() {
        *s = if i % 2 == 0 { 0.04 } else { -0.04 };
    }
    // floor = 0.04, trigger = max(0.04*8, 0.02) = 0.32; nothing reaches it.
    assert_eq!(detect_impulse_onset(&buf, 16), OnsetOutcome::NotFound);
}

#[test]
fn dead_silent_input_does_not_false_trigger_on_dither() {
    // Near-zero floor -> trigger falls back to the absolute MIN_ABS (0.02).
    // A tiny 0.005 dither blip must stay below it.
    let mut buf = vec![0.0f32; 400];
    buf[100] = 0.005;
    buf[250] = -0.004;
    assert_eq!(
        detect_impulse_onset(&buf, 16),
        OnsetOutcome::NotFound,
        "sub-MIN_ABS dither never counts as a return"
    );
}

#[test]
fn dead_silent_input_still_detects_a_real_return() {
    // Same near-silent floor, but a genuine 0.5 impulse clears MIN_ABS.
    let buf = buffer_with_impulse(400, 16, 130, 0.0, 0.5);
    assert_eq!(detect_impulse_onset(&buf, 16), OnsetOutcome::Found(130));
}

#[test]
fn too_short_buffer_asks_for_more_not_failure() {
    // Below the lead-in length the detector must say NeedMore so the poll loop
    // keeps accumulating instead of declaring failure prematurely.
    let buf = vec![0.0f32; 8];
    assert_eq!(detect_impulse_onset(&buf, 16), OnsetOutcome::NeedMore);
}

#[test]
fn onset_within_lead_in_is_not_reported() {
    // Energy strictly inside the lead-in window is treated as part of the
    // noise estimate, never as the onset (search starts at `lead_in`).
    let mut buf = vec![0.0f32; 300];
    buf[3] = 0.9; // inside the 16-frame lead-in
                  // No energy after the lead-in -> nothing to find.
    assert_eq!(detect_impulse_onset(&buf, 16), OnsetOutcome::NotFound);
}

#[test]
fn onset_to_engine_samples_rescales_between_rates() {
    // 480 input frames @ 48k == 441 engine frames @ 44.1k (rounded).
    assert_eq!(onset_to_engine_samples(480, 48_000, 44_100), 441);
    // Equal rates pass straight through.
    assert_eq!(onset_to_engine_samples(512, 48_000, 48_000), 512);
    // 44.1k capture up to 48k engine.
    assert_eq!(onset_to_engine_samples(441, 44_100, 48_000), 480);
}

#[test]
fn onset_to_engine_samples_guards_zero_input_rate() {
    // A bogus 0 input rate must not divide-by-zero; fall back to raw frames.
    assert_eq!(onset_to_engine_samples(123, 0, 48_000), 123);
}

#[test]
fn onset_to_ms_is_rate_relative() {
    // 480 frames @ 48k == 10 ms.
    assert!((onset_to_ms(480, 48_000) - 10.0).abs() < 1e-4);
    // 2205 frames @ 44.1k == 50 ms.
    assert!((onset_to_ms(2205, 44_100) - 50.0).abs() < 1e-4);
    // Zero rate guarded.
    assert_eq!(onset_to_ms(100, 0), 0.0);
}

#[test]
fn end_to_end_measurement_is_plausible() {
    // A realistic capture: 64 ms round-trip @ 48k = 3072 frames before the
    // return. Detector finds it, conversion reports ~64 ms — a plausible
    // outboard-synth latency, matching acceptance criterion 1.
    let onset = 3072;
    let buf = buffer_with_impulse(48_000, 64, onset, 0.002, 0.7);
    let OnsetOutcome::Found(idx) = detect_impulse_onset(&buf, 64) else {
        panic!("expected a detected return");
    };
    let ms = onset_to_ms(idx, 48_000);
    assert!((ms - 64.0).abs() < 0.5, "round-trip ~64 ms, got {ms}");
    // Engine runs at 44.1k here: the applied sample offset rescales.
    let samples = onset_to_engine_samples(idx, 48_000, 44_100);
    assert_eq!(samples, 2822); // 3072 * 44100/48000, rounded
}
