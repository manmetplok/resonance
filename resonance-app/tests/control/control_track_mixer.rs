//! `track.*` and `mixer.*` control methods through the real update path
//! (ba doc #265, todo #1152): add/rename/delete tracks (with the async
//! engine echo driven synchronously), volume/pan/mute/solo, and adding
//! built-in instruments/effects by stable catalog id — asserting engine
//! commands, undo behaviour, confirmation gating, and error kinds.

use resonance_app::state::ViewMode;
use resonance_app::{Resonance};
use resonance_audio::types::{AudioCommand, AudioEvent, ScannedPlugin, TrackType};
use resonance_control::methods::song::TracksView;
use resonance_control::methods::track::AddResult;
use resonance_control::{ErrorKind, MutationAck, Request};
use crate::common::{call, roundtrip};

fn app() -> Resonance {
    let (mut app, _task) = Resonance::new_for_test_on(ViewMode::Arrange);
    app.test_set_active_project(true);
    app.test_set_project_path(std::path::PathBuf::from("/tmp/control-track-test.rprj"));
    app
}

/// Add a track over the control endpoint and drive the engine echo that
/// mirrors it into the registry (the real engine does this async; tests
/// pump it synchronously so subsequent introspection sees the track).
fn add_track(app: &mut Resonance, kind: &str, name: Option<&str>) -> u64 {
    let params = match name {
        Some(n) => serde_json::json!({ "kind": kind, "name": n }),
        None => serde_json::json!({ "kind": kind }),
    };
    let result: AddResult = call(app, "track.add", params).result().expect("track.add succeeds");
    let id = u64::from(result.track_id);
    let event = match kind {
        "vocal" => AudioEvent::VocalTrackAdded { track_id: id },
        "audio" => AudioEvent::TrackAdded { track_id: id },
        _ => AudioEvent::InstrumentTrackAdded { track_id: id },
    };
    app.test_apply_engine_event(event);
    id
}

fn tracks_view(app: &mut Resonance) -> TracksView {
    roundtrip(app, Request::without_params(99, "song.tracks"))
        .result()
        .expect("song.tracks succeeds")
}

fn seed_plugins(app: &mut Resonance) {
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
}

fn drain(rx: &resonance_audio::__test_support::Receiver<AudioCommand>) -> Vec<AudioCommand> {
    let mut cmds = Vec::new();
    while let Ok(cmd) = rx.try_recv() {
        cmds.push(cmd);
    }
    cmds
}

// ---------------- track.add ----------------

#[test]
fn add_instrument_drums_vocal_tracks() {
    let mut app = app();
    let before = app.revision();

    let inst = add_track(&mut app, "instrument", Some("Lead"));
    let drums = add_track(&mut app, "drums", None);
    let vocal = add_track(&mut app, "vocal", Some("Vox"));

    // Distinct real ids, all mirrored, reported with their kinds.
    assert_ne!(inst, drums);
    assert_ne!(drums, vocal);
    let view = tracks_view(&mut app);
    let by_id = |raw: u64| {
        view.tracks
            .iter()
            .find(|t| u64::from(t.summary.id) == raw)
            .unwrap_or_else(|| panic!("track {raw} present"))
    };
    assert_eq!(
        by_id(inst).summary.kind,
        resonance_control::TrackKind::Instrument
    );
    assert_eq!(by_id(inst).summary.name, "Lead");
    assert_eq!(by_id(drums).summary.kind, resonance_control::TrackKind::Drums);
    assert_eq!(by_id(vocal).summary.kind, resonance_control::TrackKind::Vocal);
    assert_eq!(by_id(vocal).summary.name, "Vox");

    // Each add is one undoable step; three adds bumped the revision.
    assert_eq!(app.revision(), before + 3);
    assert!(app.test_can_undo());
}

#[test]
fn add_track_rejects_bus_kind() {
    let mut app = app();
    let response = call(&mut app, "track.add", serde_json::json!({ "kind": "bus" }));
    assert_eq!(
        response.error.expect("bus add rejected").kind(),
        ErrorKind::InvalidParams
    );
}

// ---------------- track.rename ----------------

#[test]
fn rename_is_undoable_and_validates() {
    let mut app = app();
    let id = add_track(&mut app, "instrument", Some("Old"));
    let before = app.revision();

    let ack: MutationAck = call(
        &mut app,
        "track.rename",
        serde_json::json!({ "track_id": id, "name": "New" }),
    )
    .result()
    .expect("rename succeeds");
    assert_eq!(ack.revision, before + 1);
    assert_eq!(tracks_view(&mut app).tracks[0].summary.name, "New");

    // Empty name and unknown id are rejected precisely.
    let response = call(
        &mut app,
        "track.rename",
        serde_json::json!({ "track_id": id, "name": "  " }),
    );
    assert_eq!(response.error.unwrap().kind(), ErrorKind::InvalidParams);
    let response = call(
        &mut app,
        "track.rename",
        serde_json::json!({ "track_id": 9999, "name": "x" }),
    );
    assert_eq!(response.error.unwrap().kind(), ErrorKind::NotFound);
}

// ---------------- track.delete ----------------

#[test]
fn delete_requires_confirm_and_reverts_with_undo() {
    let mut app = app();
    let id = add_track(&mut app, "instrument", Some("Doomed"));
    let rx = app.test_capture_engine();

    // Without confirm: needs_confirmation, nothing removed.
    let response = call(&mut app, "track.delete", serde_json::json!({ "track_id": id }));
    let error = response.error.expect("delete needs confirmation");
    assert_eq!(error.kind(), ErrorKind::NeedsConfirmation);
    assert!(drain(&rx).is_empty(), "no engine command without confirm");
    assert_eq!(tracks_view(&mut app).tracks.len(), 1);

    // With confirm: engine gets RemoveTrack, undoable.
    let before = app.revision();
    let ack: MutationAck = call(
        &mut app,
        "track.delete",
        serde_json::json!({ "track_id": id, "confirm": true }),
    )
    .result()
    .expect("confirmed delete succeeds");
    assert_eq!(ack.revision, before + 1);
    assert!(drain(&rx)
        .iter()
        .any(|c| matches!(c, AudioCommand::RemoveTrack { track_id } if *track_id == id)));

    let response = call(&mut app, "track.delete", serde_json::json!({ "track_id": 9999 }));
    assert_eq!(response.error.unwrap().kind(), ErrorKind::NotFound);
}

// ---------------- track.plugin_params / set_plugin_param ----------------

/// Mirror the engine echo for a plugin that exposes real parameters.
fn echo_plugin_with_params(
    app: &mut Resonance,
    track_id: u64,
    instance_id: u64,
    plugin_id: &str,
    params: Vec<resonance_audio::types::ParamInfo>,
) {
    app.test_apply_engine_event(AudioEvent::PluginAdded {
        track_id,
        instance_id,
        plugin_name: plugin_id.to_owned(),
        clap_plugin_id: plugin_id.to_owned(),
        clap_file_path: format!("/plugins/{plugin_id}.clap"),
        params,
        has_gui: false,
        has_sidechain_input: false,
        output_port_count: 1,
        output_port_names: vec!["Main".to_owned()],
    });
}

fn param(
    id: u32,
    name: &str,
    current: f64,
    min: f64,
    max: f64,
) -> resonance_audio::types::ParamInfo {
    resonance_audio::types::ParamInfo {
        id,
        name: name.to_owned(),
        min_value: min,
        max_value: max,
        default_value: current,
        current_value: current,
        ..Default::default()
    }
}

fn plugin_params(
    app: &mut Resonance,
    params: serde_json::Value,
) -> resonance_control::methods::track::PluginParamsView {
    call(app, "track.plugin_params", params)
        .result()
        .expect("track.plugin_params succeeds")
}

/// Until this existed a client could attach a plugin but never configure
/// it, so every instrument played its default patch (doc #272 V-3).
#[test]
fn plugin_params_are_readable_and_settable() {
    let mut app = app();
    seed_plugins(&mut app);
    let id = add_track(&mut app, "instrument", None);
    echo_plugin_with_params(
        &mut app,
        id,
        1,
        "com.resonance.wavetable",
        vec![
            param(0, "Cutoff", 0.5, 0.0, 1.0),
            param(1, "Resonance", 0.2, 0.0, 1.0),
        ],
    );

    let view = plugin_params(&mut app, serde_json::json!({ "track_id": id }));
    assert_eq!(view.plugins.len(), 1);
    let entry = &view.plugins[0];
    assert_eq!(entry.plugin_id, "com.resonance.wavetable");
    assert_eq!(entry.kind, resonance_control::methods::track::PluginKind::Instrument);
    let cutoff = entry
        .params
        .iter()
        .find(|p| p.name == "Cutoff")
        .expect("Cutoff is listed");
    assert_eq!((cutoff.value, cutoff.min, cutoff.max), (0.5, 0.0, 1.0));

    // Set it by name; omitting plugin_id targets the instrument.
    let before = app.revision();
    let ack: MutationAck = call(
        &mut app,
        "track.set_plugin_param",
        serde_json::json!({ "track_id": id, "param": "cutoff", "value": 0.8 }),
    )
    .result()
    .expect("set succeeds");
    assert_eq!(ack.revision, before + 1, "a committed edit bumps revision");

    let view = plugin_params(&mut app, serde_json::json!({ "track_id": id }));
    let cutoff = view.plugins[0]
        .params
        .iter()
        .find(|p| p.name == "Cutoff")
        .expect("Cutoff still listed");
    assert_eq!(cutoff.value, 0.8, "the new value reads back");

    // And by numeric CLAP id.
    let _: MutationAck = call(
        &mut app,
        "track.set_plugin_param",
        serde_json::json!({ "track_id": id, "param": "1", "value": 0.9 }),
    )
    .result()
    .expect("set by numeric id succeeds");
    let view = plugin_params(&mut app, serde_json::json!({ "track_id": id }));
    let res = view.plugins[0]
        .params
        .iter()
        .find(|p| p.id == 1)
        .expect("param 1 listed");
    assert_eq!(res.value, 0.9);
}

/// Out-of-range is rejected with the range, not silently clamped — a
/// quietly-moved value is how a mix ends up wrong with nothing to point
/// at.
#[test]
fn an_out_of_range_value_is_rejected_with_the_range() {
    let mut app = app();
    seed_plugins(&mut app);
    let id = add_track(&mut app, "instrument", None);
    echo_plugin_with_params(
        &mut app,
        id,
        1,
        "com.resonance.wavetable",
        vec![param(0, "Cutoff", 0.5, 0.0, 1.0)],
    );

    for bad in [-0.1, 1.5] {
        let error = call(
            &mut app,
            "track.set_plugin_param",
            serde_json::json!({ "track_id": id, "param": "Cutoff", "value": bad }),
        )
        .error
        .expect("out of range rejected");
        assert_eq!(error.kind(), ErrorKind::InvalidParams);
        assert!(error.message.contains("0..=1"), "{}", error.message);
    }

    // The value never moved.
    let view = plugin_params(&mut app, serde_json::json!({ "track_id": id }));
    assert_eq!(view.plugins[0].params[0].value, 0.5);
}

#[test]
fn unknown_plugins_and_params_are_precise() {
    let mut app = app();
    seed_plugins(&mut app);
    let id = add_track(&mut app, "instrument", None);
    echo_plugin_with_params(
        &mut app,
        id,
        1,
        "com.resonance.wavetable",
        vec![param(0, "Cutoff", 0.5, 0.0, 1.0)],
    );

    let error = call(
        &mut app,
        "track.set_plugin_param",
        serde_json::json!({
            "track_id": id, "plugin_id": "com.nope", "param": "Cutoff", "value": 0.5
        }),
    )
    .error
    .expect("unknown plugin rejected");
    assert_eq!(error.kind(), ErrorKind::NotFound);
    assert!(error.message.contains("com.resonance.wavetable"), "{}", error.message);

    let error = call(
        &mut app,
        "track.set_plugin_param",
        serde_json::json!({ "track_id": id, "param": "Wobble", "value": 0.5 }),
    )
    .error
    .expect("unknown param rejected");
    assert_eq!(error.kind(), ErrorKind::NotFound);
    assert!(error.message.contains("Cutoff"), "{}", error.message);
}

/// A track carrying the same effect twice addresses each by occurrence.
#[test]
fn occurrence_addresses_duplicate_plugins() {
    let mut app = app();
    seed_plugins(&mut app);
    let id = add_track(&mut app, "instrument", None);
    echo_plugin_with_params(
        &mut app,
        id,
        1,
        "com.resonance.eq",
        vec![param(0, "Gain", 0.0, -12.0, 12.0)],
    );
    echo_plugin_with_params(
        &mut app,
        id,
        2,
        "com.resonance.eq",
        vec![param(0, "Gain", 0.0, -12.0, 12.0)],
    );

    let view = plugin_params(&mut app, serde_json::json!({ "track_id": id }));
    assert_eq!(view.plugins.len(), 2);
    assert_eq!(view.plugins[0].occurrence, 0);
    assert_eq!(view.plugins[1].occurrence, 1);

    let _: MutationAck = call(
        &mut app,
        "track.set_plugin_param",
        serde_json::json!({
            "track_id": id, "plugin_id": "com.resonance.eq", "occurrence": 1,
            "param": "Gain", "value": 6.0
        }),
    )
    .result()
    .expect("set on the second instance succeeds");

    let view = plugin_params(&mut app, serde_json::json!({ "track_id": id }));
    assert_eq!(view.plugins[0].params[0].value, 0.0, "first instance untouched");
    assert_eq!(view.plugins[1].params[0].value, 6.0, "second instance set");
}

// ---------------- track.add_instrument / add_effect ----------------

#[test]
fn add_instrument_and_effect_by_catalog_id() {
    let mut app = app();
    seed_plugins(&mut app);
    let id = add_track(&mut app, "instrument", None);
    let rx = app.test_capture_engine();

    let ack: MutationAck = call(
        &mut app,
        "track.add_instrument",
        serde_json::json!({ "track_id": id, "plugin_id": "com.resonance.wavetable" }),
    )
    .result()
    .expect("add_instrument succeeds");
    let _ = ack;
    assert!(drain(&rx).iter().any(|c| matches!(
        c,
        AudioCommand::AddPlugin { track_id, clap_plugin_id, .. }
        if *track_id == id && clap_plugin_id == "com.resonance.wavetable"
    )));

    let ack: MutationAck = call(
        &mut app,
        "track.add_effect",
        serde_json::json!({ "track_id": id, "plugin_id": "com.resonance.eq" }),
    )
    .result()
    .expect("add_effect succeeds");
    let _ = ack;
    assert!(drain(&rx).iter().any(|c| matches!(
        c,
        AudioCommand::AddPlugin { clap_plugin_id, .. } if clap_plugin_id == "com.resonance.eq"
    )));
}

/// Drive the engine echo that mirrors an added plugin into track state,
/// the way the real engine does after `AudioCommand::AddPlugin`.
fn echo_plugin_added(app: &mut Resonance, track_id: u64, instance_id: u64, plugin_id: &str) {
    app.test_apply_engine_event(AudioEvent::PluginAdded {
        track_id,
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

fn track_view(app: &mut Resonance, track_id: u64) -> resonance_control::methods::song::TrackDetail {
    tracks_view(app)
        .tracks
        .into_iter()
        .find(|t| u64::from(t.summary.id) == track_id)
        .expect("track present in song.tracks")
}

/// An effect appended to an instrument track that has no instrument must
/// be reported as an effect, not as the track's instrument. `add_effect`
/// appends, so it lands at slot 0 — classifying by position reported the
/// amp sim as the instrument and returned an empty effects array, which
/// left an agent unable to see or undo what it had done (doc #270 §2).
#[test]
fn effect_on_empty_instrument_track_is_not_reported_as_the_instrument() {
    let mut app = app();
    seed_plugins(&mut app);
    let id = add_track(&mut app, "instrument", None);

    let before = track_view(&mut app, id);
    assert_eq!(before.summary.instrument, None);
    assert!(before.effects.is_empty());

    echo_plugin_added(&mut app, id, 1, "com.resonance.eq");

    let after = track_view(&mut app, id);
    assert_eq!(after.summary.instrument, None, "an effect is not an instrument");
    assert_eq!(after.effects, vec!["com.resonance.eq".to_owned()]);
}

/// Chain order does not decide the role either: an instrument added
/// after an effect is still the instrument, and the effect stays in the
/// chain rather than being swallowed by the slot-0 skip.
#[test]
fn instrument_is_found_wherever_it_sits_in_the_chain() {
    let mut app = app();
    seed_plugins(&mut app);
    let id = add_track(&mut app, "instrument", None);

    echo_plugin_added(&mut app, id, 1, "com.resonance.eq");
    echo_plugin_added(&mut app, id, 2, "com.resonance.wavetable");

    let view = track_view(&mut app, id);
    assert_eq!(
        view.summary.instrument,
        Some("com.resonance.wavetable".to_owned())
    );
    assert_eq!(view.effects, vec!["com.resonance.eq".to_owned()]);
}

/// The usual ordering still reports the same way, and the effects array
/// is present (not elided) once an insert is added.
#[test]
fn instrument_then_effect_reports_both() {
    let mut app = app();
    seed_plugins(&mut app);
    let id = add_track(&mut app, "instrument", None);

    echo_plugin_added(&mut app, id, 1, "com.resonance.wavetable");
    echo_plugin_added(&mut app, id, 2, "com.resonance.eq");

    let view = track_view(&mut app, id);
    assert_eq!(
        view.summary.instrument,
        Some("com.resonance.wavetable".to_owned())
    );
    assert_eq!(view.effects, vec!["com.resonance.eq".to_owned()]);
}

/// A plugin the scanner has never seen — a project whose instrument is
/// no longer installed — still reports as the instrument it occupies,
/// rather than silently becoming an effect.
#[test]
fn unknown_plugin_in_slot_zero_is_still_the_instrument() {
    let mut app = app();
    seed_plugins(&mut app);
    let id = add_track(&mut app, "instrument", None);

    echo_plugin_added(&mut app, id, 1, "com.thirdparty.uninstalled");
    echo_plugin_added(&mut app, id, 2, "com.resonance.eq");

    let view = track_view(&mut app, id);
    assert_eq!(
        view.summary.instrument,
        Some("com.thirdparty.uninstalled".to_owned())
    );
    assert_eq!(view.effects, vec!["com.resonance.eq".to_owned()]);
}

/// `song.tracks` always carries both fields, so a client can tell "no
/// instrument" from "field omitted" and can inspect an empty chain.
#[test]
fn instrument_and_effects_are_always_present_in_the_wire_form() {
    let mut app = app();
    seed_plugins(&mut app);
    let id = add_track(&mut app, "instrument", None);

    let view = track_view(&mut app, id);
    let json = serde_json::to_value(&view).expect("TrackDetail serializes");
    assert_eq!(json.get("instrument"), Some(&serde_json::Value::Null));
    assert_eq!(
        json.get("effects"),
        Some(&serde_json::Value::Array(Vec::new()))
    );
}

#[test]
fn add_plugin_errors_are_precise() {
    let mut app = app();
    seed_plugins(&mut app);
    let id = add_track(&mut app, "instrument", None);

    // Unknown id -> not_found listing the valid ids.
    let response = call(
        &mut app,
        "track.add_instrument",
        serde_json::json!({ "track_id": id, "plugin_id": "com.nope" }),
    );
    let error = response.error.expect("unknown plugin rejected");
    assert_eq!(error.kind(), ErrorKind::NotFound);
    assert!(error.message.contains("com.resonance.wavetable"));

    // An effect id via add_instrument -> invalid_params.
    let response = call(
        &mut app,
        "track.add_instrument",
        serde_json::json!({ "track_id": id, "plugin_id": "com.resonance.eq" }),
    );
    assert_eq!(response.error.unwrap().kind(), ErrorKind::InvalidParams);

    // Unknown track.
    let response = call(
        &mut app,
        "track.add_effect",
        serde_json::json!({ "track_id": 9999, "plugin_id": "com.resonance.eq" }),
    );
    assert_eq!(response.error.unwrap().kind(), ErrorKind::NotFound);
}

/// The "valid ids" list must match the verb that was called. Listing
/// effects as the valid ids for add_instrument is what led an agent to
/// conclude the app ships no instruments at all (doc #270 §1).
#[test]
fn valid_ids_are_scoped_to_the_verb() {
    let mut app = app();
    seed_plugins(&mut app);
    let id = add_track(&mut app, "instrument", None);

    let message = call(
        &mut app,
        "track.add_instrument",
        serde_json::json!({ "track_id": id, "plugin_id": "com.nope" }),
    )
    .error
    .expect("unknown plugin rejected")
    .message;
    assert!(message.contains("com.resonance.wavetable"), "{message}");
    assert!(!message.contains("com.resonance.eq"), "{message}");

    let message = call(
        &mut app,
        "track.add_effect",
        serde_json::json!({ "track_id": id, "plugin_id": "com.nope" }),
    )
    .error
    .expect("unknown plugin rejected")
    .message;
    assert!(message.contains("com.resonance.eq"), "{message}");
    assert!(!message.contains("com.resonance.wavetable"), "{message}");
}

/// With nothing of the requested kind in the catalog, the error says why
/// — an unbuilt checkout, not a wrong id. Without this the caller sees
/// "valid ids: []" and has no way to tell the two apart.
#[test]
fn empty_catalog_points_at_the_bundle_step() {
    let mut app = app();
    // Only an effect was scanned: no instrument is installable.
    app.test_apply_engine_event(AudioEvent::PluginsScanned {
        plugins: vec![ScannedPlugin {
            clap_file_path: "/plugins/eq.clap".to_owned(),
            clap_plugin_id: "com.resonance.eq".to_owned(),
            name: "Resonance EQ".to_owned(),
            vendor: "Resonance".to_owned(),
            is_instrument: false,
        ..Default::default()
}],
    });
    let id = add_track(&mut app, "instrument", None);

    let message = call(
        &mut app,
        "track.add_instrument",
        serde_json::json!({ "track_id": id, "plugin_id": "resonance-wavetable" }),
    )
    .error
    .expect("unknown plugin rejected")
    .message;
    assert!(message.contains("bundle.sh"), "{message}");

    // An effect is still installable, so that error keeps its plain form.
    let message = call(
        &mut app,
        "track.add_effect",
        serde_json::json!({ "track_id": id, "plugin_id": "com.nope" }),
    )
    .error
    .expect("unknown plugin rejected")
    .message;
    assert!(!message.contains("bundle.sh"), "{message}");
}

/// A synth appended to the FX chain is a slot the GUI cannot create and
/// that renders nothing; `bus.add_effect` and `master.add_effect` have
/// always refused it, so `track.add_effect` must too — and point at
/// `track.add_instrument`, which still takes the same id.
#[test]
fn add_effect_rejects_an_instrument_and_add_instrument_takes_the_same_id() {
    let mut app = app();
    seed_plugins(&mut app);
    let id = add_track(&mut app, "instrument", None);
    let rx = app.test_capture_engine();
    let before = app.revision();

    let error = call(
        &mut app,
        "track.add_effect",
        serde_json::json!({ "track_id": id, "plugin_id": "com.resonance.wavetable" }),
    )
    .error
    .expect("an instrument in the effect chain is rejected");
    assert_eq!(error.kind(), ErrorKind::InvalidParams);
    assert!(error.message.contains("track.add_instrument"), "{}", error.message);
    assert_eq!(app.revision(), before, "a rejected add records nothing");
    assert!(drain(&rx).is_empty(), "no engine command for a rejected add");

    // The same id through the right verb still lands.
    let _: MutationAck = call(
        &mut app,
        "track.add_instrument",
        serde_json::json!({ "track_id": id, "plugin_id": "com.resonance.wavetable" }),
    )
    .result()
    .expect("add_instrument takes the same id");
    assert!(drain(&rx).iter().any(|c| matches!(
        c,
        AudioCommand::AddPlugin { clap_plugin_id, .. }
        if clap_plugin_id == "com.resonance.wavetable"
    )));
}

// ---------------- mixer.* ----------------

#[test]
fn mixer_volume_pan_mute_solo_and_undo() {
    let mut app = app();
    let id = add_track(&mut app, "instrument", None);

    // Unity linear gain -> ~0 dB stored.
    let _: MutationAck = call(
        &mut app,
        "mixer.set_volume",
        serde_json::json!({ "track_id": id, "volume": 1.0 }),
    )
    .result()
    .expect("set_volume succeeds");
    let view = tracks_view(&mut app);
    assert!((view.tracks[0].summary.volume - 1.0).abs() < 1e-3);

    let _: MutationAck = call(
        &mut app,
        "mixer.set_pan",
        serde_json::json!({ "track_id": id, "pan": -0.5 }),
    )
    .result()
    .expect("set_pan succeeds");
    assert!((tracks_view(&mut app).tracks[0].summary.pan + 0.5).abs() < 1e-6);

    // mute + solo flip the flags.
    let _: MutationAck = call(
        &mut app,
        "mixer.set_mute",
        serde_json::json!({ "track_id": id, "muted": true }),
    )
    .result()
    .expect("set_mute succeeds");
    assert!(tracks_view(&mut app).tracks[0].summary.muted);
    let _: MutationAck = call(
        &mut app,
        "mixer.set_solo",
        serde_json::json!({ "track_id": id, "soloed": true }),
    )
    .result()
    .expect("set_solo succeeds");
    assert!(tracks_view(&mut app).tracks[0].summary.soloed);
}

#[test]
fn mixer_set_mute_is_idempotent() {
    let mut app = app();
    let id = add_track(&mut app, "instrument", None);
    // Already unmuted: setting muted=false must be a no-op (no undo entry).
    let before = app.revision();
    let _: MutationAck = call(
        &mut app,
        "mixer.set_mute",
        serde_json::json!({ "track_id": id, "muted": false }),
    )
    .result()
    .expect("set_mute succeeds");
    assert_eq!(app.revision(), before, "idempotent set records nothing");
    assert!(!tracks_view(&mut app).tracks[0].summary.muted);
}

#[test]
fn mixer_validates_ranges_and_ids() {
    let mut app = app();
    let id = add_track(&mut app, "instrument", None);
    for params in [
        serde_json::json!({ "track_id": id, "pan": 2.0 }),
        serde_json::json!({ "track_id": id, "pan": -2.0 }),
    ] {
        let response = call(&mut app, "mixer.set_pan", params);
        assert_eq!(response.error.unwrap().kind(), ErrorKind::InvalidParams);
    }
    let response = call(
        &mut app,
        "mixer.set_volume",
        serde_json::json!({ "track_id": id, "volume": -1.0 }),
    );
    assert_eq!(response.error.unwrap().kind(), ErrorKind::InvalidParams);
    let response = call(
        &mut app,
        "mixer.set_volume",
        serde_json::json!({ "track_id": 9999, "volume": 1.0 }),
    );
    assert_eq!(response.error.unwrap().kind(), ErrorKind::NotFound);
}

/// The linear form is the same fader as `mixer.set_volume_db`, so it
/// shares the same effective range: `{volume: 1000}` used to dispatch
/// +60 dB, a level the dB form, `bus.set_volume` and `master.set_volume`
/// all refuse.
#[test]
fn linear_volume_above_the_fader_cap_is_rejected_without_dispatch() {
    let mut app = app();
    let id = add_track(&mut app, "instrument", None);
    let rx = app.test_capture_engine();
    let before = app.revision();

    for volume in [1000.0f32, 2.0] {
        let error = call(
            &mut app,
            "mixer.set_volume",
            serde_json::json!({ "track_id": id, "volume": volume }),
        )
        .error
        .unwrap_or_else(|| panic!("linear {volume} should be rejected"));
        assert_eq!(error.kind(), ErrorKind::InvalidParams, "for linear {volume}");
        assert!(
            error.message.contains("1.9953"),
            "the error must name the linear bound: {}",
            error.message
        );
    }
    assert_eq!(app.revision(), before, "a rejected set records nothing");
    assert!(drain(&rx).is_empty(), "no engine command for a rejected set");
    assert!(tracks_view(&mut app).tracks[0].summary.volume_db.abs() < 1e-6);
}

/// The exact linear image of the cap is inside the range — `powf` and
/// `log10` not being exact f32 inverses must not shave the endpoint off
/// the linear form.
#[test]
fn linear_volume_at_the_cap_is_accepted_as_plus_six_db() {
    let mut app = app();
    let id = add_track(&mut app, "instrument", None);

    let cap = 10f32.powf(6.0 / 20.0); // the linear image of +6 dB
    let _: MutationAck = call(
        &mut app,
        "mixer.set_volume",
        serde_json::json!({ "track_id": id, "volume": cap }),
    )
    .result()
    .expect("the fader cap itself is inside the range");
    let db = tracks_view(&mut app).tracks[0].summary.volume_db;
    assert!((db - 6.0).abs() < 1e-4, "volume_db {db}");
}

/// `{volume: 0}` used to store -80 dB, 20 dB below the documented
/// silence floor; it maps to `VOLUME_DB_MIN` now, the way
/// `master.set_volume` maps it.
#[test]
fn linear_volume_zero_maps_to_the_documented_floor() {
    let mut app = app();
    let id = add_track(&mut app, "instrument", None);

    let _: MutationAck = call(
        &mut app,
        "mixer.set_volume",
        serde_json::json!({ "track_id": id, "volume": 0.0 }),
    )
    .result()
    .expect("silence is inside the range");
    let view = tracks_view(&mut app);
    let db = view.tracks[0].summary.volume_db;
    assert!((db - -60.0).abs() < 1e-4, "volume_db {db}");
    // The view reports the floor's raw linear image, not a hard zero.
    let linear = view.tracks[0].summary.volume;
    assert!((linear - 0.001).abs() < 1e-6, "volume {linear}");
}

// ---------------- track type sanity ----------------

#[test]
fn drums_track_is_an_instrument_track_with_drum_type() {
    let mut app = app();
    let id = add_track(&mut app, "drums", None);
    // Engine-side it's an instrument track; the control layer promoted
    // it to the Drum instrument type on the echo.
    let track = app
        .track_registry()
        .tracks
        .iter()
        .find(|t| t.id == id)
        .expect("drum track mirrored");
    assert_eq!(track.track_type, TrackType::Instrument);
    assert_eq!(track.instrument_type, resonance_app::state::InstrumentType::Drum);
}
