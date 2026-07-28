//! Live smoke test for the native PipeWire output backend (doc #260
//! finding #11). Ignored by default: it needs a running PipeWire
//! daemon and a real (or virtual) sink, opens an actual graph node and
//! plays ~0.5 s of engine silence. Run explicitly with:
//!
//! ```sh
//! cargo test -p resonance-audio --test pw_output_smoke -- --ignored
//! ```
//!
//! What it pins: `AudioEngine::new()` comes up on the native backend
//! (or falls back to cpal without erroring), survives half a second of
//! RT process callbacks, and tears down cleanly (thread-loop join, no
//! hang, no crash).

use std::time::Duration;

use resonance_audio::AudioEngine;

#[test]
#[ignore = "requires a running PipeWire daemon and an audio sink"]
fn engine_comes_up_and_tears_down_on_native_output() {
    let engine = AudioEngine::new().expect("AudioEngine::new must succeed with a live daemon");
    // Let the RT process callback run a few dozen cycles.
    std::thread::sleep(Duration::from_millis(500));
    drop(engine);
}

/// The graph-reported I/O latency surfaces through the engine query
/// (doc #260 finding #13): with the native output stream running, the
/// playback delay must be non-zero (at least the device buffering) and
/// the report must arrive promptly.
#[test]
#[ignore = "requires a running PipeWire daemon and an audio sink"]
fn io_latency_query_reports_playback_delay_on_native_output() {
    use resonance_audio::types::{AudioCommand, AudioEvent};

    let engine = AudioEngine::new().expect("AudioEngine::new must succeed with a live daemon");
    // Give the graph a few cycles to attach the stream and publish
    // pw_time (the process callback stores it every cycle).
    std::thread::sleep(Duration::from_millis(300));
    engine
        .send(AudioCommand::QueryIoLatency)
        .expect("engine accepts the query");

    let deadline = std::time::Instant::now() + Duration::from_secs(2);
    let report = loop {
        if let Some(AudioEvent::IoLatencyReport {
            capture_samples,
            playback_samples,
            round_trip_samples,
        }) = engine.try_recv()
        {
            break (capture_samples, playback_samples, round_trip_samples);
        }
        assert!(std::time::Instant::now() < deadline, "no IoLatencyReport within 2 s");
        std::thread::sleep(Duration::from_millis(10));
    };
    let (capture, playback, round_trip) = report;
    assert_eq!(capture, 0, "no input stream open — capture must read 0");
    assert!(
        playback > 0,
        "native output must report a non-zero graph delay (got {playback})"
    );
    assert_eq!(round_trip, capture + playback);
    drop(engine);
}
