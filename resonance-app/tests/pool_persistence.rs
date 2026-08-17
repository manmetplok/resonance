//! Persistence coverage for the media pool + clip asset references
//! (doc #175, ba todo #596): serialize the imported-asset pool and each
//! placed clip's `asset_ref` into the on-disk `ProjectFile`, restore it
//! back, and prove an imported+placed project round-trips identically —
//! while a now-missing backing file degrades to `missing` (not dropped),
//! a legacy project without the pool field loads cleanly, and the
//! favourites / recent folders persist through user settings rather than
//! the project file.

use std::path::{Path, PathBuf};

use resonance_app::project::ProjectFile;
use resonance_app::state::{AssetRef, ClipState, PoolAsset};
use resonance_app::Resonance;
use resonance_audio::types::{AssetId, ClipId, FadeCurve, TrackId, TrackType};

/// A fresh app with an active project anchored at `project_dir`, so the
/// pool restore path can resolve project-relative asset paths and the
/// undo gate (which needs a saved path) is satisfied.
fn app_at(project_dir: &Path) -> Resonance {
    let (mut app, _task) = Resonance::new_for_test();
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

/// A pool asset whose backing WAV lives at `rel` inside the project dir.
fn asset(id: AssetId, rel: &str) -> PoolAsset {
    PoolAsset {
        id,
        project_relative_path: rel.to_string(),
        original_path: format!("/imports/source_{id}.flac"),
        format: resonance_common::AudioFormat::Flac,
        channels: 1,
        source_sample_rate: 44_100,
        duration_frames: 48_000,
        thumbnail_peaks: vec![(-0.4, 0.4), (-0.6, 0.6)],
        missing: false,
    }
}

/// Create the project dir + an empty backing WAV for each asset so the
/// restore path's existence check takes the "present" branch.
fn make_project_dir(tag: &str, asset_rels: &[&str]) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "resonance-pool-persist-{tag}-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(dir.join("audio")).expect("create audio dir");
    for rel in asset_rels {
        std::fs::write(dir.join(rel), b"").expect("write backing wav");
    }
    dir
}

#[test]
fn build_project_file_captures_pool_and_asset_refs() {
    let dir = make_project_dir("build", &["audio/asset_1.wav", "audio/asset_2.wav"]);
    let mut app = app_at(&dir);

    app.test_add_track(10, TrackType::Audio);
    app.test_add_pool_asset(asset(1, "audio/asset_1.wav"));
    app.test_add_pool_asset(asset(2, "audio/asset_2.wav"));
    // Two clips placed from asset 1, one from asset 2.
    app.test_push_clip(clip(100, 10, Some(1)));
    app.test_push_clip(clip(101, 10, Some(1)));
    app.test_push_clip(clip(102, 10, Some(2)));
    // Recompute usage now that clips reference assets.
    app.test_relink_clip(100, Some(1));

    let file = app.test_build_project_file();

    assert_eq!(file.pool_assets.len(), 2, "both assets serialized");
    assert_eq!(file.pool_assets[0].id, 1);
    assert_eq!(
        file.pool_assets[0].project_relative_path,
        "audio/asset_1.wav"
    );
    assert_eq!(file.pool_assets[0].original_path, "/imports/source_1.flac");
    assert_eq!(file.pool_assets[0].format, "flac", "format -> tag");
    assert_eq!(file.pool_assets[0].channels, 1);
    assert_eq!(file.pool_assets[0].source_sample_rate, 44_100);
    assert_eq!(file.pool_assets[0].duration_frames, 48_000);

    // Per-clip asset refs.
    let by_id = |id: u64| file.clips.iter().find(|c| c.id == id).unwrap();
    assert_eq!(by_id(100).asset_ref, Some(1));
    assert_eq!(by_id(101).asset_ref, Some(1));
    assert_eq!(by_id(102).asset_ref, Some(2));

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn imported_and_placed_audio_round_trips_identically() {
    let dir = make_project_dir("round", &["audio/asset_1.wav", "audio/asset_2.wav"]);

    // -- Author a project with imported + placed audio --------------
    let mut authored = app_at(&dir);
    authored.test_add_track(10, TrackType::Audio);
    authored.test_add_pool_asset(asset(1, "audio/asset_1.wav"));
    authored.test_add_pool_asset(asset(2, "audio/asset_2.wav"));
    authored.test_push_clip(clip(100, 10, Some(1)));
    authored.test_push_clip(clip(101, 10, Some(2)));
    authored.test_relink_clip(100, Some(1)); // refresh usage

    let saved = authored.test_build_project_file();

    // Persist through real JSON, exactly as a save/reload would.
    let json = serde_json::to_string(&saved).expect("serialize project");
    let reloaded: ProjectFile = serde_json::from_str(&json).expect("deserialize project");

    // -- Restore into a fresh app -----------------------------------
    // Rebuild the same non-pool structure a full project load would
    // (track + clips with their asset refs), then restore the pool from
    // the reloaded file. This mirrors what `replay_loaded_project` does:
    // clips come back with `asset_ref` set, and `restore_pool` rebuilds
    // the asset list + usage tally.
    let mut restored = app_at(&dir);
    restored.test_add_track(10, TrackType::Audio);
    restored.test_push_clip(clip(100, 10, reloaded.clips[0].asset_ref));
    restored.test_push_clip(clip(101, 10, reloaded.clips[1].asset_ref));
    restored.test_restore_pool(&reloaded, &dir);

    // Pool restored faithfully, nothing missing (files exist).
    let pool = restored.test_pool();
    assert_eq!(pool.assets.len(), 2);
    assert_eq!(pool.assets[0].id, 1);
    assert_eq!(pool.assets[0].project_relative_path, "audio/asset_1.wav");
    assert_eq!(pool.assets[0].original_path, "/imports/source_1.flac");
    assert_eq!(pool.assets[0].format, resonance_common::AudioFormat::Flac);
    assert_eq!(pool.assets[0].channels, 1);
    assert_eq!(pool.assets[0].source_sample_rate, 44_100);
    assert_eq!(pool.assets[0].duration_frames, 48_000);
    assert!(!pool.assets[0].missing, "present file is not missing");
    assert!(!pool.assets[1].missing);
    // Usage counts derived from the restored clips' refs.
    assert_eq!(pool.usage_count(1), 1);
    assert_eq!(pool.usage_count(2), 1);

    // Re-serializing the restored app yields a byte-identical durable
    // shape — the definition of a clean round-trip.
    let re_saved = restored.test_build_project_file();
    let re_json = serde_json::to_string(&re_saved).expect("re-serialize");
    assert_eq!(re_json, json, "project round-trips identically");

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn missing_backing_file_is_flagged_not_dropped() {
    // Project dir exists but asset_2's WAV is absent.
    let dir = make_project_dir("missing", &["audio/asset_1.wav"]);

    let file = ProjectFile {
        pool_assets: vec![
            resonance_app::project::ProjectPoolAsset {
                id: 1,
                project_relative_path: "audio/asset_1.wav".into(),
                original_path: "/imports/a.wav".into(),
                format: "wav".into(),
                channels: 2,
                source_sample_rate: 48_000,
                duration_frames: 24_000,
            },
            resonance_app::project::ProjectPoolAsset {
                id: 2,
                project_relative_path: "audio/asset_2.wav".into(),
                original_path: "/imports/b.wav".into(),
                format: "wav".into(),
                channels: 2,
                source_sample_rate: 48_000,
                duration_frames: 24_000,
            },
        ],
        ..ProjectFile::default()
    };

    let mut app = app_at(&dir);
    // A clip still references the now-missing asset 2; it must survive.
    app.test_push_clip(clip(200, 10, Some(2)));
    app.test_restore_pool(&file, &dir);

    let pool = app.test_pool();
    assert_eq!(pool.assets.len(), 2, "missing asset kept, not dropped");
    assert!(!pool.assets[0].missing, "asset 1 present");
    assert!(pool.assets[1].missing, "asset 2 flagged missing");
    // The dangling clip ref is preserved (the clip wasn't deleted); the
    // asset is still present in the pool, so it's counted.
    assert_eq!(
        pool.usage_count(2),
        1,
        "missing asset still tallies its clip"
    );
    assert_eq!(app.test_clips().len(), 1, "clip preserved offline");

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn legacy_project_without_pool_loads_clean() {
    // A project authored before the pool existed has no `pool_assets`
    // key and clips with no `asset_ref`. `#[serde(default)]` must fill
    // them in without error.
    let mut json = serde_json::to_value(ProjectFile::default()).unwrap();
    let obj = json.as_object_mut().unwrap();
    obj.remove("pool_assets");

    let file: ProjectFile = serde_json::from_value(json).expect("legacy project deserializes");
    assert!(file.pool_assets.is_empty());

    // A legacy clip object missing the `asset_ref` key deserializes with
    // `asset_ref: None`.
    let clip_json = serde_json::json!({
        "id": 1,
        "track_id": 1,
        "start_sample": 0,
        "name": "old clip",
        "total_frames": 1000,
        "trim_start_frames": 0,
        "trim_end_frames": 0,
        "audio_file": "audio/clip_1.wav"
    });
    let pc: resonance_app::project::ProjectClip =
        serde_json::from_value(clip_json).expect("legacy clip deserializes");
    assert_eq!(pc.asset_ref, None, "absent asset_ref defaults to None");

    // Restoring the empty legacy pool is a clean no-op.
    let dir = make_project_dir("legacy", &[]);
    let mut app = app_at(&dir);
    app.test_restore_pool(&file, &dir);
    assert!(app.test_pool().assets.is_empty());

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn favourites_and_recent_persist_via_settings_not_project() {
    let dir = make_project_dir("favs", &["audio/asset_1.wav"]);
    let mut app = app_at(&dir);

    let fav = PathBuf::from("/music/loops");
    let recent_a = PathBuf::from("/music/vox");
    let recent_b = PathBuf::from("/music/drums");

    // Mutate the pool's favourite / recent lists (as the browser
    // handlers would), then sync them into user settings. Use the
    // hermetic sync (not the disk-persisting variant) so the test never
    // touches the real config dir.
    app.test_pool_add_favourite(fav.clone());
    app.test_pool_push_recent(recent_a.clone());
    app.test_pool_push_recent(recent_b.clone());
    app.test_sync_media_browser_settings();

    // They land in app settings, project-independent.
    let media = &app.test_settings().media;
    assert_eq!(media.favourites, vec![fav.clone()]);
    // push_recent is most-recent-first.
    assert_eq!(
        media.recent_folders,
        vec![recent_b.clone(), recent_a.clone()]
    );

    // They must NOT leak into the project file.
    let file = app.test_build_project_file();
    let json = serde_json::to_string(&file).unwrap();
    assert!(
        !json.contains("/music/loops") && !json.contains("/music/vox"),
        "favourites / recent must not be written into the project file"
    );

    // The settings document survives a JSON round-trip across sessions.
    let s_json = serde_json::to_string(app.test_settings()).expect("serialize settings");
    let restored: resonance_app::settings::AppSettings =
        serde_json::from_str(&s_json).expect("deserialize settings");
    assert_eq!(restored.media.favourites, vec![fav]);
    assert_eq!(restored.media.recent_folders, vec![recent_b, recent_a]);

    let _ = std::fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------------------
// Asset-id collisions (ba doc #276 BUG 2)
// ---------------------------------------------------------------------------

/// An id arriving with a DIFFERENT file must not rebind the slot.
///
/// The engine's asset-id allocator is engine-thread-local and starts at 1
/// each session, while a loaded project's assets keep their saved ids, so
/// the first import after opening a project used to be handed an id the
/// project was already using. `MediaPool::add` replaced in place, and
/// every clip referencing that id silently started playing the newly
/// imported file: a clap arrived as id 4 carrying the ride's usage count
/// of 2, and two clips placed as rides were now claps. Nothing errored.
///
/// The allocator is fixed at the source (`ReserveAssetIds` on load); this
/// is the backstop that turns any remaining collision into a visible
/// refusal instead of silently wrong audio.
#[test]
fn an_id_collision_does_not_rebind_an_existing_asset() {
    let dir = make_project_dir("collide", &["audio/asset_1.wav"]);
    let mut app = app_at(&dir);
    app.test_add_pool_asset(asset(4, "audio/ride.wav"));

    // Same id, different file — an id collision.
    app.test_add_pool_asset(asset(4, "audio/clap.wav"));

    let assets = &app.test_pool().assets;
    assert_eq!(assets.len(), 1, "no second asset takes the id");
    assert_eq!(
        assets[0].project_relative_path, "audio/ride.wav",
        "the asset that owns the id keeps it, so clips pointing at 4 still play the ride"
    );
}

/// Re-importing the SAME file at the same id still refreshes it — that is
/// what the replace-in-place behaviour is for.
#[test]
fn re_importing_the_same_file_refreshes_the_asset() {
    let dir = make_project_dir("refresh", &["audio/asset_1.wav"]);
    let mut app = app_at(&dir);
    app.test_add_pool_asset(asset(4, "audio/ride.wav"));

    let mut refreshed = asset(4, "audio/ride.wav");
    refreshed.duration_frames = 96_000;
    app.test_add_pool_asset(refreshed);

    let assets = &app.test_pool().assets;
    assert_eq!(assets.len(), 1);
    assert_eq!(
        assets[0].duration_frames, 96_000,
        "the metadata refresh lands"
    );
}
