//! `AudioEvent::RecordingOverflow` must reach the user (silent-data-loss
//! fix): the engine emits it when the capture ring dropped frames during
//! an active recording, and the app surfaces it on the standard error
//! banner — the same surface `AudioEvent::Error` uses — so a damaged,
//! time-compressed take is visibly flagged the moment it happens instead
//! of masquerading as a healthy recording.

use resonance_app::Resonance;
use resonance_audio::types::AudioEvent;

#[test]
fn recording_overflow_event_sets_the_error_banner() {
    let (mut app, _task) = Resonance::new_for_test();
    assert!(
        app.test_error_message().is_none(),
        "fresh app starts without an error banner"
    );

    app.test_apply_engine_event(AudioEvent::RecordingOverflow {
        dropped_frames: 4096,
    });

    let msg = app
        .test_error_message()
        .expect("a recording overflow must surface a user-visible message");
    assert!(
        msg.contains("4096"),
        "the banner reports how many frames were lost, got {msg:?}"
    );
    assert!(
        msg.to_lowercase().contains("dropped"),
        "the banner says audio was dropped, got {msg:?}"
    );
}
