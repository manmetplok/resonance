//! One domain order for every restore (ARCH-01 A-13).
//!
//! The project domains are restored by `Reconcile` impls that the driver
//! runs from one table, `reconcile::DOMAINS`, on both origins: a disk load
//! (`replay_loaded_project`, after `ClearAll`) and an undo/redo (in place,
//! by diff). Both run the whole table through `reconcile_all` (one entry
//! point since A-13j); the guard is
//! that the domains each restore actually ran (`io.reconcile_trace`) are
//! exactly the table, in table order, under the right origin. A domain
//! restored inline by one path, or run out of sequence, fails here; the
//! command-order tests below pin where each moved piece now goes out.

use std::path::PathBuf;

use resonance_app::message::{ExternalInstrumentMessage as Eim, Message, ProjectIoMessage};
use resonance_app::project::{LoadedProject, ProjectFile, ProjectPluginParam};
use resonance_app::compose::{GenerateParams, SectionDefinitionState};
use resonance_app::state::{ClipState, MidiClipState, TempoEvent, TrackState};
use resonance_app::update::project_io::reconcile::{domain_order, Origin, Stage};
use resonance_app::update::project_io::replay_loaded_project;
use resonance_app::Resonance;
use resonance_audio::test_support::Receiver;
use resonance_audio::types::{ChainOwner, 
    AudioCommand, AudioEvent, FadeCurve, ParamInfo, SendSource, TrackType,
};
use resonance_common::{AutomationLane, AutomationTarget, Breakpoint, CurveKind};
use resonance_music_theory::MotifSource;

fn drain(rx: &Receiver<AudioCommand>) -> Vec<AudioCommand> {
    let mut cmds = Vec::new();
    while let Ok(cmd) = rx.try_recv() {
        cmds.push(cmd);
    }
    cmds
}

fn expected(origin: Origin) -> Vec<(Origin, &'static str)> {
    domain_order().into_iter().map(|(_, name)| (origin, name)).collect()
}

struct Loaded {
    app: Resonance,
    rx: Receiver<AudioCommand>,
    /// Commands the disk load's `AllCleared` replay sent.
    load_cmds: Vec<AudioCommand>,
    _tmp: tempfile::TempDir,
}

fn disk_load() -> Loaded {
    let tmp = tempfile::tempdir().unwrap();
    let (mut app, _task) = Resonance::new_for_test();
    let rx = app.test_capture_engine();
    let project = tmp.path().join("project.rproj");
    std::fs::create_dir_all(&project).unwrap();
    app.test_set_project_path(project.clone());
    let loaded = LoadedProject {
        file: ProjectFile::default(),
        project_dir: project,
        midi_notes: Default::default(),
        plugin_states: Default::default(),
    };
    let _ = app.update(Message::ProjectIo(ProjectIoMessage::ProjectLoaded(Ok(
        Box::new(loaded),
    ))));
    let _ = drain(&rx);
    app.test_apply_engine_event(AudioEvent::AllCleared);
    let load_cmds = drain(&rx);
    Loaded {
        app,
        rx,
        load_cmds,
        _tmp: tmp,
    }
}

#[test]
fn the_table_is_sorted_by_stage() {
    let order = domain_order();
    assert!(!order.is_empty());
    assert!(
        order.windows(2).all(|w| w[0].0 <= w[1].0),
        "each path calls the stages in sequence, so the table must be sorted \
         by stage for both to run it in table order: {order:?}"
    );
    let mut names: Vec<_> = order.iter().map(|(_, n)| *n).collect();
    names.sort_unstable();
    names.dedup();
    assert_eq!(names.len(), order.len(), "a domain is listed twice");
}

#[test]
fn a_disk_load_runs_every_domain_in_table_order() {
    let l = disk_load();
    assert_eq!(l.app.test_reconcile_trace(), expected(Origin::DiskLoad).as_slice());
}

#[test]
fn a_diff_undo_runs_every_domain_in_table_order() {
    let mut l = disk_load();
    let same = l.app.test_snapshot_for_undo();
    l.app.test_begin_restore_from_snapshot(same);
    assert!(
        !drain(&l.rx).iter().any(|c| matches!(c, AudioCommand::ClearAll)),
        "an unchanged shape takes the diff path"
    );
    assert_eq!(l.app.test_reconcile_trace(), expected(Origin::Undo).as_slice());
}

/// A structural undo (a track the snapshot lacks) used to take the full
/// `ClearAll` replay under `Origin::UndoFull`; since A-13j there is no such
/// path, and it runs the same table under `Origin::Undo`, in place.
#[test]
fn a_structural_undo_runs_every_domain_in_table_order_without_a_clear() {
    let mut l = disk_load();
    let snapshot = l.app.test_snapshot_for_undo();
    l.app.test_add_track(9_999, TrackType::Audio);
    let _ = drain(&l.rx);
    l.app.test_begin_restore_from_snapshot(snapshot);
    assert!(
        !drain(&l.rx).iter().any(|c| matches!(c, AudioCommand::ClearAll)),
        "no undo sends ClearAll"
    );
    assert_eq!(l.app.test_reconcile_trace(), expected(Origin::Undo).as_slice());
}

/// `Globals` runs before `Timeline` on a disk load: every transport
/// scalar goes out before `SetTempoEvents` (A-13c; `SetTimeSignature` used
/// to follow it — the two write independent fields of the engine's tempo
/// map, see `globals::Transport`). `SetBpm` still precedes the events, whose
/// first point must win the map's `bpm`.
#[test]
fn the_full_path_sends_the_transport_scalars_then_tempo_events() {
    let l = disk_load();
    let pos = |pred: fn(&AudioCommand) -> bool| {
        l.load_cmds
            .iter()
            .position(pred)
            .expect("the replay sends every transport command")
    };
    let bpm = pos(|c| matches!(c, AudioCommand::SetBpm { .. }));
    let meter = pos(|c| matches!(c, AudioCommand::SetTimeSignature { .. }));
    let lp = pos(|c| matches!(c, AudioCommand::SetLoopRange { .. }));
    let events = pos(|c| matches!(c, AudioCommand::SetTempoEvents { .. }));
    assert!(
        bpm < meter && meter < lp && lp < events,
        "{bpm} < {meter} < {lp} < {events}"
    );
}

/// The diff path's tempo converged on the full path's position (A-13c):
/// `SetTempoEvents` goes out before any clip command, not after the clips
/// as it did through A-13b. And only the scalars that changed are sent.
#[test]
fn a_diff_undo_sends_tempo_before_the_clips_and_only_changed_scalars() {
    let mut l = disk_load();
    l.app.test_push_track(TrackState::new_instrument(1, 0));
    l.app.test_push_midi_clip(MidiClipState {
        id: 10,
        track_id: 1,
        start_sample: 0,
        duration_ticks: 3840,
        name: "clip".to_string(),
        notes: Vec::new().into(),
        trim_start_ticks: 0,
        trim_end_ticks: 0,
    });
    let mut target = l.app.test_snapshot_for_undo();
    target.project.file.midi_clips[0].start_sample = 96_000;
    target.project.file.bpm = 90.0;
    target.project.file.tempo_events = vec![TempoEvent { bar: 0, bpm: 90.0 }];
    let _ = drain(&l.rx);
    l.app.test_begin_restore_from_snapshot(target);
    let cmds = drain(&l.rx);
    assert!(
        !cmds.iter().any(|c| matches!(c, AudioCommand::ClearAll)),
        "an unchanged shape takes the diff path"
    );
    let pos = |pred: fn(&AudioCommand) -> bool| {
        cmds.iter().position(pred).expect("the diff replay sends it")
    };
    let bpm = pos(|c| matches!(c, AudioCommand::SetBpm { bpm } if *bpm == 90.0));
    let events = pos(|c| matches!(c, AudioCommand::SetTempoEvents { .. }));
    let moved = pos(|c| {
        matches!(c, AudioCommand::MoveMidiClip { clip_id: 10, new_start_sample: 96_000, .. })
    });
    assert!(bpm < events && events < moved, "{bpm} < {events} < {moved}");
    assert!(
        !cmds.iter().any(|c| matches!(
            c,
            AudioCommand::SetTimeSignature { .. }
                | AudioCommand::SetLoopRange { .. }
                | AudioCommand::SetMasterVolume { .. }
        )),
        "unchanged scalars are not re-sent on the diff path: {cmds:?}"
    );
    assert_eq!(l.app.test_transport_bpm(), 90.0);
}

/// The table itself, pinned: a domain added, dropped or moved is a
/// decision, recorded in `docs/design/A-13-reconcile.md`. `Globals` feeds
/// `Timeline` (the tempo map is rebuilt from the transport scalars) and
/// the compose load precedes `Clips` (it resets the derived map). A diff
/// restore's `Removals` (edges, then plugin instances and busses) precede
/// every add (A-13h). `Routing` (sends, then key routes) follows every
/// entity it connects. Within
/// `Tail`, external instruments come before the lanes (a `DeviceParam` lane needs
/// the device bindings) and freeze is last (a disk load's baseline
/// fingerprints the lanes); derived clips follow the clips. The clip-id
/// grant closes the replay (ARCH-04 D-7d): every domain above has seeded
/// the counter it draws from.
#[test]
fn the_table_is_the_agreed_order() {
    assert_eq!(
        domain_order(),
        vec![
            (Stage::Globals, "transport"),
            (Stage::Globals, "transient_ui"),
            (Stage::Globals, "compose_sections"),
            (Stage::Globals, "drum_patterns"),
            (Stage::Timeline, "tempo_events"),
            (Stage::Timeline, "chord_track"),
            (Stage::Timeline, "markers"),
            (Stage::Timeline, "section_chord_trim"),
            (Stage::Removals, "routing_removals"),
            (Stage::Removals, "clip_removals"),
            (Stage::Removals, "entity_removals"),
            (Stage::Entities, "tracks"),
            (Stage::Entities, "busses"),
            (Stage::Entities, "master"),
            (Stage::Entities, "track_outputs"),
            (Stage::Entities, "plugin_state"),
            (Stage::Entities, "entity_order"),
            (Stage::Routing, "sends"),
            (Stage::Routing, "sidechain_routes"),
            (Stage::Clips, "audio_clips"),
            (Stage::Clips, "midi_clips"),
            (Stage::Clips, "clip_lyrics"),
            (Stage::Clips, "derived_clips"),
            (Stage::Clips, "vocal_audio_clips"),
            (Stage::Content, "references"),
            (Stage::Content, "pool"),
            (Stage::Content, "quantize"),
            (Stage::Content, "performance"),
            (Stage::Content, "track_groups"),
            (Stage::Content, "take_groups"),
            (Stage::Tail, "external_instruments"),
            (Stage::Tail, "automation_lanes"),
            (Stage::Tail, "missing_plugins"),
            (Stage::Tail, "freeze"),
            (Stage::Tail, "clip_id_grant"),
        ]
    );
}

/// External instruments left `replay_track` (A-13b): the full path now
/// sends every external track's config once all tracks are registered,
/// and its device bindings before any automation lane that targets them.
#[test]
fn the_full_path_sends_external_config_after_the_tracks_and_before_the_lanes() {
    const EXT: u64 = 1;
    let dir = PathBuf::from("/tmp/resonance-test-a13b");
    let (mut app, _task) = Resonance::new_for_test();
    app.test_set_active_project(true);
    app.test_set_project_path(dir.clone());
    app.test_push_track(TrackState::new_instrument(EXT, 0));
    app.test_push_track(TrackState::new_audio(2, 1));
    app.test_dispatch(Message::ExternalInstrument(Eim::Enable(EXT)));
    // The bundled Moog Muse preset always ships in the registry.
    app.test_dispatch(Message::ExternalInstrument(Eim::SetDevice(
        EXT,
        Some("moog-muse".to_string()),
    )));
    let lane = AutomationLane::new(
        7,
        AutomationTarget::DeviceParam {
            track: EXT,
            param_id: "glide-time".to_string(),
        },
        vec![Breakpoint::new(0, 0.1, CurveKind::Linear)],
    );
    app.test_apply_engine_event(AudioEvent::AutomationLaneChanged { lane });
    let file = app.test_build_project_file();

    let (mut fresh, _task) = Resonance::new_for_test();
    let rx = fresh.test_capture_engine();
    replay_loaded_project(
        &mut fresh,
        Box::new(LoadedProject {
            file,
            project_dir: dir,
            midi_notes: Default::default(),
            plugin_states: Default::default(),
        }),
    );
    let cmds = drain(&rx);
    let last_track = cmds
        .iter()
        .rposition(|c| {
            matches!(
                c,
                AudioCommand::AddTrack { .. } | AudioCommand::AddInstrumentTrack { .. }
            )
        })
        .expect("the replay registers the tracks");
    let pos = |pred: &dyn Fn(&AudioCommand) -> bool| {
        cmds.iter().position(pred).expect("the replay sends it")
    };
    let config = pos(&|c| {
        matches!(c, AudioCommand::SetExternalInstrument { config } if config.track_id == EXT)
    });
    let bindings = pos(&|c| {
        matches!(c, AudioCommand::SetTrackDeviceParams { track_id, params }
            if *track_id == EXT && !params.is_empty())
    });
    let lane = pos(&|c| matches!(c, AudioCommand::SetAutomationLane { .. }));
    assert!(
        last_track < config && config < bindings && bindings < lane,
        "{last_track} < {config} < {bindings} < {lane}"
    );
}

/// The vocal audio-clip map is the last `Clips` domain on both paths
/// (A-13d). The diff path used to rebuild it inline after the `Tail` stage
/// and the registry resort; it now runs right after `DerivedClips`, as on
/// the full path (the trace tests pin the position). Nothing in `Content`
/// or `Tail` reads the map or the derived counter it reserves. Here: the
/// live clip sits off the section start, so no lane claims it; the target
/// puts it back on bar 0 and the diff undo must key it to the lane.
#[test]
fn a_diff_undo_rebuilds_the_vocal_audio_clip_map_from_the_target() {
    const VOCAL: u64 = 1;
    const CLIP: u64 = 10;
    const DEF: u64 = 5;
    let mut l = disk_load();
    l.app.test_push_track(TrackState::new_vocal(VOCAL, 0));
    l.app.test_push_section_definition(SectionDefinitionState {
        id: DEF,
        name: "Verse".to_string(),
        color: [0, 0, 0],
        length_bars: 4,
        chords: Vec::new(),
        scale: None,
        progression_seed: 0,
        generate_params: GenerateParams::default(),
        generator_spec: None,
        generator_seed: 0,
        generated_material: None,
        lane_generators: std::collections::HashMap::new(),
        beats_per_chord: 4,
        seventh_chords: false,
        motif_source: MotifSource::default(),
        arrangement: Vec::new(),
    });
    l.app.test_place_section(DEF, 0);
    l.app.test_push_clip(ClipState {
        id: CLIP,
        track_id: VOCAL,
        start_sample: 12_345,
        duration_samples: 48_000,
        name: "vocal".to_string(),
        total_frames: 48_000,
        trim_start_frames: 0,
        trim_end_frames: 0,
        fade_in_frames: 0,
        fade_in_curve: FadeCurve::default(),
        fade_out_frames: 0,
        fade_out_curve: FadeCurve::default(),
        gain_db: 0.0,
        waveform_peaks: Vec::new(),
        vocal_tuning: None,
        asset_ref: None,
        warp: Default::default(),
    });
    assert!(l.app.test_vocal_audio_clips(VOCAL).is_empty());
    let mut target = l.app.test_snapshot_for_undo();
    target.project.file.clips[0].start_sample = 0;
    let _ = drain(&l.rx);
    l.app.test_begin_restore_from_snapshot(target);
    let cmds = drain(&l.rx);
    assert!(
        !cmds.iter().any(|c| matches!(c, AudioCommand::ClearAll)),
        "an unchanged shape takes the diff path"
    );
    assert!(cmds.iter().any(|c| matches!(
        c,
        AudioCommand::MoveClip { clip_id: CLIP, new_start_sample: 0, .. }
    )));
    assert_eq!(l.app.test_vocal_audio_clips(VOCAL), vec![(DEF, CLIP)]);
}

// ---------------------------------------------------------------------------
// Routing (A-13e): sends and key routes after every entity they connect
// ---------------------------------------------------------------------------

const KICK: u64 = 1;
const BUS: u64 = 10;
const SEND: u64 = 3;
const MASTER_COMP: u64 = 300;
const THRESHOLD: u32 = 1;

fn threshold_param() -> ParamInfo {
    ParamInfo {
        id: THRESHOLD,
        name: "Threshold".to_string(),
        min_value: 0.0,
        max_value: 1.0,
        default_value: 0.0,
        current_value: 0.0,
        ..Default::default()
    }
}

fn threshold_override(value: f64) -> Vec<ProjectPluginParam> {
    vec![ProjectPluginParam {
        id: THRESHOLD,
        name: "Threshold".to_string(),
        value,
    }]
}

/// A kick track sending into a bus, a compressor on the master keyed from
/// the kick, and a MIDI clip on the kick.
fn app_with_routing() -> Resonance {
    let (mut app, _task) = Resonance::new_for_test();
    app.test_set_active_project(true);
    app.test_set_project_path(PathBuf::from("/tmp/resonance-test-a13e.rproj"));
    app.test_push_track(TrackState::new_instrument(KICK, 0));
    app.test_add_bus(BUS, "Bus");
    app.test_apply_engine_event(AudioEvent::PluginAdded {
        owner: ChainOwner::Master,
        instance_id: MASTER_COMP,
        plugin_name: "Compressor".to_string(),
        clap_plugin_id: "com.resonance.compressor".to_string(),
        clap_file_path: "/plugins/compressor.clap".to_string(),
        params: vec![threshold_param()],
        has_gui: false,
        has_sidechain_input: true,
        output_port_count: 1,
        output_port_names: Vec::new(),
    });
    app.test_apply_engine_event(AudioEvent::AuxSendChanged {
        send_id: SEND,
        source: SendSource::Track(KICK),
        dest: BUS,
        level_db: -6.0,
        pre_fader: false,
        enabled: true,
    });
    app.test_apply_engine_event(AudioEvent::SidechainRouteChanged {
        plugin: MASTER_COMP,
        source: Some(SendSource::Track(KICK)),
        enabled: true,
    });
    app.test_push_midi_clip(MidiClipState {
        id: 20,
        track_id: KICK,
        start_sample: 0,
        duration_ticks: 3840,
        name: "clip".to_string(),
        notes: Vec::new().into(),
        trim_start_ticks: 0,
        trim_end_ticks: 0,
    });
    app
}

/// Full path: the send and the key route go out after the master chain
/// (the route names a master plugin's instance id) and before the clips.
/// `Stage::Routing` sits where `replay_sends` / `replay_sidechain_routes`
/// ran, at the end of the entity replay.
#[test]
fn the_full_path_sends_routing_after_the_master_chain_and_before_the_clips() {
    let file = app_with_routing().test_build_project_file();
    let (mut fresh, _task) = Resonance::new_for_test();
    let rx = fresh.test_capture_engine();
    replay_loaded_project(
        &mut fresh,
        Box::new(LoadedProject {
            file,
            project_dir: PathBuf::from("/tmp/resonance-test-a13e"),
            midi_notes: Default::default(),
            plugin_states: Default::default(),
        }),
    );
    let cmds = drain(&rx);
    let pos = |pred: &dyn Fn(&AudioCommand) -> bool| {
        cmds.iter().position(pred).expect("the replay sends it")
    };
    let master = pos(&|c| matches!(c, AudioCommand::AddPlugin { owner: ChainOwner::Master, id: MASTER_COMP, .. }));
    let send = pos(&|c| matches!(c, AudioCommand::AddAuxSend { id: SEND, dest: BUS, .. }));
    let route = pos(&|c| matches!(c, AudioCommand::SetSidechainRoute { plugin: MASTER_COMP, .. }));
    let clip = pos(&|c| matches!(c, AudioCommand::LoadMidiClipDirect { .. }));
    assert!(
        master < send && send < route && route < clip,
        "{master} < {send} < {route} < {clip}"
    );
    assert_eq!(fresh.test_aux_sends().len(), 1);
    assert_eq!(fresh.test_sidechain_routes().len(), 1);
}

/// Diff path: the routing edges now go out after `apply_master` (A-13e;
/// they used to precede it), as on the full path, and before the clips.
/// The engine's send and key-route tables are independent of the master
/// and plugin bypass flags (design doc §10). Only the changed edges are
/// re-sent, and a send the engine already holds as a `SetAuxSend`.
#[test]
fn a_diff_undo_sends_routing_after_the_master_and_before_the_clips() {
    let mut app = app_with_routing();
    let rx = app.test_capture_engine();
    let mut target = app.test_snapshot_for_undo();
    target.project.file.master_fx_bypassed = true;
    target.project.file.master_plugins[0].bypassed = true;
    target.project.file.sends[0].level_db = -12.0;
    target.project.file.sidechain_routes[0].enabled = false;
    target.project.file.midi_clips[0].start_sample = 96_000;
    let _ = drain(&rx);
    app.test_begin_restore_from_snapshot(target);
    let cmds = drain(&rx);
    assert!(
        !cmds.iter().any(|c| matches!(c, AudioCommand::ClearAll)),
        "an unchanged shape takes the diff path"
    );
    let pos = |pred: &dyn Fn(&AudioCommand) -> bool| {
        cmds.iter().position(pred).expect("the diff replay sends it")
    };
    let master = pos(&|c| matches!(c, AudioCommand::SetFxBypass { owner: ChainOwner::Master, bypassed: true }));
    let bypass = pos(&|c| {
        matches!(c, AudioCommand::SetPluginBypass { instance_id: MASTER_COMP, bypassed: true })
    });
    let send = pos(&|c| {
        matches!(c, AudioCommand::SetAuxSend { id: SEND, level_db, .. } if *level_db == -12.0)
    });
    let route = pos(&|c| {
        matches!(c, AudioCommand::SetSidechainRoute { plugin: MASTER_COMP, enabled: false, .. })
    });
    let clip = pos(&|c| matches!(c, AudioCommand::MoveMidiClip { clip_id: 20, .. }));
    assert!(
        master < bypass && bypass < send && send < route && route < clip,
        "{master} < {bypass} < {send} < {route} < {clip}"
    );
    assert!(
        !cmds.iter().any(|c| matches!(c, AudioCommand::AddAuxSend { .. })),
        "a send the engine already holds is edited, not re-added: {cmds:?}"
    );
    assert_eq!(app.test_aux_sends()[0].level_db, -12.0);
    assert!(!app.test_sidechain_routes()[0].enabled);
}

/// Full path (A-13f): every entity is added before any plugin's state is
/// restored. `LoadPluginState` used to follow its own `AddPlugin*` inside
/// the chain replay; it now goes out after every track, bus, the master
/// chain and the track outputs (`PluginState`, last in `Entities`), still
/// before `Routing`. Per instance the blob still precedes the bypass, and
/// the saved param overrides are parked for the `PluginAdded` echo, which
/// applies them after the blob.
#[test]
fn the_full_path_restores_plugin_state_after_every_entity_and_before_routing() {
    let mut file = app_with_routing().test_build_project_file();
    file.tracks[0].output_bus = Some(BUS);
    file.master_plugins[0].bypassed = true;
    file.master_plugins[0].params = threshold_override(0.5);
    let (mut fresh, _task) = Resonance::new_for_test();
    let rx = fresh.test_capture_engine();
    let blob: std::sync::Arc<[u8]> = std::sync::Arc::from(vec![1u8, 2, 3]);
    replay_loaded_project(
        &mut fresh,
        Box::new(LoadedProject {
            file,
            project_dir: PathBuf::from("/tmp/resonance-test-a13f"),
            midi_notes: Default::default(),
            plugin_states: [(MASTER_COMP, blob)].into_iter().collect(),
        }),
    );
    let cmds = drain(&rx);
    let pos = |pred: &dyn Fn(&AudioCommand) -> bool| {
        cmds.iter().position(pred).expect("the replay sends it")
    };
    let master = pos(&|c| matches!(c, AudioCommand::AddPlugin { owner: ChainOwner::Master, id: MASTER_COMP, .. }));
    let output = pos(&|c| matches!(c, AudioCommand::SetTrackOutput { track_id: KICK, .. }));
    let state = pos(&|c| {
        matches!(c, AudioCommand::LoadPluginState { instance_id: MASTER_COMP, data } if data[..] == [1, 2, 3])
    });
    let bypass = pos(&|c| {
        matches!(c, AudioCommand::SetPluginBypass { instance_id: MASTER_COMP, bypassed: true })
    });
    let send = pos(&|c| matches!(c, AudioCommand::AddAuxSend { id: SEND, .. }));
    assert!(
        master < output && output < state && state < bypass && bypass < send,
        "{master} < {output} < {state} < {bypass} < {send}"
    );
    assert!(
        !cmds.iter().any(|c| matches!(c, AudioCommand::SetPluginParam { .. })),
        "the overrides wait for the PluginAdded echo: {cmds:?}"
    );
    fresh.test_apply_engine_event(AudioEvent::PluginAdded {
        owner: ChainOwner::Master,
        instance_id: MASTER_COMP,
        plugin_name: "Compressor".to_string(),
        clap_plugin_id: "com.resonance.compressor".to_string(),
        clap_file_path: "/plugins/compressor.clap".to_string(),
        params: vec![threshold_param()],
        has_gui: false,
        has_sidechain_input: true,
        output_port_count: 1,
        output_port_names: Vec::new(),
    });
    assert!(
        drain(&rx).iter().any(|c| matches!(
            c,
            AudioCommand::SetPluginParam { instance_id: MASTER_COMP, param_id: THRESHOLD, value }
                if *value == 0.5
        )),
        "the parked override is applied on the echo"
    );
}

/// Diff path (A-13f): the plugin blobs and params go out before `Routing`
/// (they used to follow it), and the per-slot bypass after the blob (it
/// used to precede it, with the master scalars). The engine's send and
/// key-route tables touch no plugin state (design doc §11).
#[test]
fn a_diff_undo_restores_plugin_state_before_routing() {
    let mut app = app_with_routing();
    let rx = app.test_capture_engine();
    let mut target = app.test_snapshot_for_undo();
    target.project.file.master_fx_bypassed = true;
    target.project.file.master_plugins[0].bypassed = true;
    target.project.file.master_plugins[0].params = threshold_override(0.5);
    // A blob the live cache does not hold, so it is re-pushed.
    target
        .project
        .plugin_states
        .insert(MASTER_COMP, std::sync::Arc::from(vec![7u8, 7, 7]));
    target.project.file.sends[0].level_db = -12.0;
    let _ = drain(&rx);
    app.test_begin_restore_from_snapshot(target);
    let cmds = drain(&rx);
    assert!(
        !cmds.iter().any(|c| matches!(c, AudioCommand::ClearAll)),
        "an unchanged shape takes the diff path"
    );
    let pos = |pred: &dyn Fn(&AudioCommand) -> bool| {
        cmds.iter().position(pred).expect("the diff replay sends it")
    };
    let master = pos(&|c| matches!(c, AudioCommand::SetFxBypass { owner: ChainOwner::Master, bypassed: true }));
    let state = pos(&|c| matches!(c, AudioCommand::LoadPluginState { instance_id: MASTER_COMP, .. }));
    let bypass = pos(&|c| {
        matches!(c, AudioCommand::SetPluginBypass { instance_id: MASTER_COMP, bypassed: true })
    });
    let param = pos(&|c| {
        matches!(
            c,
            AudioCommand::SetPluginParam { instance_id: MASTER_COMP, param_id: THRESHOLD, value }
                if *value == 0.5
        )
    });
    let send = pos(&|c| matches!(c, AudioCommand::SetAuxSend { id: SEND, .. }));
    assert!(
        master < state && state < bypass && bypass < param && param < send,
        "{master} < {state} < {bypass} < {param} < {send}"
    );
}

/// Diff path (A-13f): the registry resort, the output-picker rebuild and
/// the lane count moved from after `Tail` to the end of `Entities`
/// (`EntityOrder`; the trace tests pin the position). An undo that swaps
/// two tracks' `.order` and renames a bus leaves the registry sorted.
#[test]
fn a_diff_undo_resorts_the_registry() {
    let mut app = app_with_routing();
    app.test_push_track(TrackState::new_instrument(KICK + 1, 1));
    let mut target = app.test_snapshot_for_undo();
    for pt in &mut target.project.file.tracks {
        pt.order = if pt.id == KICK { 1 } else { 0 };
    }
    target.project.file.busses[0].name = "Drums".to_string();
    app.test_begin_restore_from_snapshot(target);
    let ids: Vec<_> = app.test_registry().tracks.iter().map(|t| t.id).collect();
    assert_eq!(ids, vec![KICK + 1, KICK], "sorted by the restored .order");
    assert_eq!(app.test_registry().busses[0].name, "Drums");
}

/// Diff path (A-13h): the routing removals left `Sends` /
/// `SidechainRoutes` for `Stage::Removals`, ahead of every entity command,
/// so an edge is gone before any endpoint it names can be. They used to go
/// out after the track, bus and master scalars and the plugin state. The
/// engine's send and key-route tables are independent of those (design
/// doc §10, §11), so only the position moves.
#[test]
fn a_diff_undo_removes_edges_before_any_entity_command() {
    let mut app = app_with_routing();
    let rx = app.test_capture_engine();
    let mut target = app.test_snapshot_for_undo();
    target.project.file.sends.clear();
    target.project.file.sidechain_routes.clear();
    target.project.file.tracks[0].volume = -3.0;
    target.project.file.master_fx_bypassed = true;
    let _ = drain(&rx);
    app.test_begin_restore_from_snapshot(target);
    let cmds = drain(&rx);
    assert!(
        !cmds.iter().any(|c| matches!(c, AudioCommand::ClearAll)),
        "an unchanged shape takes the diff path"
    );
    let pos = |pred: &dyn Fn(&AudioCommand) -> bool| {
        cmds.iter().position(pred).expect("the diff replay sends it")
    };
    let send = pos(&|c| matches!(c, AudioCommand::RemoveAuxSend { send_id: SEND }));
    let route = pos(&|c| matches!(c, AudioCommand::ClearSidechainRoute { plugin: MASTER_COMP }));
    let volume = pos(&|c| matches!(c, AudioCommand::SetTrackVolume { track_id: KICK, .. }));
    let master = pos(&|c| matches!(c, AudioCommand::SetFxBypass { owner: ChainOwner::Master, bypassed: true }));
    assert!(
        send < route && route < volume && volume < master,
        "{send} < {route} < {volume} < {master}"
    );
    assert!(app.test_aux_sends().is_empty());
    assert!(app.test_sidechain_routes().is_empty());
}
