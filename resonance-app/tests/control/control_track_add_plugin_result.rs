//! `track.add_effect` / `track.add_instrument` return a handle and
//! commit synchronously (ba doc #273, todo #1234).
//!
//! The add used to reply with a bare `MutationAck` and commit only when
//! the engine echoed `PluginAdded`. A `track.set_plugin_param` issued
//! right afterwards failed with "... it carries: []", which reads as
//! "the add failed" — a field agent concluded from exactly that message
//! that the whole plugin surface was broken. These assert the plugin is
//! addressable in the same update cycle as the reply, that the reply
//! carries `(plugin_id, occurrence, slot)`, that the engine echo does
//! not duplicate the slot, and that the one remaining window (params not
//! yet reported) is refused with a message that says so.

use resonance_app::message::Message;
use resonance_app::state::ViewMode;
use resonance_app::{Resonance};
use resonance_audio::types::{AudioCommand, AudioEvent, ParamInfo, ScannedPlugin, TrackType};
use resonance_control::methods::track::{AddPluginResult, PluginParamsView};
use resonance_control::ErrorKind;
use crate::common::call;

const TRACK: u64 = 1;

fn app() -> Resonance {
    let (mut app, _task) = Resonance::new_for_test_on(ViewMode::Arrange);
    app.test_set_active_project(true);
    app.test_set_project_path(std::path::PathBuf::from("/tmp/control-add-plugin.rprj"));
    app.test_add_track(TRACK, TrackType::Instrument);
    app.test_apply_engine_event(AudioEvent::PluginsScanned {
        plugins: vec![
            ScannedPlugin {
                clap_file_path: "/plugins/wavetable.clap".to_owned(),
                clap_plugin_id: "com.resonance.wavetable".to_owned(),
                name: "Resonance Wavetable".to_owned(),
                vendor: "Resonance".to_owned(),
                is_instrument: true,
            ..Default::default()
},
            ScannedPlugin {
                clap_file_path: "/plugins/eq.clap".to_owned(),
                clap_plugin_id: "com.resonance.eq".to_owned(),
                name: "Resonance EQ".to_owned(),
                vendor: "Resonance".to_owned(),
                is_instrument: false,
            ..Default::default()
},
        ],
    });
    app
}

fn add_effect(app: &mut Resonance, plugin_id: &str) -> AddPluginResult {
    call(
        app,
        "track.add_effect",
        serde_json::json!({"track_id": TRACK, "plugin_id": plugin_id}),
    )
    .result()
    .expect("track.add_effect succeeds")
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

/// The instance id the app allocated on the most recent add.
fn hinted_instance(rx: &crossbeam_channel::Receiver<AudioCommand>) -> u64 {
    std::iter::from_fn(|| rx.try_recv().ok())
        .find_map(|c| match c {
            AudioCommand::AddPlugin { id, .. } => Some(id),
            _ => None,
        })
        .expect("an AddPlugin command reached the engine")
}

#[test]
fn the_added_effect_is_addressable_in_the_same_cycle_as_the_reply() {
    let mut app = app();
    let result = add_effect(&mut app, "com.resonance.eq");
    assert_eq!(result.plugin_id, "com.resonance.eq");
    assert_eq!(result.occurrence, 0);
    assert_eq!(result.slot, 0, "first plugin on an empty chain");

    // No engine echo has been applied — the plugin must be visible
    // anyway, which is the whole point of the todo.
    let view = chain(&mut app);
    assert_eq!(view.plugins.len(), 1, "visible with no engine round-trip");
    assert_eq!(view.plugins[0].plugin_id, "com.resonance.eq");
    assert_eq!(view.plugins[0].slot, result.slot);
    assert_eq!(view.plugins[0].occurrence, result.occurrence);
}

#[test]
fn adding_the_same_effect_twice_numbers_the_occurrences() {
    let mut app = app();
    let first = add_effect(&mut app, "com.resonance.eq");
    let second = add_effect(&mut app, "com.resonance.eq");
    assert_eq!((first.occurrence, first.slot), (0, 0));
    assert_eq!((second.occurrence, second.slot), (1, 1));

    let view = chain(&mut app);
    let occurrences: Vec<u32> = view.plugins.iter().map(|p| p.occurrence).collect();
    assert_eq!(occurrences, vec![0, 1], "the chain agrees with the replies");
}

#[test]
fn add_instrument_returns_the_same_shape() {
    let mut app = app();
    let result: AddPluginResult = call(
        &mut app,
        "track.add_instrument",
        serde_json::json!({"track_id": TRACK, "plugin_id": "com.resonance.wavetable"}),
    )
    .result()
    .expect("track.add_instrument succeeds");
    assert_eq!(result.plugin_id, "com.resonance.wavetable");
    assert_eq!((result.occurrence, result.slot), (0, 0));
    assert_eq!(chain(&mut app).plugins.len(), 1);
}

#[test]
fn the_engine_echo_fills_the_placeholder_instead_of_duplicating_it() {
    let mut app = app();
    let rx = app.test_capture_engine();
    let result = add_effect(&mut app, "com.resonance.eq");
    let instance_id = hinted_instance(&rx);

    // The echo the engine will send for exactly this instance.
    app.test_apply_engine_event(AudioEvent::PluginAdded {
        track_id: TRACK,
        instance_id,
        plugin_name: "Resonance EQ".to_owned(),
        clap_plugin_id: "com.resonance.eq".to_owned(),
        clap_file_path: "/plugins/eq.clap".to_owned(),
        params: vec![ParamInfo {
            id: 7,
            name: "Gain".to_owned(),
            min_value: -24.0,
            max_value: 24.0,
            default_value: 0.0,
            current_value: 0.0,
            ..Default::default()
        }],
        has_gui: false,
        has_sidechain_input: false,
        output_port_count: 1,
        output_port_names: vec!["Main".to_owned()],
    });

    let view = chain(&mut app);
    assert_eq!(
        view.plugins.len(),
        1,
        "the echo must fill the placeholder, not push a second slot"
    );
    assert_eq!(view.plugins[0].slot, result.slot);
    assert_eq!(
        view.plugins[0].params.len(),
        1,
        "and the echo is what supplies the parameter list"
    );
}

#[test]
fn the_gui_add_now_carries_an_app_allocated_id_but_still_waits_for_the_echo() {
    use resonance_app::message::PluginMessage;
    let mut app = app();
    let rx = app.test_capture_engine();
    let eq = ScannedPlugin {
        clap_file_path: "/plugins/eq.clap".to_owned(),
        clap_plugin_id: "com.resonance.eq".to_owned(),
        name: "Resonance EQ".to_owned(),
        vendor: "Resonance".to_owned(),
        is_instrument: false,
    ..Default::default()
};
    let _ = app.update(Message::Plugin(PluginMessage::AddPluginToTrack(TRACK, eq)));
    // ARCH-04 D-1: the command's `id` field is mandatory now (no more
    // `Option`), so simply matching it out here is the proof that the
    // app supplied a concrete id up front rather than the engine.
    let _id = std::iter::from_fn(|| rx.try_recv().ok())
        .find_map(|c| match c {
            AudioCommand::AddPlugin { id, .. } => Some(id),
            _ => None,
        })
        .expect("an AddPlugin command reached the engine");
    assert!(
        chain(&mut app).plugins.is_empty(),
        "but nothing is mirrored until the echo lands — same as before D-1, \
         only the id's origin changed"
    );
}

#[test]
fn setting_a_param_before_the_echo_says_initializing_not_no_such_parameter() {
    let mut app = app();
    add_effect(&mut app, "com.resonance.eq");

    let error = call(
        &mut app,
        "track.set_plugin_param",
        serde_json::json!({
            "track_id": TRACK,
            "plugin_id": "com.resonance.eq",
            "param": "Gain",
            "value": 3.0,
        }),
    )
    .error
    .expect("the parameter list has not arrived yet");
    assert_eq!(
        error.kind(),
        ErrorKind::Busy,
        "a retryable window, not a not_found: {}",
        error.message
    );
    assert!(
        error.message.contains("still initializing"),
        "the message must say what is actually happening: {}",
        error.message
    );
    assert!(
        !error.message.contains("has no parameter"),
        "and must NOT read as a failed add: {}",
        error.message
    );
}

#[test]
fn the_initializing_window_is_distinguishable_from_the_other_two_failures() {
    let mut app = app();
    let rx = app.test_capture_engine();
    add_effect(&mut app, "com.resonance.eq");
    let instance_id = hinted_instance(&rx);
    app.test_apply_engine_event(AudioEvent::PluginAdded {
        track_id: TRACK,
        instance_id,
        plugin_name: "Resonance EQ".to_owned(),
        clap_plugin_id: "com.resonance.eq".to_owned(),
        clap_file_path: "/plugins/eq.clap".to_owned(),
        params: vec![ParamInfo {
            id: 7,
            name: "Gain".to_owned(),
            min_value: -24.0,
            max_value: 24.0,
            default_value: 0.0,
            current_value: 0.0,
            ..Default::default()
        }],
        has_gui: false,
        has_sidechain_input: false,
        output_port_count: 1,
        output_port_names: vec!["Main".to_owned()],
    });

    // No such plugin on the track.
    let error = call(
        &mut app,
        "track.set_plugin_param",
        serde_json::json!({
            "track_id": TRACK,
            "plugin_id": "com.resonance.reverb",
            "param": "Gain",
            "value": 1.0,
        }),
    )
    .error
    .expect("unknown plugin rejected");
    assert_eq!(error.kind(), ErrorKind::NotFound);

    // No such parameter on a plugin whose params ARE known.
    let error = call(
        &mut app,
        "track.set_plugin_param",
        serde_json::json!({
            "track_id": TRACK,
            "plugin_id": "com.resonance.eq",
            "param": "Nonexistent",
            "value": 1.0,
        }),
    )
    .error
    .expect("unknown parameter rejected");
    assert_eq!(error.kind(), ErrorKind::NotFound);
    assert!(error.message.contains("has no parameter"), "{}", error.message);

    // ...and the set that names a real parameter now works.
    let _: resonance_control::MutationAck = call(
        &mut app,
        "track.set_plugin_param",
        serde_json::json!({
            "track_id": TRACK,
            "plugin_id": "com.resonance.eq",
            "param": "Gain",
            "value": 3.0,
        }),
    )
    .result()
    .expect("once the echo lands the set succeeds");
}

#[test]
fn the_add_is_one_committed_undoable_edit() {
    let mut app = app();
    let before = app.revision();
    let result = add_effect(&mut app, "com.resonance.eq");
    assert_eq!(app.revision(), before + 1);
    assert_eq!(result.revision, app.revision(), "the reply reports it");
}

#[test]
fn an_unknown_plugin_id_is_still_refused_before_anything_is_allocated() {
    let mut app = app();
    let error = call(
        &mut app,
        "track.add_effect",
        serde_json::json!({"track_id": TRACK, "plugin_id": "com.resonance.nope"}),
    )
    .error
    .expect("unknown plugin id rejected");
    assert_eq!(error.kind(), ErrorKind::NotFound);
    assert!(chain(&mut app).plugins.is_empty(), "nothing was mirrored");
}

// ---------------------------------------------------------------------------
// CTL-04: `track.add_instrument` SETS the track's instrument
// ---------------------------------------------------------------------------

fn app_with_drums() -> Resonance {
    let mut app = app();
    let plugin = |id: &str, is_instrument: bool| ScannedPlugin {
        clap_file_path: format!("/plugins/{id}.clap"),
        clap_plugin_id: id.to_owned(),
        name: id.to_owned(),
        vendor: "Resonance".to_owned(),
        is_instrument,
        ..Default::default()
    };
    let plugins = vec![
        plugin("com.resonance.wavetable", true),
        plugin("com.resonance.eq", false),
        plugin("com.resonance.drums", true),
    ];
    app.test_apply_engine_event(AudioEvent::PluginsScanned { plugins });
    app
}

fn add_instrument(app: &mut Resonance, plugin_id: &str) -> AddPluginResult {
    call(
        app,
        "track.add_instrument",
        serde_json::json!({"track_id": TRACK, "plugin_id": plugin_id}),
    )
    .result()
    .expect("track.add_instrument succeeds")
}

#[test]
fn a_second_add_instrument_replaces_the_first_in_place() {
    let mut app = app_with_drums();
    add_instrument(&mut app, "com.resonance.wavetable");
    add_effect(&mut app, "com.resonance.eq");

    let result = add_instrument(&mut app, "com.resonance.drums");
    assert_eq!(result.plugin_id, "com.resonance.drums");
    assert_eq!((result.occurrence, result.slot), (0, 0));

    let ids: Vec<String> = chain(&mut app)
        .plugins
        .into_iter()
        .map(|p| p.plugin_id)
        .collect();
    assert_eq!(
        ids,
        vec!["com.resonance.drums", "com.resonance.eq"],
        "exactly one instrument, at the old instrument's slot, effects untouched"
    );
}

#[test]
fn repeating_add_instrument_with_the_same_id_is_a_no_op() {
    let mut app = app_with_drums();
    add_instrument(&mut app, "com.resonance.wavetable");
    let before = app.revision();
    let again = add_instrument(&mut app, "com.resonance.wavetable");
    assert_eq!((again.occurrence, again.slot), (0, 0));
    assert_eq!(chain(&mut app).plugins.len(), 1, "a retry must not stack a copy");
    assert_eq!(app.revision(), before, "and must not burn an undo entry");
}
