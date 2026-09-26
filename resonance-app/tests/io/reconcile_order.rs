//! One domain order for every restore (ARCH-01 A-13).
//!
//! The migrated project domains are restored by `Reconcile` impls that the
//! driver runs from one table, `reconcile::DOMAINS`, on all three origins:
//! a disk load and an undo's full replay (`replay_loaded_project`, after
//! `ClearAll`) and an undo's diff replay (`try_diff_replay`). Each path
//! calls the table's stages at different points in its not-yet-migrated
//! inline code, so the guard is that the domains each restore actually ran
//! (`io.reconcile_trace`) are exactly the table, in table order, under the
//! right origin. A domain restored inline by one path, or a stage called
//! out of sequence, fails here.

use std::path::PathBuf;

use resonance_app::message::{ExternalInstrumentMessage as Eim, Message, ProjectIoMessage};
use resonance_app::project::{LoadedProject, ProjectFile};
use resonance_app::state::{MidiClipState, TempoEvent, TrackState};
use resonance_app::update::project_io::reconcile::{domain_order, Origin, Stage};
use resonance_app::update::project_io::replay_loaded_project;
use resonance_app::Resonance;
use resonance_audio::test_support::Receiver;
use resonance_audio::types::{AudioCommand, AudioEvent, TrackType};
use resonance_common::{AutomationLane, AutomationTarget, Breakpoint, CurveKind};

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
    assert_eq!(l.app.test_reconcile_trace(), expected(Origin::UndoDiff).as_slice());
}

#[test]
fn a_full_undo_runs_every_domain_in_table_order() {
    let mut l = disk_load();
    let snapshot = l.app.test_snapshot_for_undo();
    // An extra track forces the structural fallback.
    l.app.test_add_track(9_999, TrackType::Audio);
    let _ = drain(&l.rx);
    l.app.test_begin_restore_from_snapshot(snapshot);
    assert!(
        drain(&l.rx).iter().any(|c| matches!(c, AudioCommand::ClearAll)),
        "a structural change takes the full path"
    );
    l.app.test_apply_engine_event(AudioEvent::AllCleared);
    assert_eq!(l.app.test_reconcile_trace(), expected(Origin::UndoFull).as_slice());
}

/// `Globals` runs before `Timeline` on the full path: every transport
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
        notes: Vec::new(),
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
/// the compose load precedes `Clips` (it resets the derived map). Within
/// `Tail`,
/// external instruments come before the lanes (a `DeviceParam` lane needs
/// the device bindings) and freeze is last (a disk load's baseline
/// fingerprints the lanes); derived clips follow the clips.
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
            (Stage::Clips, "audio_clips"),
            (Stage::Clips, "midi_clips"),
            (Stage::Clips, "clip_lyrics"),
            (Stage::Clips, "derived_clips"),
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
