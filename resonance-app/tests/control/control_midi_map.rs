//! `midi_map.*` — MIDI Learn over the control API (W1): an agent reads
//! the bindings, arms learn for a target the user then moves a control
//! for, cancels, and clears — each write through `update()`, undoable.

use resonance_app::Resonance;
use resonance_audio::test_support::Receiver;
use resonance_audio::types::{AudioCommand, AudioEvent, ParamInfo, TrackType};
use resonance_common::{CcMode, ControlSource, MidiTarget};
use resonance_control::methods::midi_map::{
    BindingsResult, CancelLearnResult, ClearResult, LearnResult, MidiControl,
};
use resonance_control::ErrorKind;
use serde_json::json;

use crate::common::call;

const TRACK: u64 = 1;

fn app() -> (Resonance, Receiver<AudioCommand>) {
    let (mut app, _task, rx) = Resonance::new_for_test_with_capture();
    app.test_set_active_project(true);
    app.test_set_project_path(std::path::PathBuf::from("/tmp/control-midi-map.rproj"));
    app.test_add_track(TRACK, TrackType::Instrument);
    let _ = rx.try_iter().count();
    (app, rx)
}

fn capture(app: &mut Resonance, target: MidiTarget, cc: u8) {
    app.test_apply_engine_event(AudioEvent::MidiLearnCaptured {
        target,
        source: ControlSource::Cc {
            channel: 0,
            cc,
            mode: CcMode::Absolute,
        },
    });
}

#[test]
fn learn_arms_the_engine_and_the_capture_reads_back() {
    let (mut app, rx) = app();
    let armed: LearnResult = call(
        &mut app,
        "midi_map.learn",
        json!({"track_id": TRACK, "control": "volume"}),
    )
    .result()
    .unwrap();
    assert_eq!(armed.learning.spec.control, Some(MidiControl::Volume));
    assert_eq!(armed.learning.label, "Instrument 1 \u{b7} Volume");
    assert!(rx
        .try_iter()
        .any(|c| matches!(c, AudioCommand::EnterMidiLearn { target: MidiTarget::TrackVolume(TRACK) })));

    let read: BindingsResult = call(&mut app, "midi_map.bindings", json!({})).result().unwrap();
    assert!(read.learning.is_some());
    assert!(read.control_surface_input.is_none());

    capture(&mut app, MidiTarget::TrackVolume(TRACK), 7);
    let read: BindingsResult = call(&mut app, "midi_map.bindings", json!({})).result().unwrap();
    assert!(read.learning.is_none());
    assert_eq!(read.bindings.len(), 1);
    let b = &read.bindings[0];
    assert_eq!((b.source.kind.as_str(), b.source.channel, b.source.number), ("cc", 1, 7));
    assert_eq!(b.target.spec.track_id.map(|t| t.0), Some(TRACK));
}

#[test]
fn cancel_learn_reports_whether_it_was_armed() {
    let (mut app, rx) = app();
    let r: CancelLearnResult = call(&mut app, "midi_map.cancel_learn", json!({})).result().unwrap();
    assert!(!r.cancelled);
    call(&mut app, "midi_map.learn", json!({"transport": "play"})).result::<LearnResult>().unwrap();
    let _ = rx.try_iter().count();
    let r: CancelLearnResult = call(&mut app, "midi_map.cancel_learn", json!({})).result().unwrap();
    assert!(r.cancelled);
    assert!(rx.try_iter().any(|c| matches!(c, AudioCommand::CancelMidiLearn)));
}

#[test]
fn a_plugin_parameter_resolves_like_set_plugin_param() {
    let (mut app, _rx) = app();
    app.test_apply_engine_event(AudioEvent::PluginAdded {
        owner: resonance_audio::types::ChainOwner::Track(TRACK),
        instance_id: 77,
        plugin_name: "Synth".into(),
        clap_plugin_id: "com.example.synth".into(),
        clap_file_path: "/x.clap".into(),
        params: vec![ParamInfo {
            id: 12,
            name: "Cutoff".into(),
            max_value: 1.0,
            ..ParamInfo::default()
        }],
        has_gui: false,
        has_sidechain_input: false,
        output_port_count: 1,
        output_port_names: Vec::new(),
    });
    let armed: LearnResult = call(
        &mut app,
        "midi_map.learn",
        json!({"track_id": TRACK, "param": "cutoff", "plugin_id": "com.example.synth"}),
    )
    .result()
    .unwrap();
    assert_eq!(armed.learning.spec.param.as_deref(), Some("12"));
    assert_eq!(armed.learning.spec.plugin_id.as_deref(), Some("com.example.synth"));
    assert_eq!(
        app.test_midi_map().learn_target,
        Some(MidiTarget::PluginParam {
            instance: 77,
            param_id: 12
        })
    );
}

#[test]
fn bad_targets_are_refused_with_a_reason() {
    let (mut app, _rx) = app();
    for (params, kind) in [
        (json!({}), ErrorKind::InvalidParams),
        (json!({"track_id": 99, "control": "pan"}), ErrorKind::NotFound),
        (json!({"track_id": TRACK}), ErrorKind::InvalidParams),
        (json!({"track_id": TRACK, "control": "pan", "send_id": 1}), ErrorKind::InvalidParams),
        (json!({"track_id": TRACK, "transport": "stop"}), ErrorKind::InvalidParams),
        (json!({"track_id": TRACK, "send_id": 5}), ErrorKind::NotFound),
    ] {
        let err = call(&mut app, "midi_map.learn", params.clone()).result::<LearnResult>().unwrap_err();
        assert_eq!(err.kind(), kind, "{params}: {err:?}");
    }
    assert!(app.test_midi_map().learn_target.is_none());
}

#[test]
fn clear_and_clear_all_are_undoable_edits() {
    let (mut app, rx) = app();
    capture(&mut app, MidiTarget::TrackVolume(TRACK), 7);
    capture(&mut app, MidiTarget::TrackPan(TRACK), 10);
    let read: BindingsResult = call(&mut app, "midi_map.bindings", json!({})).result().unwrap();
    let _ = rx.try_iter().count();

    let err = call(&mut app, "midi_map.clear", json!({"id": 999})).result::<ClearResult>().unwrap_err();
    assert_eq!(err.kind(), ErrorKind::NotFound);

    let r: ClearResult = call(&mut app, "midi_map.clear", json!({"id": read.bindings[0].id}))
        .result()
        .unwrap();
    assert_eq!(r.cleared, 1);
    assert!(rx.try_iter().any(|c| matches!(c, AudioCommand::ClearMidiBinding { .. })));

    let r: ClearResult = call(&mut app, "midi_map.clear_all", json!({})).result().unwrap();
    assert_eq!(r.cleared, 1);
    assert!(app.test_midi_map().bindings.is_empty());
    let before = app.revision();
    let r: ClearResult = call(&mut app, "midi_map.clear_all", json!({})).result().unwrap();
    assert_eq!((r.cleared, r.revision), (0, before), "nothing left: no edit");

    let _ = call(&mut app, "edit.undo", json!({}));
    assert_eq!(app.test_midi_map().bindings.len(), 1);
}
