//! Update-layer coverage for the External-Instrument **device preset** picker
//! (architecture doc #201 §5, epic #40, ba todo #724).
//!
//! Selecting a preset stores its `device_id` on the app-side
//! external-instrument track state and hands the engine the definition's
//! automatable params via `AudioCommand::SetTrackDeviceParams`; clearing the
//! selection sends an empty-params command. Undo/redo round-trips the
//! selection, and the engine's `TrackDeviceParamsApplied` echo mirrors back
//! into app state. Command-dispatch tests swap in a capturing engine (like
//! `tests/aux_send_handlers.rs`); the undo round-trip drives the real reducer.

use resonance_app::message::{ExternalInstrumentMessage as Eim, Message};
use resonance_app::state::TrackState;
use resonance_app::undo::{classify, UndoAction};
use resonance_app::Resonance;
use resonance_audio::test_support::Receiver;
use resonance_audio::types::{AudioCommand, AudioEvent, TrackId};

const TRACK: TrackId = 1;
/// The bundled Moog Muse preset ships in the registry (`resonance-common`
/// bundled definitions), so `Resonance::new_for_test()` always has it available.
const MUSE: &str = "moog-muse";

/// App with an active project and a single instrument track already marked as
/// an external instrument, plus a command-capturing engine. The `Enable`
/// dispatch's own commands are drained by the caller before asserting.
fn external_capturing_app() -> (Resonance, Receiver<AudioCommand>) {
    let (mut app, _task) = Resonance::new_for_test();
    app.test_set_active_project(true);
    app.test_push_track(TrackState::new_instrument(TRACK, 0));
    let rx = app.test_capture_engine();
    app.test_dispatch(Message::ExternalInstrument(Eim::Enable(TRACK)));
    (app, rx)
}

fn drain(rx: &Receiver<AudioCommand>) -> Vec<AudioCommand> {
    let mut cmds = Vec::new();
    while let Ok(cmd) = rx.try_recv() {
        cmds.push(cmd);
    }
    cmds
}

/// The single `SetTrackDeviceParams` command in `cmds` (its param count).
/// Panics if there isn't exactly one, so a test asserting "no command" or
/// "one command" is unambiguous.
fn one_device_params(cmds: &[AudioCommand]) -> (TrackId, usize) {
    let mut found = None;
    for cmd in cmds {
        if let AudioCommand::SetTrackDeviceParams { track_id, params } = cmd {
            assert!(
                found.is_none(),
                "expected exactly one SetTrackDeviceParams command"
            );
            found = Some((*track_id, params.len()));
        }
    }
    found.expect("a SetTrackDeviceParams command was dispatched")
}

fn has_device_params_cmd(cmds: &[AudioCommand]) -> bool {
    cmds.iter()
        .any(|c| matches!(c, AudioCommand::SetTrackDeviceParams { .. }))
}

#[test]
fn select_device_stores_id_and_dispatches_params() {
    let (mut app, rx) = external_capturing_app();
    let _ = drain(&rx); // discard the Enable round-trip's commands.

    app.test_dispatch(Message::ExternalInstrument(Eim::SetDevice(
        TRACK,
        Some(MUSE.to_string()),
    )));

    // App state records the chosen preset id.
    let ext = app.test_external_instrument(TRACK).expect("still external");
    assert_eq!(ext.device_id.as_deref(), Some(MUSE));

    // Engine gets the definition's params (the bundled Muse has many).
    let cmds = drain(&rx);
    let (track_id, param_count) = one_device_params(&cmds);
    assert_eq!(track_id, TRACK);
    assert!(
        param_count > 0,
        "the Muse preset supplies its automatable params"
    );
}

#[test]
fn clear_device_dispatches_empty_params() {
    let (mut app, rx) = external_capturing_app();
    app.test_dispatch(Message::ExternalInstrument(Eim::SetDevice(
        TRACK,
        Some(MUSE.to_string()),
    )));
    let _ = drain(&rx); // discard the select round-trip.

    app.test_dispatch(Message::ExternalInstrument(Eim::SetDevice(TRACK, None)));

    let ext = app.test_external_instrument(TRACK).expect("still external");
    assert_eq!(ext.device_id, None, "clearing drops the selection");

    let cmds = drain(&rx);
    let (track_id, param_count) = one_device_params(&cmds);
    assert_eq!(track_id, TRACK);
    assert_eq!(param_count, 0, "clearing sends an empty param map");
}

#[test]
fn unknown_device_id_dispatches_empty_params() {
    let (mut app, rx) = external_capturing_app();
    let _ = drain(&rx);

    app.test_dispatch(Message::ExternalInstrument(Eim::SetDevice(
        TRACK,
        Some("no-such-device".to_string()),
    )));

    // The id is still stored (the picker only offers real ids; this keeps
    // clear vs. select unambiguous), but no params resolve.
    let ext = app.test_external_instrument(TRACK).expect("still external");
    assert_eq!(ext.device_id.as_deref(), Some("no-such-device"));
    let cmds = drain(&rx);
    let (_, param_count) = one_device_params(&cmds);
    assert_eq!(param_count, 0, "an unknown id clears the engine map");
}

#[test]
fn select_device_ignored_for_non_external_track() {
    let (mut app, _task) = Resonance::new_for_test();
    app.test_set_active_project(true);
    app.test_push_track(TrackState::new_instrument(TRACK, 0));
    let rx = app.test_capture_engine();

    // Track is NOT external (no Enable) — the message is a no-op.
    app.test_dispatch(Message::ExternalInstrument(Eim::SetDevice(
        TRACK,
        Some(MUSE.to_string()),
    )));

    assert!(
        app.test_external_instrument(TRACK).is_none(),
        "a non-external track gains no external state"
    );
    assert!(
        !has_device_params_cmd(&drain(&rx)),
        "no device-params command for a non-external track"
    );
}

#[test]
fn applied_params_event_mirrors_into_state() {
    let (mut app, _task) = Resonance::new_for_test();
    app.test_set_active_project(true);
    app.test_push_track(TrackState::new_instrument(TRACK, 0));
    app.test_dispatch(Message::ExternalInstrument(Eim::Enable(TRACK)));

    // The engine confirms the applied param ids; the mirror records them.
    let ids = vec!["low-cut".to_string(), "glide-time".to_string()];
    app.test_apply_engine_event(AudioEvent::TrackDeviceParamsApplied {
        track_id: TRACK,
        param_ids: ids.clone(),
    });

    let ext = app.test_external_instrument(TRACK).expect("still external");
    assert_eq!(ext.applied_param_ids, ids);

    // An empty confirmation clears the mirror.
    app.test_apply_engine_event(AudioEvent::TrackDeviceParamsApplied {
        track_id: TRACK,
        param_ids: Vec::new(),
    });
    let ext = app.test_external_instrument(TRACK).expect("still external");
    assert!(ext.applied_param_ids.is_empty());
}

#[test]
fn undo_redo_round_trips_device_selection() {
    let (mut app, _task) = Resonance::new_for_test();
    app.test_set_active_project(true);
    app.test_set_project_path(std::path::PathBuf::from("/tmp/resonance-test-724"));
    app.test_push_track(TrackState::new_instrument(TRACK, 0));

    // Full reducer path so undo records each edit.
    let _ = app.update(Message::ExternalInstrument(Eim::Enable(TRACK)));
    let _ = app.update(Message::ExternalInstrument(Eim::SetDevice(
        TRACK,
        Some(MUSE.to_string()),
    )));
    assert_eq!(
        app.test_external_instrument(TRACK)
            .and_then(|e| e.device_id),
        Some(MUSE.to_string()),
    );

    // Undo drops back to the no-device selection (the track stays external).
    let _ = app.update(Message::Undo);
    let ext = app
        .test_external_instrument(TRACK)
        .expect("track still external after undo");
    assert_eq!(ext.device_id, None, "undo clears the device selection");

    // Redo restores it.
    let _ = app.update(Message::Redo);
    assert_eq!(
        app.test_external_instrument(TRACK)
            .and_then(|e| e.device_id),
        Some(MUSE.to_string()),
        "redo restores the device selection",
    );
}

#[test]
fn set_device_is_recorded_for_undo() {
    assert!(
        matches!(
            classify(&Message::ExternalInstrument(Eim::SetDevice(
                TRACK,
                Some(MUSE.to_string())
            ))),
            UndoAction::Record
        ),
        "device selection is a reversible edit"
    );
}
