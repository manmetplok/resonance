//! Regression: plugin parameter values must round-trip through project
//! save/load.
//!
//! Measured before the fix:
//!
//! ```text
//! track.set_plugin_param(param="Filter Cutoff", value=777)
//! track.set_plugin_param(param="Distortion On", value=1)
//! mixer.set_volume(volume=0.33)
//! project.save_as(path); project.open(path)
//! -> Filter Cutoff reads 8000 (the DEFAULT), Distortion On reads 0
//! -> mixer volume correctly reads 0.33
//! ```
//!
//! The mixer fader is a plain scalar in `project.json`, written on save and
//! pushed back into the GUI mirror on load. Plugin parameters had neither:
//! `project.json` carried only an opaque CLAP state blob path, and the
//! app-side `ParamInfo` mirror that every reader consults
//! (`track.plugin_params`, the mixer panel, automation, the freeze
//! fingerprint) is populated *once*, from the `PluginAdded` event the
//! engine emits at instantiation — i.e. at the plugin's defaults — with
//! nothing re-reading it after `LoadPluginState`. A saved project could
//! therefore not reproduce its own bounce.
//!
//! The fix persists the non-default values explicitly in `project.json`
//! and re-applies them (to the mirror *and* to the engine) when the
//! instance's `PluginAdded` lands.

use resonance_app::control_socket::{ControlMessage, ControlRequest, ReplySender};
use resonance_app::message::Message;
use resonance_app::state::{PluginSlotState, ViewMode};
use resonance_app::{Resonance, STARTUP_TAB};
use resonance_audio::types::{AudioCommand, AudioEvent, ParamInfo, TrackType};
use resonance_control::ids::TrackId as ProtoTrackId;
use resonance_control::methods::{mixer as mixer_proto, track as track_proto};
use resonance_control::{Request, Response};

const TRACK: u64 = 80;
const INSTANCE: u64 = 900;
const PLUGIN_ID: &str = "com.resonance.test-synth";

const CUTOFF_ID: u32 = 1;
const DISTORTION_ID: u32 = 2;
const RESONANCE_ID: u32 = 3;

fn app_with_project() -> Resonance {
    let _ = STARTUP_TAB.set(ViewMode::Arrange);
    let (mut app, _task) = Resonance::new();
    app.test_set_active_project(true);
    app.test_set_project_path(std::path::PathBuf::from("/tmp/control-plugin-params.rprj"));
    app
}

fn roundtrip(app: &mut Resonance, request: Request) -> Response {
    let (reply, rx) = ReplySender::test_pair();
    let _ = app.update(Message::Control(ControlMessage::Request(ControlRequest {
        conn: 1,
        request,
        reply,
    })));
    rx.try_recv().expect("every request gets exactly one reply")
}

fn call<T: serde::Serialize>(app: &mut Resonance, method: &str, params: &T) -> Response {
    roundtrip(app, Request::new(1, method, params).expect("params serialize"))
}

/// The plugin's parameter list exactly as the engine reports it at
/// instantiation: every value sitting at its default. This is the list
/// that used to clobber whatever a load had restored.
fn params_at_defaults() -> Vec<ParamInfo> {
    vec![
        ParamInfo {
            id: CUTOFF_ID,
            name: "Filter Cutoff".to_owned(),
            min_value: 20.0,
            max_value: 20_000.0,
            default_value: 8000.0,
            current_value: 8000.0,
        },
        ParamInfo {
            id: DISTORTION_ID,
            name: "Distortion On".to_owned(),
            min_value: 0.0,
            max_value: 1.0,
            default_value: 0.0,
            current_value: 0.0,
        },
        ParamInfo {
            id: RESONANCE_ID,
            name: "Resonance".to_owned(),
            min_value: 0.0,
            max_value: 1.0,
            default_value: 0.5,
            current_value: 0.5,
        },
    ]
}

fn add_track_with_plugin(app: &mut Resonance) {
    app.test_add_track(TRACK, TrackType::Instrument);
    app.test_push_track_plugin(
        TRACK,
        PluginSlotState::new(
            INSTANCE,
            "Test Synth".to_owned(),
            PLUGIN_ID.to_owned(),
            "/nonexistent/test-synth.clap".to_owned(),
            params_at_defaults(),
            false,
        ),
    );
}

fn set_param(app: &mut Resonance, param: &str, value: f64) {
    let response = call(
        app,
        "track.set_plugin_param",
        &track_proto::SetPluginParamParams {
            track_id: ProtoTrackId(TRACK),
            plugin_id: Some(PLUGIN_ID.to_owned()),
            occurrence: None,
            param: param.to_owned(),
            value,
        },
    );
    assert!(
        response.error.is_none(),
        "set {param}={value} failed: {}",
        response.error.map(|e| e.message).unwrap_or_default()
    );
}

/// Read a parameter back the way a remote client does — through
/// `track.plugin_params`, not through app internals.
fn read_param(app: &mut Resonance, name: &str) -> f64 {
    let view: track_proto::PluginParamsView = call(
        app,
        "track.plugin_params",
        &track_proto::PluginParamsParams {
            track_id: ProtoTrackId(TRACK),
            plugin_id: Some(PLUGIN_ID.to_owned()),
            occurrence: None,
        },
    )
    .result()
    .expect("track.plugin_params succeeds");
    view.plugins
        .iter()
        .flat_map(|p| p.params.iter())
        .find(|p| p.name == name)
        .unwrap_or_else(|| panic!("no parameter named {name:?}"))
        .value
}

/// Replay the project as if reopened, then drive the `PluginAdded` echo
/// the engine sends back for each re-instantiated plugin — carrying, as
/// the real engine does, the params *as instantiated*, i.e. at defaults.
fn reopen(app: &mut Resonance, file: resonance_app::project::ProjectFile) {
    app.test_replay_loaded_project(file);
    app.test_apply_engine_event(AudioEvent::PluginAdded {
        track_id: TRACK,
        instance_id: INSTANCE,
        plugin_name: "Test Synth".to_owned(),
        clap_plugin_id: PLUGIN_ID.to_owned(),
        clap_file_path: "/nonexistent/test-synth.clap".to_owned(),
        params: params_at_defaults(),
        has_gui: false,
        output_port_count: 1,
        output_port_names: vec!["Main".to_owned()],
    });
}

// ---------------------------------------------------------------------------

/// The reported reproduction, end to end.
#[test]
fn plugin_params_survive_save_and_reopen_like_mixer_volume_does() {
    let mut app = app_with_project();
    add_track_with_plugin(&mut app);

    set_param(&mut app, "Filter Cutoff", 777.0);
    set_param(&mut app, "Distortion On", 1.0);
    call(
        &mut app,
        "mixer.set_volume",
        &mixer_proto::SetVolumeParams {
            track_id: ProtoTrackId(TRACK),
            volume: 0.33,
        },
    )
    .result::<resonance_control::MutationAck>()
    .expect("mixer.set_volume succeeds");

    assert_eq!(read_param(&mut app, "Filter Cutoff"), 777.0);

    let file = app.test_build_project_file();
    reopen(&mut app, file);

    assert_eq!(
        read_param(&mut app, "Filter Cutoff"),
        777.0,
        "Filter Cutoff reverted to its default across save/reopen"
    );
    assert_eq!(
        read_param(&mut app, "Distortion On"),
        1.0,
        "Distortion On reverted to its default across save/reopen"
    );
    // Untouched parameters keep their defaults — restoring must not
    // invent values for knobs nobody moved.
    assert_eq!(read_param(&mut app, "Resonance"), 0.5);
}

/// `project.json` must carry the values as plain, inspectable data —
/// and only the ones that actually differ from the plugin's defaults, so
/// an untouched chain adds nothing to the file.
#[test]
fn only_non_default_values_are_written_to_the_project_file() {
    let mut app = app_with_project();
    add_track_with_plugin(&mut app);

    let untouched = app.test_build_project_file();
    assert!(
        untouched.tracks[0].plugins[0].params.is_empty(),
        "an untouched plugin chain writes no parameter entries"
    );

    set_param(&mut app, "Filter Cutoff", 777.0);
    let file = app.test_build_project_file();
    let saved = &file.tracks[0].plugins[0].params;
    assert_eq!(saved.len(), 1, "only the moved parameter is written: {saved:?}");
    assert_eq!(saved[0].id, CUTOFF_ID);
    assert_eq!(saved[0].name, "Filter Cutoff");
    assert_eq!(saved[0].value, 777.0);

    // And the JSON really is human-readable, not an opaque blob.
    let json = serde_json::to_string(&file.tracks[0].plugins[0]).expect("serializes");
    assert!(json.contains("Filter Cutoff"), "{json}");
    assert!(json.contains("777"), "{json}");
}

/// Restoring must reach the *engine*, not just the GUI mirror — otherwise
/// the reopened project shows the right numbers and bounces the wrong
/// audio, which is the failure mode that makes this bug expensive.
#[test]
fn restored_values_are_pushed_to_the_engine() {
    let mut app = app_with_project();
    add_track_with_plugin(&mut app);
    set_param(&mut app, "Filter Cutoff", 777.0);
    set_param(&mut app, "Distortion On", 1.0);
    let file = app.test_build_project_file();

    let rx = app.test_capture_engine();
    reopen(&mut app, file);

    let mut sent: Vec<(u32, f64)> = Vec::new();
    while let Ok(cmd) = rx.try_recv() {
        if let AudioCommand::SetPluginParam {
            instance_id,
            param_id,
            value,
        } = cmd
        {
            assert_eq!(instance_id, INSTANCE);
            sent.push((param_id, value));
        }
    }
    sent.sort_by_key(|(id, _)| *id);
    assert_eq!(
        sent,
        vec![(CUTOFF_ID, 777.0), (DISTORTION_ID, 1.0)],
        "both restored values must be pushed to the engine"
    );
}

/// A parameter that no longer exists on the instantiated plugin (renamed,
/// renumbered, or dropped in a newer plugin version) is skipped rather
/// than pushed at the engine as a stale id.
#[test]
fn a_stale_param_id_is_skipped_on_load() {
    let mut app = app_with_project();
    add_track_with_plugin(&mut app);
    set_param(&mut app, "Filter Cutoff", 777.0);

    let mut file = app.test_build_project_file();
    file.tracks[0].plugins[0].params.push(resonance_app::project::ProjectPluginParam {
        id: 4242,
        name: "Removed Knob".to_owned(),
        value: 0.9,
    });

    let rx = app.test_capture_engine();
    reopen(&mut app, file);

    let mut ids: Vec<u32> = Vec::new();
    while let Ok(cmd) = rx.try_recv() {
        if let AudioCommand::SetPluginParam { param_id, .. } = cmd {
            ids.push(param_id);
        }
    }
    assert_eq!(ids, vec![CUTOFF_ID], "the vanished parameter is not pushed");
    assert_eq!(read_param(&mut app, "Filter Cutoff"), 777.0);
}

/// Projects saved before the field existed still load — the parameter
/// list is `#[serde(default)]`.
#[test]
fn a_project_file_without_the_params_field_still_loads() {
    let mut app = app_with_project();
    add_track_with_plugin(&mut app);
    set_param(&mut app, "Filter Cutoff", 777.0);

    let file = app.test_build_project_file();
    let mut json = serde_json::to_value(&file).expect("serializes");
    for track in json["tracks"].as_array_mut().expect("tracks array") {
        for plugin in track["plugins"].as_array_mut().expect("plugins array") {
            plugin.as_object_mut().expect("object").remove("params");
        }
    }
    let legacy: resonance_app::project::ProjectFile =
        serde_json::from_value(json).expect("a pre-params project still deserializes");
    assert!(legacy.tracks[0].plugins[0].params.is_empty());

    reopen(&mut app, legacy);
    assert_eq!(
        read_param(&mut app, "Filter Cutoff"),
        8000.0,
        "with nothing persisted, the plugin's own default stands"
    );
}
