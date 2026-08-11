//! `track.remove_effect` + `slot` on `track.plugin_params` (ba doc #273,
//! todo #1223).
//!
//! Chain *reporting* already existed; what did not was any way to take
//! an effect back off. `track.add_effect` appends a fresh instance on
//! every call, so before this a wrong add was unrecoverable over the
//! control API, and the entries carried no chain position at all — a
//! client could not even describe where a plugin sat.

use resonance_app::control_socket::{ControlMessage, ControlRequest, ReplySender};
use resonance_app::message::Message;
use resonance_app::state::ViewMode;
use resonance_app::{Resonance, STARTUP_TAB};
use resonance_audio::types::{AudioCommand, AudioEvent, ScannedPlugin, TrackType};
use resonance_control::methods::track::{PluginKind, PluginParamsView};
use resonance_control::{ErrorKind, MutationAck, Request, Response};

const TRACK: u64 = 1;

fn app() -> Resonance {
    let _ = STARTUP_TAB.set(ViewMode::Arrange);
    let (mut app, _task) = Resonance::new();
    app.test_set_active_project(true);
    app.test_set_project_path(std::path::PathBuf::from("/tmp/control-remove-effect.rprj"));
    app.test_add_track(TRACK, TrackType::Instrument);
    app.test_apply_engine_event(AudioEvent::PluginsScanned {
        plugins: vec![
            ScannedPlugin {
                clap_file_path: "/plugins/wavetable.clap".to_owned(),
                clap_plugin_id: "com.resonance.wavetable".to_owned(),
                name: "Resonance Wavetable".to_owned(),
                vendor: "Resonance".to_owned(),
                is_instrument: true,
            },
            ScannedPlugin {
                clap_file_path: "/plugins/eq.clap".to_owned(),
                clap_plugin_id: "com.resonance.eq".to_owned(),
                name: "Resonance EQ".to_owned(),
                vendor: "Resonance".to_owned(),
                is_instrument: false,
            },
            ScannedPlugin {
                clap_file_path: "/plugins/reverb.clap".to_owned(),
                clap_plugin_id: "com.resonance.reverb".to_owned(),
                name: "Resonance Reverb".to_owned(),
                vendor: "Resonance".to_owned(),
                is_instrument: false,
            },
        ],
    });
    app
}

fn roundtrip(app: &mut Resonance, req: Request) -> Response {
    let (reply, rx) = ReplySender::test_pair();
    let _ = app.update(Message::Control(ControlMessage::Request(ControlRequest {
        conn: 1,
        request: req,
        reply,
    })));
    rx.try_recv().expect("one reply per request")
}

fn call(app: &mut Resonance, method: &str, params: serde_json::Value) -> Response {
    roundtrip(app, Request::new(1, method, &params).expect("params serialize"))
}

/// Mirror the engine echo that mounts a plugin on the track.
fn echo_plugin(app: &mut Resonance, instance_id: u64, plugin_id: &str) {
    app.test_apply_engine_event(AudioEvent::PluginAdded {
        track_id: TRACK,
        instance_id,
        plugin_name: plugin_id.to_owned(),
        clap_plugin_id: plugin_id.to_owned(),
        clap_file_path: format!("/plugins/{plugin_id}.clap"),
        params: Vec::new(),
        has_gui: false,
        has_sidechain_input: false,
        output_port_count: 1,
        output_port_names: vec!["Main".to_owned()],
    });
}

fn chain(app: &mut Resonance) -> PluginParamsView {
    call(
        app,
        "track.plugin_params",
        serde_json::json!({"track_id": TRACK}),
    )
    .result()
    .expect("track.plugin_params succeeds")
}

fn remove(app: &mut Resonance, params: serde_json::Value) -> Response {
    let mut p = params;
    p["track_id"] = serde_json::json!(TRACK);
    call(app, "track.remove_effect", p)
}

#[test]
fn plugin_params_reports_ascending_slots_in_chain_order() {
    let mut app = app();
    echo_plugin(&mut app, 10, "com.resonance.wavetable");
    echo_plugin(&mut app, 11, "com.resonance.eq");
    echo_plugin(&mut app, 12, "com.resonance.reverb");

    let view = chain(&mut app);
    let slots: Vec<u32> = view.plugins.iter().map(|p| p.slot).collect();
    assert_eq!(slots, vec![0, 1, 2], "slots ascend and match chain order");
    assert_eq!(view.plugins[0].kind, PluginKind::Instrument);
    assert_eq!(view.plugins[1].plugin_id, "com.resonance.eq");
    assert_eq!(view.plugins[2].plugin_id, "com.resonance.reverb");
}

#[test]
fn removing_a_slot_leaves_the_other_effect_renumbered() {
    let mut app = app();
    echo_plugin(&mut app, 10, "com.resonance.wavetable");
    echo_plugin(&mut app, 11, "com.resonance.eq");
    echo_plugin(&mut app, 12, "com.resonance.reverb");

    let rx = app.test_capture_engine();
    let _: MutationAck = remove(&mut app, serde_json::json!({"slot": 1}))
        .result()
        .expect("track.remove_effect succeeds");
    assert!(
        std::iter::from_fn(|| rx.try_recv().ok()).any(
            |c| matches!(c, AudioCommand::RemovePlugin { instance_id, .. } if instance_id == 11)
        ),
        "the engine must be told to unload the EQ instance"
    );

    app.test_apply_engine_event(AudioEvent::PluginRemoved {
        track_id: TRACK,
        instance_id: 11,
    });
    let view = chain(&mut app);
    let ids: Vec<&str> = view.plugins.iter().map(|p| p.plugin_id.as_str()).collect();
    assert_eq!(ids, vec!["com.resonance.wavetable", "com.resonance.reverb"]);
    let slots: Vec<u32> = view.plugins.iter().map(|p| p.slot).collect();
    assert_eq!(slots, vec![0, 1], "the survivor renumbers");
}

#[test]
fn occurrence_targets_the_right_copy_of_a_repeated_effect() {
    let mut app = app();
    echo_plugin(&mut app, 10, "com.resonance.wavetable");
    echo_plugin(&mut app, 11, "com.resonance.eq");
    echo_plugin(&mut app, 12, "com.resonance.eq");

    let view = chain(&mut app);
    let occurrences: Vec<u32> = view
        .plugins
        .iter()
        .filter(|p| p.plugin_id == "com.resonance.eq")
        .map(|p| p.occurrence)
        .collect();
    assert_eq!(occurrences, vec![0, 1]);

    let rx = app.test_capture_engine();
    let _: MutationAck = remove(
        &mut app,
        serde_json::json!({"plugin_id": "com.resonance.eq", "occurrence": 1}),
    )
    .result()
    .expect("succeeds");
    assert!(
        std::iter::from_fn(|| rx.try_recv().ok()).any(
            |c| matches!(c, AudioCommand::RemovePlugin { instance_id, .. } if instance_id == 12)
        ),
        "occurrence 1 is the SECOND EQ (instance 12)"
    );
}

#[test]
fn removing_the_instrument_is_refused() {
    let mut app = app();
    echo_plugin(&mut app, 10, "com.resonance.wavetable");
    echo_plugin(&mut app, 11, "com.resonance.eq");

    for params in [
        serde_json::json!({"slot": 0}),
        serde_json::json!({"plugin_id": "com.resonance.wavetable"}),
    ] {
        let error = remove(&mut app, params.clone())
            .error
            .unwrap_or_else(|| panic!("{params} should be refused"));
        assert_eq!(error.kind(), ErrorKind::InvalidParams, "for {params}");
        assert!(
            error.message.contains("track.add_instrument"),
            "the error must point at the replacement path: {}",
            error.message
        );
    }
    assert_eq!(chain(&mut app).plugins.len(), 2, "nothing was removed");
}

#[test]
fn addressing_must_be_unambiguous_and_misses_are_not_found() {
    let mut app = app();
    echo_plugin(&mut app, 10, "com.resonance.wavetable");
    echo_plugin(&mut app, 11, "com.resonance.eq");

    for params in [
        serde_json::json!({}),
        serde_json::json!({"slot": 1, "plugin_id": "com.resonance.eq"}),
    ] {
        let error = remove(&mut app, params.clone())
            .error
            .unwrap_or_else(|| panic!("{params} should be rejected"));
        assert_eq!(error.kind(), ErrorKind::InvalidParams, "for {params}");
    }

    let error = remove(&mut app, serde_json::json!({"slot": 9}))
        .error
        .expect("missing slot rejected");
    assert_eq!(error.kind(), ErrorKind::NotFound);
    assert!(error.message.contains("com.resonance.eq"), "{}", error.message);

    let error = remove(
        &mut app,
        serde_json::json!({"plugin_id": "com.resonance.reverb"}),
    )
    .error
    .expect("effect not on the track rejected");
    assert_eq!(error.kind(), ErrorKind::NotFound);

    let error = call(
        &mut app,
        "track.remove_effect",
        serde_json::json!({"track_id": 4242, "slot": 0}),
    )
    .error
    .expect("unknown track rejected");
    assert_eq!(error.kind(), ErrorKind::NotFound);
}

#[test]
fn the_removal_is_recorded_on_the_undo_stack() {
    let mut app = app();
    echo_plugin(&mut app, 10, "com.resonance.wavetable");
    echo_plugin(&mut app, 11, "com.resonance.eq");

    let before = app.revision();
    let _: MutationAck = remove(&mut app, serde_json::json!({"slot": 1}))
        .result()
        .expect("succeeds");
    assert_eq!(app.revision(), before + 1, "the removal is a committed edit");
    // The engine echo is what actually drops it from the app's chain.
    app.test_apply_engine_event(AudioEvent::PluginRemoved {
        track_id: TRACK,
        instance_id: 11,
    });
    assert_eq!(chain(&mut app).plugins.len(), 1);

    // Removing a plugin is a structural change, so undo takes the
    // ClearAll -> replay path; the engine round-trip that finishes it is
    // asynchronous, so assert the restore actually starts.
    let rx = app.test_capture_engine();
    let _ = app.update(Message::Undo);
    assert!(
        std::iter::from_fn(|| rx.try_recv().ok()).any(|c| matches!(c, AudioCommand::ClearAll)),
        "undo must find the removal and start restoring the pre-removal snapshot"
    );
}
