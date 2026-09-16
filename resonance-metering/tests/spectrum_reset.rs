//! Functional coverage for [`SpectrumAnalyzer::reset`].
//!
//! `reset()` is called from the producer (audio) thread — CLAP `reset()`
//! in the eq and mastering plugins — so it must not touch the ring's
//! consumer index itself. It sets a clear request that the FFT worker
//! services on its own thread: drop pending samples, zero the FFT
//! accumulation state, publish a silent snapshot. This test drives the
//! real worker end to end and asserts the reset actually lands.

mod common;

use std::time::{Duration, Instant};

use resonance_metering::spectrum::fft_worker::FLOOR_DB;
use resonance_metering::SpectrumAnalyzer;

const SAMPLE_RATE: f32 = 48_000.0;

#[test]
fn reset_empties_worker_state_and_publishes_silence() {
    let analyzer = SpectrumAnalyzer::spawn(SAMPLE_RATE);
    let handle = analyzer.handle();

    // Feed a loud 1 kHz sine in audio-sized blocks until the worker has
    // drained enough for an FFT and published non-silent bars.
    let (left, right) = common::sine_mono(SAMPLE_RATE, 1_000.0, -6.0, 0.05);
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        analyzer.push_stereo(&left, &right);
        let snap = handle.latest();
        if snap.magnitudes_db.iter().any(|&db| db > FLOOR_DB + 6.0) {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "worker never published a non-silent snapshot"
        );
        std::thread::sleep(Duration::from_millis(2));
    }

    // Reset from this thread — the producer side — and stop feeding.
    analyzer.reset();

    // The worker services the clear within one poll (~16 ms) and must
    // publish a snapshot with every bar at the floor. This can only be
    // the reset path: peak-hold decay runs per FFT frame, and with no
    // further input no frames run, so without the reset the bars would
    // stay loud forever.
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let snap = handle.latest();
        if snap.magnitudes_db.iter().all(|&db| db == FLOOR_DB) {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "reset never reached the worker: bars still above the floor"
        );
        std::thread::sleep(Duration::from_millis(4));
    }

    // The analyzer must keep working after a reset: feed again and the
    // bars must come back up (the clear is one-shot, not a latch).
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        analyzer.push_stereo(&left, &right);
        let snap = handle.latest();
        if snap.magnitudes_db.iter().any(|&db| db > FLOOR_DB + 6.0) {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "analyzer never recovered after reset"
        );
        std::thread::sleep(Duration::from_millis(2));
    }
}
