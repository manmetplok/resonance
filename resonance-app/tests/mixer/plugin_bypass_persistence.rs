//! Per-slot bypass survives save + reload (ba doc #275 finding X3, todo
//! #1305).
//!
//! Like the sidechain route before ba todo #1311, a bypassed slot was
//! engine-only state: `SetPluginBypass` fired the command and the
//! `PluginBypassChanged` echo was consumed by an empty arm, so save had
//! nothing to write down. Bypass a plugin, save, reopen, and it was
//! running again with no error anywhere.
//!
//! These take the **real on-disk hop** (`save_project` -> `load_project`)
//! rather than a serde round trip in memory, because the failure being
//! guarded against is a field that never reaches `project.json` at all.
//!
//! Covered here: all three chains (a bypassed plugin on a track, on a bus
//! and on the master), the restore issuing the same command a user toggle
//! does, and back-compat with projects written before the field existed.

use resonance_app::project::{load_project, save_project, ProjectFile};
use resonance_app::Resonance;
use resonance_audio::test_support::Receiver;
use resonance_audio::types::{AudioCommand, AudioEvent, ParamInfo, TrackType};

const GTR: u64 = 1;
const BUS: u64 = 10;

const TRACK_EQ: u64 = 100;
const BUS_EQ: u64 = 200;
const MASTER_EQ: u64 = 300;

const EQ: &str = "com.resonance.eq";

fn drain(rx: &Receiver<AudioCommand>) -> Vec<AudioCommand> {
    std::iter::from_fn(|| rx.try_recv().ok()).collect()
}

fn add_track_plugin(app: &mut Resonance, track_id: u64, instance_id: u64) {
    app.test_apply_engine_event(AudioEvent::PluginAdded {
        track_id,
        instance_id,
        plugin_name: "EQ".to_string(),
        clap_plugin_id: EQ.to_string(),
        clap_file_path: "/plugins/eq.clap".to_string(),
        params: Vec::<ParamInfo>::new(),
        has_gui: false,
        has_sidechain_input: false,
        output_port_count: 1,
        output_port_names: vec!["Main".to_string()],
    });
}

fn add_bus_plugin(app: &mut Resonance, bus_id: u64, instance_id: u64) {
    app.test_apply_engine_event(AudioEvent::BusPluginAdded {
        bus_id,
        instance_id,
        plugin_name: "EQ".to_string(),
        clap_plugin_id: EQ.to_string(),
        clap_file_path: "/plugins/eq.clap".to_string(),
        params: Vec::<ParamInfo>::new(),
        has_gui: false,
        has_sidechain_input: false,
    });
}

fn add_master_plugin(app: &mut Resonance, instance_id: u64) {
    app.test_apply_engine_event(AudioEvent::MasterPluginAdded {
        instance_id,
        plugin_name: "EQ".to_string(),
        clap_plugin_id: EQ.to_string(),
        clap_file_path: "/plugins/eq.clap".to_string(),
        params: Vec::<ParamInfo>::new(),
        has_gui: false,
        has_sidechain_input: false,
    });
}

/// An EQ on a track, on a bus and on the master — every chain a slot can
/// sit in.
fn app_with_chains() -> Resonance {
    let (mut app, _task) = Resonance::new_for_test();
    app.test_set_active_project(true);
    app.test_add_track(GTR, TrackType::Audio);
    app.test_add_bus(BUS, "Gtr Bus");
    add_track_plugin(&mut app, GTR, TRACK_EQ);
    add_bus_plugin(&mut app, BUS, BUS_EQ);
    add_master_plugin(&mut app, MASTER_EQ);
    app
}

/// Bypass a slot the way the live app gets there: the engine echoes the
/// change back, which is the only thing that moves the app's flag.
fn bypass(app: &mut Resonance, instance_id: u64) {
    app.test_apply_engine_event(AudioEvent::PluginBypassChanged {
        instance_id,
        bypassed: true,
        own_bypass_param: false,
    });
}

fn save_and_reload(file: &ProjectFile) -> (Resonance, Vec<AudioCommand>) {
    let dir = tempfile::tempdir().expect("temp dir");
    save_project(dir.path(), file, &[], &[]).expect("save project");
    let loaded = load_project(dir.path()).expect("load project");

    let (mut app, _task) = Resonance::new_for_test();
    let rx = app.test_capture_engine();
    app.test_replay_loaded_project(loaded.file);
    let cmds = drain(&rx);
    (app, cmds)
}

fn bypass_commands(cmds: &[AudioCommand]) -> Vec<(u64, bool)> {
    cmds.iter()
        .filter_map(|c| match c {
            AudioCommand::SetPluginBypass {
                instance_id,
                bypassed,
            } => Some((*instance_id, *bypassed)),
            _ => None,
        })
        .collect()
}

#[test]
fn a_bypassed_slot_reaches_the_project_file() {
    let mut app = app_with_chains();
    bypass(&mut app, TRACK_EQ);

    let file = app.test_build_project_file();
    let saved = file.tracks[0].plugins[0].bypassed;
    assert!(
        saved,
        "the bypassed flag must reach project.json — this is the field that \
         did not exist, so the state lived only in the engine and save had \
         nothing to write"
    );
}

#[test]
fn bypass_survives_the_round_trip_on_all_three_chains() {
    let mut app = app_with_chains();
    bypass(&mut app, TRACK_EQ);
    bypass(&mut app, BUS_EQ);
    bypass(&mut app, MASTER_EQ);

    let (reloaded, _cmds) = save_and_reload(&app.test_build_project_file());

    let flags = reloaded.test_plugin_bypass_flags();
    assert_eq!(
        flags.get(&TRACK_EQ),
        Some(&true),
        "a bypassed track slot came back running"
    );
    assert_eq!(
        flags.get(&BUS_EQ),
        Some(&true),
        "a bypassed bus slot came back running"
    );
    assert_eq!(
        flags.get(&MASTER_EQ),
        Some(&true),
        "a bypassed master slot came back running"
    );
}

#[test]
fn the_restore_tells_the_engine_through_the_ordinary_command() {
    // Not a private restore path: load issues the same SetPluginBypass a
    // user toggle and a control-API call do, so there is one way into the
    // engine and the crossfade/settle rule cannot differ between them.
    let mut app = app_with_chains();
    bypass(&mut app, TRACK_EQ);
    bypass(&mut app, MASTER_EQ);

    let (_reloaded, cmds) = save_and_reload(&app.test_build_project_file());

    let mut issued = bypass_commands(&cmds);
    issued.sort();
    assert_eq!(
        issued,
        vec![(TRACK_EQ, true), (MASTER_EQ, true)]
            .into_iter()
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>(),
        "exactly the bypassed slots are restored, each once"
    );
}

#[test]
fn a_running_slot_costs_no_restore_command() {
    // The engine's default is running, so a command per slot on every
    // load would be pure noise on a project that bypasses nothing.
    let app = app_with_chains();
    let (_reloaded, cmds) = save_and_reload(&app.test_build_project_file());
    assert!(
        bypass_commands(&cmds).is_empty(),
        "a project with nothing bypassed must issue no bypass commands"
    );
}

#[test]
fn a_project_written_before_the_field_existed_loads_as_running() {
    // Back-compat: `#[serde(default)]` is what makes an older
    // project.json — which has no `bypassed` key at all — load as
    // not-bypassed, which is what it was.
    let app = app_with_chains();
    let file = app.test_build_project_file();

    let mut json = serde_json::to_value(&file).expect("serialize");
    let mut removed = 0;
    for track in json["tracks"].as_array_mut().expect("tracks") {
        for plugin in track["plugins"].as_array_mut().expect("plugins") {
            if plugin.as_object_mut().expect("object").remove("bypassed").is_some() {
                removed += 1;
            }
        }
    }
    assert!(removed > 0, "the fixture must actually carry the key to remove");

    let older: ProjectFile = serde_json::from_value(json).expect("an older project still loads");
    assert!(
        !older.tracks[0].plugins[0].bypassed,
        "a missing `bypassed` key must read as not-bypassed"
    );
}

// ---------------------------------------------------------------------------
// Undo (ba todo #1305, caught by review)
// ---------------------------------------------------------------------------

/// Bypass is UNDOABLE, not merely recorded.
///
/// The first cut of this todo classified `SetPluginBypass` as
/// `UndoAction::Record` and stopped there, which looks right and is not:
/// undo restored through the diff replay, and the diff copied only
/// `plugin_name` per slot. So an undo pushed its entry, replayed, and
/// changed nothing at all — the flag stayed put and the engine was never
/// told.
///
/// It could not fall back to a full reload either. `plugin_set_matches`
/// compares slot IDENTITY only (instance id + clap id + path), by design:
/// a bypass-only change is not structural and must not cost a reload. So
/// the difference has to be applied by the diff, or nowhere.
///
/// This asserts BOTH halves of what undo has to do — move the app flag
/// back, and tell the engine — because either alone leaves the mixer and
/// the audio disagreeing.
#[test]
fn undo_restores_a_bypassed_slot_and_tells_the_engine() {
    let mut app = app_with_chains();
    let before = app.test_snapshot_for_undo();

    bypass(&mut app, TRACK_EQ);
    assert_eq!(
        app.test_plugin_bypass_flags().get(&TRACK_EQ),
        Some(&true),
        "precondition: the slot is bypassed before we undo"
    );

    let rx = app.test_capture_engine();
    app.test_begin_restore_from_snapshot(before);

    assert_eq!(
        app.test_plugin_bypass_flags().get(&TRACK_EQ),
        Some(&false),
        "undo left the slot bypassed — the diff replay dropped the flag"
    );
    assert_eq!(
        bypass_commands(&drain(&rx)),
        vec![(TRACK_EQ, false)],
        "undo moved the app flag but never told the engine, so the mixer \
         would read un-bypassed while the audio stayed bypassed"
    );
}

/// The same, one chain over, because the diff applies each chain in its
/// own arm and a fix that reaches only tracks is the likeliest shape of a
/// partial one.
#[test]
fn undo_restores_a_bypassed_bus_and_master_slot_too() {
    let mut app = app_with_chains();
    let before = app.test_snapshot_for_undo();

    bypass(&mut app, BUS_EQ);
    bypass(&mut app, MASTER_EQ);

    let rx = app.test_capture_engine();
    app.test_begin_restore_from_snapshot(before);

    let flags = app.test_plugin_bypass_flags();
    assert_eq!(flags.get(&BUS_EQ), Some(&false), "the bus slot did not undo");
    assert_eq!(
        flags.get(&MASTER_EQ),
        Some(&false),
        "the master slot did not undo"
    );

    let mut issued = bypass_commands(&drain(&rx));
    issued.sort();
    assert_eq!(
        issued,
        vec![(BUS_EQ, false), (MASTER_EQ, false)],
        "each restored slot must be reported to the engine exactly once"
    );
}

/// A slot the undo does not change costs no command. The engine
/// crossfades a bypass, so re-asserting a value that is not moving would
/// start a fade for nothing.
#[test]
fn undo_says_nothing_about_slots_that_did_not_move() {
    let mut app = app_with_chains();
    bypass(&mut app, TRACK_EQ);

    // Snapshot taken WITH the track slot already bypassed, so undoing the
    // later bus change must leave it alone.
    let before = app.test_snapshot_for_undo();
    bypass(&mut app, BUS_EQ);

    let rx = app.test_capture_engine();
    app.test_begin_restore_from_snapshot(before);

    assert_eq!(
        bypass_commands(&drain(&rx)),
        vec![(BUS_EQ, false)],
        "only the slot that actually moved may be re-sent"
    );
    assert_eq!(
        app.test_plugin_bypass_flags().get(&TRACK_EQ),
        Some(&true),
        "the untouched slot must keep its state"
    );
}

// ---------------------------------------------------------------------------
// The GUI reaches every chain (ba todo #1305, caught by review)
// ---------------------------------------------------------------------------

/// A human can bypass a plugin on ALL THREE chains, not just the two the
/// inspector draws.
///
/// The first cut wired the inspector chain row, which covers a selected
/// track and a selected bus. `view/mixer/inspector/mod.rs` has no master
/// branch — the master chain is drawn only by the strip — so a master
/// plugin could be bypassed over MCP and not by hand. That is exactly the
/// inversion ba doc #276's dual-surface rule exists to prevent, and this
/// todo's DONE WHEN names the master chain explicitly.
#[test]
fn every_chain_offers_a_bypass_control_including_the_master() {
    let app = app_with_chains();
    for (instance_id, chain) in [
        (TRACK_EQ, "track"),
        (BUS_EQ, "bus"),
        (MASTER_EQ, "master"),
    ] {
        assert!(
            app.test_strip_bypass_toggle(instance_id).is_some(),
            "the {chain} chain draws no bypass control, so an agent can \
             bypass it and a human cannot"
        );
    }
}

/// The control SETS rather than toggles, and it sets the opposite of what
/// the slot currently is — so the button and the wire raise the identical
/// message and cannot drift apart.
#[test]
fn the_strip_control_sets_the_opposite_of_the_current_state() {
    use resonance_app::message::{Message, PluginMessage};

    let mut app = app_with_chains();
    let (message, _) = app
        .test_strip_bypass_toggle(MASTER_EQ)
        .expect("the master slot draws a bypass control");
    assert!(
        matches!(
            message,
            Message::Plugin(PluginMessage::SetPluginBypass {
                instance_id,
                bypassed: true,
            }) if instance_id == MASTER_EQ
        ),
        "a running slot's control must ask for bypassed: true"
    );

    bypass(&mut app, MASTER_EQ);
    let (message, _) = app
        .test_strip_bypass_toggle(MASTER_EQ)
        .expect("still drawn once bypassed");
    assert!(
        matches!(
            message,
            Message::Plugin(PluginMessage::SetPluginBypass {
                instance_id,
                bypassed: false,
            }) if instance_id == MASTER_EQ
        ),
        "a bypassed slot's control must ask for bypassed: false — a \
         control that always sent `true` would look like a toggle and be \
         a one-way trip"
    );
}
