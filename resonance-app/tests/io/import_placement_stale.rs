//! A queued import placement does not outlive its context (code review
//! UPD-04).
//!
//! A drop queues "place this file on track T at sample S" until the
//! engine's off-thread transcode reports `AssetImported`. Nothing cleared
//! that queue or checked it against the current state, so an undo, a
//! track delete or a project switch in the meantime still placed a clip
//! later — onto a deleted track id, outside the undo history.

use std::collections::HashMap;
use std::path::PathBuf;

use resonance_app::message::{DropTarget, Message, PoolMessage, ProjectIoMessage, TrackMessage};
use resonance_app::project::{LoadedProject, ProjectFile};
use resonance_app::Resonance;
use resonance_audio::__test_support::Receiver;
use resonance_audio::types::{AudioCommand, AudioEvent, TrackType};
use resonance_common::AudioFormat;

const TRACK: u64 = 10;
const SOURCE: &str = "/imports/loop.flac";

fn app_with_queued_drop() -> (Resonance, Receiver<AudioCommand>) {
    let (mut app, _task) = Resonance::new_for_test();
    app.test_set_active_project(true);
    app.test_set_project_path(PathBuf::from("/proj/song.rproj"));
    let rx = app.test_capture_engine();
    app.test_add_track(TRACK, TrackType::Audio);
    let _ = app.update(Message::Pool(PoolMessage::ImportAndPlace {
        paths: vec![PathBuf::from(SOURCE)],
        target: DropTarget::ExistingTrack {
            track_id: TRACK,
            start_sample: 0,
        },
    }));
    assert_eq!(app.test_pending_import_count(), 1, "the placement is queued");
    while rx.try_recv().is_ok() {}
    (app, rx)
}

fn asset_imported() -> AudioEvent {
    AudioEvent::AssetImported {
        asset_id: 1,
        project_relative_path: "audio/asset_1.wav".into(),
        original_path: SOURCE.into(),
        format: AudioFormat::Flac,
        channels: 2,
        source_sample_rate: 48_000,
        duration_frames: 24_000,
        peaks: Vec::new(),
    }
}

fn assert_nothing_placed(app: &Resonance, rx: &Receiver<AudioCommand>) {
    assert!(
        !app.test_clips().iter().any(|c| c.track_id == TRACK),
        "no clip may be placed from a stale queue entry"
    );
    assert!(
        !rx.try_iter()
            .any(|c| matches!(c, AudioCommand::LoadClipFromWav { .. })),
        "the engine must not be asked to load the stale placement"
    );
}

#[test]
fn a_placement_onto_a_deleted_track_is_dropped() {
    let (mut app, rx) = app_with_queued_drop();
    app.test_apply_engine_event(AudioEvent::TrackRemoved { track_id: TRACK });

    app.test_apply_engine_event(asset_imported());

    assert_nothing_placed(&app, &rx);
}

#[test]
fn undoing_the_import_drops_its_queued_placement() {
    let (mut app, rx) = app_with_queued_drop();
    let _ = app.update(Message::Undo);

    app.test_apply_engine_event(asset_imported());

    assert_nothing_placed(&app, &rx);
}

#[test]
fn undoing_a_later_edit_keeps_the_queued_placement() {
    let (mut app, rx) = app_with_queued_drop();
    let _ = app.update(Message::Track(TrackMessage::SetTrackVolume(TRACK, -6.0)));
    let _ = app.update(Message::Undo);

    app.test_apply_engine_event(asset_imported());

    assert!(
        app.test_clips().iter().any(|c| c.track_id == TRACK),
        "the import itself was not undone, so its clip still lands"
    );
    assert!(rx
        .try_iter()
        .any(|c| matches!(c, AudioCommand::LoadClipFromWav { .. })));
}

#[test]
fn opening_another_project_drops_the_queued_placement() {
    let (mut app, rx) = app_with_queued_drop();
    let loaded = LoadedProject {
        file: ProjectFile::default(),
        project_dir: PathBuf::from("/proj/other.rproj"),
        midi_notes: HashMap::new(),
        plugin_states: HashMap::new(),
    };
    let _ = app.update(Message::ProjectIo(ProjectIoMessage::ProjectLoaded(Ok(Box::new(loaded)))));
    app.test_apply_engine_event(AudioEvent::AllCleared);
    // The new project happens to have a track with the old id.
    app.test_add_track(TRACK, TrackType::Audio);

    app.test_apply_engine_event(asset_imported());

    assert_nothing_placed(&app, &rx);
}
