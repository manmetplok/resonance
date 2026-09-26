//! Undo / redo of a plugin parameter change or a preset recall restores
//! the parameter values (STATE-03).
//!
//! A param edit never changes the project's shape, so its undo takes the
//! quick-restore (diff replay) path. That path used to re-push only the
//! cached CLAP state blob — refreshed on plugin add, editor close and save,
//! never on a param change — and ignored the snapshot's param values: the
//! engine jumped back to the stale blob (reverting *every* param since),
//! the knobs kept showing the undone value, and the next save wrote the
//! undone value back into the project.

use resonance_app::message::{Message, PluginMessage};
use resonance_app::state::{PluginSlotState, ViewMode};
use resonance_app::Resonance;
use resonance_audio::test_support::Receiver;
use resonance_audio::types::{AudioCommand, ParamInfo, PluginInstanceId, TrackType};

const TRACK: u64 = 1;
const EQ: PluginInstanceId = 42;
const A: u32 = 1;
const B: u32 = 2;

fn param(id: u32, name: &str) -> ParamInfo {
    ParamInfo {
        id,
        name: name.to_owned(),
        min_value: 0.0,
        max_value: 10.0,
        default_value: 0.0,
        current_value: 0.0,
        ..Default::default()
    }
}

fn app() -> (Resonance, Receiver<AudioCommand>) {
    let (mut app, _task, rx) = Resonance::new_for_test_with_capture();
    app.test_set_view_mode(ViewMode::Arrange);
    app.test_set_active_project(true);
    // The history only records once the project has a path on disk.
    app.test_set_project_path(std::path::PathBuf::from("/tmp/plugin-param-undo.rprj"));
    app.test_add_track(TRACK, TrackType::Audio);
    app.test_push_track_plugin(
        TRACK,
        PluginSlotState::new(
            EQ,
            "EQ".to_owned(),
            "test.eq".to_owned(),
            "/plugins/test.eq.clap".to_owned(),
            vec![param(A, "A"), param(B, "B")],
            false,
        ),
    );
    // The blob cached at plugin add: every param at its default.
    app.test_seed_plugin_state(EQ, vec![0xD0]);
    (app, rx)
}

fn drain(rx: &Receiver<AudioCommand>) -> Vec<AudioCommand> {
    let mut cmds = Vec::new();
    while let Ok(cmd) = rx.try_recv() {
        cmds.push(cmd);
    }
    cmds
}

/// The value of `param_id` the engine was last told, if any command in
/// `cmds` set it.
fn last_sent(cmds: &[AudioCommand], param_id: u32) -> Option<f64> {
    cmds.iter().rev().find_map(|c| match c {
        AudioCommand::SetPluginParam {
            instance_id,
            param_id: p,
            value,
        } if *instance_id == EQ && *p == param_id => Some(*value),
        _ => None,
    })
}

/// The value `param_id` would be saved with (absent = default 0).
fn saved(app: &Resonance, param_id: u32) -> f64 {
    let file = app.test_build_project_file();
    let plugin = file
        .tracks
        .iter()
        .find(|t| t.id == TRACK)
        .and_then(|t| t.plugins.iter().find(|p| p.instance_id == EQ))
        .expect("the plugin is saved");
    plugin
        .params
        .iter()
        .find(|p| p.id == param_id)
        .map_or(0.0, |p| p.value)
}

fn set(app: &mut Resonance, param_id: u32, value: f64) {
    let _ = app.update(Message::Plugin(PluginMessage::SetPluginParam(
        EQ, param_id, value,
    )));
}

#[test]
fn undoing_a_param_change_restores_only_that_param_everywhere() {
    let (mut app, rx) = app();
    set(&mut app, A, 5.0);
    set(&mut app, B, 3.0);
    drain(&rx);

    let _ = app.update(Message::Undo);
    let cmds = drain(&rx);

    // The mirror (knob display) shows the undone state.
    assert_eq!(app.test_plugin_param(EQ, A), Some(5.0), "A keeps its edit");
    assert_eq!(app.test_plugin_param(EQ, B), Some(0.0), "B is undone");
    // The engine is told B's old value, and A's value survives: either no
    // stale blob is pushed (the cache hasn't moved since the snapshot), or
    // A is re-sent after it.
    assert_eq!(last_sent(&cmds, B), Some(0.0));
    let blob_at = cmds
        .iter()
        .position(|c| matches!(c, AudioCommand::LoadPluginState { .. }));
    if let Some(blob_at) = blob_at {
        assert_eq!(last_sent(&cmds, A), Some(5.0));
        let param_at = cmds
            .iter()
            .rposition(|c| matches!(c, AudioCommand::SetPluginParam { .. }))
            .expect("params re-sent");
        assert!(param_at > blob_at, "explicit values win over the blob");
    }
    // The next save persists the undone values.
    assert_eq!(saved(&app, A), 5.0);
    assert_eq!(saved(&app, B), 0.0);

    // Redo brings B back, in the mirror and the engine.
    let _ = app.update(Message::Redo);
    let cmds = drain(&rx);
    assert_eq!(app.test_plugin_param(EQ, B), Some(3.0));
    assert_eq!(last_sent(&cmds, B), Some(3.0));
    assert_eq!(saved(&app, B), 3.0);
}

#[test]
fn undoing_a_preset_recall_restores_the_previous_values() {
    let (mut app, rx) = app();
    set(&mut app, A, 2.0);
    let _ = app.update(Message::Plugin(PluginMessage::LoadPluginPreset {
        instance_id: EQ,
        values: vec![(A, 7.0), (B, 9.0)],
        preset_name: "Bright".to_owned(),
    }));
    drain(&rx);

    let _ = app.update(Message::Undo);
    let cmds = drain(&rx);
    assert_eq!(app.test_plugin_param(EQ, A), Some(2.0));
    assert_eq!(app.test_plugin_param(EQ, B), Some(0.0));
    assert_eq!(last_sent(&cmds, A), Some(2.0));
    assert_eq!(last_sent(&cmds, B), Some(0.0));
    assert_eq!(saved(&app, A), 2.0);
    assert_eq!(saved(&app, B), 0.0);
}

/// Undoing one knob re-sends that knob only (FU-A2b). The blob cached at
/// plugin add is the same one the snapshot holds, so the engine's live
/// state already matches the snapshot everywhere but the undone param:
/// the restore used to push that stale blob anyway and then re-send every
/// non-default param to repair what it reset — each one also echoing a
/// `PluginParamText` back.
#[test]
fn undoing_one_knob_resends_only_that_knob() {
    let (mut app, rx) = app();
    set(&mut app, A, 5.0);
    set(&mut app, B, 3.0);
    drain(&rx);

    let _ = app.update(Message::Undo);
    let cmds = drain(&rx);
    assert!(
        !cmds
            .iter()
            .any(|c| matches!(c, AudioCommand::LoadPluginState { .. })),
        "the unchanged cached blob is not re-pushed"
    );
    let sent: Vec<u32> = cmds
        .iter()
        .filter_map(|c| match c {
            AudioCommand::SetPluginParam { param_id, .. } => Some(*param_id),
            _ => None,
        })
        .collect();
    assert_eq!(sent, vec![B], "only the undone param is re-sent");
    assert_eq!(app.test_plugin_param(EQ, B), Some(0.0));
}

/// When the cache moved on since the snapshot (an editor close or a save
/// refreshed it), the snapshot's blob is pushed, and every param it may
/// have reset is driven back explicitly.
#[test]
fn a_refreshed_blob_is_pushed_and_params_reapplied_after_it() {
    let (mut app, rx) = app();
    set(&mut app, A, 5.0);
    set(&mut app, B, 3.0);
    // The engine's fresh blob arrives after the edits.
    app.test_seed_plugin_state(EQ, vec![0xD1]);
    drain(&rx);

    let _ = app.update(Message::Undo);
    let cmds = drain(&rx);
    let blob_at = cmds
        .iter()
        .position(|c| matches!(c, AudioCommand::LoadPluginState { .. }))
        .expect("the snapshot's blob differs from the live cache");
    assert_eq!(last_sent(&cmds, A), Some(5.0));
    assert_eq!(last_sent(&cmds, B), Some(0.0));
    let param_at = cmds
        .iter()
        .rposition(|c| matches!(c, AudioCommand::SetPluginParam { .. }))
        .expect("params re-sent");
    assert!(param_at > blob_at);
}
