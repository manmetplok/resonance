//! Import + placement orchestration (doc #175, ba todo #598).
//!
//! Drives the app-side flow with a command-capturing engine: a
//! `PoolMessage` fans out into `ImportAudioToPool` (+ an `AddTrack` for a
//! new-track drop) and queues per-file placements; the engine's
//! `AssetImported` echo then mirrors the asset into the pool and places
//! the queued clip via `LoadClipFromWav`, tying it to its `AssetRef`.
//! Also covers the single-action undo (one pre-import snapshot reverts the
//! whole import + placement) and the failure / no-project paths.

use resonance_app::message::{DropTarget, Message, PoolMessage};
use resonance_app::undo::{classify, UndoAction};
use resonance_app::Resonance;
use resonance_audio::test_support::Receiver;
use resonance_audio::types::{AudioCommand, AudioEvent, TrackType};
use resonance_common::AudioFormat;
use std::path::PathBuf;

/// A fresh app with an active, saved project (so imports have a home and
/// the undo gate — which needs `has_active_project` + a `project_path` —
/// is satisfied) and a command-capturing engine.
fn app() -> (Resonance, Receiver<AudioCommand>) {
    let (mut app, _task) = Resonance::new_for_test();
    app.test_set_active_project(true);
    app.test_set_project_path(PathBuf::from("/proj/song.rproj"));
    let rx = app.test_capture_engine();
    (app, rx)
}

fn drain(rx: &Receiver<AudioCommand>) -> Vec<AudioCommand> {
    let mut cmds = Vec::new();
    while let Ok(cmd) = rx.try_recv() {
        cmds.push(cmd);
    }
    cmds
}

/// An `AssetImported` event for the source file `original` landing at pool
/// asset `id` (its engine-format WAV written to `audio/asset_{id}.wav`).
fn asset_imported(id: u64, original: &str) -> AudioEvent {
    AudioEvent::AssetImported {
        asset_id: id,
        project_relative_path: format!("audio/asset_{id}.wav"),
        original_path: original.to_string(),
        format: AudioFormat::Wav,
        channels: 2,
        source_sample_rate: 48_000,
        duration_frames: 24_000,
        peaks: vec![(-0.5, 0.5), (-0.7, 0.7)],
    }
}

fn count_cmd(cmds: &[AudioCommand], f: impl Fn(&AudioCommand) -> bool) -> usize {
    cmds.iter().filter(|c| f(c)).count()
}

// --------------------------------------------------------------------
// Pool-only import (dialog / "Import audio…")
// --------------------------------------------------------------------

#[test]
fn pool_only_import_sends_command_and_places_nothing() {
    let (mut app, rx) = app();

    let _ = app.update(Message::Pool(PoolMessage::ImportFilesToPool(vec![
        PathBuf::from("/imports/kick.wav"),
        PathBuf::from("/imports/snare.flac"),
    ])));

    let cmds = drain(&rx);
    // One batch import command for both files, no track spawned. Each
    // file carries the asset id the app already allocated for it (D-7a).
    let import = cmds
        .iter()
        .find_map(|c| match c {
            AudioCommand::ImportAudioToPool { files } => Some(files.clone()),
            _ => None,
        })
        .expect("ImportAudioToPool sent");
    assert_eq!(
        import,
        vec![
            resonance_audio::types::PoolImportFile {
                asset_id: 1,
                path: "/imports/kick.wav".to_string(),
            },
            resonance_audio::types::PoolImportFile {
                asset_id: 2,
                path: "/imports/snare.flac".to_string(),
            },
        ]
    );
    assert_eq!(
        count_cmd(&cmds, |c| matches!(c, AudioCommand::AddTrack { .. })),
        0,
        "pool-only import spawns no track"
    );
    assert_eq!(app.test_pending_import_count(), 2, "both files queued");

    // Both assets land: pool gains them, no clips are placed, queue drains.
    app.test_handle_engine_event(asset_imported(1, "/imports/kick.wav"));
    app.test_handle_engine_event(asset_imported(2, "/imports/snare.flac"));

    assert_eq!(app.test_pool().assets.len(), 2, "both assets in pool");
    assert!(app.test_clips().is_empty(), "pool-only import places no clip");
    assert_eq!(app.test_pending_import_count(), 0, "queue drained");
}

// --------------------------------------------------------------------
// Drop on an existing track
// --------------------------------------------------------------------

#[test]
fn import_and_place_on_existing_track() {
    let (mut app, rx) = app();
    app.test_add_track(10, TrackType::Audio);

    let _ = app.update(Message::Pool(PoolMessage::ImportAndPlace {
        paths: vec![PathBuf::from("/imports/loop.wav")],
        target: DropTarget::ExistingTrack {
            track_id: 10,
            start_sample: 0,
        },
    }));

    let cmds = drain(&rx);
    assert_eq!(
        count_cmd(&cmds, |c| matches!(c, AudioCommand::ImportAudioToPool { .. })),
        1,
    );
    assert_eq!(
        count_cmd(&cmds, |c| matches!(c, AudioCommand::AddTrack { .. })),
        0,
        "dropping on an existing lane spawns no track"
    );
    assert_eq!(app.test_pending_import_count(), 1);

    // The asset lands and is placed as a clip on the target track. The app
    // allocated the id itself (D-7a) — this is the first import in a fresh
    // app, so it's 1.
    app.test_handle_engine_event(asset_imported(1, "/imports/loop.wav"));

    let cmds = drain(&rx);
    let loaded = cmds
        .iter()
        .find_map(|c| match c {
            AudioCommand::LoadClipFromWav {
                track_id,
                path,
                start_sample,
                ..
            } => Some((*track_id, path.clone(), *start_sample)),
            _ => None,
        })
        .expect("LoadClipFromWav sent for placement");
    assert_eq!(loaded.0, 10, "clip loaded onto the target track");
    assert_eq!(
        loaded.1,
        PathBuf::from("/proj/song.rproj/audio/asset_1.wav"),
        "clip loads from the asset's engine-format WAV"
    );

    assert_eq!(app.test_clips().len(), 1);
    let clip = &app.test_clips()[0];
    assert_eq!(clip.track_id, 10);
    assert_eq!(clip.start_sample, 0);
    assert_eq!(clip.name, "loop", "clip named from the source file stem");
    assert_eq!(
        clip.asset_ref.map(|a| a.asset_id),
        Some(1),
        "clip tied to its pool asset"
    );
    assert_eq!(app.test_pool().usage_count(1), 1, "asset now used by 1 clip");
    assert_eq!(app.test_pending_import_count(), 0);
}

// --------------------------------------------------------------------
// Two imports of the SAME path, finishing out of order (D-7a)
// --------------------------------------------------------------------

/// Two separate drops of the SAME source path — each targeting a
/// different track — must each place onto the track its OWN drop asked
/// for, even when the engine reports the second drop's asset before the
/// first's.
///
/// Before D-7a `PendingImports` matched an `AssetImported` back by its
/// source path, so with two entries queued for the same path the FIRST
/// one queued always resolved first, regardless of which asset actually
/// landed. Each batch decodes on its own worker thread, so nothing
/// guarantees they finish in send order — the second drop could (and,
/// under load, did) report its asset before the first, silently placing
/// it on the first drop's track and vice versa. D-7a keys each queued
/// placement by its own asset id instead, so there is no shared key left
/// to resolve out of order.
#[test]
fn two_drops_of_the_same_path_place_correctly_even_out_of_order() {
    let (mut app, rx) = app();
    app.test_add_track(10, TrackType::Audio);
    app.test_add_track(20, TrackType::Audio);

    // First drop: same path onto track 10. `ImportAndPlaceExact` (no grid
    // snap) so the exact sample positions below are unambiguous.
    let _ = app.update(Message::Pool(PoolMessage::ImportAndPlaceExact {
        paths: vec![PathBuf::from("/imports/shared.wav")],
        track_id: 10,
        start_sample: 0,
    }));
    // Second drop: same path onto track 20.
    let _ = app.update(Message::Pool(PoolMessage::ImportAndPlaceExact {
        paths: vec![PathBuf::from("/imports/shared.wav")],
        track_id: 20,
        start_sample: 4_800,
    }));
    assert_eq!(app.test_pending_import_count(), 2, "both drops queued");

    // The app allocated a distinct id per drop (D-7a), first drop = 1,
    // second = 2, in send order — but the ENGINE reports them back out of
    // order: id 2 (the second drop) lands first.
    let _ = drain(&rx);
    app.test_handle_engine_event(asset_imported(2, "/imports/shared.wav"));
    app.test_handle_engine_event(asset_imported(1, "/imports/shared.wav"));

    assert_eq!(app.test_clips().len(), 2);
    let clip_for_track = |track_id: u64| {
        app.test_clips()
            .iter()
            .find(|c| c.track_id == track_id)
            .unwrap_or_else(|| panic!("no clip on track {track_id}"))
            .clone()
    };
    let on_10 = clip_for_track(10);
    let on_20 = clip_for_track(20);
    assert_eq!(
        on_10.asset_ref.map(|a| a.asset_id),
        Some(1),
        "track 10 got the FIRST drop's asset, not the one that happened to land first"
    );
    assert_eq!(
        on_20.asset_ref.map(|a| a.asset_id),
        Some(2),
        "track 20 got the SECOND drop's asset"
    );
    assert_eq!(on_10.start_sample, 0);
    assert_eq!(on_20.start_sample, 4_800);
    assert_eq!(app.test_pending_import_count(), 0);
}

// --------------------------------------------------------------------
// Drop on the new-audio-track zone
// --------------------------------------------------------------------

#[test]
fn drop_on_new_track_zone_spawns_track_then_places() {
    let (mut app, rx) = app();
    assert!(app.test_registry().tracks.is_empty());

    let _ = app.update(Message::Pool(PoolMessage::ImportAndPlace {
        paths: vec![PathBuf::from("/imports/vocal take.wav")],
        target: DropTarget::NewTrack { start_sample: 0 },
    }));

    // A fresh audio track is reserved and added up front.
    let cmds = drain(&rx);
    let new_id = cmds
        .iter()
        .find_map(|c| match c {
            AudioCommand::AddTrack { id, .. } => Some(*id),
            _ => None,
        })
        .expect("AddTrack sent for new-track drop");
    assert_eq!(
        count_cmd(&cmds, |c| matches!(c, AudioCommand::ImportAudioToPool { .. })),
        1,
    );

    // The engine echoes the track back (creating its app-side state), then
    // the asset lands and is placed on the new track.
    app.test_handle_engine_event(AudioEvent::TrackAdded { track_id: new_id });
    assert_eq!(app.test_registry().tracks.len(), 1, "new track mirrored");

    // The app allocated the id itself (D-7a) — first import in a fresh app.
    app.test_handle_engine_event(asset_imported(1, "/imports/vocal take.wav"));

    assert_eq!(app.test_clips().len(), 1);
    let clip = &app.test_clips()[0];
    assert_eq!(clip.track_id, new_id, "clip placed on the spawned track");
    assert_eq!(clip.asset_ref.map(|a| a.asset_id), Some(1));
    assert_eq!(clip.name, "vocal take");
    assert_eq!(app.test_pending_import_count(), 0);
}

// --------------------------------------------------------------------
// Single-action undo
// --------------------------------------------------------------------

#[test]
fn import_and_place_is_one_undoable_action() {
    // Classifier: both variants record exactly one undo entry.
    assert!(matches!(
        classify(&Message::Pool(PoolMessage::ImportFilesToPool(vec![]))),
        UndoAction::Record
    ));

    let (mut app, _rx) = app();
    app.test_add_track(10, TrackType::Audio);

    let _ = app.update(Message::Pool(PoolMessage::ImportAndPlace {
        paths: vec![PathBuf::from("/imports/loop.wav")],
        target: DropTarget::ExistingTrack {
            track_id: 10,
            start_sample: 0,
        },
    }));
    // The app allocated the id itself (D-7a) — first import in a fresh app.
    app.test_handle_engine_event(asset_imported(1, "/imports/loop.wav"));

    // The placement landed…
    assert_eq!(app.test_clips().len(), 1);
    assert_eq!(app.test_pool().assets.len(), 1);

    // …and it is captured as exactly one undo entry whose snapshot predates
    // the whole import: no placed clip, no pool asset. One undo of it
    // therefore removes both (the async placement never records its own
    // entry).
    let history = app.test_undo_history();
    let entries = history.test_undo_entries();
    assert_eq!(entries.len(), 1, "exactly one undo entry for the import");
    let pre = &entries[0].project.file;
    assert!(pre.clips.is_empty(), "pre-import snapshot has no placed clip");
    assert!(
        pre.pool_assets.is_empty(),
        "pre-import snapshot has no pool asset"
    );
}

// --------------------------------------------------------------------
// Failure + no-project guards
// --------------------------------------------------------------------

#[test]
fn import_failure_drops_placement_and_reports() {
    let (mut app, _rx) = app();
    app.test_add_track(10, TrackType::Audio);

    let _ = app.update(Message::Pool(PoolMessage::ImportAndPlace {
        paths: vec![PathBuf::from("/imports/broken.wav")],
        target: DropTarget::ExistingTrack {
            track_id: 10,
            start_sample: 0,
        },
    }));
    assert_eq!(app.test_pending_import_count(), 1);

    // The app allocated the id itself (D-7a) — first import in a fresh app.
    app.test_handle_engine_event(AudioEvent::ImportFailed {
        asset_id: 1,
        path: "/imports/broken.wav".to_string(),
        reason: "unsupported codec".to_string(),
    });

    assert_eq!(app.test_pending_import_count(), 0, "queued placement dropped");
    assert!(app.test_clips().is_empty(), "no clip placed on failure");
    assert!(app.test_pool().assets.is_empty(), "no asset added on failure");
}

#[test]
fn import_without_project_is_refused() {
    // Active project but no saved path — importing has nowhere to copy to.
    let (mut app, _task) = Resonance::new_for_test();
    app.test_set_active_project(true);
    let rx = app.test_capture_engine();

    let _ = app.update(Message::Pool(PoolMessage::ImportFilesToPool(vec![
        PathBuf::from("/imports/kick.wav"),
    ])));

    let cmds = drain(&rx);
    assert_eq!(
        count_cmd(&cmds, |c| matches!(c, AudioCommand::ImportAudioToPool { .. })),
        0,
        "no import issued without a project directory"
    );
    assert_eq!(app.test_pending_import_count(), 0);
}
