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
