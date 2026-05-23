//! Regression for the silent-drop bug fixed alongside this test:
//! `AudioEngine::send` used to return `()` and quietly throw away
//! commands once the engine thread had been joined. The fix changed
//! the signature to `Result<(), EngineSendError>` and made it
//! `#[must_use]`, so call sites can no longer ignore a disconnect by
//! accident.
//!
//! This test uses `AudioEngine::for_test_disconnected`, a `#[doc(hidden)]`
//! constructor that builds an engine handle with the command-channel
//! receiver already dropped — no cpal stream, no engine thread, no
//! audio device required. That keeps the test deterministic and
//! headless-CI-friendly while still driving real `AudioEngine::send`
//! through its disconnect branch.

use resonance_audio::types::AudioCommand;
use resonance_audio::AudioEngine;

#[test]
fn send_returns_err_when_command_channel_disconnected() {
    // The shared one-shot stderr latch is process-wide; reset it so
    // the test doesn't depend on whether some other integration test
    // already tripped it.
    resonance_audio::__test_support::__reset_engine_disconnect_latch_for_test();

    let engine = AudioEngine::for_test_disconnected();

    let result = engine.send(AudioCommand::Play);
    let err = result.expect_err("send must report disconnect when the engine receiver is dropped");

    // The error carries the original command so callers can retry /
    // surface a UI message without reconstructing it.
    assert!(
        matches!(err.0, AudioCommand::Play),
        "expected EngineSendError to carry back the original Play command, got {:?}",
        err.0
    );
}

#[test]
fn send_disconnect_is_sticky() {
    // Once the receiver is dropped, every subsequent send must also
    // fail — there is no "auto-reconnect", and call sites shouldn't be
    // tempted to retry hoping the channel comes back.
    resonance_audio::__test_support::__reset_engine_disconnect_latch_for_test();

    let engine = AudioEngine::for_test_disconnected();

    for _ in 0..16 {
        assert!(
            engine.send(AudioCommand::Stop).is_err(),
            "every send after disconnect must report failure"
        );
    }
}

#[test]
fn engine_send_error_display_includes_command_name() {
    // Wraps the original command in its Debug form so a log line is
    // actually useful for diagnosing which command was dropped.
    resonance_audio::__test_support::__reset_engine_disconnect_latch_for_test();

    let engine = AudioEngine::for_test_disconnected();
    let err = engine.send(AudioCommand::Pause).unwrap_err();

    let rendered = format!("{err}");
    assert!(
        rendered.contains("Pause"),
        "Display impl should mention the dropped command (got: {rendered:?})"
    );
    assert!(
        rendered.contains("disconnected"),
        "Display impl should mention disconnect (got: {rendered:?})"
    );
}
