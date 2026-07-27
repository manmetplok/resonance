//! Freeze persistence + cache lifecycle coverage (ba todo #577).
//!
//! Proves the per-track [`TrackFreezeState`] survives the on-disk project
//! round-trip (serde + a real `save_project` / `load_project`), that
//! projects predating freeze still load (absent field => live), and that
//! the load-time rehydrate re-attaches a present cache (status `Frozen`,
//! engine handed the decoded buffer) while a missing / corrupt cache
//! degrades the track to `Stale` without crashing.
//!
//! The freeze cache lives in a *sibling* `<project>.freeze/` directory
//! next to the `.rproj` bundle — a build artifact excluded from the save —
//! so the rehydrate tests anchor the project at a `.rproj` path and write
//! the cache WAV to its sibling.

use std::path::Path;

use resonance_app::message::{Message, TrackMessage};
use resonance_app::project::{load_project, save_project, ProjectFile};
use resonance_app::state::FreezeStatus;
use resonance_app::Resonance;
use resonance_audio::__test_support::Receiver;
use resonance_audio::types::{AudioCommand, TrackType};
use resonance_common::{FreezeCacheRef, FreezeCacheStatus, TrackFreezeState};

// ---------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------

fn frozen_ref(filename: &str, fingerprint: u64, status: FreezeCacheStatus) -> FreezeCacheRef {
    FreezeCacheRef::new(filename.to_string(), 48_000, 32, fingerprint, status)
}

/// Build a one-track project whose single track carries `freeze`.
fn project_with_freeze(freeze: TrackFreezeState) -> ProjectFile {
    ProjectFile {
        tracks: vec![project_track(1, freeze)],
        ..ProjectFile::default()
    }
}

/// A neutral instrument [`ProjectTrack`] carrying `freeze`. Built via the
/// serializer's shape so the test stays in step with the struct.
fn project_track(id: u64, freeze: TrackFreezeState) -> resonance_app::project::ProjectTrack {
    use resonance_app::state::{InstrumentIcon, InstrumentType};
    resonance_app::project::ProjectTrack {
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

/// Write a valid 32-bit float stereo WAV (the freeze-cache format) with
/// `frames` silent stereo frames so the decoder has something to read.
fn write_cache_wav(path: &Path, sample_rate: u32, frames: u32) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let spec = hound::WavSpec {
        channels: 2,
        sample_rate,
        bits_per_sample: 32,
        sample_format: hound::SampleFormat::Float,
    };
    let mut writer = hound::WavWriter::create(path, spec).unwrap();
    for _ in 0..frames {
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

/// A capturing app anchored at `<tmp>/project.rproj`, so the sibling
/// freeze-cache dir resolves to `<tmp>/project.freeze/`.
fn capturing_app(tmp: &Path) -> (Resonance, Receiver<AudioCommand>) {
    let (mut app, _task) = Resonance::new();
    let rx = app.test_capture_engine();
    app.test_set_project_path(tmp.join("project.rproj"));
    (app, rx)
}

// ---------------------------------------------------------------------
// Serde + disk round-trip
// ---------------------------------------------------------------------

#[test]
fn frozen_track_survives_serde_round_trip() {
    let cache_ref = frozen_ref("freeze_1.wav", 0xABCD, FreezeCacheStatus::Frozen);
    let file = project_with_freeze(TrackFreezeState::frozen(cache_ref.clone()));

    let json = serde_json::to_string(&file).expect("serialize");
    let restored: ProjectFile = serde_json::from_str(&json).expect("deserialize");

    let freeze = &restored.tracks[0].freeze;
    assert!(freeze.is_frozen);
    assert_eq!(freeze.cache_ref.as_ref(), Some(&cache_ref));
}

#[test]
fn stale_track_round_trips_with_stale_status() {
    let cache_ref = frozen_ref("freeze_1.wav", 7, FreezeCacheStatus::Stale);
    let file = project_with_freeze(TrackFreezeState::frozen(cache_ref.clone()));

    let json = serde_json::to_string(&file).expect("serialize");
    let restored: ProjectFile = serde_json::from_str(&json).expect("deserialize");

    let cr = restored.tracks[0].freeze.cache_ref.clone().expect("cache ref");
    assert_eq!(cr.status, FreezeCacheStatus::Stale);
}

#[test]
fn frozen_track_survives_disk_save_load_round_trip() {
    let dir = std::env::temp_dir().join(format!(
        "resonance_freeze_persist_{}.rproj",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);

    let cache_ref = frozen_ref("freeze_1.wav", 0x1234, FreezeCacheStatus::Frozen);
    let file = project_with_freeze(TrackFreezeState::frozen(cache_ref.clone()));

    save_project(&dir, &file, &[], &[]).expect("save project");
    let loaded = load_project(&dir).expect("load project");

    let freeze = &loaded.file.tracks[0].freeze;
    assert!(freeze.is_frozen);
    assert_eq!(freeze.cache_ref.as_ref(), Some(&cache_ref));

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn legacy_project_without_freeze_loads_unfrozen() {
    // Serialize a frozen-track project, strip the `freeze` key from the
    // track JSON to mimic a project authored before freeze existed, and
    // confirm it deserializes to the live/unfrozen default.
    let file = project_with_freeze(TrackFreezeState::frozen(frozen_ref(
        "freeze_1.wav",
        1,
        FreezeCacheStatus::Frozen,
    )));
    let mut value: serde_json::Value = serde_json::to_value(&file).expect("to value");
    let track = &mut value["tracks"][0];
    assert!(
        track.as_object_mut().unwrap().remove("freeze").is_some(),
        "the freeze field should have been present to remove"
    );

    let restored: ProjectFile = serde_json::from_value(value).expect("legacy deserialize");
    assert_eq!(restored.tracks[0].freeze, TrackFreezeState::unfrozen());
    assert!(!restored.tracks[0].freeze.is_frozen);
}

// ---------------------------------------------------------------------
// Live status -> persisted shape
// ---------------------------------------------------------------------

#[test]
fn serialize_projects_live_frozen_status_into_project_file() {
    let tmp = tempfile::tempdir().unwrap();
    let (mut app, _rx) = capturing_app(tmp.path());
    app.test_add_track(1, TrackType::Instrument);
    let cache_ref = frozen_ref("freeze_1.wav", 99, FreezeCacheStatus::Frozen);
    app.test_set_freeze_status(
        1,
        FreezeStatus::Frozen {
            cache_ref: cache_ref.clone(),
        },
    );

    let file = app.test_build_project_file();
    let freeze = &file.tracks[0].freeze;
    assert!(freeze.is_frozen);
    assert_eq!(freeze.cache_ref.as_ref(), Some(&cache_ref));
}

#[test]
fn serialize_treats_transient_freeze_status_as_unfrozen() {
    let tmp = tempfile::tempdir().unwrap();
    let (mut app, _rx) = capturing_app(tmp.path());
    app.test_add_track(1, TrackType::Instrument);
    // An in-flight render is transient — it must not persist as frozen.
    app.test_set_freeze_status(1, FreezeStatus::Freezing { fraction: 0.5 });

    let file = app.test_build_project_file();
    assert_eq!(file.tracks[0].freeze, TrackFreezeState::unfrozen());
}

// ---------------------------------------------------------------------
// Load-time rehydrate
// ---------------------------------------------------------------------

#[test]
fn rehydrate_attaches_present_cache_as_frozen() {
    let tmp = tempfile::tempdir().unwrap();
    let (mut app, rx) = capturing_app(tmp.path());
    app.test_add_track(1, TrackType::Instrument);

    // Write a real cache WAV into the sibling freeze dir.
    let cache_wav = tmp.path().join("project.freeze").join("freeze_1.wav");
    write_cache_wav(&cache_wav, 48_000, 128);

    let cache_ref = frozen_ref("freeze_1.wav", 42, FreezeCacheStatus::Frozen);
    let project_dir = tmp.path().join("project.rproj");
    app.test_rehydrate_frozen_tracks(
        &project_dir,
        &[(1, TrackFreezeState::frozen(cache_ref.clone()))],
    );

    // The track loads Frozen and the engine is handed the decoded buffer
    // (so reopening replays the cache rather than re-rendering).
    assert!(matches!(app.test_freeze_status(1), FreezeStatus::Frozen { .. }));
    assert!(
        drain(&rx).iter().any(|c| matches!(
            c,
            AudioCommand::SetTrackFrozenSource {
                track_id: 1,
                source: Some(_)
            }
        )),
        "expected the decoded cache to be attached via SetTrackFrozenSource",
    );
}

#[test]
fn rehydrate_missing_cache_loads_stale() {
    let tmp = tempfile::tempdir().unwrap();
    let (mut app, rx) = capturing_app(tmp.path());
    app.test_add_track(1, TrackType::Instrument);

    // No cache file on disk.
    let cache_ref = frozen_ref("freeze_1.wav", 42, FreezeCacheStatus::Frozen);
    let project_dir = tmp.path().join("project.rproj");
    app.test_rehydrate_frozen_tracks(&project_dir, &[(1, TrackFreezeState::frozen(cache_ref))]);

    assert!(matches!(app.test_freeze_status(1), FreezeStatus::Stale { .. }));
    assert!(
        !drain(&rx).iter().any(|c| matches!(
            c,
            AudioCommand::SetTrackFrozenSource {
                source: Some(_),
                ..
            }
        )),
        "a missing cache must not attach a frozen source",
    );
}

#[test]
fn rehydrate_corrupt_cache_loads_stale() {
    let tmp = tempfile::tempdir().unwrap();
    let (mut app, _rx) = capturing_app(tmp.path());
    app.test_add_track(1, TrackType::Instrument);

    // Write garbage where the cache WAV should be.
    let cache_wav = tmp.path().join("project.freeze").join("freeze_1.wav");
    std::fs::create_dir_all(cache_wav.parent().unwrap()).unwrap();
    std::fs::write(&cache_wav, b"not a wav file").unwrap();

    let cache_ref = frozen_ref("freeze_1.wav", 42, FreezeCacheStatus::Frozen);
    let project_dir = tmp.path().join("project.rproj");
    app.test_rehydrate_frozen_tracks(&project_dir, &[(1, TrackFreezeState::frozen(cache_ref))]);

    assert!(matches!(app.test_freeze_status(1), FreezeStatus::Stale { .. }));
}

#[test]
fn rehydrate_skips_unfrozen_tracks() {
    let tmp = tempfile::tempdir().unwrap();
    let (mut app, rx) = capturing_app(tmp.path());
    app.test_add_track(1, TrackType::Instrument);

    let project_dir = tmp.path().join("project.rproj");
    app.test_rehydrate_frozen_tracks(&project_dir, &[(1, TrackFreezeState::unfrozen())]);

    assert_eq!(app.test_freeze_status(1), FreezeStatus::Idle);
    assert!(drain(&rx).is_empty(), "an unfrozen track issues no commands");
}

// ---------------------------------------------------------------------
// Cache lifecycle on track delete
// ---------------------------------------------------------------------

#[test]
fn deleting_a_frozen_track_removes_its_cache() {
    let tmp = tempfile::tempdir().unwrap();
    let (mut app, rx) = capturing_app(tmp.path());
    app.test_add_track(1, TrackType::Instrument);

    let cache_wav = tmp.path().join("project.freeze").join("freeze_1.wav");
    write_cache_wav(&cache_wav, 48_000, 16);
    app.test_set_freeze_status(
        1,
        FreezeStatus::Frozen {
            cache_ref: frozen_ref("freeze_1.wav", 1, FreezeCacheStatus::Frozen),
        },
    );

    // A track with no clips deletes immediately (no confirm dialog).
    app.test_dispatch(Message::Track(TrackMessage::RequestRemoveTrack(1)));

    assert!(!cache_wav.exists(), "the cache file is deleted with the track");
    assert!(drain(&rx)
        .iter()
        .any(|c| matches!(c, AudioCommand::UnfreezeTrack { track_id: 1 })));
}
