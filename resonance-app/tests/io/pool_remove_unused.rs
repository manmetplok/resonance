//! "Remove unused" in the media pool (W4, FU-A1a3): the Pool tab action
//! and the `pool.remove_unused` control method.
//!
//! Removing takes assets out of the pool index only. The pooled WAVs stay
//! in `audio/`, and the pool rides the undo snapshot, so one undo brings
//! every removed asset back intact.

use crate::common::roundtrip;

use resonance_app::message::{Message, PoolMessage, RelinkMessage};
use resonance_app::state::{AssetRef, ClipState, PoolAsset};
use resonance_app::Resonance;
use resonance_audio::types::{FadeCurve, TrackType};
use resonance_control::methods::pool::{self as pool_proto, RemoveUnusedParams, RemoveUnusedResult};
use resonance_control::{Request, Response};

const SR: u32 = 48_000;
const USED: u64 = 5;
const UNUSED: u64 = 6;
const MISSING_UNUSED: u64 = 7;

fn asset(id: u64, missing: bool) -> PoolAsset {
    PoolAsset {
        id,
        project_relative_path: format!("audio/asset_{id}.wav"),
        original_path: format!("/samples/take_{id}.wav"),
        format: resonance_common::AudioFormat::Wav,
        channels: 2,
        source_sample_rate: SR,
        duration_frames: SR as u64,
        thumbnail_peaks: Vec::new(),
        missing,
    }
}

fn clip_on(asset_id: u64) -> ClipState {
    ClipState {
        id: 10,
        track_id: 1,
        start_sample: 0,
        duration_samples: SR as u64,
        name: "take".into(),
        total_frames: SR as u64,
        trim_start_frames: 0,
        trim_end_frames: 0,
        fade_in_frames: 0,
        fade_in_curve: FadeCurve::default(),
        fade_out_frames: 0,
        fade_out_curve: FadeCurve::default(),
        gain_db: 0.0,
        waveform_peaks: Vec::new(),
        vocal_tuning: None,
        asset_ref: Some(AssetRef::new(asset_id)),
    }
}

/// A saved project with one used, one unused and one missing-and-unused
/// asset.
fn app_with_pool() -> (Resonance, tempfile::TempDir) {
    let (mut app, _task, _rx) = Resonance::new_for_test_with_capture();
    let dir = tempfile::tempdir().expect("temp project dir");
    app.test_set_project_path(dir.path().to_path_buf());
    app.test_set_active_project(true);
    app.test_set_sample_rate(SR);
    app.test_add_track(1, TrackType::Audio);
    app.test_add_pool_asset(asset(USED, false));
    app.test_add_pool_asset(asset(UNUSED, false));
    app.test_add_pool_asset(asset(MISSING_UNUSED, true));
    app.test_push_clip(clip_on(USED));
    app.test_relink_clip(10, Some(USED)); // refresh usage
    (app, dir)
}

fn pool_ids(app: &Resonance) -> Vec<u64> {
    app.test_pool().assets.iter().map(|a| a.id).collect()
}

#[test]
fn remove_unused_keeps_only_the_assets_a_clip_plays() {
    let (mut app, _dir) = app_with_pool();

    let _ = app.update(Message::Pool(PoolMessage::RemoveUnusedAssets));

    assert_eq!(pool_ids(&app), vec![USED]);
    assert_eq!(app.test_pool().usage_count(USED), 1);
}

#[test]
fn one_undo_brings_every_removed_asset_back() {
    let (mut app, _dir) = app_with_pool();

    let _ = app.update(Message::Pool(PoolMessage::RemoveUnusedAssets));
    assert_eq!(pool_ids(&app), vec![USED]);

    let _ = app.update(Message::Undo);
    assert_eq!(pool_ids(&app), vec![USED, UNUSED, MISSING_UNUSED]);
    let restored = app.test_pool_asset(UNUSED).expect("unused asset is back");
    assert_eq!(restored.original_path, "/samples/take_6.wav");

    let _ = app.update(Message::Redo);
    assert_eq!(pool_ids(&app), vec![USED]);
}

#[test]
fn the_confirmation_opens_only_with_something_to_remove() {
    let (mut app, _dir) = app_with_pool();

    let _ = app.update(Message::Pool(PoolMessage::ConfirmRemoveUnused(true)));
    assert!(app.test_browser().confirm_remove_unused);
    let _ = app.update(Message::Pool(PoolMessage::ConfirmRemoveUnused(false)));
    assert!(!app.test_browser().confirm_remove_unused);
    assert_eq!(pool_ids(&app).len(), 3, "cancel removes nothing");

    let _ = app.update(Message::Pool(PoolMessage::ConfirmRemoveUnused(true)));
    let _ = app.update(Message::Pool(PoolMessage::RemoveUnusedAssets));
    assert!(
        !app.test_browser().confirm_remove_unused,
        "removing closes the confirmation"
    );

    // Everything left is in use: there is nothing to confirm.
    let _ = app.update(Message::Pool(PoolMessage::ConfirmRemoveUnused(true)));
    assert!(!app.test_browser().confirm_remove_unused);
}

#[test]
fn a_relink_in_flight_keeps_its_asset() {
    let (mut app, _dir) = app_with_pool();
    // Starting a relink marks the asset in flight; the import task is
    // never run here.
    let _ = app.update(Message::Relink(RelinkMessage::Located(
        MISSING_UNUSED,
        Some(std::path::PathBuf::from("/samples/elsewhere/take_7.wav")),
    )));
    assert!(app.test_relink().is_in_flight(MISSING_UNUSED));

    let _ = app.update(Message::Pool(PoolMessage::RemoveUnusedAssets));

    assert_eq!(pool_ids(&app), vec![USED, MISSING_UNUSED]);
}

fn call(app: &mut Resonance, confirm: bool) -> Response {
    roundtrip(
        app,
        Request::new(1, pool_proto::REMOVE_UNUSED, &RemoveUnusedParams { confirm })
            .expect("params serialize"),
    )
}

#[test]
fn control_remove_unused_needs_confirmation_and_names_the_assets() {
    let (mut app, _dir) = app_with_pool();

    let refused = call(&mut app, false);
    let error = refused.error.expect("refused without confirm");
    assert!(error.message.contains("take_6"), "{}", error.message);
    assert!(error.message.contains("take_7"), "{}", error.message);
    assert_eq!(pool_ids(&app).len(), 3, "a refusal changes nothing");

    let done: RemoveUnusedResult = call(&mut app, true).result().expect("removes");
    let removed: Vec<u64> = done.removed.iter().map(|id| id.0).collect();
    assert_eq!(removed, vec![UNUSED, MISSING_UNUSED]);
    assert_eq!(pool_ids(&app), vec![USED]);
}

#[test]
fn control_remove_unused_with_nothing_unused_is_an_empty_success() {
    let (mut app, _dir) = app_with_pool();
    let _ = app.update(Message::Pool(PoolMessage::RemoveUnusedAssets));
    let undo_depth = app.test_undo_history().undo_len();

    // No confirm needed when there is nothing to remove.
    let done: RemoveUnusedResult = call(&mut app, false).result().expect("empty success");
    assert!(done.removed.is_empty());
    assert_eq!(app.test_undo_history().undo_len(), undo_depth, "nothing was recorded");
}
