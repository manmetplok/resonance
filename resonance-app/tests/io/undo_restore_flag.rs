//! A disk load and an undo/redo take different restores (ARCH-01 A-13j;
//! this file guarded `io.restoring_undo`, the flag that told them apart
//! when both went through `ClearAll → AllCleared`, A-7 / FU-A7a).
//!
//! Since A-13j an undo never sends `ClearAll`: it runs the `Reconcile`
//! driver in place, synchronously, under `Origin::Undo`, and only a disk
//! load or template instantiate leaves a `pending_load` for `AllCleared`.
//! So the `AllCleared` handler needs no flag, and:
//!
//!   * a disk load re-attaches each frozen track's cache (`rehydrate`,
//!     `SetTrackFrozenSource`) and re-sends every external instrument's
//!     patch (`ResendExternalInstrumentPatches`), under `Origin::DiskLoad`;
//!   * an undo reconciles freeze against the live statuses
//!     (`apply_freeze_restore`: undoing a freeze deletes its cache), never
//!     re-fires MIDI patches, keeps the project path and leaves no loading
//!     window.
//!
//! FU-A7a's race — a disk load or template landing while an undo's
//! `ClearAll` was in flight, whose replay then took the undo branches — has
//! no undo side left; its successor here is the other order: an undo
//! requested while a disk load's `ClearAll` is in flight is refused
//! (`can_undo_redo_now` requires `!io.loading`), and the load replays as a
//! disk load.

use std::path::Path;

use resonance_app::message::{Message, ProjectIoMessage, TrackMessage};
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
        color: None,
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
    let _ = app.update(Message::ProjectIo(ProjectIoMessage::ProjectLoaded(Ok(
        Box::new(loaded),
    ))));
    let cmds = drain(&rx);
    assert!(cmds.iter().any(|c| matches!(c, AudioCommand::ClearAll)));
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

fn traced_as(app: &Resonance, origin: Origin) -> bool {
    let trace = app.test_reconcile_trace();
    !trace.is_empty() && trace.iter().all(|(o, _)| *o == origin)
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
    assert!(traced_as(&l.app, Origin::DiskLoad));
}

#[test]
fn an_undo_takes_the_undo_branches_in_place() {
    let mut l = disk_load();
    let _ = drain(&l.rx);

    // Undo of the freeze: the target has track 1 live. An extra track makes
    // it structural — the shape that used to take the `ClearAll` fallback.
    let mut snapshot = l.app.test_snapshot_for_undo();
    snapshot.project.file.tracks[0].freeze = TrackFreezeState::unfrozen();
    l.app.test_add_track(9_999, TrackType::Audio);
    let _ = drain(&l.rx);
    l.app.test_begin_restore_from_snapshot(snapshot);
    let cmds = drain(&l.rx);
    assert!(
        !cmds.iter().any(|c| matches!(c, AudioCommand::ClearAll)),
        "an undo never clears the engine"
    );
    assert!(traced_as(&l.app, Origin::Undo), "the restore ran, synchronously, as an undo");
    assert!(
        !resent_patches(&cmds),
        "an undo must never re-fire external-instrument patches"
    );
    assert!(!attached(&cmds, 1), "an undo does not rehydrate");
    // The undo reconciles against the live statuses: undoing the freeze
    // retires its cache. A disk load wipes the statuses first and would
    // leave the file alone.
    assert_eq!(l.app.test_freeze_status(1), FreezeStatus::Idle);
    assert!(!l.cache.exists(), "undoing a freeze deletes its cache");
    assert_eq!(
        l.app.test_project_path(),
        Some(l.project.as_path()),
        "the undo keeps the project path"
    );
    // Nothing is pending, so a stray `AllCleared` replays nothing.
    l.app.test_apply_engine_event(AudioEvent::AllCleared);
    assert!(drain(&l.rx).is_empty(), "no replay was left pending");
    assert!(traced_as(&l.app, Origin::Undo));

    // A later disk load is a disk load.
    let loaded = LoadedProject {
        file: ProjectFile::default(),
        project_dir: l.project.clone(),
        midi_notes: Default::default(),
        plugin_states: Default::default(),
    };
    let _ = l.app.update(Message::ProjectIo(ProjectIoMessage::ProjectLoaded(Ok(
        Box::new(loaded),
    ))));
    l.app.test_apply_engine_event(AudioEvent::AllCleared);
    assert!(resent_patches(&drain(&l.rx)));
    assert!(traced_as(&l.app, Origin::DiskLoad));
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

/// Request an undo while the load's `ClearAll` is in flight, then deliver
/// `AllCleared` and assert the replay that ran is `other`'s, as a disk load.
fn undo_mid_load_is_refused(l: &mut Loaded) {
    let before = l.app.test_reconcile_trace().to_vec();
    let _ = l.app.update(Message::Undo);
    let cmds = drain(&l.rx);
    assert!(
        cmds.is_empty(),
        "an undo during a load's ClearAll must be refused, got {cmds:?}"
    );
    assert_eq!(l.app.test_reconcile_trace(), before.as_slice(), "no restore ran");

    l.app.test_apply_engine_event(AudioEvent::AllCleared);
    let cmds = drain(&l.rx);
    assert!(
        resent_patches(&cmds),
        "the load must re-send external-instrument patches"
    );
    assert!(
        attached(&cmds, 3),
        "the load must rehydrate its own frozen track"
    );
    assert!(traced_as(&l.app, Origin::DiskLoad), "the replay is a disk load");
}

/// FU-A7a's successor: an undo requested while a GUI `ProjectLoaded(Ok)`'s
/// `ClearAll` is in flight is refused, and the eventual `AllCleared`
/// replays the opened project through the disk-load branches.
#[test]
fn an_undo_during_a_project_loads_clear_is_refused() {
    let mut l = disk_load();
    // An undoable edit, so the refusal is not just an empty history.
    let _ = l.app.update(Message::Track(TrackMessage::SetTrackVolume(2, -6.0)));
    assert!(l.app.test_can_undo(), "the edit is undoable");
    let _ = drain(&l.rx);

    let tmp_dir = l.project.parent().unwrap().to_path_buf();
    let other = other_project(&tmp_dir);
    let _ = l.app.update(Message::ProjectIo(ProjectIoMessage::ProjectLoaded(Ok(
        Box::new(other),
    ))));
    let _ = drain(&l.rx); // the ClearAll this handler sends
    undo_mid_load_is_refused(&mut l);
}

/// The template variant: `begin_instantiate` in flight, then an undo.
#[test]
fn an_undo_during_a_template_instantiates_clear_is_refused() {
    let mut l = disk_load();
    let _ = l.app.update(Message::Track(TrackMessage::SetTrackVolume(2, -6.0)));
    let _ = drain(&l.rx);

    let tmp_dir = l.project.parent().unwrap().to_path_buf();
    let other = other_project(&tmp_dir);
    begin_instantiate(&mut l.app, Box::new(other));
    let _ = drain(&l.rx); // the ClearAll `begin_instantiate` sends
    undo_mid_load_is_refused(&mut l);
    assert_eq!(
        l.app.test_project_path(),
        None,
        "a template always lands untitled"
    );
}
