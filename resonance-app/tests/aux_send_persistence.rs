//! Aux-send persistence: the send graph survives save + reload
//! (ba doc #273, ba todo #1269).
//!
//! Before this landed, `build_project_file` wrote tracks, busses and
//! routing but never the sends, so a send created in the mixer (or over
//! the control API) was silently gone the next time the project opened.
//! These tests pin the whole round-trip: the serialized shape, its JSON
//! form, the replay back into a fresh app, the removal case, and the
//! back-compat rule that a project file written before `sends` existed
//! still loads — with no sends and no complaints.
//!
//! The graph is set up the way the live app gets there: through the
//! engine's own echoes (`AuxSendChanged` / `BusRoleChanged`), which are
//! what the app mirror is built from.

use resonance_app::project::ProjectFile;
use resonance_app::state::TrackState;
use resonance_app::Resonance;
use resonance_audio::__test_support::Receiver;
use resonance_audio::types::{AudioCommand, AudioEvent, SendSource};

const TRACK: u64 = 7;
const RETURN_BUS: u64 = 10;
const SEND: u64 = 3;

/// An app with one audio track, one FX return bus, and a send from the
/// track into the bus at `level_db` / `pre_fader`.
fn app_with_send(level_db: f32, pre_fader: bool) -> Resonance {
    let (mut app, _task) = Resonance::new();
    app.test_set_active_project(true);
    app.test_push_track(TrackState::new_audio(TRACK, 0));
    app.test_apply_engine_event(AudioEvent::BusAdded {
        bus_id: RETURN_BUS,
        name: "FX Return 1".to_string(),
    });
    app.test_apply_engine_event(AudioEvent::BusRoleChanged {
        bus_id: RETURN_BUS,
        is_return: true,
    });
    app.test_apply_engine_event(AudioEvent::AuxSendChanged {
        send_id: SEND,
        source: SendSource::Track(TRACK),
        dest: RETURN_BUS,
        level_db,
        pre_fader,
        enabled: true,
    });
    app
}

fn drain(rx: &Receiver<AudioCommand>) -> Vec<AudioCommand> {
    let mut cmds = Vec::new();
    while let Ok(cmd) = rx.try_recv() {
        cmds.push(cmd);
    }
    cmds
}

/// Write `file` out and read it back as JSON — the actual on-disk hop —
/// then replay it into a brand-new app whose engine is a capturing stub.
/// Returns the reloaded app plus every command the replay emitted.
fn save_and_reload(file: &ProjectFile) -> (Resonance, Vec<AudioCommand>) {
    let json = serde_json::to_string_pretty(file).expect("serialize project");
    let back: ProjectFile = serde_json::from_str(&json).expect("deserialize project");

    let (mut app, _task) = Resonance::new();
    let rx = app.test_capture_engine();
    app.test_replay_loaded_project(back);
    let cmds = drain(&rx);
    (app, cmds)
}

/// The single `SetAuxSend` the replay issued for send `id`.
fn set_aux_send_for(cmds: &[AudioCommand], id: u64) -> &AudioCommand {
    let matching: Vec<&AudioCommand> = cmds
        .iter()
        .filter(|c| matches!(c, AudioCommand::SetAuxSend { id_hint: Some(h), .. } if *h == id))
        .collect();
    assert_eq!(
        matching.len(),
        1,
        "expected exactly one SetAuxSend for send {id}, got {matching:?}"
    );
    matching[0]
}

// ---------------------------------------------------------------------------
// Serialized shape
// ---------------------------------------------------------------------------

#[test]
fn build_project_file_captures_the_send_graph() {
    let app = app_with_send(-6.0, true);
    let file = app.test_build_project_file();

    assert_eq!(file.sends.len(), 1, "the send must reach the project file");
    let ps = &file.sends[0];
    assert_eq!(ps.id, SEND);
    assert_eq!(ps.source_kind, "track");
    assert_eq!(ps.source_id, TRACK);
    assert_eq!(ps.dest_bus, RETURN_BUS);
    assert_eq!(ps.level_db, -6.0);
    assert!(ps.pre_fader);
    assert!(ps.enabled);

    // The destination's return role is the other half of the route.
    let bus = file
        .busses
        .iter()
        .find(|b| b.id == RETURN_BUS)
        .expect("the return bus must serialize");
    assert!(bus.is_return, "an FX return must persist as one");
}

#[test]
fn a_project_without_sends_serializes_an_empty_list() {
    let (mut app, _task) = Resonance::new();
    app.test_set_active_project(true);
    app.test_push_track(TrackState::new_audio(TRACK, 0));

    let file = app.test_build_project_file();
    assert!(file.sends.is_empty());
}

#[test]
fn json_round_trip_preserves_every_send_field() {
    let app = app_with_send(-3.5, false);
    let file = app.test_build_project_file();

    let json = serde_json::to_string_pretty(&file).expect("serialize");
    let back: ProjectFile = serde_json::from_str(&json).expect("deserialize");

    assert_eq!(back.sends, file.sends);
}

// ---------------------------------------------------------------------------
// Round trip: save, reload, and the send is still there
// ---------------------------------------------------------------------------

#[test]
fn save_and_reload_restores_the_send_with_its_level_and_tap_point() {
    let app = app_with_send(-6.0, true);
    let file = app.test_build_project_file();

    let (reloaded, cmds) = save_and_reload(&file);

    // The GUI mirror — what the mixer draws and what `song.tracks`
    // reports — carries the send again.
    let sends = reloaded.test_aux_sends();
    assert_eq!(sends.len(), 1, "the reloaded project must have its send");
    let s = &sends[0];
    assert_eq!(s.id, SEND, "the send id survives the reload");
    assert_eq!(s.source, SendSource::Track(TRACK));
    assert_eq!(s.dest, RETURN_BUS);
    assert_eq!(s.level_db, -6.0, "level survives the round trip");
    assert!(s.pre_fader, "tap point survives the round trip");
    assert!(s.enabled);

    // …and the engine was told about it, so the reloaded project makes
    // the same sound, not just the same picture.
    match set_aux_send_for(&cmds, SEND) {
        AudioCommand::SetAuxSend {
            source,
            dest,
            level_db,
            pre_fader,
            enabled,
            ..
        } => {
            assert_eq!(*source, SendSource::Track(TRACK));
            assert_eq!(*dest, RETURN_BUS);
            assert_eq!(*level_db, -6.0);
            assert!(*pre_fader);
            assert!(*enabled);
        }
        other => panic!("expected SetAuxSend, got {other:?}"),
    }

    // The destination is a return bus again, in the mirror and in the
    // engine.
    let bus = reloaded
        .test_registry()
        .busses
        .iter()
        .find(|b| b.id == RETURN_BUS)
        .expect("the return bus must be restored");
    assert!(bus.is_return);
    assert!(
        cmds.iter().any(|c| matches!(
            c,
            AudioCommand::SetBusRole { bus_id, is_return: true } if *bus_id == RETURN_BUS
        )),
        "replay must re-flag the destination as a return bus"
    );
}

#[test]
fn a_disabled_send_reloads_disabled() {
    let mut app = app_with_send(0.0, false);
    app.test_apply_engine_event(AudioEvent::AuxSendChanged {
        send_id: SEND,
        source: SendSource::Track(TRACK),
        dest: RETURN_BUS,
        level_db: 0.0,
        pre_fader: false,
        enabled: false,
    });

    let file = app.test_build_project_file();
    let (reloaded, _cmds) = save_and_reload(&file);

    let s = reloaded.test_aux_sends().first().copied().expect("send");
    assert!(!s.enabled, "a send silenced before the save stays silenced");
}

#[test]
fn removing_a_send_removes_it_from_the_saved_project() {
    let mut app = app_with_send(-6.0, false);
    app.test_apply_engine_event(AudioEvent::AuxSendRemoved { send_id: SEND });

    let file = app.test_build_project_file();
    assert!(
        file.sends.is_empty(),
        "a removed send must not be written back out"
    );

    let (reloaded, cmds) = save_and_reload(&file);
    assert!(
        reloaded.test_aux_sends().is_empty(),
        "a removed send must not come back on reload"
    );
    assert!(
        !cmds
            .iter()
            .any(|c| matches!(c, AudioCommand::SetAuxSend { .. })),
        "replay must not re-register a deleted send"
    );
}

#[test]
fn loading_a_project_drops_the_previous_projects_sends() {
    // Open a project with a send, then load one without: the mirror must
    // not keep routing into a bus the new project never had. (`ClearAll`
    // empties the engine's table without a per-send removal echo.)
    let mut app = app_with_send(-6.0, false);
    assert_eq!(app.test_aux_sends().len(), 1);

    app.test_replay_loaded_project(ProjectFile::default());
    assert!(
        app.test_aux_sends().is_empty(),
        "a stale send must not survive into the next project"
    );
}

// ---------------------------------------------------------------------------
// Back-compat: projects written before `sends` existed
// ---------------------------------------------------------------------------

/// A minimal project file in the shape older builds wrote: no `sends`
/// key anywhere, and a bus with no `is_return` key.
const LEGACY_PROJECT_JSON: &str = r#"{
  "version": 2,
  "sample_rate": 48000,
  "bpm": 120.0,
  "time_sig_num": 4,
  "time_sig_den": 4,
  "metronome_enabled": false,
  "master_volume": 0.0,
  "loop_enabled": false,
  "loop_in": 0,
  "loop_out": 0,
  "tracks": [],
  "clips": [],
  "busses": [
    {
      "id": 10,
      "name": "Bus 1",
      "order": 0,
      "volume": 0.0,
      "pan": 0.0,
      "muted": false,
      "plugins": []
    }
  ]
}"#;

#[test]
fn a_legacy_project_without_sends_still_loads() {
    let file: ProjectFile =
        serde_json::from_str(LEGACY_PROJECT_JSON).expect("an old project must still parse");
    assert!(file.sends.is_empty(), "the field defaults to no sends");
    assert!(
        !file.busses[0].is_return,
        "an old bus defaults to a plain sub-mix, not a return"
    );

    let (reloaded, cmds) = save_and_reload(&file);
    assert!(reloaded.test_aux_sends().is_empty());
    assert!(
        !cmds
            .iter()
            .any(|c| matches!(c, AudioCommand::SetAuxSend { .. })),
        "an old project must not invent sends"
    );
    // The bus itself still loads.
    assert!(reloaded
        .test_registry()
        .busses
        .iter()
        .any(|b| b.id == RETURN_BUS));
}

// ---------------------------------------------------------------------------
// Undo / redo: the diff replay reconciles sends surgically
// ---------------------------------------------------------------------------

#[test]
fn diff_replay_restores_a_send_level_on_undo() {
    let mut app = app_with_send(-6.0, false);
    app.test_set_project_path(std::path::PathBuf::from("/tmp/resonance-send-undo.rproj"));
    let snapshot = app.test_snapshot_for_undo();

    // The user pulls the send down; the engine echoes the new level.
    app.test_apply_engine_event(AudioEvent::AuxSendChanged {
        send_id: SEND,
        source: SendSource::Track(TRACK),
        dest: RETURN_BUS,
        level_db: -24.0,
        pre_fader: false,
        enabled: true,
    });
    assert_eq!(app.test_aux_sends()[0].level_db, -24.0);

    let rx = app.test_capture_engine();
    app.test_begin_restore_from_snapshot(snapshot);

    assert_eq!(
        app.test_aux_sends()[0].level_db,
        -6.0,
        "undo must put the send level back"
    );
    match set_aux_send_for(&drain(&rx), SEND) {
        AudioCommand::SetAuxSend { level_db, .. } => assert_eq!(*level_db, -6.0),
        other => panic!("expected SetAuxSend, got {other:?}"),
    }
}

#[test]
fn diff_replay_drops_a_send_added_after_the_snapshot() {
    let (mut app, _task) = Resonance::new();
    app.test_set_active_project(true);
    app.test_push_track(TrackState::new_audio(TRACK, 0));
    app.test_apply_engine_event(AudioEvent::BusAdded {
        bus_id: RETURN_BUS,
        name: "FX Return 1".to_string(),
    });
    app.test_set_project_path(std::path::PathBuf::from("/tmp/resonance-send-undo.rproj"));

    // Snapshot with no sends, then add one.
    let snapshot = app.test_snapshot_for_undo();
    app.test_apply_engine_event(AudioEvent::AuxSendChanged {
        send_id: SEND,
        source: SendSource::Track(TRACK),
        dest: RETURN_BUS,
        level_db: 0.0,
        pre_fader: false,
        enabled: true,
    });
    assert_eq!(app.test_aux_sends().len(), 1);

    let rx = app.test_capture_engine();
    app.test_begin_restore_from_snapshot(snapshot);

    assert!(
        app.test_aux_sends().is_empty(),
        "undoing past the add must remove the send"
    );
    assert!(
        drain(&rx).iter().any(|c| matches!(
            c,
            AudioCommand::RemoveAuxSend { send_id } if *send_id == SEND
        )),
        "the engine must be told to drop the send too"
    );
}

// ---------------------------------------------------------------------------
// Review follow-ups: a send is an edge, so it must not outlive either end
// ---------------------------------------------------------------------------

/// Deleting the source track drops the send from the mirror, so it is
/// never written to the project file.
///
/// Before this, the engine dropped the route silently on `RemoveTrack`
/// (no per-send echo) while the mirror kept it — and the mirror is what
/// save serializes. The dangling send was written out, refused by the
/// loader on reopen, and rewritten by every save after that, one entry
/// per deleted track, forever.
#[test]
fn deleting_the_source_track_drops_its_sends() {
    let mut app = app_with_send(-6.0, false);
    assert_eq!(app.test_aux_sends().len(), 1);

    app.test_apply_engine_event(AudioEvent::TrackRemoved { track_id: TRACK });

    assert!(
        app.test_aux_sends().is_empty(),
        "a send out of a deleted track must not survive in the mirror"
    );
    let file = app.test_build_project_file();
    assert!(
        file.sends.is_empty(),
        "and must not be written to the project file"
    );
}

/// Same for the destination bus, which can be either end of the edge.
#[test]
fn deleting_the_destination_bus_drops_its_sends() {
    let mut app = app_with_send(-6.0, false);
    app.test_apply_engine_event(AudioEvent::BusRemoved {
        bus_id: RETURN_BUS,
    });

    assert!(app.test_aux_sends().is_empty());
    assert!(app.test_build_project_file().sends.is_empty());
}

/// A send whose endpoints are missing from the file is not mirrored on
/// load, so the mixer cannot show a route the engine refused.
///
/// `AuxSendRejected` only records `last_rejection`; it never removes a
/// seeded entry. Seeding unconditionally therefore left a phantom send
/// that the mixer drew and `song.tracks` reported to MCP clients while
/// no audio was routed at all.
#[test]
fn a_send_with_a_missing_endpoint_is_not_mirrored_on_load() {
    let mut file = app_with_send(-6.0, false).test_build_project_file();
    // Re-point the send at a track the file does not contain.
    file.sends[0].source_id = 999;

    let (app, cmds) = save_and_reload(&file);

    assert!(
        app.test_aux_sends().is_empty(),
        "a send with a missing source must not appear in the mirror"
    );
    assert!(
        !cmds
            .iter()
            .any(|c| matches!(c, AudioCommand::SetAuxSend { .. })),
        "and no SetAuxSend should be sent for it"
    );
}

/// An unrecognised `source_kind` drops the send instead of silently
/// re-pointing it at a different entity.
///
/// Track ids and bus ids are independent namespaces that both start at
/// 1, so defaulting an unknown tag to `Track(id)` would usually hit a
/// track that exists and sum the wrong signal into the bus, with no
/// error anywhere.
#[test]
fn an_unknown_source_kind_drops_the_send_rather_than_guessing() {
    let mut file = app_with_send(-6.0, false).test_build_project_file();
    file.sends[0].source_kind = "Bus".to_string(); // wrong case: not "bus"

    let (app, cmds) = save_and_reload(&file);

    assert!(
        app.test_aux_sends().is_empty(),
        "an unknown source kind must not be guessed into a Track route"
    );
    assert!(!cmds
        .iter()
        .any(|c| matches!(c, AudioCommand::SetAuxSend { .. })));
}
