//! Project-file persistence of the External-Instrument **device preset** and
//! its **DeviceParam automation lanes** (architecture doc #201 §5, epic #40,
//! ba todo #727).
//!
//! `build_project_file` captures the selected `device_id` per external track
//! (plus an embedded copy of a *user-authored* definition so the project
//! reopens on another machine) and every automation lane. On load,
//! `replay_loaded_project` re-sends `SetTrackDeviceParams` — resolving the
//! params from the embedded copy or the registry — and re-applies the lanes
//! via `SetAutomationLane`, so the engine is rehydrated and the project
//! round-trips. Back-compat: a project with no device field (every existing
//! project) loads and behaves exactly as before.

use std::collections::HashMap;
use std::path::PathBuf;

use resonance_app::message::{ExternalInstrumentMessage as Eim, Message};
use resonance_app::project::{LoadedProject, ProjectFile, ProjectTrack};
use resonance_app::state::TrackState;
use resonance_app::update::project_io::replay_loaded_project;
use resonance_app::Resonance;
use resonance_audio::__test_support::Receiver;
use resonance_audio::types::{AudioCommand, AudioEvent, TrackId};
use resonance_common::{AutomationLane, AutomationTarget, Breakpoint, CurveKind};

const TRACK: TrackId = 1;
/// The bundled Moog Muse preset always ships in the registry.
const MUSE: &str = "moog-muse";

fn drain(rx: &Receiver<AudioCommand>) -> Vec<AudioCommand> {
    let mut cmds = Vec::new();
    while let Ok(cmd) = rx.try_recv() {
        cmds.push(cmd);
    }
    cmds
}

/// A two-point DeviceParam lane targeting `TRACK`'s `param_id`.
fn device_param_lane(id: u64, param_id: &str) -> AutomationLane {
    AutomationLane::new(
        id,
        AutomationTarget::DeviceParam {
            track: TRACK,
            param_id: param_id.to_string(),
        },
        vec![
            Breakpoint::new(0, 0.1, CurveKind::Linear),
            Breakpoint::new(48_000, 0.9, CurveKind::Linear),
        ],
    )
}

/// Fresh app with an active project + one instrument track, ready to be wired
/// as an external instrument.
fn app_with_track() -> Resonance {
    let (mut app, _task) = Resonance::new_for_test();
    app.test_set_active_project(true);
    app.test_set_project_path(PathBuf::from("/tmp/resonance-test-727"));
    app.test_push_track(TrackState::new_instrument(TRACK, 0));
    app
}

/// The single `ProjectTrack` from a serialized project file.
fn only_track(file: &ProjectFile) -> &ProjectTrack {
    assert_eq!(file.tracks.len(), 1, "expected exactly one track");
    &file.tracks[0]
}

/// Replay `file` into a fresh app with a capturing engine, returning the app
/// and the commands the replay dispatched.
fn replay_into_fresh(file: ProjectFile) -> (Resonance, Vec<AudioCommand>) {
    let (mut app, _task) = Resonance::new_for_test();
    let rx = app.test_capture_engine();
    let loaded = LoadedProject {
        file,
        project_dir: PathBuf::from("/tmp/resonance-test-727"),
        midi_notes: HashMap::new(),
        plugin_states: HashMap::new(),
    };
    replay_loaded_project(&mut app, Box::new(loaded));
    let cmds = drain(&rx);
    (app, cmds)
}

fn device_params_cmd(cmds: &[AudioCommand], track: TrackId) -> Option<usize> {
    cmds.iter().find_map(|c| match c {
        AudioCommand::SetTrackDeviceParams { track_id, params } if *track_id == track => {
            Some(params.len())
        }
        _ => None,
    })
}

fn has_set_lane_for(cmds: &[AudioCommand], target: &AutomationTarget) -> bool {
    cmds.iter().any(|c| match c {
        AudioCommand::SetAutomationLane { lane } => &lane.target == target,
        _ => false,
    })
}

// =====================================================================
// Save side: build_project_file captures the selection + lanes
// =====================================================================

#[test]
fn build_project_file_captures_device_id_and_lane() {
    let mut app = app_with_track();
    app.test_dispatch(Message::ExternalInstrument(Eim::Enable(TRACK)));
    app.test_dispatch(Message::ExternalInstrument(Eim::SetDevice(
        TRACK,
        Some(MUSE.to_string()),
    )));
    // A DeviceParam lane, as the engine would mirror it back after an edit.
    let lane = device_param_lane(7, "glide-time");
    app.test_apply_engine_event(AudioEvent::AutomationLaneChanged { lane: lane.clone() });

    let file = app.test_build_project_file();
    let pt = only_track(&file);
    let ext = pt
        .external_instrument
        .as_ref()
        .expect("external track serializes its config");

    assert_eq!(ext.device_id.as_deref(), Some(MUSE));
    // The Muse is bundled — its definition ships with the app, so we don't
    // embed a copy (kept lean; re-resolved from the registry on load).
    assert!(
        ext.device_definition.is_none(),
        "a bundled device is not embedded"
    );

    assert_eq!(file.automation_lanes.len(), 1, "the lane is persisted");
    assert_eq!(file.automation_lanes[0], lane);
}

// =====================================================================
// Round-trip: save -> JSON -> load re-sends params + lane
// =====================================================================

#[test]
fn save_load_round_trips_device_selection_and_device_param_lane() {
    let mut app = app_with_track();
    app.test_dispatch(Message::ExternalInstrument(Eim::Enable(TRACK)));
    app.test_dispatch(Message::ExternalInstrument(Eim::SetDevice(
        TRACK,
        Some(MUSE.to_string()),
    )));
    let lane = device_param_lane(11, "glide-time");
    let target = lane.target.clone();
    app.test_apply_engine_event(AudioEvent::AutomationLaneChanged { lane: lane.clone() });

    // Save -> JSON -> load, exactly as a real project write/read does.
    let file = app.test_build_project_file();
    let json = serde_json::to_string_pretty(&file).expect("serialize");
    let back: ProjectFile = serde_json::from_str(&json).expect("deserialize");

    let (loaded_app, cmds) = replay_into_fresh(back);

    // Device selection is restored on the rebuilt track state.
    let ext = loaded_app
        .test_external_instrument(TRACK)
        .expect("track is external after load");
    assert_eq!(ext.device_id.as_deref(), Some(MUSE));

    // The engine is rehydrated: params (bundled Muse has many) then the lane.
    let param_count = device_params_cmd(&cmds, TRACK)
        .expect("SetTrackDeviceParams re-sent on load");
    assert!(param_count > 0, "the Muse preset supplies its params");
    assert!(
        has_set_lane_for(&cmds, &target),
        "the DeviceParam lane is re-applied on load"
    );

    // The app-side lane mirror is restored too.
    let stored = loaded_app
        .test_automation()
        .lanes
        .get(&target)
        .expect("DeviceParam lane restored into the mirror");
    assert_eq!(*stored, lane);
}

// =====================================================================
// User-authored definition: embedded copy makes the project portable
// =====================================================================

#[test]
fn user_authored_device_is_embedded_and_reopens_without_the_definition() {
    // A user-authored preset registered from a "user folder" (temp dir).
    let dir = std::env::temp_dir().join("resonance-test-727-userdef");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("temp dir");
    let user_json = r#"{
        "id": "acme-synth-one",
        "manufacturer": "Acme",
        "model": "Synth One",
        "schema_version": 1,
        "params": [
            { "id": "cutoff", "name": "Cutoff",
              "binding": { "Cc": { "cc": 74 } }, "min": 0, "max": 127, "curve": "Linear" }
        ],
        "patches": []
    }"#;
    std::fs::write(dir.join("acme-synth-one.json"), user_json).expect("write def");

    let mut app = app_with_track();
    app.test_scan_device_dir(&dir);
    app.test_dispatch(Message::ExternalInstrument(Eim::Enable(TRACK)));
    app.test_dispatch(Message::ExternalInstrument(Eim::SetDevice(
        TRACK,
        Some("acme-synth-one".to_string()),
    )));

    let file = app.test_build_project_file();
    let ext = only_track(&file)
        .external_instrument
        .as_ref()
        .expect("external config");
    assert_eq!(ext.device_id.as_deref(), Some("acme-synth-one"));
    let embedded = ext
        .device_definition
        .as_ref()
        .expect("a user-authored device embeds a copy so the project is portable");
    assert_eq!(embedded.id, "acme-synth-one");
    assert_eq!(embedded.params.len(), 1);

    // Simulate opening on another machine: JSON round-trip, then replay into a
    // fresh app whose registry does NOT have the user definition. The embedded
    // copy must still rehydrate the engine.
    let json = serde_json::to_string_pretty(&file).unwrap();
    let back: ProjectFile = serde_json::from_str(&json).unwrap();
    let _ = std::fs::remove_dir_all(&dir); // gone on the "other machine".

    let (loaded_app, cmds) = replay_into_fresh(back);
    assert_eq!(
        loaded_app
            .test_external_instrument(TRACK)
            .and_then(|e| e.device_id)
            .as_deref(),
        Some("acme-synth-one"),
    );
    let param_count = device_params_cmd(&cmds, TRACK)
        .expect("SetTrackDeviceParams re-sent from the embedded definition");
    assert_eq!(
        param_count, 1,
        "the embedded copy supplies the params even without the user folder"
    );
}

// =====================================================================
// Back-compat: projects without the device field load unchanged
// =====================================================================

#[test]
fn legacy_external_track_without_device_field_loads_and_sends_no_params() {
    // A project authored before device presets: an external track with
    // bank/program but no `device_id` / `device_definition`, and no
    // `automation_lanes`. `#[serde(default)]` fills the new fields.
    let legacy = r#"{
        "version": 2,
        "sample_rate": 44100,
        "bpm": 120.0,
        "time_sig_num": 4,
        "time_sig_den": 4,
        "metronome_enabled": false,
        "master_volume": 0.0,
        "loop_enabled": false,
        "loop_in": 0,
        "loop_out": 0,
        "tracks": [{
            "id": 1,
            "name": "Synth",
            "order": 0,
            "volume": 0.0,
            "pan": 0.0,
            "muted": false,
            "soloed": false,
            "record_armed": false,
            "monitor_enabled": false,
            "mono": false,
            "input_device_name": null,
            "plugins": [],
            "external_instrument": { "bank": 258, "program": 12, "latency_offset_samples": 0 }
        }],
        "clips": []
    }"#;
    let file: ProjectFile = serde_json::from_str(legacy).expect("legacy project parses");
    let pt = only_track(&file);
    let ext = pt.external_instrument.as_ref().expect("external config");
    assert_eq!(ext.device_id, None, "no device selected on a legacy project");
    assert!(ext.device_definition.is_none());
    assert!(file.automation_lanes.is_empty());

    // Loading it drives no device-param or automation traffic — it behaves
    // exactly as an external track did before device presets existed.
    let (loaded_app, cmds) = replay_into_fresh(file);
    let ext = loaded_app
        .test_external_instrument(TRACK)
        .expect("track loads as external");
    assert_eq!(ext.device_id, None);
    assert_eq!(ext.bank, Some(258));
    assert_eq!(ext.program, Some(12));
    assert!(
        device_params_cmd(&cmds, TRACK).is_none(),
        "no SetTrackDeviceParams for a track with no device selected"
    );
    assert!(
        !cmds
            .iter()
            .any(|c| matches!(c, AudioCommand::SetAutomationLane { .. })),
        "no automation lanes to re-apply"
    );
}

#[test]
fn legacy_project_without_any_new_fields_loads_clean() {
    // The minimal legacy project (no external tracks at all) still loads and
    // its new top-level `automation_lanes` defaults to empty.
    let legacy = r#"{
        "version": 2,
        "sample_rate": 44100,
        "bpm": 120.0,
        "time_sig_num": 4,
        "time_sig_den": 4,
        "metronome_enabled": false,
        "master_volume": 0.0,
        "loop_enabled": false,
        "loop_in": 0,
        "loop_out": 0,
        "tracks": [],
        "clips": []
    }"#;
    let file: ProjectFile = serde_json::from_str(legacy).expect("legacy project parses");
    assert!(file.automation_lanes.is_empty());
    let (_app, cmds) = replay_into_fresh(file);
    assert!(
        !cmds
            .iter()
            .any(|c| matches!(c, AudioCommand::SetTrackDeviceParams { .. })),
    );
}
