//! Removing or swapping out a plugin takes its automation lanes with it
//! (automation-control-api.md §2.2, decision D1).
//!
//! A lane aimed at a plugin instance that has left its chain is inert,
//! but it is still saved, and instance ids are reused across a save/load:
//! left in, it reattaches to whichever plugin later occupies the id (the
//! leak ba todo #1311 closed for key routes). Track deletion already
//! pruned its chain's lanes; a single plugin's removal — on a track, a bus
//! or the master — and a swap in `plugin_replace` did not.
//!
//! What is pinned here:
//!
//! 1. a removal drops the plugin's lanes in the mirror and tells the
//!    engine (`ClearAutomationLane`), and leaves every other lane alone;
//! 2. undo of the removal brings the lanes back exactly, and re-sends
//!    them to the engine;
//! 3. a swap drops the outgoing plugin's lanes;
//! 4. a relocate (a missing plugin found again) keeps its instance id and
//!    therefore its lanes;
//! 5. a swap owes the outgoing instance's removal echo, so an undo that
//!    lands before it keeps the restored slot and lanes when it arrives,
//!    and the debt settles (the restored instance is still adopted).

use std::path::PathBuf;

use crossbeam_channel::Receiver;
use resonance_app::message::{BusMessage, MasterMessage, Message, PluginMessage};
use resonance_app::state::PluginSlotState;
use resonance_app::update::automation::AutomationMessage;
use resonance_app::Resonance;
use resonance_audio::types::{
    AudioCommand, AudioEvent, ParamInfo, PluginInstanceId, ScannedPlugin, TrackType,
};
use resonance_common::{AutomationLane, AutomationTarget, CurveKind};

const TRACK: u64 = 7;
const BUS: u64 = 3;

const EQ: PluginInstanceId = 100;
const COMP: PluginInstanceId = 101;
const BUS_COMP: PluginInstanceId = 110;
const MASTER_LIM: PluginInstanceId = 120;

const EQ_ID: &str = "com.resonance.eq";
const COMP_ID: &str = "com.resonance.compressor";

struct Fixture {
    app: Resonance,
    rx: Receiver<AudioCommand>,
    root: PathBuf,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

/// A saved, active project (so edits record undo entries) with one audio
/// track carrying `EQ` and `COMP`, a bus carrying `BUS_COMP` and the
/// master carrying `MASTER_LIM`.
fn fixture(tag: &str) -> Fixture {
    let root = std::env::temp_dir().join(format!(
        "resonance-plugin-lanes-{tag}-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&root);
    let project = root.join("fixture.rproj");
    std::fs::create_dir_all(project.join("audio")).expect("create project dir");

    let (mut app, _task, rx) = Resonance::new_for_test_with_capture();
    app.test_set_active_project(true);
    app.test_set_project_path(project);
    app.test_add_track(TRACK, TrackType::Audio);
    app.test_add_bus(BUS, "Drums");
    app.test_push_track_plugin(TRACK, slot(EQ, EQ_ID));
    app.test_push_track_plugin(TRACK, slot(COMP, COMP_ID));
    app.test_push_bus_plugin(BUS, slot(BUS_COMP, COMP_ID));
    app.test_push_master_plugin(slot(MASTER_LIM, COMP_ID));
    let mut f = Fixture { app, rx, root };
    drain(&f.rx);
    // Two lanes on the plugin that will be removed, so "all of them" is
    // distinguishable from "the first one", and one lane each on a
    // neighbouring plugin and on the track's own fader.
    lane(&mut f, param(EQ, 1));
    lane(&mut f, param(EQ, 2));
    lane(&mut f, param(COMP, 1));
    lane(&mut f, AutomationTarget::TrackGain(TRACK));
    lane(&mut f, param(BUS_COMP, 1));
    lane(&mut f, param(MASTER_LIM, 1));
    drain(&f.rx);
    f
}

fn slot(instance_id: PluginInstanceId, plugin_id: &str) -> PluginSlotState {
    PluginSlotState::new(
        instance_id,
        plugin_id.to_owned(),
        plugin_id.to_owned(),
        format!("/plugins/{plugin_id}.clap"),
        Vec::new(),
        false,
    )
}

fn param(instance: PluginInstanceId, param_id: u32) -> AutomationTarget {
    AutomationTarget::PluginParam { instance, param_id }
}

/// A two-point lane on `target`, through the recorded edit path.
fn lane(f: &mut Fixture, target: AutomationTarget) {
    for (time_frames, value) in [(0, 0.25), (48_000, 0.75)] {
        let _ = f
            .app
            .update(Message::Automation(AutomationMessage::AddBreakpoint {
                target: target.clone(),
                time_frames,
                value,
                curve: CurveKind::Linear,
            }));
    }
    assert_eq!(
        f.app
            .test_automation()
            .lanes
            .get(&target)
            .map(|l| l.points.len()),
        Some(2),
        "fixture lane on {target:?}"
    );
}

fn drain(rx: &Receiver<AudioCommand>) -> Vec<AudioCommand> {
    std::iter::from_fn(|| rx.try_recv().ok()).collect()
}

fn lanes(f: &Fixture) -> Vec<AutomationLane> {
    let mut lanes: Vec<AutomationLane> = f.app.test_automation().lanes.values().cloned().collect();
    lanes.sort_by_key(|l| l.id);
    lanes
}

fn targets(f: &Fixture) -> Vec<AutomationTarget> {
    lanes(f).into_iter().map(|l| l.target).collect()
}

fn cleared(cmds: &[AudioCommand]) -> Vec<AutomationTarget> {
    cmds.iter()
        .filter_map(|c| match c {
            AudioCommand::ClearAutomationLane { target } => Some(target.clone()),
            _ => None,
        })
        .collect()
}

fn resent(cmds: &[AudioCommand]) -> Vec<AutomationTarget> {
    cmds.iter()
        .filter_map(|c| match c {
            AudioCommand::SetAutomationLane { lane } => Some(lane.target.clone()),
            _ => None,
        })
        .collect()
}

/// Remove, then undo, and check both sides of the round trip: the
/// removal drops exactly `gone` (mirror and engine), the undo restores
/// the pre-removal lane set exactly and re-sends `gone` to the engine.
fn remove_then_undo(f: &mut Fixture, remove: Message, gone: &[AutomationTarget]) {
    let before = lanes(f);
    let depth = f.app.test_undo_history().undo_len();

    let _ = f.app.update(remove);
    let cmds = drain(&f.rx);
    assert_eq!(
        f.app.test_undo_history().undo_len(),
        depth + 1,
        "the removal records one undo entry"
    );
    let expected: Vec<AutomationTarget> = before
        .iter()
        .map(|l| l.target.clone())
        .filter(|t| !gone.contains(t))
        .collect();
    assert_eq!(targets(f), expected, "only the removed plugin's lanes go");
    let mut sent = cleared(&cmds);
    sent.sort_by_key(|t| format!("{t:?}"));
    let mut want = gone.to_vec();
    want.sort_by_key(|t| format!("{t:?}"));
    assert_eq!(sent, want, "the engine is told to drop exactly those lanes");

    let _ = f.app.update(Message::Undo);
    let cmds = drain(&f.rx);
    assert_eq!(lanes(f), before, "undo brings the lanes back exactly");
    let mut sent = resent(&cmds);
    sent.sort_by_key(|t| format!("{t:?}"));
    assert_eq!(sent, want, "and re-sends exactly those lanes to the engine");
}

// ---------------------------------------------------------------------------
// 1 + 2. Removal prunes; undo restores
// ---------------------------------------------------------------------------

#[test]
fn removing_a_track_effect_drops_its_lanes_and_undo_restores_them() {
    let mut f = fixture("track");
    remove_then_undo(
        &mut f,
        Message::Plugin(PluginMessage::RemovePluginFromTrack(TRACK, EQ)),
        &[param(EQ, 1), param(EQ, 2)],
    );
    assert_eq!(
        f.app.test_track_plugin_instance_ids(TRACK),
        vec![EQ, COMP],
        "the undo put the plugin back under its own id, which the lanes name"
    );
}

#[test]
fn removing_a_bus_effect_drops_its_lanes_and_undo_restores_them() {
    let mut f = fixture("bus");
    remove_then_undo(
        &mut f,
        Message::Bus(BusMessage::RemovePluginFromBus(BUS, BUS_COMP)),
        &[param(BUS_COMP, 1)],
    );
}

#[test]
fn removing_a_master_effect_drops_its_lanes_and_undo_restores_them() {
    let mut f = fixture("master");
    remove_then_undo(
        &mut f,
        Message::Master(MasterMessage::RemovePluginFromMaster(MASTER_LIM)),
        &[param(MASTER_LIM, 1)],
    );
}

/// Deleting a whole bus takes its insert chain without a per-plugin
/// removal, so its plugins' lanes — and the bus's own fader lane — went
/// nowhere and reloaded onto the next bus (and plugin) handed those ids.
/// The bus deletion prunes them all, and its undo brings them back.
#[test]
fn deleting_a_bus_drops_its_own_and_its_plugins_lanes_and_undo_restores_them() {
    let mut f = fixture("bus-delete");
    lane(&mut f, AutomationTarget::BusGain(BUS));
    drain(&f.rx);
    remove_then_undo(
        &mut f,
        Message::Bus(BusMessage::RemoveBus(BUS)),
        &[param(BUS_COMP, 1), AutomationTarget::BusGain(BUS)],
    );
}

/// A removal nobody mirrored yet arrives as the engine's echo (a plugin
/// the engine dropped on its own). The echo path prunes the same way.
#[test]
fn an_unmirrored_removal_echo_drops_the_lanes_too() {
    let mut f = fixture("echo");
    f.app.test_apply_engine_event(AudioEvent::PluginRemoved {
        track_id: TRACK,
        instance_id: COMP,
    });
    assert!(!targets(&f).contains(&param(COMP, 1)));
    assert_eq!(cleared(&drain(&f.rx)), vec![param(COMP, 1)]);
    assert!(
        targets(&f).contains(&param(EQ, 1)),
        "a neighbour's lane stays"
    );
}

// ---------------------------------------------------------------------------
// 3. Swap prunes the outgoing plugin's lanes
// ---------------------------------------------------------------------------

#[test]
fn swapping_a_plugin_drops_the_outgoing_instances_lanes() {
    let mut f = fixture("swap");
    let before = lanes(&f);
    let _ = f.app.update(Message::Plugin(PluginMessage::ReplacePlugin {
        instance_id: EQ,
        plugin: scanned(COMP_ID),
    }));
    let cmds = drain(&f.rx);

    let ids = f.app.test_track_plugin_instance_ids(TRACK);
    assert_ne!(ids[0], EQ, "a swap issues a fresh instance id");
    assert!(
        !targets(&f).iter().any(|t| matches!(
            t,
            AutomationTarget::PluginParam { instance, .. } if *instance == EQ || *instance == ids[0]
        )),
        "no lane names the outgoing instance, and none was carried onto the new one"
    );
    let mut sent = cleared(&cmds);
    sent.sort_by_key(|t| format!("{t:?}"));
    assert_eq!(sent, vec![param(EQ, 1), param(EQ, 2)]);

    let _ = f.app.update(Message::Undo);
    assert_eq!(lanes(&f), before, "undo of the swap brings the lanes back");
}

// ---------------------------------------------------------------------------
// 4. A relocate keeps the lanes
// ---------------------------------------------------------------------------

/// The missing-plugin restore keeps the slot's instance id, so its lanes
/// still mean what they meant and must survive — through the load
/// failure, the relocate, and the engine's answer to it.
#[test]
fn relocating_a_missing_plugin_keeps_its_lanes() {
    let mut f = fixture("relocate");
    f.app.test_apply_engine_event(AudioEvent::PluginLoadFailed {
        instance_id: Some(EQ),
        clap_plugin_id: EQ_ID.to_owned(),
        clap_file_path: format!("/plugins/{EQ_ID}.clap"),
        reason: "Failed to load plugin: no such file".to_owned(),
    });
    let before = lanes(&f);
    assert!(before.iter().any(|l| l.target == param(EQ, 1)));

    let _ = f.app.update(Message::Plugin(PluginMessage::ReplacePlugin {
        instance_id: EQ,
        plugin: ScannedPlugin {
            clap_file_path: "/moved/eq.clap".to_owned(),
            ..scanned(EQ_ID)
        },
    }));
    let cmds = drain(&f.rx);
    assert!(
        cmds.iter()
            .any(|c| matches!(c, AudioCommand::AddPlugin { id, .. } if *id == EQ)),
        "a relocate re-adds under the same id: {cmds:?}"
    );
    assert_eq!(cleared(&cmds), Vec::<AutomationTarget>::new());
    assert_eq!(lanes(&f), before, "the relocate keeps every lane");

    f.app.test_apply_engine_event(AudioEvent::PluginAdded {
        track_id: TRACK,
        instance_id: EQ,
        plugin_name: EQ_ID.to_owned(),
        clap_plugin_id: EQ_ID.to_owned(),
        clap_file_path: "/moved/eq.clap".to_owned(),
        params: Vec::new(),
        has_gui: false,
        has_sidechain_input: false,
        output_port_count: 1,
        output_port_names: vec!["Main".to_owned()],
    });
    assert_eq!(lanes(&f), before, "and so does the instance turning up");
}

// ---------------------------------------------------------------------------
// 5. A late swap echo must not undo the undo
// ---------------------------------------------------------------------------

/// The engine's `PluginAdded` answer to the undo re-adding `EQ`, carrying
/// one parameter — the fixture's slots have none, so the parameter turning
/// up proves the echo was adopted rather than ignored as owed.
fn eq_added() -> AudioEvent {
    AudioEvent::PluginAdded {
        track_id: TRACK,
        instance_id: EQ,
        plugin_name: EQ_ID.to_owned(),
        clap_plugin_id: EQ_ID.to_owned(),
        clap_file_path: format!("/plugins/{EQ_ID}.clap"),
        params: vec![ParamInfo {
            id: 1,
            name: "Gain".to_owned(),
            min_value: -24.0,
            max_value: 24.0,
            default_value: 0.0,
            current_value: 3.0,
            ..Default::default()
        }],
        has_gui: false,
        has_sidechain_input: false,
        output_port_count: 1,
        output_port_names: vec!["Main".to_owned()],
    }
}

/// Swap, undo before the engine has answered, then deliver the swap's
/// late `PluginRemoved` for the outgoing id. The undo re-added the plugin
/// under that same id, so an echo nobody owed would drop the restored
/// slot and its lanes from the mirror while the engine kept it running.
#[test]
fn a_late_swap_removal_echo_keeps_the_plugin_an_undo_restored() {
    let mut f = fixture("swap-late-echo");
    let before = lanes(&f);
    let _ = f.app.update(Message::Plugin(PluginMessage::ReplacePlugin {
        instance_id: EQ,
        plugin: scanned(COMP_ID),
    }));
    let _ = f.app.update(Message::Undo);
    assert!(f.app.test_track_plugin_instance_ids(TRACK).contains(&EQ));
    assert_eq!(lanes(&f), before, "undo of the swap brings the lanes back");

    f.app.test_apply_engine_event(AudioEvent::PluginRemoved {
        track_id: TRACK,
        instance_id: EQ,
    });
    assert!(
        f.app.test_track_plugin_instance_ids(TRACK).contains(&EQ),
        "the late echo left the restored slot alone"
    );
    assert_eq!(lanes(&f), before, "and its lanes");

    // The echo settled the debt: the engine's answer to the undo's re-add
    // is adopted, not swallowed as a removal still owed.
    f.app.test_apply_engine_event(eq_added());
    assert_eq!(f.app.test_plugin_param(EQ, 1), Some(3.0));
}

/// A missing plugin swapped for a different one: the engine has no
/// instance to drop but still echoes the removal, so the owed echo
/// settles and the id is not left blocked for a later re-add.
#[test]
fn swapping_out_a_missing_plugin_settles_its_removal_echo() {
    let mut f = fixture("swap-missing");
    f.app.test_apply_engine_event(AudioEvent::PluginLoadFailed {
        instance_id: Some(EQ),
        clap_plugin_id: EQ_ID.to_owned(),
        clap_file_path: format!("/plugins/{EQ_ID}.clap"),
        reason: "Failed to load plugin: no such file".to_owned(),
    });
    let _ = f.app.update(Message::Plugin(PluginMessage::ReplacePlugin {
        instance_id: EQ,
        plugin: scanned(COMP_ID),
    }));
    assert!(!f.app.test_track_plugin_instance_ids(TRACK).contains(&EQ));
    f.app.test_apply_engine_event(AudioEvent::PluginRemoved {
        track_id: TRACK,
        instance_id: EQ,
    });

    let _ = f.app.update(Message::Undo);
    assert!(f.app.test_track_plugin_instance_ids(TRACK).contains(&EQ));
    f.app.test_apply_engine_event(eq_added());
    assert_eq!(
        f.app.test_plugin_param(EQ, 1),
        Some(3.0),
        "the re-added instance is adopted: no removal debt was left behind"
    );
}

fn scanned(plugin_id: &str) -> ScannedPlugin {
    ScannedPlugin {
        clap_file_path: format!("/plugins/{plugin_id}.clap"),
        clap_plugin_id: plugin_id.to_owned(),
        name: plugin_id.to_owned(),
        vendor: "Resonance".to_owned(),
        is_instrument: false,
        ..Default::default()
    }
}
