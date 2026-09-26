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

use resonance_app::message::{Message, ProjectIoMessage};
use resonance_app::project::{LoadedProject, ProjectFile};
use resonance_app::update::project_io::reconcile::{domain_order, Origin};
use resonance_app::Resonance;
use resonance_audio::test_support::Receiver;
use resonance_audio::types::{AudioCommand, AudioEvent, TrackType};

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

/// The full path's `Timeline` stage sits after `SetBpm` and before
/// `SetTimeSignature`: the engine sees the same tempo sequence it did when
/// the tempo events were restored inline.
#[test]
fn the_full_path_sends_bpm_then_tempo_events_then_meter() {
    let l = disk_load();
    let pos = |pred: fn(&AudioCommand) -> bool| {
        l.load_cmds
            .iter()
            .position(pred)
            .expect("the replay sends every tempo command")
    };
    let bpm = pos(|c| matches!(c, AudioCommand::SetBpm { .. }));
    let events = pos(|c| matches!(c, AudioCommand::SetTempoEvents { .. }));
    let meter = pos(|c| matches!(c, AudioCommand::SetTimeSignature { .. }));
    assert!(bpm < events && events < meter, "{bpm} < {events} < {meter}");
}
