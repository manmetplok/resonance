//! `io.restoring_undo` tells an undo's full replay from a disk load
//! (ARCH-01 A-7; it replaced the `pending_undo_extras` marker).
//!
//! Both go through the same `ClearAll → AllCleared → replay_loaded_project`
//! pipeline, and branch on the flag:
//!
//!   * a disk load re-attaches each frozen track's cache (`rehydrate`,
//!     `SetTrackFrozenSource`) and re-sends every external instrument's
//!     patch (`ResendExternalInstrumentPatches`);
//!   * an undo's slow path reconciles freeze against the live statuses
//!     (`apply_freeze_restore`: undoing a freeze deletes its cache) and
//!     never re-fires MIDI patches.
//!
//! The flag is set only where the undo slow path sends `ClearAll`, and
//! cleared by the `AllCleared` handler after the replay; the diff replay
//! never sets it.
//!
//! FU-A7a: a GUI `ProjectLoaded(Ok)` (there is no `io.loading` gate on
//! Open / Open Recent / Ctrl+O — only `refuse_project_switch_during_render`,
//! which checks an offline render, not an in-flight undo) or a template's
//! `begin_instantiate` can land while an undo's slow-path `ClearAll` is
//! still in flight. Both replace `io.pending_load` without touching
//! `io.restoring_undo`, so the disk/template project would otherwise
//! replay through the undo branches. The control API is not exposed to
//! this: `busy_guard` (`update/control/project.rs`) refuses `project.open`
//! / `project.new` while `io.loading` is set, and the undo slow path sets
//! `io.loading` in the same statement as `io.restoring_undo`.

use std::path::Path;

use resonance_app::message::{Message, ProjectIoMessage};
use resonance_app::project::{LoadedProject, ProjectFile, ProjectTrack};
use resonance_app::state::FreezeStatus;
use resonance_app::update::project_io::begin_instantiate;
use resonance_app::update::project_io::reconcile::Origin;
use resonance_app::Resonance;
use resonance_audio::test_support::Receiver;
use resonance_audio::types::{AudioCommand, AudioEvent, TrackType};
use resonance_common::{FreezeCacheRef, FreezeCacheStatus, TrackFreezeState};

fn project_track(id: u64, freeze: TrackFreezeState) -> ProjectTrack {
    use resonance_app::state::{InstrumentIcon, InstrumentType};
    ProjectTrack {
        id,
        name: format!("Track {id}"),
        order: (id as usize) - 1,
        volume: 0.0,
        pan: 0.0,
        muted: false,
        soloed: false,
        fx_bypassed: false,
        record_armed: false,
        monitor_enabled: false,
        playback_source: resonance_common::PlaybackSource::Live,
        mono: false,
        input_device_name: None,
        input_port_index: None,
        plugins: Vec::new(),
        track_type: "instrument".to_string(),
        output_bus: None,
        instrument_type: InstrumentType::Synth,
        instrument_icon: InstrumentIcon::Music,
        role: None,
        sub_track: None,
        midi_input_device: None,
        midi_input_channel: None,
        midi_output_device: None,
        midi_output_channel: None,
        external_instrument: None,
        freeze,
    }
}

/// A valid 32-bit float stereo WAV, the freeze-cache format.
fn write_cache_wav(path: &Path) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let spec = hound::WavSpec {
        channels: 2,
        sample_rate: 48_000,
        bits_per_sample: 32,
        sample_format: hound::SampleFormat::Float,
    };
    let mut writer = hound::WavWriter::create(path, spec).unwrap();
    for _ in 0..64 {
        writer.write_sample(0.0f32).unwrap();
        writer.write_sample(0.0f32).unwrap();
    }
    writer.finalize().unwrap();
}

fn drain(rx: &Receiver<AudioCommand>) -> Vec<AudioCommand> {
    let mut cmds = Vec::new();
    while let Ok(cmd) = rx.try_recv() {
        cmds.push(cmd);
    }
    cmds
}

fn attached(cmds: &[AudioCommand], id: u64) -> bool {
    cmds.iter().any(|c| {
        matches!(c, AudioCommand::SetTrackFrozenSource { track_id, source: Some(_) }
            if *track_id == id)
    })
}

fn resent_patches(cmds: &[AudioCommand]) -> bool {
    cmds.iter()
        .any(|c| matches!(c, AudioCommand::ResendExternalInstrumentPatches))
}

struct Loaded {
    app: Resonance,
    rx: Receiver<AudioCommand>,
    project: std::path::PathBuf,
    cache: std::path::PathBuf,
    /// Commands the disk load's `AllCleared` replay sent.
    load_cmds: Vec<AudioCommand>,
    _tmp: tempfile::TempDir,
}

/// Open a two-track project from disk — track 1 frozen with its cache
/// present, track 2 live — through the real `ProjectLoaded` → `ClearAll`
/// → `AllCleared` round-trip.
fn disk_load() -> Loaded {
    let tmp = tempfile::tempdir().unwrap();
    let (mut app, _task) = Resonance::new_for_test();
    let rx = app.test_capture_engine();
    let project = tmp.path().join("project.rproj");
    app.test_set_project_path(project.clone());
    let cache = tmp.path().join("project.freeze").join("freeze_1.wav");
    write_cache_wav(&cache);

    let file = ProjectFile {
        tracks: vec![
            project_track(
                1,
                TrackFreezeState::frozen(FreezeCacheRef::new(
                    "freeze_1.wav".to_string(),
                    48_000,
                    32,
                    1,
                    FreezeCacheStatus::Frozen,
                )),
            ),
            project_track(2, TrackFreezeState::unfrozen()),
        ],
        ..ProjectFile::default()
    };
    let loaded = LoadedProject {
        file,
        project_dir: project.clone(),
        midi_notes: Default::default(),
        plugin_states: Default::default(),
    };
    assert!(!app.test_restoring_undo(), "a fresh app has no undo in flight");
    let _ = app.update(Message::ProjectIo(ProjectIoMessage::ProjectLoaded(Ok(
        Box::new(loaded),
    ))));
    let cmds = drain(&rx);
    assert!(cmds.iter().any(|c| matches!(c, AudioCommand::ClearAll)));
    assert!(
        !app.test_restoring_undo(),
        "a disk load must not mark its replay as an undo"
    );
    app.test_apply_engine_event(AudioEvent::AllCleared);
    let load_cmds = drain(&rx);
    Loaded {
        app,
        rx,
        project,
        cache,
        load_cmds,
        _tmp: tmp,
    }
}

#[test]
fn a_disk_load_takes_the_disk_load_branches() {
    let l = disk_load();
    assert!(
        attached(&l.load_cmds, 1),
        "a disk load rehydrates the frozen track (SetTrackFrozenSource)"
    );
    assert!(matches!(l.app.test_freeze_status(1), FreezeStatus::Frozen { .. }));
    assert!(
        resent_patches(&l.load_cmds),
        "a disk load re-sends external-instrument patches"
    );
    assert!(!l.app.test_restoring_undo());
}

#[test]
fn an_undo_slow_path_takes_the_undo_branches() {
    let mut l = disk_load();
    let _ = drain(&l.rx);

    // The fast path never raises the flag: nothing waits for `AllCleared`.
    let same = l.app.test_snapshot_for_undo();
    l.app.test_begin_restore_from_snapshot(same);
    let cmds = drain(&l.rx);
    assert!(!cmds.iter().any(|c| matches!(c, AudioCommand::ClearAll)));
    assert!(!l.app.test_restoring_undo(), "the diff replay must not set the flag");

    // Undo of the freeze: the target has track 1 live. An extra live track
    // forces the structural fallback.
    let mut snapshot = l.app.test_snapshot_for_undo();
    snapshot.project.file.tracks[0].freeze = TrackFreezeState::unfrozen();
    l.app.test_add_track(9_999, TrackType::Audio);
    let _ = drain(&l.rx);
    l.app.test_begin_restore_from_snapshot(snapshot);
    let cmds = drain(&l.rx);
    assert!(
        cmds.iter().any(|c| matches!(c, AudioCommand::ClearAll)),
        "a structural change must take the slow path"
    );
    assert!(
        l.app.test_restoring_undo(),
        "the slow path marks the pending replay as an undo"
    );

    l.app.test_apply_engine_event(AudioEvent::AllCleared);
    let cmds = drain(&l.rx);
    assert!(
        !l.app.test_restoring_undo(),
        "the AllCleared handler clears the flag after the replay"
    );
    assert!(
        !resent_patches(&cmds),
        "an undo must never re-fire external-instrument patches"
    );
    assert!(!attached(&cmds, 1), "an undo does not rehydrate");
    // The undo branch reconciles against the live statuses: undoing the
    // freeze retires its cache. A disk load wipes the statuses first and
    // would leave the file alone.
    assert_eq!(l.app.test_freeze_status(1), FreezeStatus::Idle);
    assert!(!l.cache.exists(), "undoing a freeze deletes its cache");
    assert_eq!(
        l.app.test_project_path(),
        Some(l.project.as_path()),
        "the undo replay keeps the project path"
    );

    // A later disk load after the undo completed is a disk load again.
    let _ = drain(&l.rx);
    let loaded = LoadedProject {
        file: ProjectFile::default(),
        project_dir: l.project.clone(),
        midi_notes: Default::default(),
        plugin_states: Default::default(),
    };
    let _ = l.app.update(Message::ProjectIo(ProjectIoMessage::ProjectLoaded(Ok(
        Box::new(loaded),
    ))));
    assert!(!l.app.test_restoring_undo());
    l.app.test_apply_engine_event(AudioEvent::AllCleared);
    assert!(resent_patches(&drain(&l.rx)));
}

/// Force the undo slow (structural) path and leave its `ClearAll` in
/// flight: `io.loading` and `io.restoring_undo` are up, `AllCleared` has
/// not been delivered yet. Mirrors the setup half of
/// `an_undo_slow_path_takes_the_undo_branches`.
fn start_undo_slow_path(l: &mut Loaded) {
    let mut snapshot = l.app.test_snapshot_for_undo();
    snapshot.project.file.tracks[0].freeze = TrackFreezeState::unfrozen();
    l.app.test_add_track(9_999, TrackType::Audio);
    let _ = drain(&l.rx);
    l.app.test_begin_restore_from_snapshot(snapshot);
    let cmds = drain(&l.rx);
    assert!(
        cmds.iter().any(|c| matches!(c, AudioCommand::ClearAll)),
        "the structural change must take the slow path"
    );
    assert!(
        l.app.test_restoring_undo(),
        "the slow path's ClearAll must be in flight with the flag up"
    );
}

/// A second, distinct on-disk project with one frozen track (id 3), used to
/// prove which project's `AllCleared` branches actually ran.
fn other_project(dir: &Path) -> LoadedProject {
    let project_dir = dir.join("other.rproj");
    let cache = dir.join("other.freeze").join("freeze_3.wav");
    write_cache_wav(&cache);
    let file = ProjectFile {
        tracks: vec![project_track(
            3,
            TrackFreezeState::frozen(FreezeCacheRef::new(
                "freeze_3.wav".to_string(),
                48_000,
                32,
                1,
                FreezeCacheStatus::Frozen,
            )),
        )],
        ..ProjectFile::default()
    };
    LoadedProject {
        file,
        project_dir,
        midi_notes: Default::default(),
        plugin_states: Default::default(),
    }
}

/// FU-A7a: a GUI `ProjectLoaded(Ok)` landing while an undo's slow-path
/// `ClearAll` is still in flight replaces `pending_load` with the disk
/// project. It must also clear `restoring_undo`, so the eventual
/// `AllCleared` takes the disk-load branches (patch resend, rehydrate,
/// `Origin::DiskLoad`) for the project that actually ends up loaded —
/// never the undo branches.
#[test]
fn a_project_load_superseding_an_in_flight_undo_takes_the_disk_load_branches() {
    let mut l = disk_load();
    let _ = drain(&l.rx);
    start_undo_slow_path(&mut l);

    let tmp_dir = l.project.parent().unwrap().to_path_buf();
    let other = other_project(&tmp_dir);
    let _ = l.app.update(Message::ProjectIo(ProjectIoMessage::ProjectLoaded(Ok(
        Box::new(other),
    ))));
    let _ = drain(&l.rx); // the second ClearAll this handler sends

    assert!(
        !l.app.test_restoring_undo(),
        "a disk load superseding an in-flight undo must clear the flag"
    );

    l.app.test_apply_engine_event(AudioEvent::AllCleared);
    let cmds = drain(&l.rx);
    assert!(
        resent_patches(&cmds),
        "the superseding disk load must re-send external-instrument patches"
    );
    assert!(
        attached(&cmds, 3),
        "the superseding disk load must rehydrate its own frozen track"
    );
    assert!(
        l.app
            .test_reconcile_trace()
            .iter()
            .all(|(origin, _)| *origin == Origin::DiskLoad),
        "the replay that actually ran must be tagged DiskLoad, not UndoFull"
    );
}

/// FU-A7a, template variant: `begin_instantiate` landing mid-undo must also
/// clear the flag, so the template it loads (not the superseded undo
/// snapshot) takes the disk-load branches.
#[test]
fn a_template_instantiate_superseding_an_in_flight_undo_takes_the_disk_load_branches() {
    let mut l = disk_load();
    let _ = drain(&l.rx);
    start_undo_slow_path(&mut l);

    let tmp_dir = l.project.parent().unwrap().to_path_buf();
    let other = other_project(&tmp_dir);
    begin_instantiate(&mut l.app, Box::new(other));
    let _ = drain(&l.rx); // the second ClearAll `begin_instantiate` sends

    assert!(
        !l.app.test_restoring_undo(),
        "a template instantiate superseding an in-flight undo must clear the flag"
    );

    l.app.test_apply_engine_event(AudioEvent::AllCleared);
    let cmds = drain(&l.rx);
    assert!(
        resent_patches(&cmds),
        "the superseding template must re-send external-instrument patches"
    );
    assert!(
        attached(&cmds, 3),
        "the superseding template must rehydrate its own frozen track"
    );
    assert!(
        l.app
            .test_reconcile_trace()
            .iter()
            .all(|(origin, _)| *origin == Origin::DiskLoad),
        "the replay that actually ran must be tagged DiskLoad, not UndoFull"
    );
    assert_eq!(
        l.app.test_project_path(),
        None,
        "a template always lands untitled"
    );
}
