//! Sidechain key routes survive save + reload (ba doc #157/#159, doc
//! #275 finding P4, ba todo #1311).
//!
//! Before this landed a key route existed *only* inside the engine
//! thread: `SetPluginSidechain` fired the command and kept nothing app
//! side, and the `SidechainRouteChanged` echo was consumed with an empty
//! arm. Save serializes app state, so there was nothing to write down —
//! dial in a duck, save, reopen, and it was gone with no error anywhere.
//!
//! These tests pin the whole round trip, and deliberately take the
//! **real on-disk hop** (`save_project` -> `load_project`) rather than a
//! serde round trip in memory: the failure being guarded against is a
//! field that never reaches `project.json` at all.
//!
//! They also cover the widened targets — the point of the finding is
//! that a plugin on a *bus* or the *master* chain can be keyed, not just
//! one on a track — plus the edge-pruning rules a route shares with an
//! aux send, and back-compat with projects written before the field
//! existed.

use resonance_app::project::{load_project, save_project, ProjectFile};
use resonance_app::Resonance;
use resonance_audio::test_support::Receiver;
use resonance_audio::types::{AudioCommand, AudioEvent, ParamInfo, SendSource, TrackType};

const KICK: u64 = 1;
const BASS: u64 = 2;
const BUS: u64 = 10;

/// Plugin instance ids. Chosen distinct per host so a route can only
/// match the plugin it was meant for.
const TRACK_COMP: u64 = 100;
const BUS_COMP: u64 = 200;
const MASTER_COMP: u64 = 300;

const COMPRESSOR: &str = "com.resonance.compressor";

fn drain(rx: &Receiver<AudioCommand>) -> Vec<AudioCommand> {
    std::iter::from_fn(|| rx.try_recv().ok()).collect()
}

/// Add a compressor to a track / bus / the master through the engine's
/// own echo, so the slot carries the real `has_sidechain_input` flag the
/// control layer and the mixer both key off.
fn add_track_plugin(app: &mut Resonance, track_id: u64, instance_id: u64) {
    app.test_apply_engine_event(AudioEvent::PluginAdded {
        track_id,
        instance_id,
        plugin_name: "Compressor".to_string(),
        clap_plugin_id: COMPRESSOR.to_string(),
        clap_file_path: "/plugins/compressor.clap".to_string(),
        params: Vec::<ParamInfo>::new(),
        has_gui: false,
        has_sidechain_input: true,
        output_port_count: 1,
        output_port_names: vec!["Main".to_string()],
    });
}

fn add_bus_plugin(app: &mut Resonance, bus_id: u64, instance_id: u64) {
    app.test_apply_engine_event(AudioEvent::BusPluginAdded {
        bus_id,
        instance_id,
        plugin_name: "Compressor".to_string(),
        clap_plugin_id: COMPRESSOR.to_string(),
        clap_file_path: "/plugins/compressor.clap".to_string(),
        params: Vec::<ParamInfo>::new(),
        has_gui: false,
        has_sidechain_input: true,
    });
}

fn add_master_plugin(app: &mut Resonance, instance_id: u64) {
    app.test_apply_engine_event(AudioEvent::MasterPluginAdded {
        instance_id,
        plugin_name: "Compressor".to_string(),
        clap_plugin_id: COMPRESSOR.to_string(),
        clap_file_path: "/plugins/compressor.clap".to_string(),
        params: Vec::<ParamInfo>::new(),
        has_gui: false,
        has_sidechain_input: true,
    });
}

/// A kick, a bass track with a compressor, a bus with a compressor, and
/// a compressor on the master — every place a key can land.
fn app_with_chains() -> Resonance {
    let (mut app, _task) = Resonance::new_for_test();
    app.test_set_active_project(true);
    app.test_add_track(KICK, TrackType::Instrument);
    app.test_add_track(BASS, TrackType::Instrument);
    app.test_add_bus(BUS, "Bass Bus");
    add_track_plugin(&mut app, BASS, TRACK_COMP);
    add_bus_plugin(&mut app, BUS, BUS_COMP);
    add_master_plugin(&mut app, MASTER_COMP);
    app
}

/// Key `plugin` from `source` the way the live app gets there: the app
/// dispatches the route, the engine echoes it back resolved.
fn route(app: &mut Resonance, plugin: u64, source: SendSource) {
    app.test_apply_engine_event(AudioEvent::SidechainRouteChanged {
        plugin,
        source: Some(source),
        enabled: true,
    });
}

/// The real hop: write the project to a temp directory, read it back off
/// disk, and replay it into a brand-new app whose engine is a capturing
/// stub. Returns the reloaded app plus every command the replay emitted.
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

/// The single `SetSidechainRoute` the replay issued for `plugin`.
fn set_route_for(cmds: &[AudioCommand], plugin: u64) -> &AudioCommand {
    let matching: Vec<&AudioCommand> = cmds
        .iter()
        .filter(|c| matches!(c, AudioCommand::SetSidechainRoute { plugin: p, .. } if *p == plugin))
        .collect();
    assert_eq!(
        matching.len(),
        1,
        "expected exactly one SetSidechainRoute for plugin {plugin}, got {matching:?}"
    );
    matching[0]
}

// ---------------------------------------------------------------------------
// Serialized shape
// ---------------------------------------------------------------------------

#[test]
fn build_project_file_captures_a_track_plugins_key_route() {
    let mut app = app_with_chains();
    route(&mut app, TRACK_COMP, SendSource::Track(KICK));

    let file = app.test_build_project_file();
    assert_eq!(file.sidechain_routes.len(), 1, "the route must be written");
    let pr = &file.sidechain_routes[0];
    assert_eq!(pr.plugin_instance_id, TRACK_COMP);
    assert_eq!(pr.source_kind, "track");
    assert_eq!(pr.source_id, KICK);
    assert!(pr.enabled);
}

#[test]
fn a_project_with_no_routes_serializes_an_empty_list() {
    let app = app_with_chains();
    assert!(app.test_build_project_file().sidechain_routes.is_empty());
}

#[test]
fn routes_are_written_in_a_stable_order_regardless_of_echo_order() {
    // Route the master first, then the bus, then the track: the reverse
    // of the id order the file must come out in.
    let mut app = app_with_chains();
    route(&mut app, MASTER_COMP, SendSource::Bus(BUS));
    route(&mut app, BUS_COMP, SendSource::Track(KICK));
    route(&mut app, TRACK_COMP, SendSource::Track(KICK));

    let ids: Vec<u64> = app
        .test_build_project_file()
        .sidechain_routes
        .iter()
        .map(|r| r.plugin_instance_id)
        .collect();
    assert_eq!(ids, vec![TRACK_COMP, BUS_COMP, MASTER_COMP]);
}

// ---------------------------------------------------------------------------
// The round trip, through a real project directory
// ---------------------------------------------------------------------------

#[test]
fn a_track_plugins_key_route_survives_a_real_save_and_reload() {
    let mut app = app_with_chains();
    route(&mut app, TRACK_COMP, SendSource::Track(KICK));

    let (reloaded, cmds) = save_and_reload(&app.test_build_project_file());

    // The GUI mirror carries the route again…
    let routes = reloaded.test_sidechain_routes();
    assert_eq!(routes.len(), 1, "the reloaded project must have its route");
    assert_eq!(routes[0].plugin, TRACK_COMP);
    assert_eq!(routes[0].source, SendSource::Track(KICK));
    assert!(routes[0].enabled);

    // …and the engine was told, so the reloaded project makes the same
    // sound, not just the same picture.
    match set_route_for(&cmds, TRACK_COMP) {
        AudioCommand::SetSidechainRoute { source, enabled, .. } => {
            assert_eq!(*source, SendSource::Track(KICK));
            assert!(*enabled);
        }
        other => panic!("expected SetSidechainRoute, got {other:?}"),
    }
}

/// The finding's headline case: a compressor on a BUS, keyed from a
/// track. `track.set_sidechain` could not even express this, and nothing
/// persisted it.
#[test]
fn a_bus_keyed_route_survives_a_real_save_and_reload() {
    let mut app = app_with_chains();
    route(&mut app, BUS_COMP, SendSource::Track(KICK));

    let file = app.test_build_project_file();
    assert_eq!(file.sidechain_routes[0].plugin_instance_id, BUS_COMP);

    let (reloaded, cmds) = save_and_reload(&file);

    let routes = reloaded.test_sidechain_routes();
    assert_eq!(routes.len(), 1);
    assert_eq!(routes[0].plugin, BUS_COMP);
    assert_eq!(routes[0].source, SendSource::Track(KICK));

    // The target really is the plugin on the bus, not a same-named one
    // on a track.
    let bus = reloaded
        .test_registry()
        .busses
        .iter()
        .find(|b| b.id == BUS)
        .expect("the bus must reload");
    assert!(bus.plugins.iter().any(|p| p.instance_id == BUS_COMP));
    assert!(matches!(
        set_route_for(&cmds, BUS_COMP),
        AudioCommand::SetSidechainRoute { .. }
    ));
}

/// …and a plugin on the master chain, keyed from a bus — the other half
/// of "not just tracks", with a bus as the source for good measure.
#[test]
fn a_master_plugin_keyed_from_a_bus_survives_a_real_save_and_reload() {
    let mut app = app_with_chains();
    route(&mut app, MASTER_COMP, SendSource::Bus(BUS));

    let file = app.test_build_project_file();
    assert_eq!(file.sidechain_routes[0].source_kind, "bus");

    let (reloaded, _cmds) = save_and_reload(&file);

    let routes = reloaded.test_sidechain_routes();
    assert_eq!(routes.len(), 1);
    assert_eq!(routes[0].plugin, MASTER_COMP);
    assert_eq!(routes[0].source, SendSource::Bus(BUS));
    assert!(reloaded
        .test_registry()
        .busses
        .iter()
        .any(|b| b.id == BUS));
}

#[test]
fn three_routes_onto_three_different_hosts_all_come_back() {
    let mut app = app_with_chains();
    route(&mut app, TRACK_COMP, SendSource::Track(KICK));
    route(&mut app, BUS_COMP, SendSource::Track(KICK));
    route(&mut app, MASTER_COMP, SendSource::Bus(BUS));

    let (reloaded, cmds) = save_and_reload(&app.test_build_project_file());

    let mut got: Vec<(u64, SendSource)> = reloaded
        .test_sidechain_routes()
        .iter()
        .map(|r| (r.plugin, r.source))
        .collect();
    got.sort_by_key(|(plugin, _)| *plugin);
    assert_eq!(
        got,
        vec![
            (TRACK_COMP, SendSource::Track(KICK)),
            (BUS_COMP, SendSource::Track(KICK)),
            (MASTER_COMP, SendSource::Bus(BUS)),
        ]
    );
    assert_eq!(
        cmds.iter()
            .filter(|c| matches!(c, AudioCommand::SetSidechainRoute { .. }))
            .count(),
        3
    );
}

#[test]
fn a_disabled_route_reloads_disabled() {
    let mut app = app_with_chains();
    app.test_apply_engine_event(AudioEvent::SidechainRouteChanged {
        plugin: TRACK_COMP,
        source: Some(SendSource::Track(KICK)),
        enabled: false,
    });

    let (reloaded, cmds) = save_and_reload(&app.test_build_project_file());

    assert!(
        !reloaded.test_sidechain_routes()[0].enabled,
        "a route parked before the save stays parked"
    );
    match set_route_for(&cmds, TRACK_COMP) {
        AudioCommand::SetSidechainRoute { enabled, .. } => assert!(!*enabled),
        other => panic!("expected SetSidechainRoute, got {other:?}"),
    }
}

#[test]
fn clearing_a_route_removes_it_from_the_saved_project() {
    let mut app = app_with_chains();
    route(&mut app, TRACK_COMP, SendSource::Track(KICK));
    app.test_apply_engine_event(AudioEvent::SidechainRouteChanged {
        plugin: TRACK_COMP,
        source: None,
        enabled: false,
    });

    let file = app.test_build_project_file();
    assert!(
        file.sidechain_routes.is_empty(),
        "a cleared route must not be written back out"
    );

    let (reloaded, cmds) = save_and_reload(&file);
    assert!(reloaded.test_sidechain_routes().is_empty());
    assert!(!cmds
        .iter()
        .any(|c| matches!(c, AudioCommand::SetSidechainRoute { .. })));
}

#[test]
fn loading_a_project_drops_the_previous_projects_routes() {
    // `ClearAll` empties the engine's route table with no per-route echo,
    // so the mirror has to be wiped by the replay or the new project
    // inherits keys from the old one's tracks.
    let mut app = app_with_chains();
    route(&mut app, TRACK_COMP, SendSource::Track(KICK));
    assert_eq!(app.test_sidechain_routes().len(), 1);

    app.test_replay_loaded_project(ProjectFile::default());
    assert!(app.test_sidechain_routes().is_empty());
}

// ---------------------------------------------------------------------------
// A route is an edge: it must not outlive either end
// ---------------------------------------------------------------------------

#[test]
fn deleting_the_key_source_track_drops_the_route() {
    let mut app = app_with_chains();
    route(&mut app, TRACK_COMP, SendSource::Track(KICK));

    app.test_apply_engine_event(AudioEvent::TrackRemoved { track_id: KICK });

    assert!(
        app.test_sidechain_routes().is_empty(),
        "a key from a deleted track must not survive in the mirror"
    );
    assert!(
        app.test_build_project_file().sidechain_routes.is_empty(),
        "and must not be written to the project file"
    );
}

#[test]
fn deleting_the_key_source_bus_drops_the_route() {
    let mut app = app_with_chains();
    route(&mut app, MASTER_COMP, SendSource::Bus(BUS));

    app.test_apply_engine_event(AudioEvent::BusRemoved { bus_id: BUS });

    assert!(app.test_sidechain_routes().is_empty());
    assert!(app.test_build_project_file().sidechain_routes.is_empty());
}

#[test]
fn removing_the_keyed_plugin_drops_the_route() {
    let mut app = app_with_chains();
    route(&mut app, TRACK_COMP, SendSource::Track(KICK));

    app.test_apply_engine_event(AudioEvent::PluginRemoved {
        track_id: BASS,
        instance_id: TRACK_COMP,
    });

    assert!(app.test_sidechain_routes().is_empty());
    assert!(app.test_build_project_file().sidechain_routes.is_empty());
}

/// Taking the plugin off a BUS must also tell the engine, because its
/// `RemovePluginFromBus` arm — unlike `RemovePlugin` — does not drop the
/// route itself. Pruning only the mirror would leave the engine keying an
/// instance id the next `bus.add_effect` can be handed.
#[test]
fn removing_a_keyed_bus_plugin_tells_the_engine_to_drop_the_route_too() {
    let mut app = app_with_chains();
    route(&mut app, BUS_COMP, SendSource::Track(KICK));
    let rx = app.test_capture_engine();

    app.test_apply_engine_event(AudioEvent::BusPluginRemoved {
        bus_id: BUS,
        instance_id: BUS_COMP,
    });

    assert!(app.test_sidechain_routes().is_empty());
    let cmds = drain(&rx);
    assert!(
        cmds.iter().any(|c| matches!(
            c,
            AudioCommand::ClearSidechainRoute { plugin } if *plugin == BUS_COMP
        )),
        "expected ClearSidechainRoute for the orphaned route, got {cmds:?}"
    );
}

/// Same for the master chain, the other arm the engine does not prune.
#[test]
fn removing_a_keyed_master_plugin_tells_the_engine_to_drop_the_route_too() {
    let mut app = app_with_chains();
    route(&mut app, MASTER_COMP, SendSource::Bus(BUS));
    let rx = app.test_capture_engine();

    app.test_apply_engine_event(AudioEvent::MasterPluginRemoved {
        instance_id: MASTER_COMP,
    });

    assert!(app.test_sidechain_routes().is_empty());
    assert!(drain(&rx).iter().any(|c| matches!(
        c,
        AudioCommand::ClearSidechainRoute { plugin } if *plugin == MASTER_COMP
    )));
}

#[test]
fn a_route_whose_source_is_missing_from_the_file_is_not_mirrored_on_load() {
    let mut app = app_with_chains();
    route(&mut app, TRACK_COMP, SendSource::Track(KICK));
    let mut file = app.test_build_project_file();
    file.sidechain_routes[0].source_id = 999;

    let (reloaded, cmds) = save_and_reload(&file);

    assert!(
        reloaded.test_sidechain_routes().is_empty(),
        "a route with a missing source must not appear in the mirror"
    );
    assert!(!cmds
        .iter()
        .any(|c| matches!(c, AudioCommand::SetSidechainRoute { .. })));
}

#[test]
fn a_route_onto_a_plugin_missing_from_the_file_is_not_mirrored_on_load() {
    let mut app = app_with_chains();
    route(&mut app, TRACK_COMP, SendSource::Track(KICK));
    let mut file = app.test_build_project_file();
    file.sidechain_routes[0].plugin_instance_id = 987_654;

    let (reloaded, cmds) = save_and_reload(&file);

    assert!(reloaded.test_sidechain_routes().is_empty());
    assert!(!cmds
        .iter()
        .any(|c| matches!(c, AudioCommand::SetSidechainRoute { .. })));
}

/// An unrecognised `source_kind` drops the route rather than silently
/// re-pointing the detector at a different channel. Track ids and bus
/// ids are independent namespaces that both start at 1, so a guess would
/// usually land on something that exists.
#[test]
fn an_unknown_source_kind_drops_the_route_rather_than_guessing() {
    let mut app = app_with_chains();
    route(&mut app, TRACK_COMP, SendSource::Track(KICK));
    let mut file = app.test_build_project_file();
    file.sidechain_routes[0].source_kind = "Bus".to_string(); // wrong case

    let (reloaded, cmds) = save_and_reload(&file);

    assert!(reloaded.test_sidechain_routes().is_empty());
    assert!(!cmds
        .iter()
        .any(|c| matches!(c, AudioCommand::SetSidechainRoute { .. })));
}

// ---------------------------------------------------------------------------
// Back-compat
// ---------------------------------------------------------------------------

/// A project file in the shape older builds wrote: no `sidechain_routes`
/// key at all.
const LEGACY_PROJECT_JSON: &str = r#"{
  "version": 2,
  "sample_rate": 48000,
  "bpm": 120.0,
  "time_sig_num": 4,
  "time_sig_den": 4,
  "metronome_enabled": false,
  "master_volume": 0.0,
  "loop_enabled": false,
  "loop_in": 0,
  "loop_out": 0,
  "tracks": [],
  "clips": [],
  "busses": []
}"#;

#[test]
fn a_legacy_project_without_routes_still_loads() {
    let file: ProjectFile =
        serde_json::from_str(LEGACY_PROJECT_JSON).expect("an old project must still parse");
    assert!(file.sidechain_routes.is_empty(), "the field defaults empty");

    let (reloaded, cmds) = save_and_reload(&file);
    assert!(reloaded.test_sidechain_routes().is_empty());
    assert!(
        !cmds
            .iter()
            .any(|c| matches!(c, AudioCommand::SetSidechainRoute { .. })),
        "an old project must not invent key routes"
    );
}

/// A hand-written entry that omits `enabled` is a live route — writing a
/// route down means wanting it.
#[test]
fn an_entry_without_enabled_loads_as_a_live_route() {
    let pr: resonance_app::project::ProjectSidechainRoute =
        serde_json::from_str(r#"{"plugin_instance_id": 100, "source_id": 1}"#)
            .expect("minimal entry parses");
    assert!(pr.enabled);
    assert_eq!(pr.source_kind, "track");
}

// ---------------------------------------------------------------------------
// Undo / redo: the diff replay reconciles routes surgically
// ---------------------------------------------------------------------------

#[test]
fn diff_replay_restores_a_route_removed_after_the_snapshot() {
    let mut app = app_with_chains();
    route(&mut app, BUS_COMP, SendSource::Track(KICK));
    app.test_set_project_path(std::path::PathBuf::from("/tmp/resonance-key-undo.rproj"));
    let snapshot = app.test_snapshot_for_undo();

    // The user unkeys the bus compressor.
    app.test_apply_engine_event(AudioEvent::SidechainRouteChanged {
        plugin: BUS_COMP,
        source: None,
        enabled: false,
    });
    assert!(app.test_sidechain_routes().is_empty());

    let rx = app.test_capture_engine();
    app.test_begin_restore_from_snapshot(snapshot);

    assert_eq!(
        app.test_sidechain_routes().len(),
        1,
        "undo must put the key back"
    );
    assert_eq!(app.test_sidechain_routes()[0].source, SendSource::Track(KICK));
    assert!(drain(&rx).iter().any(|c| matches!(
        c,
        AudioCommand::SetSidechainRoute { plugin, .. } if *plugin == BUS_COMP
    )));
}

#[test]
fn diff_replay_drops_a_route_added_after_the_snapshot() {
    let mut app = app_with_chains();
    app.test_set_project_path(std::path::PathBuf::from("/tmp/resonance-key-undo.rproj"));
    let snapshot = app.test_snapshot_for_undo();

    route(&mut app, TRACK_COMP, SendSource::Track(KICK));
    assert_eq!(app.test_sidechain_routes().len(), 1);

    let rx = app.test_capture_engine();
    app.test_begin_restore_from_snapshot(snapshot);

    assert!(
        app.test_sidechain_routes().is_empty(),
        "undoing past the route must unkey the plugin"
    );
    assert!(
        drain(&rx).iter().any(|c| matches!(
            c,
            AudioCommand::ClearSidechainRoute { plugin } if *plugin == TRACK_COMP
        )),
        "the engine must be told to drop the route too"
    );
}

#[test]
fn diff_replay_repoints_a_route_whose_source_changed() {
    let mut app = app_with_chains();
    route(&mut app, TRACK_COMP, SendSource::Track(KICK));
    app.test_set_project_path(std::path::PathBuf::from("/tmp/resonance-key-undo.rproj"));
    let snapshot = app.test_snapshot_for_undo();

    route(&mut app, TRACK_COMP, SendSource::Bus(BUS));
    assert_eq!(app.test_sidechain_routes()[0].source, SendSource::Bus(BUS));

    let rx = app.test_capture_engine();
    app.test_begin_restore_from_snapshot(snapshot);

    assert_eq!(
        app.test_sidechain_routes()[0].source,
        SendSource::Track(KICK),
        "undo must put the key source back"
    );
    assert!(drain(&rx).iter().any(|c| matches!(
        c,
        AudioCommand::SetSidechainRoute { plugin, source, .. }
            if *plugin == TRACK_COMP && *source == SendSource::Track(KICK)
    )));
}
