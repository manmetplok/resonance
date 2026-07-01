//! Missing-file relink coverage (doc #175, ba todo #600).
//!
//! When a loaded project references a pool asset whose backing WAV is
//! gone, `restore_pool` keeps the asset flagged `missing` so its clips
//! survive offline. These tests exercise the relink logic that resolves
//! the file again:
//!
//! * a missing asset loads with its clips preserved,
//! * a single-file relink refreshes the asset, clears the missing flag,
//!   re-copies the audio into the project folder, and reloads the clips so
//!   playback resumes,
//! * the batch folder search resolves every missing asset by filename
//!   (recursively, case-insensitively),
//! * a failed relink surfaces an error and never wedges the in-flight set,
//! * and an applied relink is classified as an undoable edit whose
//!   pre-relink snapshot restores the asset's prior source path.

use std::path::{Path, PathBuf};

use resonance_app::message::{Message, RelinkMessage};
use resonance_app::state::{AssetRef, ClipState, PoolAsset};
use resonance_app::update::relink::scan_folder_for_names;
use resonance_app::Resonance;
use resonance_audio::types::{AssetId, AudioCommand, ClipId, FadeCurve, TrackId, TrackType};

const RATE: u32 = 48_000;

/// A fresh app with an active project anchored at `project_dir` so the
/// relink handlers have a folder to copy into and the undo gate (which
/// needs a saved path) is satisfied.
fn app_at(project_dir: &Path) -> Resonance {
    let (mut app, _task) = Resonance::new();
    app.test_set_active_project(true);
    app.test_set_project_path(project_dir.to_path_buf());
    app
}

/// A bare audio clip on `track_id`, optionally linked to a pool asset.
fn clip(id: ClipId, track_id: TrackId, asset: Option<AssetId>) -> ClipState {
    ClipState {
        id,
        track_id,
        start_sample: 96_000,
        duration_samples: 48_000,
        name: format!("clip {id}"),
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
        asset_ref: asset.map(AssetRef::new),
    }
}

/// A pool asset flagged `missing`, whose original source lives at
/// `original_path` (a real file the relink can decode).
fn missing_asset(id: AssetId, original_path: &str) -> PoolAsset {
    PoolAsset {
        id,
        project_relative_path: format!("audio/asset_{id}.wav"),
        original_path: original_path.to_string(),
        format: resonance_common::AudioFormat::Wav,
        channels: 2,
        source_sample_rate: RATE,
        duration_frames: 0,
        thumbnail_peaks: Vec::new(),
        missing: true,
    }
}

/// A unique temp dir for one test, plus its `audio/` subfolder.
fn temp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("resonance-relink-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("audio")).expect("create audio dir");
    dir
}

/// Write a real, decodable stereo f32 WAV of `frames` non-silent frames.
fn write_source_wav(path: &Path, frames: usize) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("create source parent");
    }
    let mut samples = Vec::with_capacity(frames * 2);
    for i in 0..frames {
        let v = ((i % 64) as f32 / 64.0) - 0.5;
        samples.push(v); // L
        samples.push(v); // R
    }
    resonance_audio::transcode_to_wav(path, &samples, RATE).expect("write source wav");
}

/// Simulate the relink worker: run the same import the background task
/// would (`import_one_to_pool` copies/transcodes the source into the
/// project folder) and hand the outcome back the way the reducer expects.
fn run_worker(asset_id: AssetId, src: &Path, project_dir: &Path) -> RelinkMessage {
    let outcome =
        resonance_audio::import_one_to_pool(asset_id, &src.to_string_lossy(), project_dir, RATE)
            .expect("import_one_to_pool");
    RelinkMessage::Imported(Ok(outcome))
}

#[test]
fn missing_asset_loads_with_clips_preserved() {
    let dir = temp_dir("preserved");
    let mut app = app_at(&dir);
    app.test_add_track(10, TrackType::Audio);
    app.test_add_pool_asset(missing_asset(7, "/gone/loop.wav"));
    app.test_push_clip(clip(100, 10, Some(7)));
    app.test_relink_clip(100, Some(7)); // refresh usage

    let pool = app.test_pool();
    assert!(pool.has_missing(), "asset flagged missing");
    assert_eq!(pool.missing_assets().count(), 1);
    assert_eq!(app.test_clips().len(), 1, "clip preserved offline");
    assert_eq!(
        pool.usage_count(7),
        1,
        "missing asset still tallies its clip"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn single_file_relink_restores_asset_and_recopies() {
    let dir = temp_dir("single");
    let src = dir.join("sources/kick.wav");
    write_source_wav(&src, 4_800);

    let mut app = app_at(&dir);
    app.test_add_track(10, TrackType::Audio);
    // Two clips placed from the same missing asset must BOTH be reloaded.
    app.test_add_pool_asset(missing_asset(7, "/originally/here/kick.wav"));
    app.test_push_clip(clip(100, 10, Some(7)));
    app.test_push_clip(clip(101, 10, Some(7)));
    app.test_relink_clip(100, Some(7));

    // Run the worker (copies the audio into the project folder).
    let applied = run_worker(7, &src, &dir);
    let copied = dir.join("audio/asset_7.wav");
    assert!(
        copied.exists(),
        "relinked audio re-copied into project folder"
    );

    // Capture engine commands, then apply the relink outcome.
    let rx = app.test_capture_engine();
    app.test_dispatch(Message::Relink(applied));

    // Asset resolved: missing cleared, provenance + duration refreshed.
    let asset = app.test_pool_asset(7).expect("asset present");
    assert!(!asset.missing, "missing flag cleared after relink");
    assert_eq!(asset.original_path, src.to_string_lossy());
    assert_eq!(
        asset.duration_frames, 4_800,
        "duration from re-imported wav"
    );
    assert!(!asset.thumbnail_peaks.is_empty(), "thumbnail rebuilt");
    assert!(!app.test_relink().any_in_flight(), "in-flight cleared");

    // Both referencing clips were reloaded from the restored WAV so
    // playback resumes.
    let mut reloaded: Vec<ClipId> = Vec::new();
    while let Ok(cmd) = rx.try_recv() {
        if let AudioCommand::LoadClipFromWav { clip_id, path, .. } = cmd {
            assert_eq!(path, copied, "clip reloaded from the asset's project WAV");
            reloaded.push(clip_id);
        }
    }
    reloaded.sort_unstable();
    assert_eq!(reloaded, vec![100, 101], "every clip of the asset reloaded");

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn batch_folder_search_resolves_all_missing_by_name() {
    let dir = temp_dir("batch");
    // A search folder with the two lost files, one buried in a subfolder,
    // and a decoy with a non-matching name.
    let search = dir.join("search");
    write_source_wav(&search.join("kick.wav"), 2_400);
    write_source_wav(&search.join("nested/deeper/Snare.WAV"), 3_600); // case-insensitive
    write_source_wav(&search.join("unrelated.wav"), 1_200);

    let names = vec!["kick.wav".to_string(), "snare.wav".to_string()];
    let found = scan_folder_for_names(&search, &names);
    assert_eq!(found.len(), 2, "both missing files resolved by filename");
    assert_eq!(found.get("kick.wav"), Some(&search.join("kick.wav")));
    assert_eq!(
        found.get("snare.wav"),
        Some(&search.join("nested/deeper/Snare.WAV")),
        "recursive + case-insensitive match"
    );

    // Applying each resolved outcome relinks its asset and re-copies it.
    let mut app = app_at(&dir);
    app.test_add_track(10, TrackType::Audio);
    app.test_add_pool_asset(missing_asset(1, "/lost/kick.wav"));
    app.test_add_pool_asset(missing_asset(2, "/lost/snare.wav"));
    app.test_push_clip(clip(100, 10, Some(1)));
    app.test_push_clip(clip(101, 10, Some(2)));

    for (id, src) in [
        (1u64, found.get("kick.wav")),
        (2u64, found.get("snare.wav")),
    ] {
        let applied = run_worker(id, src.unwrap(), &dir);
        app.test_dispatch(Message::Relink(applied));
    }

    assert!(!app.test_pool().has_missing(), "no assets remain missing");
    assert!(dir.join("audio/asset_1.wav").exists());
    assert!(dir.join("audio/asset_2.wav").exists());

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn folder_search_leaves_unfound_assets_missing() {
    let dir = temp_dir("partial");
    let search = dir.join("search");
    write_source_wav(&search.join("kick.wav"), 2_400);

    // Only "kick.wav" exists in the folder; "ghost.wav" cannot resolve.
    let names = vec!["kick.wav".to_string(), "ghost.wav".to_string()];
    let found = scan_folder_for_names(&search, &names);
    assert_eq!(found.len(), 1);
    assert!(found.contains_key("kick.wav"));
    assert!(
        !found.contains_key("ghost.wav"),
        "absent file stays unresolved"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn failed_relink_surfaces_error_and_clears_in_flight() {
    let dir = temp_dir("error");
    let mut app = app_at(&dir);
    app.test_add_track(10, TrackType::Audio);
    app.test_add_pool_asset(missing_asset(7, "/gone/loop.wav"));

    app.test_dispatch(Message::Relink(RelinkMessage::Imported(Err(
        resonance_app::message::RelinkError {
            asset_id: 7,
            path: "/some/bad.wav".into(),
            reason: "decode failed".into(),
        },
    ))));

    let relink = app.test_relink();
    assert!(relink.last_error.is_some(), "error surfaced");
    assert!(
        relink
            .last_error
            .as_deref()
            .unwrap()
            .contains("decode failed"),
        "reason included"
    );
    assert!(!relink.any_in_flight(), "in-flight cleared on failure");
    // The asset stays missing — a failed relink resolves nothing.
    assert!(app.test_pool_asset(7).unwrap().missing);

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn applied_relink_is_undoable_and_snapshot_restores_prior_path() {
    let dir = temp_dir("undo");
    let src = dir.join("sources/kick.wav");
    write_source_wav(&src, 4_800);

    let mut app = app_at(&dir);
    app.test_add_track(10, TrackType::Audio);
    app.test_add_pool_asset(missing_asset(7, "/originally/here/kick.wav"));
    app.test_push_clip(clip(100, 10, Some(7)));

    // The applied relink is classified as a recordable (undoable) edit,
    // while opening the picker / a failed import are not.
    assert!(matches!(
        resonance_app::undo::classify(&Message::Relink(RelinkMessage::Locate(7))),
        resonance_app::undo::UndoAction::Skip
    ));
    let applied = run_worker(7, &src, &dir);
    assert!(matches!(
        resonance_app::undo::classify(&Message::Relink(applied.clone())),
        resonance_app::undo::UndoAction::Record
    ));

    // A pre-relink snapshot (what `record_undo` captures) still carries the
    // asset's OLD source path — so undoing the relink restores it.
    let snapshot = app.test_snapshot_for_undo();
    let snap_asset = snapshot
        .project
        .file
        .pool_assets
        .iter()
        .find(|a| a.id == 7)
        .expect("asset in snapshot");
    assert_eq!(
        snap_asset.original_path, "/originally/here/kick.wav",
        "snapshot preserves the pre-relink source path for undo"
    );

    // Apply the relink: the asset now points at the resolved source.
    app.test_dispatch(Message::Relink(applied));
    assert_eq!(
        app.test_pool_asset(7).unwrap().original_path,
        src.to_string_lossy()
    );
    assert!(!app.test_pool_asset(7).unwrap().missing);

    let _ = std::fs::remove_dir_all(&dir);
}
