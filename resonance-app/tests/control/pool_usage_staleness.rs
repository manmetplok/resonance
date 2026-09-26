//! Pool "used ×N" counts follow clip deletes, splits, track removal and
//! bar removal (review VIEW-30).
//!
//! `recompute_pool_usage` ran only on pool add/remove, relink, load and
//! undo replay, so the Pool badge and `pool.list`'s `usage_count` kept the
//! old number after the clip set changed any other way.

use resonance_app::state::{AssetRef, ClipState, PoolAsset};
use resonance_app::update::{arrangement, clips};
use resonance_app::Resonance;
use resonance_audio::types::{AudioEvent, FadeCurve, TrackType};

const SR: u32 = 48_000;
const ASSET: u64 = 5;

fn app_with_placed_clip() -> Resonance {
    let (mut app, _task, _rx) = Resonance::new_for_test_with_capture();
    app.test_set_active_project(true);
    app.test_set_sample_rate(SR);
    app.test_add_track(1, TrackType::Audio);
    app.test_add_pool_asset(PoolAsset {
        id: ASSET,
        project_relative_path: format!("audio/asset_{ASSET}.wav"),
        original_path: "/samples/loop.wav".into(),
        format: resonance_common::AudioFormat::Wav,
        channels: 2,
        source_sample_rate: SR,
        duration_frames: 2 * SR as u64,
        thumbnail_peaks: Vec::new(),
        missing: false,
    });
    app.test_push_clip(ClipState {
        id: 10,
        track_id: 1,
        start_sample: 0,
        duration_samples: 2 * SR as u64,
        name: "loop".into(),
        total_frames: 2 * SR as u64,
        trim_start_frames: 0,
        trim_end_frames: 0,
        fade_in_frames: 0,
        fade_in_curve: FadeCurve::default(),
        fade_out_frames: 0,
        fade_out_curve: FadeCurve::default(),
        gain_db: 0.0,
        waveform_peaks: Vec::new(),
        vocal_tuning: None,
        asset_ref: Some(AssetRef::new(ASSET)),
    });
    app.test_relink_clip(10, Some(ASSET));
    assert_eq!(app.test_pool().usage_count(ASSET), 1);
    app
}

#[test]
fn clip_delete_drops_the_usage_count() {
    let mut app = app_with_placed_clip();
    app.test_apply_engine_event(AudioEvent::ClipDeleted { clip_id: 10 });
    assert_eq!(app.test_pool().usage_count(ASSET), 0);
}

#[test]
fn clip_split_counts_both_halves() {
    let mut app = app_with_placed_clip();
    clips::split_clip_at(&mut app, 10, 11, SR as u64);
    assert_eq!(app.test_pool().usage_count(ASSET), 2);
}

#[test]
fn track_removal_drops_its_clips_usage() {
    let mut app = app_with_placed_clip();
    app.test_apply_engine_event(AudioEvent::TrackRemoved { track_id: 1 });
    assert_eq!(app.test_pool().usage_count(ASSET), 0);
}

#[test]
fn bar_removal_drops_deleted_clips_usage() {
    let mut app = app_with_placed_clip();
    arrangement::remove_bars(&mut app, 1, 1);
    assert!(app.test_clips().is_empty(), "the clip starting in bar 1 is removed");
    assert_eq!(app.test_pool().usage_count(ASSET), 0);
}
