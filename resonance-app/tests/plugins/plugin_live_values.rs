//! What the plugin plays is what a save keeps, and what an undo restores
//! (code review PUX-01 host side, STATE2-03, STATE2-07, STATE2-10).
//!
//! * An edit the plugin makes itself — its editor, its own preset bar —
//!   never reaches the app's param mirror unless the plugin announces it.
//!   The project file writes every non-default mirror value as an override
//!   that reopen re-sends *after* the state blob, so a stale mirror used to
//!   revert the edit. The engine's `SaveAllPluginStates` now reports the
//!   values that moved ahead of the states (`resonance-audio`
//!   `tests/clap_host/plugin_live_values.rs`), and the file the app writes
//!   when the states arrive carries them.
//! * A parameter the plugin's state carries in its own form (the drums'
//!   `kit_select`) set from the host refreshed neither the blob nor the
//!   snapshot's params, so its undo restored nothing. Undo snapshots now
//!   carry it, and an undo that does not push the blob drives it back.
//! * An editor kit pick refreshes the cached blob; a snapshot recorded
//!   before the fresh blob is back is owed it.
//! * A write to a read-only output records nothing.

use resonance_app::message::{Message, PluginMessage, ProjectIoMessage, TrackMessage};
use resonance_app::state::{PluginSlotState, ViewMode};
use resonance_app::Resonance;
use resonance_audio::test_support::Receiver;
use resonance_audio::types::{
    AudioCommand, AudioEvent, ParamInfo, ParamValueUpdate, PluginInstanceId, PluginParamEdit,
    TrackType,
};

const TRACK: u64 = 7;
const PLUGIN: PluginInstanceId = 71;
const DRUMS_ID: &str = "com.resonance.drums";
const GAIN: u32 = 1;
const PROGRESS: u32 = 3;

fn kit() -> u32 {
    resonance_plugin::stable_hash("kit_select")
}

fn params() -> Vec<ParamInfo> {
    vec![
        ParamInfo {
            id: GAIN,
            name: "Gain".to_owned(),
            max_value: 10.0,
            ..Default::default()
        },
        ParamInfo {
            id: kit(),
            name: "kit_select".to_owned(),
            min_value: -2.0,
            max_value: 999.0,
            default_value: -1.0,
            current_value: 2.0,
            stepped: true,
            automatable: false,
            state_excluded: true,
            ..Default::default()
        },
        ParamInfo {
            id: PROGRESS,
            name: "Kit Load Progress".to_owned(),
            max_value: 1.0,
            current_value: 1.0,
            automatable: false,
            read_only: true,
            state_excluded: true,
            ..Default::default()
        },
    ]
}

fn project_dir(tag: &str) -> std::path::PathBuf {
    let root = std::env::temp_dir().join(format!(
        "resonance_plugin_live_values_{tag}_{}",
        std::process::id()
    ));
    std::env::set_var("XDG_CONFIG_HOME", root.join("config"));
    let dir = root.join("song.rproj");
    std::fs::create_dir_all(&dir).expect("create project dir");
    dir
}

fn app(tag: &str) -> (Resonance, Receiver<AudioCommand>) {
    let (mut app, _task, rx) = Resonance::new_for_test_with_capture();
    app.test_set_view_mode(ViewMode::Arrange);
    app.test_set_active_project(true);
    app.test_set_project_path(project_dir(tag));
    app.test_add_track(TRACK, TrackType::Instrument);
    app.test_push_track_plugin(
        TRACK,
        PluginSlotState::new(
            PLUGIN,
            "Drums".to_owned(),
            DRUMS_ID.to_owned(),
            "/plugins/drums.clap".to_owned(),
            params(),
            false,
        ),
    );
    app.test_seed_plugin_state(PLUGIN, vec![0xA0]);
    drain(&rx);
    (app, rx)
}

fn drain(rx: &Receiver<AudioCommand>) -> Vec<AudioCommand> {
    let mut cmds = Vec::new();
    while let Ok(cmd) = rx.try_recv() {
        cmds.push(cmd);
    }
    cmds
}

fn sets(cmds: &[AudioCommand]) -> Vec<(u32, f64)> {
    cmds.iter()
        .filter_map(|c| match c {
            AudioCommand::SetPluginParam {
                param_id, value, ..
            } => Some((*param_id, *value)),
            _ => None,
        })
        .collect()
}

fn loaded_blobs(cmds: &[AudioCommand]) -> Vec<Vec<u8>> {
    cmds.iter()
        .filter_map(|c| match c {
            AudioCommand::LoadPluginState { data, .. } => Some(data.clone()),
            _ => None,
        })
        .collect()
}

/// Run a save to the point the app captures the project file: the engine
/// reports the values the plugin moved itself, then the states.
fn save_with_engine_report(
    app: &mut Resonance,
    moved: Vec<ParamValueUpdate>,
) -> resonance_app::project::ProjectFile {
    let _ = app.update(Message::ProjectIo(ProjectIoMessage::SaveProject));
    app.test_apply_engine_event(AudioEvent::ClipsSavedToProjectDir {
        clip_files: Vec::new(),
    });
    app.test_apply_engine_event(AudioEvent::PluginParamValuesChanged {
        instance_id: PLUGIN,
        values: moved,
    });
    app.test_apply_engine_event(AudioEvent::AllPluginStatesSaved {
        states: vec![(PLUGIN, vec![0xB0])],
    });
    app.test_build_project_file()
}

fn gain(value: f64) -> ParamValueUpdate {
    ParamValueUpdate {
        id: GAIN,
        value,
        text: format!("{value}"),
    }
}

/// Reopen `file` and return what the app sent after the blob.
fn reopen(
    app: &mut Resonance,
    rx: &Receiver<AudioCommand>,
    file: resonance_app::project::ProjectFile,
) -> Vec<AudioCommand> {
    drain(rx);
    app.test_replay_loaded_project(file);
    app.test_apply_engine_event(AudioEvent::PluginAdded {
        track_id: TRACK,
        instance_id: PLUGIN,
        plugin_name: "Drums".to_owned(),
        clap_plugin_id: DRUMS_ID.to_owned(),
        clap_file_path: "/plugins/drums.clap".to_owned(),
        params: params(),
        has_gui: false,
        has_sidechain_input: false,
        output_port_count: 1,
        output_port_names: vec!["Main".to_owned()],
    });
    drain(rx)
}

#[test]
fn a_plugin_side_edit_survives_save_and_reopen() {
    let (mut app, rx) = app("plugin_side");
    // The plugin's editor moved Gain to 7; the app was never told.
    let file = save_with_engine_report(&mut app, vec![gain(7.0)]);
    let saved: Vec<(u32, f64)> = file.tracks[0].plugins[0]
        .params
        .iter()
        .map(|p| (p.id, p.value))
        .collect();
    assert_eq!(saved, vec![(GAIN, 7.0)]);

    let cmds = reopen(&mut app, &rx, file);
    assert_eq!(sets(&cmds), vec![(GAIN, 7.0)], "the override re-sent is the edit");
    assert_eq!(app.test_plugin_param(PLUGIN, GAIN), Some(7.0));
}

/// Host preset A, then preset B picked in the plugin's own preset bar:
/// the file must not bring A's values back over B's blob.
#[test]
fn an_editor_preset_pick_after_a_host_preset_load_survives_save_and_reopen() {
    let (mut app, rx) = app("preset_pick");
    // The host loaded preset A; the engine re-read the params after it.
    app.test_apply_engine_event(AudioEvent::PluginParamsRefreshed {
        instance_id: PLUGIN,
        params: vec![ParamInfo {
            current_value: 3.0,
            ..params()[0].clone()
        }],
    });
    assert_eq!(app.test_plugin_param(PLUGIN, GAIN), Some(3.0));
    // Preset B (Gain 8) picked in the editor: silent.
    let file = save_with_engine_report(&mut app, vec![gain(8.0)]);
    assert_eq!(file.tracks[0].plugins[0].params[0].value, 8.0);

    let cmds = reopen(&mut app, &rx, file);
    assert_eq!(sets(&cmds), vec![(GAIN, 8.0)], "B, not A");
}

/// `track.set_plugin_param kit_select` from the host, then undo: the
/// snapshot's blob is the very one cached (a host write refreshes no
/// blob), so the old kit has to be driven back as a param.
#[test]
fn undoing_a_host_kit_change_puts_the_old_kit_back() {
    let (mut app, rx) = app("host_kit");
    let _ = app.update(Message::Plugin(PluginMessage::SetPluginParam(PLUGIN, kit(), 4.0)));
    assert_eq!(app.test_plugin_param(PLUGIN, kit()), Some(4.0));
    drain(&rx);

    let _ = app.update(Message::Undo);
    let cmds = drain(&rx);
    assert!(loaded_blobs(&cmds).is_empty(), "the cached blob is the live one");
    assert_eq!(sets(&cmds), vec![(kit(), 2.0)], "the old kit, driven back");
    assert_eq!(app.test_plugin_param(PLUGIN, kit()), Some(2.0));

    // And redo picks the new kit again.
    let _ = app.update(Message::Redo);
    assert_eq!(sets(&drain(&rx)), vec![(kit(), 4.0)]);
}

/// The kit slot is undo state, not file state.
#[test]
fn the_kit_slot_rides_in_undo_snapshots_but_not_in_the_file() {
    let (app, _rx) = app("snapshot_only");
    let file = app.test_build_project_file();
    assert!(file.tracks[0].plugins[0].params.is_empty());
    let snap = app.test_snapshot_for_undo();
    let ids: Vec<u32> = snap.project.file.tracks[0].plugins[0]
        .params
        .iter()
        .map(|p| p.id)
        .collect();
    assert_eq!(ids, vec![kit()], "the slot, not the read-only progress");
}

/// An editor kit pick asks for a fresh blob. An unrelated edit recorded
/// before it is back holds the old blob; undoing that edit must not put
/// the old kit back once the fresh blob has replaced the cache.
#[test]
fn an_edit_recorded_while_a_kit_pick_blob_is_owed_does_not_undo_the_pick() {
    let (mut app, rx) = app("owed_blob");
    app.test_apply_engine_event(AudioEvent::PluginParamEdited {
        instance_id: PLUGIN,
        edit: PluginParamEdit {
            param_id: kit(),
            value: 5.0,
            text: "Garage".to_owned(),
            gesture: true,
        },
    });
    let asks_for_blob = |c: &AudioCommand| {
        matches!(c, AudioCommand::SavePluginState { instance_id } if *instance_id == PLUGIN)
    };
    assert!(drain(&rx).iter().any(asks_for_blob));
    // An unrelated edit before the echo.
    let _ = app.update(Message::Track(TrackMessage::SetTrackVolume(TRACK, -6.0)));
    // The fresh blob arrives.
    app.test_apply_engine_event(AudioEvent::PluginStateSaved {
        instance_id: PLUGIN,
        data: vec![0xA5],
    });
    assert_eq!(app.test_cached_plugin_state(PLUGIN), Some(vec![0xA5]));
    drain(&rx);

    // Undo the volume: the kit stays.
    let _ = app.update(Message::Undo);
    let cmds = drain(&rx);
    assert!(
        loaded_blobs(&cmds).is_empty(),
        "the snapshot was filled with the fresh blob: {:?}",
        loaded_blobs(&cmds)
    );
    assert!(sets(&cmds).iter().all(|(id, _)| *id != kit()));
    assert_eq!(app.test_plugin_param(PLUGIN, kit()), Some(5.0));

    // Undo the pick: the old kit comes back with its blob.
    let _ = app.update(Message::Undo);
    let cmds = drain(&rx);
    assert_eq!(loaded_blobs(&cmds), vec![vec![0xA0]]);
}

/// An echo that lands after an undo describes the state the undo
/// replaced: it fills what waits on it, but the cache keeps what the
/// restore put there.
#[test]
fn an_owed_blob_landing_after_an_undo_does_not_replace_the_restored_cache() {
    let (mut app, rx) = app("owed_after_undo");
    app.test_apply_engine_event(AudioEvent::PluginParamEdited {
        instance_id: PLUGIN,
        edit: PluginParamEdit {
            param_id: kit(),
            value: 5.0,
            text: "Garage".to_owned(),
            gesture: true,
        },
    });
    let _ = app.update(Message::Undo);
    let cmds = drain(&rx);
    assert_eq!(sets(&cmds), vec![(kit(), 2.0)], "driven back before the echo");
    app.test_apply_engine_event(AudioEvent::PluginStateSaved {
        instance_id: PLUGIN,
        data: vec![0xA5],
    });
    assert_eq!(app.test_cached_plugin_state(PLUGIN), Some(vec![0xA0]));

    // Redo: the redo snapshot was owed the echo, so it brings the pick's
    // blob back.
    let _ = app.update(Message::Redo);
    assert_eq!(loaded_blobs(&drain(&rx)), vec![vec![0xA5]]);
}

/// A write to a read-only output is gated before the undo bookkeeping.
#[test]
fn setting_a_read_only_param_records_nothing() {
    let (mut app, rx) = app("read_only");
    let revision = app.revision();
    let dirty = app.is_dirty();
    let undo_len = app.test_undo_history().undo_len();

    let _ = app.update(Message::Plugin(PluginMessage::SetPluginParam(PLUGIN, PROGRESS, 0.3)));

    assert!(sets(&drain(&rx)).is_empty());
    assert_eq!(app.revision(), revision);
    assert_eq!(app.is_dirty(), dirty);
    assert_eq!(app.test_undo_history().undo_len(), undo_len);
}
