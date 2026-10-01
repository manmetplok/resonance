//! A parameter the plugin changed itself — its own editor, its own kit
//! browser — and reported (`AudioEvent::PluginParamEdited`, from CLAP
//! output parameter events) is an edit like any other: the mirror follows
//! and it takes an undo entry (drums-plugin-rework.md §5.1: "the editor's
//! Library Load sets this param, so a kit change is host-undoable").

use resonance_app::message::Message;
use resonance_app::state::{PluginSlotState, ViewMode};
use resonance_app::Resonance;
use resonance_audio::test_support::Receiver;
use resonance_audio::types::{
    AudioCommand, AudioEvent, ParamInfo, PluginInstanceId, PluginParamEdit, TrackType,
};

const TRACK: u64 = 3;
const DRUMS: PluginInstanceId = 33;
const GAIN: u32 = 1;
const KIT: u32 = 2;
const PROGRESS: u32 = 3;

fn app() -> (Resonance, Receiver<AudioCommand>) {
    let (mut app, _task, rx) = Resonance::new_for_test_with_capture();
    app.test_set_view_mode(ViewMode::Arrange);
    app.test_set_active_project(true);
    app.test_set_project_path(std::path::PathBuf::from("/tmp/plugin-edited-by-plugin.rprj"));
    app.test_add_track(TRACK, TrackType::Instrument);
    app.test_push_track_plugin(
        TRACK,
        PluginSlotState::new(
            DRUMS,
            "Drums".to_owned(),
            "test.drums".to_owned(),
            "/plugins/test.drums.clap".to_owned(),
            vec![
                ParamInfo {
                    id: GAIN,
                    name: "Gain".to_owned(),
                    max_value: 10.0,
                    ..Default::default()
                },
                ParamInfo {
                    id: KIT,
                    name: "Kit".to_owned(),
                    min_value: -1.0,
                    max_value: 999.0,
                    default_value: -1.0,
                    current_value: -1.0,
                    automatable: false,
                    state_excluded: true,
                    ..Default::default()
                },
                ParamInfo {
                    id: PROGRESS,
                    name: "Kit Load Progress".to_owned(),
                    max_value: 1.0,
                    automatable: false,
                    read_only: true,
                    state_excluded: true,
                    ..Default::default()
                },
            ],
            false,
        ),
    );
    app.test_seed_plugin_state(DRUMS, vec![0xA0]);
    (app, rx)
}

fn drain(rx: &Receiver<AudioCommand>) -> Vec<AudioCommand> {
    let mut cmds = Vec::new();
    while let Ok(cmd) = rx.try_recv() {
        cmds.push(cmd);
    }
    cmds
}

fn plugin_edits(app: &mut Resonance, param_id: u32, value: f64) {
    let _ = app.test_engine_event_task(AudioEvent::PluginParamEdited {
        instance_id: DRUMS,
        edit: PluginParamEdit {
            param_id,
            value,
            text: format!("{value}"),
            gesture: true,
        },
    });
}

#[test]
fn a_plugin_side_edit_is_mirrored_and_undoable() {
    let (mut app, rx) = app();
    plugin_edits(&mut app, GAIN, 4.0);
    assert_eq!(app.test_plugin_param(DRUMS, GAIN), Some(4.0));
    assert!(
        !drain(&rx)
            .iter()
            .any(|c| matches!(c, AudioCommand::SetPluginParam { .. })),
        "the plugin already holds the value"
    );
    assert!(app.test_can_undo());

    let _ = app.update(Message::Undo);
    assert_eq!(app.test_plugin_param(DRUMS, GAIN), Some(0.0));
    assert!(
        drain(&rx).iter().any(|c| matches!(
            c,
            AudioCommand::SetPluginParam { param_id: GAIN, value, .. } if *value == 0.0
        )),
        "the undo drives the plugin back"
    );
}

/// The kit selector is not in the snapshot's param list (the state carries
/// the kit as a reference), so the BLOB carries the undo: the edit
/// refreshes the cached blob, and undoing it pushes the one from before.
#[test]
fn undoing_a_plugin_side_kit_change_restores_the_previous_state_blob() {
    let (mut app, rx) = app();
    plugin_edits(&mut app, KIT, 5.0);
    assert_eq!(app.test_plugin_param(DRUMS, KIT), Some(5.0));
    assert!(
        drain(&rx)
            .iter()
            .any(|c| matches!(c, AudioCommand::SavePluginState { instance_id } if *instance_id == DRUMS)),
        "the cached blob is refreshed after a state-excluded edit"
    );
    // The engine's echo: the state with the new kit in it.
    app.test_apply_engine_event(AudioEvent::PluginStateSaved {
        instance_id: DRUMS,
        data: vec![0xA5],
    });

    let _ = app.update(Message::Undo);
    let cmds = drain(&rx);
    assert!(
        cmds.iter().any(|c| matches!(
            c,
            AudioCommand::LoadPluginState { instance_id, data } if *instance_id == DRUMS && data == &vec![0xA0]
        )),
        "the blob from before the kit change goes back: {cmds:?}"
    );
    assert!(
        !cmds
            .iter()
            .any(|c| matches!(c, AudioCommand::SetPluginParam { param_id: KIT, .. })),
        "the slot itself is the plugin's to restore"
    );
}

#[test]
fn a_read_only_output_moves_the_mirror_without_an_undo_entry() {
    let (mut app, _rx) = app();
    plugin_edits(&mut app, PROGRESS, 0.5);
    assert_eq!(app.test_plugin_param(DRUMS, PROGRESS), Some(0.5));
    assert!(!app.test_can_undo());
}
