//! Auto-detect ("ping") latency message + event coverage for
//! external-instrument tracks (architecture doc #169 / #204 / #251, engine
//! side todo #453, this app-state half todo #1068).
//!
//! Drives [`ExternalInstrumentMessage::DetectLatency`] and the two engine
//! result events through the public reducer + `test_apply_engine_event`,
//! asserting the transient runtime state (`latency_detect_in_progress` /
//! `latency_detect_error`) the inspector will surface (view lands in #1069).
//! The AudioCommand the handler emits goes to the real (idle) engine queue —
//! consistent with the other reducer tests in this crate, which assert
//! observable state rather than capturing the command stream. Here the
//! `latency_detect_in_progress` flag *is* the observable proof the dispatch
//! path (and therefore the `DetectExternalInstrumentLatency` send) ran.

use resonance_app::message::{ExternalInstrumentMessage as Eim, Message};
use resonance_app::state::TrackState;
use resonance_app::undo::{classify, UndoAction};
use resonance_app::Resonance;
use resonance_audio::types::{AudioEvent, TrackId};

const TRACK: TrackId = 1;

/// Fresh app with an active project and a single instrument track ready to be
/// wired as an external instrument.
fn app_with_track() -> Resonance {
    let (mut app, _task) = Resonance::new();
    app.test_set_active_project(true);
    app.test_push_track(TrackState::new_instrument(TRACK, 0));
    app
}

/// Fresh app whose single track is already in external-instrument mode.
fn app_with_external_track() -> Resonance {
    let mut app = app_with_track();
    dispatch(&mut app, Eim::Enable(TRACK));
    app
}

fn dispatch(app: &mut Resonance, m: Eim) {
    let _ = app.update(Message::ExternalInstrument(m));
}

#[test]
fn detect_latency_sets_in_progress_flag() {
    // Press the "auto-detect" affordance on an external track with a stopped
    // transport: the handler dispatches DetectExternalInstrumentLatency (proven
    // by the flag flip) and marks the detect in-flight.
    let mut app = app_with_external_track();
    let before = app.test_external_instrument(TRACK).unwrap();
    assert!(
        !before.latency_detect_in_progress,
        "no detect running before the press"
    );

    dispatch(&mut app, Eim::DetectLatency(TRACK));

    let after = app.test_external_instrument(TRACK).unwrap();
    assert!(
        after.latency_detect_in_progress,
        "press marks the auto-detect in flight"
    );
}

#[test]
fn detect_latency_clears_stale_error() {
    // A prior failure left an error string; kicking off a fresh detect must
    // supersede it (the inspector shouldn't show a stale failure while the new
    // ping is measuring).
    let mut app = app_with_external_track();
    app.test_apply_engine_event(AudioEvent::ExternalInstrumentLatencyDetectFailed {
        track_id: TRACK,
        reason: "MIDI output offline.".into(),
    });
    assert!(app
        .test_external_instrument(TRACK)
        .unwrap()
        .latency_detect_error
        .is_some());

    dispatch(&mut app, Eim::DetectLatency(TRACK));

    let ext = app.test_external_instrument(TRACK).unwrap();
    assert!(ext.latency_detect_in_progress);
    assert_eq!(
        ext.latency_detect_error, None,
        "a fresh detect drops the stale failure reason"
    );
}

#[test]
fn detect_latency_on_non_external_track_is_noop() {
    // The track exists but was never made external — pressing detect must not
    // conjure a mirror entry or otherwise mutate state.
    let mut app = app_with_track();
    assert!(app.test_external_instrument(TRACK).is_none());

    dispatch(&mut app, Eim::DetectLatency(TRACK));

    assert!(
        app.test_external_instrument(TRACK).is_none(),
        "no mirror entry conjured for a non-external track"
    );
}

#[test]
fn detect_latency_while_measuring_is_noop() {
    // Pressing detect again while a ping is already in flight must not restart
    // it or clobber the in-progress guard — the engine gets exactly one ping.
    let mut app = app_with_external_track();
    dispatch(&mut app, Eim::DetectLatency(TRACK));
    assert!(app
        .test_external_instrument(TRACK)
        .unwrap()
        .latency_detect_in_progress);

    // Second press mid-measure: still in progress, unchanged.
    dispatch(&mut app, Eim::DetectLatency(TRACK));
    assert!(
        app.test_external_instrument(TRACK)
            .unwrap()
            .latency_detect_in_progress,
        "a second press while measuring stays in the single in-flight state"
    );
}

#[test]
fn detect_latency_while_playing_is_noop() {
    // The engine requires a stopped transport for the ping. Pressing detect
    // during playback must be a no-op app-side — no command, no flag flip.
    let mut app = app_with_external_track();
    app.test_set_transport_playing(true);

    dispatch(&mut app, Eim::DetectLatency(TRACK));

    assert!(
        !app.test_external_instrument(TRACK)
            .unwrap()
            .latency_detect_in_progress,
        "detect is suppressed while the transport is playing"
    );
}

#[test]
fn measured_event_applies_offset_and_clears_progress() {
    // The successful measurement folds the engine-applied offset into the
    // mirror AND resolves the in-flight detect (flag + any stale error clear).
    let mut app = app_with_external_track();
    // Seed a stale error to prove the success path clears it too.
    app.test_apply_engine_event(AudioEvent::ExternalInstrumentLatencyDetectFailed {
        track_id: TRACK,
        reason: "previous attempt failed".into(),
    });
    dispatch(&mut app, Eim::DetectLatency(TRACK));
    assert!(app
        .test_external_instrument(TRACK)
        .unwrap()
        .latency_detect_in_progress);

    app.test_apply_engine_event(AudioEvent::ExternalInstrumentLatencyMeasured {
        track_id: TRACK,
        latency_samples: 2822,
        latency_ms: 64.0,
    });

    let ext = app.test_external_instrument(TRACK).unwrap();
    assert_eq!(
        ext.latency_offset_samples, 2822,
        "measured offset folds into the mirror (and PDC)"
    );
    assert!(
        !ext.latency_detect_in_progress,
        "success resolves the in-flight detect"
    );
    assert_eq!(
        ext.latency_detect_error, None,
        "success clears any stale failure reason"
    );
}

#[test]
fn failed_event_stores_reason_and_clears_progress() {
    // A clean failure leaves the offset untouched but stores the reason and
    // clears the in-flight guard so the inspector stops showing "measuring…".
    let mut app = app_with_external_track();
    dispatch(&mut app, Eim::DetectLatency(TRACK));
    assert!(app
        .test_external_instrument(TRACK)
        .unwrap()
        .latency_detect_in_progress);

    app.test_apply_engine_event(AudioEvent::ExternalInstrumentLatencyDetectFailed {
        track_id: TRACK,
        reason: "No return detected within the listen window.".into(),
    });

    let ext = app.test_external_instrument(TRACK).unwrap();
    assert!(
        !ext.latency_detect_in_progress,
        "failure resolves the in-flight detect"
    );
    assert_eq!(
        ext.latency_detect_error.as_deref(),
        Some("No return detected within the listen window."),
        "the failure reason is surfaced to the inspector"
    );
    assert_eq!(
        ext.latency_offset_samples, 0,
        "a failed ping leaves the stored offset untouched"
    );
}

#[test]
fn detect_latency_records_no_undo_step() {
    // DetectLatency is a runtime-only ping: it never records an undo entry (the
    // measured offset arrives via a separate engine event, mirrored into
    // runtime-only state).
    assert!(
        matches!(
            classify(&Message::ExternalInstrument(Eim::DetectLatency(TRACK))),
            UndoAction::Skip
        ),
        "auto-detect latency must not record an undo entry"
    );
}
