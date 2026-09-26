//! Message-level coverage for `TrackMessage::AddExternalInstrumentTrack`
//! (doc #251 gap 1 affordance 2, ba todo #1066).
//!
//! This message creates an instrument track that starts already in
//! external-instrument mode. Unlike `AddInstrumentTrack` (which lets the
//! engine allocate the id), it allocates the track id app-side so the
//! external state can be enabled on that exact id in the same reducer call —
//! the engine echoes `InstrumentTrackAdded` for the id a beat later, which
//! mirrors the track into the registry.
//!
//! Coverage:
//!  - dispatching the message enables external mode on a freshly-allocated id
//!    and emits `AddInstrumentTrack { id_hint: Some(id) }` + a
//!    `SetExternalInstrument` engine command;
//!  - once the engine echoes `InstrumentTrackAdded`, the new track is present
//!    in the registry AND in the external-instruments map;
//!  - the whole action is ONE undo step: undo removes the track and its
//!    external state together, redo restores both.

use std::path::PathBuf;

use resonance_app::message::{Message, TrackMessage};
use resonance_app::Resonance;
use resonance_audio::types::{AudioCommand, AudioEvent, TrackId};

/// App with an active, saved project so undo/redo can record + replay
/// snapshots (`can_record_undo` needs a project path).
fn app_with_project() -> Resonance {
    let (mut app, _task) = Resonance::new_for_test();
    app.test_set_active_project(true);
    app.test_set_project_path(PathBuf::from("/proj/song.rproj"));
    app
}

fn drain(rx: &resonance_audio::test_support::Receiver<AudioCommand>) -> Vec<AudioCommand> {
    let mut cmds = Vec::new();
    while let Ok(cmd) = rx.try_recv() {
        cmds.push(cmd);
    }
    cmds
}

/// The id the app's sub-track counter hands out for the first app-allocated
/// track. `allocate_sub_track_id` starts from `next_sub_track_id`; a fresh
/// project has no tracks, so the first allocation is deterministic. We read
/// it back from the emitted command rather than hard-coding it, so the test
/// stays correct even if the base counter changes.
fn allocated_id_from_add(cmds: &[AudioCommand]) -> TrackId {
    for cmd in cmds {
        if let AudioCommand::AddInstrumentTrack { id_hint: Some(id), .. } = cmd {
            return *id;
        }
    }
    panic!("expected an AddInstrumentTrack with an app-allocated id_hint");
}

#[test]
fn dispatch_emits_add_and_enable_commands() {
    let mut app = app_with_project();
    let rx = app.test_capture_engine();

    let _ = app.update(Message::Track(TrackMessage::AddExternalInstrumentTrack));

    let cmds = drain(&rx);

    // The track is created with an app-allocated id hint (so we know the id).
    let id = allocated_id_from_add(&cmds);

    // The Enable half fires the same engine command as
    // `ExternalInstrumentMessage::Enable`.
    let enabled = cmds.iter().any(|c| {
        matches!(
            c,
            AudioCommand::SetExternalInstrument { config } if config.track_id == id
        )
    });
    assert!(
        enabled,
        "AddExternalInstrumentTrack must send SetExternalInstrument for the new id; got {cmds:?}"
    );

    // And the app-side external-instrument mirror carries the new id even
    // before the engine echoes the track back.
    assert!(
        app.test_external_instrument(id).is_some(),
        "new track id must be in the external-instruments map"
    );
}

#[test]
fn engine_echo_lands_the_track_in_external_mode() {
    let mut app = app_with_project();
    let rx = app.test_capture_engine();

    let _ = app.update(Message::Track(TrackMessage::AddExternalInstrumentTrack));
    let id = allocated_id_from_add(&drain(&rx));

    // Track isn't in the registry yet — it lands when the engine echoes.
    assert!(
        !app.test_registry().tracks.iter().any(|t| t.id == id),
        "track must not be in the registry before the engine echo"
    );

    app.test_apply_engine_event(AudioEvent::InstrumentTrackAdded { track_id: id });

    let track = app
        .test_registry()
        .tracks
        .iter()
        .find(|t| t.id == id)
        .expect("engine echo mirrors the instrument track into the registry");
    assert_eq!(
        track.track_type,
        resonance_audio::types::TrackType::Instrument
    );
    assert!(
        app.test_external_instrument(id).is_some(),
        "the mirrored track is in external-instrument mode"
    );
}

#[test]
fn undo_removes_track_and_external_state_redo_restores_both() {
    // Real (idle) engine so the undo slow-path (`ClearAll` → `AllCleared` →
    // replay) can be pumped synchronously via `test_apply_engine_event`.
    let mut app = app_with_project();

    // Learn the allocated id via a throwaway capture, then rebuild the app so
    // the recorded pre-dispatch undo snapshot is the empty project.
    let probe_id = {
        let mut probe = app_with_project();
        let rx = probe.test_capture_engine();
        let _ = probe.update(Message::Track(TrackMessage::AddExternalInstrumentTrack));
        allocated_id_from_add(&drain(&rx))
    };

    // Create the external track for real and land it via the engine echo.
    let _ = app.update(Message::Track(TrackMessage::AddExternalInstrumentTrack));
    app.test_apply_engine_event(AudioEvent::InstrumentTrackAdded { track_id: probe_id });

    assert!(
        app.test_registry().tracks.iter().any(|t| t.id == probe_id),
        "precondition: track present after echo"
    );
    assert!(
        app.test_external_instrument(probe_id).is_some(),
        "precondition: external state present after echo"
    );

    // Undo: structural change (track removed) → slow ClearAll/replay path.
    let _ = app.update(Message::Undo);
    app.test_apply_engine_event(AudioEvent::AllCleared);

    assert!(
        !app.test_registry().tracks.iter().any(|t| t.id == probe_id),
        "undo removes the track"
    );
    assert!(
        app.test_external_instrument(probe_id).is_none(),
        "undo removes the external state too (one undo step for the whole action)"
    );

    // Redo: restores the track AND its external state.
    let _ = app.update(Message::Redo);
    app.test_apply_engine_event(AudioEvent::AllCleared);

    assert!(
        app.test_registry().tracks.iter().any(|t| t.id == probe_id),
        "redo restores the track"
    );
    assert!(
        app.test_external_instrument(probe_id).is_some(),
        "redo restores the external state"
    );
}
