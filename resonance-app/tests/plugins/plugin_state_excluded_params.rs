//! A parameter the plugin's own state carries in another form, and a
//! read-only output, are the plugin's to restore — never the host's
//! (drums-plugin-rework.md §5.1, §5.4).
//!
//! The drums' `kit_select` is a slot into this machine's kit library; the
//! kit itself travels in the state blob as a content reference. The app
//! used to write every non-default param into `project.json` and re-send
//! it after the blob on load (and on undo), so a project opened on a
//! machine whose library numbers its kits differently loaded the kit in
//! THAT slot — over the one the blob had just recalled.

use resonance_app::message::{Message, PluginMessage};
use resonance_app::state::{PluginSlotState, ViewMode};
use resonance_app::Resonance;
use resonance_audio::test_support::Receiver;
use resonance_audio::types::{
    AudioCommand, AudioEvent, ParamInfo, ParamValueUpdate, PluginInstanceId, TrackType,
};

const TRACK: u64 = 7;
const DRUMS: PluginInstanceId = 77;
const PLUGIN_ID: &str = "test.drums";
const GAIN: u32 = 1;
const KIT_SELECT: u32 = 2;
const PROGRESS: u32 = 3;

fn params() -> Vec<ParamInfo> {
    vec![
        ParamInfo {
            id: GAIN,
            name: "Gain".to_owned(),
            max_value: 10.0,
            ..Default::default()
        },
        // Not automatable, and the state leaves it out.
        ParamInfo {
            id: KIT_SELECT,
            name: "Kit".to_owned(),
            min_value: -1.0,
            max_value: 999.0,
            default_value: -1.0,
            current_value: -1.0,
            stepped: true,
            automatable: false,
            state_excluded: true,
            ..Default::default()
        },
        // A read-only output.
        ParamInfo {
            id: PROGRESS,
            name: "Kit Load Progress".to_owned(),
            max_value: 1.0,
            automatable: false,
            read_only: true,
            state_excluded: true,
            ..Default::default()
        },
    ]
}

fn app() -> (Resonance, Receiver<AudioCommand>) {
    let (mut app, _task, rx) = Resonance::new_for_test_with_capture();
    app.test_set_view_mode(ViewMode::Arrange);
    app.test_set_active_project(true);
    app.test_set_project_path(std::path::PathBuf::from("/tmp/plugin-state-excluded.rprj"));
    app.test_add_track(TRACK, TrackType::Instrument);
    app.test_push_track_plugin(
        TRACK,
        PluginSlotState::new(
            DRUMS,
            "Drums".to_owned(),
            PLUGIN_ID.to_owned(),
            "/plugins/test.drums.clap".to_owned(),
            params(),
            false,
        ),
    );
    app.test_seed_plugin_state(DRUMS, vec![0xD0]);
    (app, rx)
}

fn drain(rx: &Receiver<AudioCommand>) -> Vec<AudioCommand> {
    let mut cmds = Vec::new();
    while let Ok(cmd) = rx.try_recv() {
        cmds.push(cmd);
    }
    cmds
}

fn sent_ids(cmds: &[AudioCommand]) -> Vec<u32> {
    cmds.iter()
        .filter_map(|c| match c {
            AudioCommand::SetPluginParam { param_id, .. } => Some(*param_id),
            _ => None,
        })
        .collect()
}

fn set(app: &mut Resonance, param_id: u32, value: f64) {
    let _ = app.update(Message::Plugin(PluginMessage::SetPluginParam(
        DRUMS, param_id, value,
    )));
}

/// The plugin moved its own selection and progress, and its values rescan
/// reported them: the mirror shows them, as any reader expects.
fn plugin_reports(app: &mut Resonance, kit: f64, progress: f64) {
    app.test_apply_engine_event(AudioEvent::PluginParamValuesChanged {
        instance_id: DRUMS,
        values: vec![
            ParamValueUpdate {
                id: KIT_SELECT,
                value: kit,
                text: format!("Kit {kit}"),
            },
            ParamValueUpdate {
                id: PROGRESS,
                value: progress,
                text: format!("{:.0} %", progress * 100.0),
            },
        ],
    });
}

#[test]
fn the_project_file_carries_neither_the_kit_slot_nor_the_progress() {
    let (mut app, _rx) = app();
    set(&mut app, GAIN, 3.0);
    set(&mut app, KIT_SELECT, 4.0);
    plugin_reports(&mut app, 4.0, 0.5);
    assert_eq!(app.test_plugin_param(DRUMS, KIT_SELECT), Some(4.0));
    assert_eq!(app.test_plugin_param(DRUMS, PROGRESS), Some(0.5));
    // The values rescan carried the plugin's text, too.
    assert_eq!(
        app.test_plugin_param_text(DRUMS, PROGRESS).as_deref(),
        Some("50 %")
    );

    let file = app.test_build_project_file();
    let saved: Vec<u32> = file.tracks[0].plugins[0]
        .params
        .iter()
        .map(|p| p.id)
        .collect();
    assert_eq!(
        saved,
        vec![GAIN],
        "only the host-persisted param is written"
    );
}

/// A project written before the plugin declared the slot state-excluded
/// still carries it: reopening re-sends the ordinary param after the blob,
/// and not the slot — the blob's kit reference decides the kit.
#[test]
fn a_saved_kit_slot_is_not_re_sent_over_the_blob_on_reopen() {
    let (mut app, rx) = app();
    set(&mut app, GAIN, 3.0);
    let mut file = app.test_build_project_file();
    file.tracks[0].plugins[0]
        .params
        .push(resonance_app::project::ProjectPluginParam {
            id: KIT_SELECT,
            name: "Kit".to_owned(),
            value: 4.0,
        });
    drain(&rx);

    app.test_replay_loaded_project(file);
    app.test_apply_engine_event(AudioEvent::PluginAdded {
        track_id: TRACK,
        instance_id: DRUMS,
        plugin_name: "Drums".to_owned(),
        clap_plugin_id: PLUGIN_ID.to_owned(),
        clap_file_path: "/plugins/test.drums.clap".to_owned(),
        params: params(),
        has_gui: false,
        has_sidechain_input: false,
        output_port_count: 1,
        output_port_names: vec!["Main".to_owned()],
    });
    let cmds = drain(&rx);
    assert_eq!(sent_ids(&cmds), vec![GAIN], "the slot is not re-sent");
    assert_eq!(app.test_plugin_param(DRUMS, KIT_SELECT), Some(-1.0));
}

/// An undo restores from a snapshot that has no value for the slot —
/// which must not read as "at its default": driving it to -1 after the
/// pushed blob would unload the kit the blob recalls.
#[test]
fn an_undo_never_drives_the_kit_slot_or_the_progress() {
    let (mut app, rx) = app();
    plugin_reports(&mut app, 4.0, 1.0);
    set(&mut app, GAIN, 5.0);
    // The cache moved on since the snapshot, so the undo pushes the
    // snapshot's blob and re-drives every param it may have reset.
    app.test_seed_plugin_state(DRUMS, vec![0xD1]);
    drain(&rx);

    let _ = app.update(Message::Undo);
    let cmds = drain(&rx);
    assert!(cmds
        .iter()
        .any(|c| matches!(c, AudioCommand::LoadPluginState { .. })));
    assert_eq!(sent_ids(&cmds), vec![GAIN]);
    assert_eq!(app.test_plugin_param(DRUMS, GAIN), Some(0.0));
    assert_eq!(app.test_plugin_param(DRUMS, KIT_SELECT), Some(4.0));
}
