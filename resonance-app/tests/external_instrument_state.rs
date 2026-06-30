//! Reducer + mirroring coverage for external-instrument tracks
//! (architecture doc #169, epic #39, ba todo #454).
//!
//! Exercises the app-side state machine without standing up a real audio
//! engine: each [`ExternalInstrumentMessage`] is driven through the public
//! `update()` reducer and the resulting GUI state asserted; engine→app
//! mirroring is driven through `test_apply_engine_event`. The AudioCommands
//! the handlers emit go to the real (idle) engine queue — consistent with
//! every other reducer test in this crate, which assert observable state
//! rather than capturing the command stream.

use resonance_app::message::{ExternalInstrumentMessage as Eim, Message};
use resonance_app::state::{ExternalInstrumentStatus, TrackState};
use resonance_app::undo::{classify, UndoAction};
use resonance_app::Resonance;
use resonance_audio::types::{AudioEvent, TrackId};
use resonance_common::ExternalInstrument;

const TRACK: TrackId = 1;

/// Fresh app with an active project and a single instrument track ready to
/// be wired as an external instrument.
fn app_with_track() -> Resonance {
    let (mut app, _task) = Resonance::new();
    app.test_set_active_project(true);
    app.test_push_track(TrackState::new_instrument(TRACK, 0));
    app
}

fn dispatch(app: &mut Resonance, m: Eim) {
    let _ = app.update(Message::ExternalInstrument(m));
}

#[test]
fn enable_then_status_walks_unconfigured_to_live() {
    let mut app = app_with_track();

    // Not external yet.
    assert!(app.test_external_instrument(TRACK).is_none());

    dispatch(&mut app, Eim::Enable(TRACK));
    assert!(
        app.test_external_instrument(TRACK).is_some(),
        "Enable marks the track external"
    );
    assert_eq!(
        app.test_external_instrument_status(TRACK),
        Some(ExternalInstrumentStatus::Unconfigured),
        "no MIDI out yet"
    );

    dispatch(&mut app, Eim::SetMidiOutDevice(TRACK, Some("Moog Muse".into())));
    assert_eq!(
        app.test_external_instrument_status(TRACK),
        Some(ExternalInstrumentStatus::Configuring),
        "MIDI out set, return + monitor missing"
    );

    dispatch(
        &mut app,
        Eim::SetReturnDevice(TRACK, Some("Scarlett 18i20".into())),
    );
    assert_eq!(
        app.test_external_instrument_status(TRACK),
        Some(ExternalInstrumentStatus::Configuring),
        "return set but monitoring still off"
    );

    dispatch(&mut app, Eim::ToggleMonitor(TRACK));
    assert_eq!(
        app.test_external_instrument_status(TRACK),
        Some(ExternalInstrumentStatus::Live),
        "paired + monitoring => Live"
    );
}

#[test]
fn route_and_channel_mirror_onto_track_state() {
    let mut app = app_with_track();
    dispatch(&mut app, Eim::Enable(TRACK));

    dispatch(&mut app, Eim::SetMidiOutDevice(TRACK, Some("Moog Muse".into())));
    dispatch(&mut app, Eim::SetMidiOutChannel(TRACK, Some(2)));
    dispatch(
        &mut app,
        Eim::SetReturnDevice(TRACK, Some("Scarlett 18i20".into())),
    );
    dispatch(&mut app, Eim::SetReturnPort(TRACK, 2));

    let track = app
        .test_registry()
        .tracks
        .iter()
        .find(|t| t.id == TRACK)
        .unwrap();
    assert_eq!(track.midi_output_device.as_deref(), Some("Moog Muse"));
    assert_eq!(track.midi_output_channel, Some(2));
    assert_eq!(track.input_device_name.as_deref(), Some("Scarlett 18i20"));
    assert_eq!(track.input_port_index, 2);
}

#[test]
fn patch_and_latency_update_external_config() {
    let mut app = app_with_track();
    dispatch(&mut app, Eim::Enable(TRACK));

    dispatch(&mut app, Eim::SetBank(TRACK, Some(0x0102)));
    dispatch(&mut app, Eim::SetProgram(TRACK, Some(12)));
    dispatch(&mut app, Eim::SetLatencyOffset(TRACK, 256));

    let ext = app.test_external_instrument(TRACK).unwrap();
    assert_eq!(ext.bank, Some(0x0102));
    assert_eq!(ext.program, Some(12));
    assert_eq!(ext.latency_offset_samples, 256);
}

#[test]
fn disable_drops_external_mode() {
    let mut app = app_with_track();
    dispatch(&mut app, Eim::Enable(TRACK));
    assert!(app.test_external_instrument(TRACK).is_some());

    dispatch(&mut app, Eim::Disable(TRACK));
    assert!(
        app.test_external_instrument(TRACK).is_none(),
        "Disable removes the track from the external map"
    );
}

#[test]
fn offline_event_then_recheck_clears() {
    let mut app = app_with_track();
    dispatch(&mut app, Eim::Enable(TRACK));
    dispatch(&mut app, Eim::SetMidiOutDevice(TRACK, Some("Moog Muse".into())));
    dispatch(
        &mut app,
        Eim::SetReturnDevice(TRACK, Some("Scarlett 18i20".into())),
    );
    dispatch(&mut app, Eim::ToggleMonitor(TRACK));
    assert_eq!(
        app.test_external_instrument_status(TRACK),
        Some(ExternalInstrumentStatus::Live)
    );

    // The engine reports the MIDI out going offline.
    app.test_apply_engine_event(AudioEvent::ExternalInstrumentMidiOutOffline {
        track_id: TRACK,
        device: Some("Moog Muse".into()),
    });
    assert!(app.test_external_instrument(TRACK).unwrap().midi_out_offline);
    assert_eq!(
        app.test_external_instrument_status(TRACK),
        Some(ExternalInstrumentStatus::Offline),
        "offline wins over Live"
    );

    // A re-check with the device back clears the flag optimistically (the
    // idle engine emits no fresh offline event).
    dispatch(&mut app, Eim::CheckDevices(TRACK));
    assert!(!app.test_external_instrument(TRACK).unwrap().midi_out_offline);
    assert_eq!(
        app.test_external_instrument_status(TRACK),
        Some(ExternalInstrumentStatus::Live),
        "recovered device returns to Live"
    );
}

#[test]
fn return_offline_event_sets_flag() {
    let mut app = app_with_track();
    dispatch(&mut app, Eim::Enable(TRACK));

    app.test_apply_engine_event(AudioEvent::ExternalInstrumentReturnInputOffline {
        track_id: TRACK,
        device: Some("Scarlett 18i20".into()),
    });
    let ext = app.test_external_instrument(TRACK).unwrap();
    assert!(ext.return_input_offline);
    assert!(!ext.midi_out_offline);
}

#[test]
fn changed_event_mirrors_engine_config() {
    let mut app = app_with_track();
    dispatch(&mut app, Eim::Enable(TRACK));

    app.test_apply_engine_event(AudioEvent::ExternalInstrumentChanged {
        config: ExternalInstrument {
            track_id: TRACK,
            bank: Some(0x0003),
            program: Some(7),
            latency_offset_samples: -128,
        },
    });
    let ext = app.test_external_instrument(TRACK).unwrap();
    assert_eq!(ext.bank, Some(0x0003));
    assert_eq!(ext.program, Some(7));
    assert_eq!(ext.latency_offset_samples, -128);
}

#[test]
fn changed_event_marks_unseen_track_external() {
    let mut app = app_with_track();
    // Never sent Enable; the engine config echo alone marks it external.
    assert!(app.test_external_instrument(TRACK).is_none());

    app.test_apply_engine_event(AudioEvent::ExternalInstrumentChanged {
        config: ExternalInstrument::new(TRACK),
    });
    assert!(app.test_external_instrument(TRACK).is_some());
}

#[test]
fn latency_measured_event_updates_applied_offset() {
    // Auto-detect ("ping", todo #453, doc #204): the engine measures the
    // round-trip, applies the floored offset, then emits the measured event.
    // The mirror must move the displayed/applied offset to the measured value.
    let mut app = app_with_track();
    dispatch(&mut app, Eim::Enable(TRACK));
    dispatch(&mut app, Eim::SetLatencyOffset(TRACK, 100)); // prior manual offset

    app.test_apply_engine_event(AudioEvent::ExternalInstrumentLatencyMeasured {
        track_id: TRACK,
        latency_samples: 2822,
        latency_ms: 64.0,
    });
    assert_eq!(
        app.test_external_instrument(TRACK)
            .unwrap()
            .latency_offset_samples,
        2822,
        "inspector reflects the engine-applied measured offset"
    );
}

#[test]
fn latency_measured_for_unknown_track_is_ignored() {
    // A measured event for a track that isn't external (stale race) must not
    // resurrect or fabricate a mirror entry.
    let mut app = app_with_track();
    app.test_apply_engine_event(AudioEvent::ExternalInstrumentLatencyMeasured {
        track_id: TRACK,
        latency_samples: 512,
        latency_ms: 11.6,
    });
    assert!(
        app.test_external_instrument(TRACK).is_none(),
        "no mirror entry conjured for a non-external track"
    );
}

#[test]
fn latency_detect_failed_event_leaves_offset_untouched() {
    // A clean failure (no detectable return) changes nothing in the mirror —
    // the prior offset stands, the track stays external, and the app doesn't
    // hang waiting (the event itself is the resolution).
    let mut app = app_with_track();
    dispatch(&mut app, Eim::Enable(TRACK));
    dispatch(&mut app, Eim::SetLatencyOffset(TRACK, 256));

    app.test_apply_engine_event(AudioEvent::ExternalInstrumentLatencyDetectFailed {
        track_id: TRACK,
        reason: "No return detected within the listen window.".into(),
    });
    let ext = app.test_external_instrument(TRACK).unwrap();
    assert_eq!(ext.latency_offset_samples, 256, "offset unchanged on failure");
}

#[test]
fn cleared_event_drops_external_mode() {
    let mut app = app_with_track();
    dispatch(&mut app, Eim::Enable(TRACK));
    assert!(app.test_external_instrument(TRACK).is_some());

    app.test_apply_engine_event(AudioEvent::ExternalInstrumentCleared { track_id: TRACK });
    assert!(app.test_external_instrument(TRACK).is_none());
}

#[test]
fn undo_classifies_config_edits_but_skips_runtime_pings() {
    // Config-changing edits record an undo entry.
    for m in [
        Eim::Enable(TRACK),
        Eim::Disable(TRACK),
        Eim::SetMidiOutDevice(TRACK, None),
        Eim::SetReturnDevice(TRACK, None),
        Eim::SetBank(TRACK, Some(1)),
        Eim::SetProgram(TRACK, Some(1)),
        Eim::SetLatencyOffset(TRACK, 1),
        Eim::ToggleMonitor(TRACK),
        Eim::ToggleRecordArm(TRACK),
    ] {
        assert!(
            matches!(classify(&Message::ExternalInstrument(m.clone())), UndoAction::Record),
            "{m:?} should record an undo entry"
        );
    }

    // Runtime-only device traffic never touches history.
    for m in [Eim::CheckDevices(TRACK), Eim::RescanDevices] {
        assert!(
            matches!(classify(&Message::ExternalInstrument(m.clone())), UndoAction::Skip),
            "{m:?} should not record an undo entry"
        );
    }
}

#[test]
fn undo_extras_round_trip_external_config() {
    let mut app = app_with_track();
    dispatch(&mut app, Eim::Enable(TRACK));
    dispatch(&mut app, Eim::SetBank(TRACK, Some(0x0102)));
    dispatch(&mut app, Eim::SetProgram(TRACK, Some(12)));
    dispatch(&mut app, Eim::SetLatencyOffset(TRACK, 64));

    // Snapshot the reversible config, then mutate away from it.
    let extras = app.test_snapshot_undo_extras();
    assert_eq!(extras.external_instruments.len(), 1);

    dispatch(&mut app, Eim::Disable(TRACK));
    assert!(app.test_external_instrument(TRACK).is_none());

    // Restoring the snapshot brings the external config back exactly.
    app.test_restore_external_instruments(&extras);
    let ext = app.test_external_instrument(TRACK).unwrap();
    assert_eq!(ext.bank, Some(0x0102));
    assert_eq!(ext.program, Some(12));
    assert_eq!(ext.latency_offset_samples, 64);
}

#[test]
fn restore_clears_externals_absent_from_snapshot() {
    let mut app = app_with_track();
    // Empty snapshot: nothing was external at capture time.
    let empty = app.test_snapshot_undo_extras();
    assert!(empty.external_instruments.is_empty());

    dispatch(&mut app, Eim::Enable(TRACK));
    assert!(app.test_external_instrument(TRACK).is_some());

    // Restoring the empty snapshot must drop the now-external track.
    app.test_restore_external_instruments(&empty);
    assert!(app.test_external_instrument(TRACK).is_none());
}
