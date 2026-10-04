//! MIDI Learn edits and hardware moves (doc #167, W1): arming learn, the
//! capture becoming a binding, clearing, undo, persistence, and a
//! control-surface move landing on its target through the target's own
//! message.

use resonance_app::commands::{KeyChord, Mods, NamedKey};
use resonance_app::message::{Message, MidiMapMessage, UiMessage};
use resonance_app::project::ProjectFile;
use resonance_app::state::PluginSlotState;
use resonance_app::Resonance;
use resonance_audio::test_support::Receiver;
use resonance_audio::types::{
    AudioCommand, AudioEvent, AuxSend, ParamInfo, SendSource, TrackType,
};
use resonance_common::{
    BindingId, CcMode, ControlSource, MidiBinding, MidiTarget, RelativeEnc, Takeover,
    TransportAction,
};

const TRACK: u64 = 1;
const PLUGIN: u64 = 500;

fn drain(rx: &Receiver<AudioCommand>) -> Vec<AudioCommand> {
    rx.try_iter().collect()
}

/// An open, saved project (the history records once there is a path)
/// with one audio track, the engine captured.
fn app() -> (Resonance, Receiver<AudioCommand>) {
    let (mut app, _task, rx) = Resonance::new_for_test_with_capture();
    app.test_set_active_project(true);
    app.test_set_project_path(std::path::PathBuf::from("/tmp/midi-learn-test.rproj"));
    app.test_add_track(TRACK, TrackType::Audio);
    let _ = drain(&rx);
    (app, rx)
}

fn cc(channel: u8, cc: u8) -> ControlSource {
    ControlSource::Cc {
        channel,
        cc,
        mode: CcMode::Absolute,
    }
}

fn learn(app: &mut Resonance, target: MidiTarget, source: ControlSource) {
    let _ = app.update(Message::MidiMap(MidiMapMessage::Learn(target)));
    app.test_apply_engine_event(AudioEvent::MidiLearnCaptured { target, source });
}

fn track(app: &Resonance) -> &resonance_app::state::TrackState {
    app.test_tracks().iter().find(|t| t.id == TRACK).unwrap()
}

fn moved(app: &mut Resonance, binding: MidiBinding, value: u8) {
    app.test_apply_engine_event(AudioEvent::ControlSurfaceMoved { binding, value });
}

fn binding_for(app: &Resonance, target: MidiTarget) -> MidiBinding {
    *app.test_midi_map()
        .bindings
        .values()
        .find(|b| b.target == target)
        .expect("a binding for the target")
}

// ---------------------------------------------------------------------------
// Learn
// ---------------------------------------------------------------------------

#[test]
fn learn_arms_the_engine_and_a_second_learn_or_esc_cancels() {
    let (mut app, rx) = app();
    let target = MidiTarget::TrackVolume(TRACK);

    let _ = app.update(Message::MidiMap(MidiMapMessage::Learn(target)));
    assert_eq!(app.test_midi_map().learn_target, Some(target));
    assert!(matches!(
        drain(&rx).as_slice(),
        [AudioCommand::EnterMidiLearn { target: t }] if *t == target
    ));

    // The same menu entry again (the control shows "Cancel learn").
    let _ = app.update(Message::MidiMap(MidiMapMessage::Learn(target)));
    assert_eq!(app.test_midi_map().learn_target, None);
    assert!(matches!(drain(&rx).as_slice(), [AudioCommand::CancelMidiLearn]));

    // Esc disarms too.
    let _ = app.update(Message::MidiMap(MidiMapMessage::Learn(target)));
    let _ = drain(&rx);
    let _ = app.update(Message::Ui(UiMessage::ShortcutKey {
        chord: KeyChord::named(NamedKey::Escape, Mods::NONE),
        repeat: false,
        captured: false,
    }));
    assert_eq!(app.test_midi_map().learn_target, None);
    assert!(matches!(drain(&rx).as_slice(), [AudioCommand::CancelMidiLearn]));

    // Arming learn is not an edit.
    assert!(!app.test_undo_history().can_undo());
}

#[test]
fn a_capture_becomes_one_undoable_binding() {
    let (mut app, rx) = app();
    let target = MidiTarget::TrackVolume(TRACK);
    learn(&mut app, target, cc(0, 7));

    let map = app.test_midi_map();
    assert_eq!(map.learn_target, None, "the capture disarms");
    assert_eq!(map.bindings.len(), 1);
    let b = binding_for(&app, target);
    assert_eq!(b.source, cc(0, 7));
    assert_eq!(b.takeover, Takeover::Jump);
    let cmds = drain(&rx);
    assert!(
        cmds.iter()
            .any(|c| matches!(c, AudioCommand::SetMidiBinding { binding } if *binding == b)),
        "{cmds:?}"
    );
    assert!(app.test_dirty());

    // Undo removes it here and in the engine; redo puts it back.
    let _ = app.update(Message::Undo);
    assert!(app.test_midi_map().bindings.is_empty());
    assert!(drain(&rx).iter().any(|c| matches!(
        c,
        AudioCommand::SetControllerMap { map } if map.bindings.is_empty()
    )));
    let _ = app.update(Message::Redo);
    assert_eq!(app.test_midi_map().bindings.values().next(), Some(&b));
    assert!(drain(&rx).iter().any(|c| matches!(
        c,
        AudioCommand::SetControllerMap { map } if map.bindings == vec![b]
    )));
}

#[test]
fn learning_a_bound_control_moves_it_to_the_new_target() {
    let (mut app, _rx) = app();
    learn(&mut app, MidiTarget::TrackVolume(TRACK), cc(0, 7));
    learn(&mut app, MidiTarget::TrackPan(TRACK), cc(0, 7));
    let map = app.test_midi_map();
    assert_eq!(map.bindings.len(), 1, "one control drives one target");
    assert_eq!(
        map.bindings.values().next().unwrap().target,
        MidiTarget::TrackPan(TRACK)
    );
}

// ---------------------------------------------------------------------------
// Clear
// ---------------------------------------------------------------------------

#[test]
fn clear_target_and_clear_all_reach_the_engine_and_undo() {
    let (mut app, rx) = app();
    learn(&mut app, MidiTarget::TrackVolume(TRACK), cc(0, 7));
    learn(&mut app, MidiTarget::TrackPan(TRACK), cc(0, 10));
    let vol = binding_for(&app, MidiTarget::TrackVolume(TRACK));
    let _ = drain(&rx);

    let _ = app.update(Message::MidiMap(MidiMapMessage::ClearTarget(
        MidiTarget::TrackVolume(TRACK),
    )));
    assert_eq!(app.test_midi_map().bindings.len(), 1);
    assert!(matches!(
        drain(&rx).as_slice(),
        [AudioCommand::ClearMidiBinding { id }] if *id == vol.id
    ));

    let _ = app.update(Message::MidiMap(MidiMapMessage::ClearAll));
    assert!(app.test_midi_map().bindings.is_empty());
    assert!(matches!(drain(&rx).as_slice(), [AudioCommand::ClearAllMidiBindings]));

    let _ = app.update(Message::Undo);
    assert_eq!(app.test_midi_map().bindings.len(), 1);
    let _ = app.update(Message::Undo);
    assert_eq!(app.test_midi_map().bindings.len(), 2);
    assert_eq!(app.test_undo_history().undo_label(), Some("learn MIDI binding"));
}

// ---------------------------------------------------------------------------
// Persistence
// ---------------------------------------------------------------------------

#[test]
fn bindings_round_trip_through_the_project_file() {
    let (mut app, _rx) = app();
    learn(&mut app, MidiTarget::TrackVolume(TRACK), cc(0, 7));
    learn(
        &mut app,
        MidiTarget::Transport(TransportAction::Play),
        ControlSource::Note { channel: 9, note: 36 },
    );
    let file = app.test_build_project_file();
    assert_eq!(file.midi_bindings.len(), 2);
    assert!(file.midi_bindings.windows(2).all(|w| w[0].id < w[1].id), "sorted by id");

    let json = serde_json::to_string(&file).unwrap();
    assert!(json.contains("midi_bindings"));
    let back: ProjectFile = serde_json::from_str(&json).unwrap();

    let (mut fresh, _task) = Resonance::new_for_test();
    let rx = fresh.test_capture_engine();
    fresh.test_replay_loaded_project(back);
    assert_eq!(fresh.test_midi_map().sorted(), file.midi_bindings);
    assert!(drain(&rx).iter().any(|c| matches!(
        c,
        AudioCommand::SetControllerMap { map } if map.bindings == file.midi_bindings
    )));
}

#[test]
fn a_project_without_bindings_clears_the_engines() {
    // Written before the field existed: no `midi_bindings` key at all.
    let json = serde_json::to_string(&ProjectFile::default()).unwrap();
    assert!(!json.contains("midi_bindings"), "an empty set stays out of the file");
    let back: ProjectFile = serde_json::from_str(&json).unwrap();
    assert!(back.midi_bindings.is_empty());

    // Loading it over a project that had bindings drops them everywhere:
    // the engine's `ClearAll` does not touch its binding set.
    let (mut app, _rx) = app();
    learn(&mut app, MidiTarget::TrackVolume(TRACK), cc(0, 7));
    let rx = app.test_capture_engine();
    app.test_replay_loaded_project(back);
    assert!(app.test_midi_map().bindings.is_empty());
    assert!(drain(&rx).iter().any(|c| matches!(
        c,
        AudioCommand::SetControllerMap { map } if map.bindings.is_empty()
    )));
}

// ---------------------------------------------------------------------------
// Hardware moves
// ---------------------------------------------------------------------------

#[test]
fn a_fader_move_sets_the_track_volume_and_coalesces_into_one_entry() {
    let (mut app, rx) = app();
    learn(&mut app, MidiTarget::TrackVolume(TRACK), cc(0, 7));
    let b = binding_for(&app, MidiTarget::TrackVolume(TRACK));
    let entries = app.test_undo_history().test_undo_entries().len();
    let _ = drain(&rx);

    moved(&mut app, b, 127);
    assert!((track(&app).volume - 6.0).abs() < 1e-4, "top of the fader: +6 dB");
    moved(&mut app, b, 0);
    assert!((track(&app).volume + 60.0).abs() < 1e-4, "bottom: -60 dB");
    assert!(drain(&rx)
        .iter()
        .any(|c| matches!(c, AudioCommand::SetTrackVolume { track_id: TRACK, .. })));
    assert_eq!(
        app.test_undo_history().test_undo_entries().len(),
        entries + 1,
        "a sweep is one entry, like a mouse drag"
    );
}

#[test]
fn pickup_waits_for_the_control_and_an_encoder_steps_from_the_current_value() {
    let (mut app, _rx) = app();
    let mut pickup = MidiBinding::new(BindingId(1), cc(0, 10), MidiTarget::TrackPan(TRACK));
    pickup.takeover = Takeover::Pickup;
    // Pan sits at centre (64/127 of the way ≈ 0.008); a hard-left move is
    // swallowed until the knob passes the current value.
    moved(&mut app, pickup, 0);
    assert_eq!(track(&app).pan, 0.0);
    moved(&mut app, pickup, 64);
    assert!(track(&app).pan.abs() < 0.02, "caught up: follows from here");

    let encoder = MidiBinding::new(
        BindingId(2),
        ControlSource::Cc {
            channel: 0,
            cc: 11,
            mode: CcMode::Relative(RelativeEnc::TwosComplement),
        },
        MidiTarget::TrackVolume(TRACK),
    );
    let before = track(&app).volume;
    moved(&mut app, encoder, 1);
    let after = track(&app).volume;
    assert!(after > before && after - before < 1.0, "{before} -> {after}");
}

#[test]
fn pads_toggle_mute_and_solo_and_trigger_transport() {
    let (mut app, rx) = app();
    let pad = |id, target| {
        MidiBinding::new(BindingId(id), ControlSource::Note { channel: 0, note: 36 }, target)
    };
    moved(&mut app, pad(1, MidiTarget::TrackMute(TRACK)), 100);
    assert!(track(&app).muted);
    moved(&mut app, pad(1, MidiTarget::TrackMute(TRACK)), 100);
    assert!(!track(&app).muted, "every press flips");

    // A CC switch sends its state: on in the upper half, off in the lower.
    let switch = MidiBinding::new(BindingId(2), cc(0, 20), MidiTarget::TrackSolo(TRACK));
    moved(&mut app, switch, 127);
    assert!(track(&app).soloed);
    moved(&mut app, switch, 127);
    assert!(track(&app).soloed, "already on: no flip");
    moved(&mut app, switch, 0);
    assert!(!track(&app).soloed);

    let _ = drain(&rx);
    moved(&mut app, pad(3, MidiTarget::Transport(TransportAction::Play)), 127);
    assert!(drain(&rx).iter().any(|c| matches!(c, AudioCommand::Play)));
}

#[test]
fn plugin_params_and_sends_move_in_their_own_units() {
    let (mut app, rx) = app();
    let params = vec![
        ParamInfo {
            id: 1,
            name: "Cutoff".into(),
            min_value: 20.0,
            max_value: 20_000.0,
            current_value: 1_000.0,
            ..ParamInfo::default()
        },
        ParamInfo {
            id: 2,
            name: "Mode".into(),
            min_value: 0.0,
            max_value: 3.0,
            stepped: true,
            ..ParamInfo::default()
        },
    ];
    app.test_push_track_plugin(
        TRACK,
        PluginSlotState::new(PLUGIN, "Filter".into(), "x.filter".into(), "/x.clap".into(), params, false),
    );
    let param = |id, param_id| {
        MidiBinding::new(
            BindingId(id),
            cc(0, 30 + id as u8),
            MidiTarget::PluginParam {
                instance: PLUGIN,
                param_id,
            },
        )
    };
    moved(&mut app, param(1, 1), 127);
    moved(&mut app, param(2, 2), 80);
    let slot = app.test_tracks().iter().find(|t| t.id == TRACK).unwrap().plugins[0].clone();
    assert!((slot.params[0].current_value - 20_000.0).abs() < 1e-6);
    // 80/127 of 0..=3 is 1.89 — a stepped parameter lands on 2.
    assert_eq!(slot.params[1].current_value, 2.0);

    app.test_seed_aux_send(AuxSend {
        id: 9,
        source: SendSource::Track(TRACK),
        dest: 50,
        level_db: -12.0,
        pre_fader: false,
        enabled: true,
    });
    let send = MidiBinding::new(
        BindingId(3),
        cc(0, 40),
        MidiTarget::SendLevel {
            track: TRACK,
            send: resonance_common::SendId(9),
        },
    );
    let _ = drain(&rx);
    moved(&mut app, send, 127);
    // The send mirror follows the engine's echo; the edit is the command.
    assert!(drain(&rx).iter().any(|c| matches!(
        c,
        AudioCommand::SetAuxSend { id: 9, level_db, .. } if *level_db == 6.0
    )));
}

#[test]
fn a_move_on_a_deleted_target_does_nothing() {
    let (mut app, rx) = app();
    let b = MidiBinding::new(BindingId(1), cc(0, 7), MidiTarget::TrackVolume(99));
    moved(&mut app, b, 100);
    assert!(drain(&rx).is_empty());
    assert!(!app.test_undo_history().can_undo());
}

// ---------------------------------------------------------------------------
// Machine settings and controller maps
// ---------------------------------------------------------------------------

#[test]
fn the_control_surface_port_is_a_persisted_machine_setting() {
    let (mut app, rx) = app();
    let _ = app.update(Message::MidiMap(MidiMapMessage::SetControlSurfaceInput(Some(
        "nanoKONTROL2".into(),
    ))));
    assert!(matches!(
        drain(&rx).as_slice(),
        [AudioCommand::SetControlSurfaceInput { device: Some(d) }] if d == "nanoKONTROL2"
    ));
    assert_eq!(
        app.test_settings().midi.control_surface_input.as_deref(),
        Some("nanoKONTROL2")
    );
    let file = resonance_app::user_dirs::config_dir()
        .unwrap()
        .join("resonance/settings.json");
    assert_eq!(
        resonance_app::settings::load_from(&file).midi.control_surface_input.as_deref(),
        Some("nanoKONTROL2")
    );
    assert!(!app.test_undo_history().can_undo(), "not a project edit");
    let _ = app.update(Message::MidiMap(MidiMapMessage::SetControlSurfaceInput(None)));
}

#[test]
fn controller_maps_save_load_and_delete() {
    let (mut app, rx) = app();
    learn(&mut app, MidiTarget::TrackVolume(TRACK), cc(0, 7));
    let saved = app.test_midi_map().sorted();

    let name = format!("Test map {}", std::process::id());
    let _ = app.update(Message::MidiMap(MidiMapMessage::SetMapName(name.clone())));
    let _ = app.update(Message::MidiMap(MidiMapMessage::SaveControllerMap));
    assert!(app.test_midi_map().saved_maps.iter().any(|m| m.name == name && m.bindings == saved));

    let _ = app.update(Message::MidiMap(MidiMapMessage::ClearAll));
    let _ = drain(&rx);
    let _ = app.update(Message::MidiMap(MidiMapMessage::LoadControllerMap(name.clone())));
    assert_eq!(app.test_midi_map().sorted(), saved);
    assert!(drain(&rx).iter().any(|c| matches!(
        c,
        AudioCommand::SetControllerMap { map } if map.bindings == saved
    )));
    assert_eq!(app.test_undo_history().undo_label(), Some("load controller map"));

    let _ = app.update(Message::MidiMap(MidiMapMessage::DeleteControllerMap(name.clone())));
    assert!(!app.test_midi_map().saved_maps.iter().any(|m| m.name == name));
}
