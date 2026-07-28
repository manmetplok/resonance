//! Pure conversion from PipeWire `pw_time.delay` (graph time-domain
//! ticks, `rate = num/denom` seconds per tick) to engine-rate samples
//! (doc #260 finding #13, ba todo #1126).

use resonance_audio::__test_support::pw_delay_to_engine_samples;

#[test]
fn typical_graph_rate_is_identity_at_engine_rate() {
    // rate = 1/48000, engine 48 kHz: delay ticks ARE engine samples.
    assert_eq!(pw_delay_to_engine_samples(256, 1, 48_000, 48_000), 256);
    assert_eq!(pw_delay_to_engine_samples(0, 1, 48_000, 48_000), 0);
}

#[test]
fn rescales_across_rate_domains() {
    // Graph in 1/44100 ticks, engine at 48 kHz: 441 ticks = 10 ms = 480.
    assert_eq!(pw_delay_to_engine_samples(441, 1, 44_100, 48_000), 480);
    // Whole-second rate fraction (1/1): 1 tick = 1 s = 48000 samples.
    assert_eq!(pw_delay_to_engine_samples(1, 1, 1, 48_000), 48_000);
}

#[test]
fn negative_and_degenerate_inputs_clamp_to_zero() {
    // Negative delay (user latency offsets can push it below zero).
    assert_eq!(pw_delay_to_engine_samples(-100, 1, 48_000, 48_000), 0);
    // Zero denominator (unfilled pw_time) must not divide by zero.
    assert_eq!(pw_delay_to_engine_samples(100, 1, 0, 48_000), 0);
}

#[test]
fn large_delays_do_not_overflow() {
    // A pathological delay near u32::MAX ticks at 1/48000 stays exact.
    let d = u32::MAX as i64;
    assert_eq!(
        pw_delay_to_engine_samples(d, 1, 48_000, 48_000),
        u32::MAX as u64
    );
}
