//! FU-D4a: the engine control thread must not create a track unprompted
//! at startup any more.
//!
//! Before this fix, `engine_thread` inserted a literal id-1 "Track 1"
//! and sent `AudioEvent::TrackAdded { track_id: 1 }` once, before its
//! command loop ever read from `cmd_rx` (`engine/thread/mod.rs`, right
//! after the `SampleRateDetected` report). The app's own track-id
//! counter also starts at 1 (`resonance-app/src/state/ids.rs`), so a
//! user's first "Add Track" — if the GUI handled it before the app had
//! mirrored that unprompted echo — allocated id 1 too, and the engine
//! refused the resulting `AddTrack` as a collision
//! (`EngineErrorKind::Internal`): a click that visibly did nothing but
//! raise an error banner.
//!
//! Since ARCH-04 D-4 the app is the only track-id allocator; the
//! fresh-session default track is now created the same way any other
//! track is — from an `AddTrack` the app sends itself
//! (`Resonance::send_startup_default_track`, `resonance-app/src/state/ids.rs`),
//! covered app-side by `resonance-app/tests/io/startup_default_track.rs`.
//! This is the engine-side half: the thread itself must emit no track at
//! all before it ever reads a command.
//!
//! Drives the REAL `engine_thread` function via
//! `EngineHandlerHarness::startup_events` — not the harness's usual
//! piecemeal per-handler dispatch, which never ran this code either way
//! — with fake channels and no audio device, so no real cpal/PipeWire
//! stream is needed to exercise this.

use resonance_audio::test_support::EngineHandlerHarness;
use resonance_audio::types::AudioEvent;

#[test]
fn engine_thread_startup_emits_no_track() {
    let events = EngineHandlerHarness::startup_events();

    assert!(
        !events.iter().any(|e| matches!(e, AudioEvent::TrackAdded { .. })),
        "the engine thread must not create any track unprompted at \
         startup any more (FU-D4a) — the app is the only allocator now; \
         events: {events:?}"
    );

    // The rest of the startup handshake is unaffected: the sample rate
    // report still fires exactly once, same as before this fix.
    assert_eq!(
        events
            .iter()
            .filter(|e| matches!(e, AudioEvent::SampleRateDetected { .. }))
            .count(),
        1,
        "events: {events:?}"
    );
}
