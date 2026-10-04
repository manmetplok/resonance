//! Relinking a single audio clip whose own WAV is missing (W4, FU-A1a3).
//!
//! Every audio clip plays its own `audio/clip_<id>.wav`. When that file is
//! gone at load (the project moved without its `audio/` folder) the clip
//! stays in the timeline, silent. A missing *pool asset* is relinked by the
//! existing asset flow; a clip with no missing asset behind it (a recorded
//! take, a bounce, a lost copy) gets its own row in the relink modal, whose
//! `Locate…` imports the chosen file as a new pool asset, points the clip
//! at it (`relink_clip`) and reloads it. One undo takes it back.

use crate::common;

use std::path::Path;

use iced::Size;
use iced_test::simulator::Simulator;
use resonance_app::message::{Message, RelinkMessage};
use resonance_app::state::{ClipState, PoolAsset, ViewMode};
use resonance_app::{demo, theme, Resonance};
use resonance_audio::test_support::Receiver;
use resonance_audio::types::{AudioCommand, AudioEvent, ClipId, FadeCurve, TrackType};

const RATE: u32 = 48_000;
const TRACK: u64 = 10;
const CLIP: ClipId = 100;

fn clip(id: ClipId) -> ClipState {
    ClipState {
        id,
        track_id: TRACK,
        start_sample: 96_000,
        duration_samples: 40_000,
        name: "Vocal take 3".into(),
        total_frames: 48_000,
        trim_start_frames: 8_000,
        trim_end_frames: 0,
        fade_in_frames: 0,
        fade_in_curve: FadeCurve::default(),
        fade_out_frames: 0,
        fade_out_curve: FadeCurve::default(),
        gain_db: 0.0,
        waveform_peaks: Vec::new(),
        vocal_tuning: None,
        asset_ref: None,
    }
}

fn write_source_wav(path: &Path, frames: usize) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("create source parent");
    }
    let samples: Vec<f32> = (0..frames * 2).map(|i| ((i % 64) as f32 / 64.0) - 0.5).collect();
    resonance_audio::transcode_to_wav(path, &samples, RATE).expect("write source wav");
}

/// An app on a saved project at `dir` whose clip 100 was restored from a
/// snapshot while `audio/clip_100.wav` does not exist — what a load of a
/// project that lost its `audio/` folder looks like.
fn app_with_missing_clip(dir: &Path) -> (Resonance, Receiver<AudioCommand>) {
    let (mut app, _task, rx) = Resonance::new_for_test_with_capture();
    app.test_set_active_project(true);
    app.test_set_project_path(dir.to_path_buf());
    app.test_set_sample_rate(RATE);
    app.test_add_track(TRACK, TrackType::Audio);
    app.test_push_clip(clip(CLIP));
    let snapshot = app.test_snapshot_for_undo();
    app.test_apply_engine_event(AudioEvent::ClipDeleted { clip_id: CLIP });
    assert!(app.test_clips().is_empty());
    app.test_begin_restore_from_snapshot(snapshot);
    assert_eq!(app.test_clips().len(), 1, "the clip is kept, offline");
    let _ = rx.try_iter().count();
    (app, rx)
}

#[test]
fn a_clip_restored_without_its_wav_is_flagged_missing() {
    let dir = tempfile::tempdir().expect("temp dir");
    let (mut app, _rx) = app_with_missing_clip(dir.path());

    assert!(app.test_relink().missing_clips.contains(&CLIP));
    app.test_dispatch(Message::Relink(RelinkMessage::ShowModal));
    assert!(app.test_relink().modal_open);
    assert_eq!(app.test_relink().modal_clip_targets, vec![CLIP]);
}

#[test]
fn a_clip_whose_wav_is_present_is_not_flagged() {
    let dir = tempfile::tempdir().expect("temp dir");
    write_source_wav(&dir.path().join(format!("audio/clip_{CLIP}.wav")), 48_000);
    let (app, _rx) = app_with_missing_clip(dir.path());

    assert!(app.test_relink().missing_clips.is_empty());
}

#[test]
fn locating_a_missing_clip_points_it_at_a_new_asset_and_reloads_it() {
    let dir = tempfile::tempdir().expect("temp dir");
    let (mut app, rx) = app_with_missing_clip(dir.path());
    let src = dir.path().join("elsewhere/take3.wav");
    write_source_wav(&src, 6_000);

    // The picker's answer starts the import (its task is not run here).
    let _ = app.update(Message::Relink(RelinkMessage::ClipLocated(CLIP, Some(src.clone()))));
    assert!(app.test_relink().is_clip_in_flight(CLIP));

    // What the worker hands back.
    let outcome =
        resonance_audio::import_one_to_pool(900, &src.to_string_lossy(), dir.path(), RATE)
            .expect("import");
    let _ = app.update(Message::Relink(RelinkMessage::ClipImported(CLIP, Ok(outcome))));

    assert!(!app.test_relink().is_clip_in_flight(CLIP));
    assert!(app.test_relink().missing_clips.is_empty(), "no longer missing");
    let asset = app.test_pool_asset(900).expect("the replacement is pooled");
    assert!(!asset.missing);
    assert_eq!(asset.original_path, src.to_string_lossy());
    let relinked = &app.test_clips()[0];
    assert_eq!(relinked.asset_ref.map(|a| a.asset_id), Some(900));
    assert_eq!(app.test_pool().usage_count(900), 1);
    // The new file is shorter than the old trims cover: they are dropped.
    assert_eq!(relinked.total_frames, 6_000);
    assert_eq!(relinked.trim_start_frames, 0);
    assert_eq!(relinked.duration_samples, 6_000);

    let reload = rx
        .try_iter()
        .find_map(|c| match c {
            AudioCommand::LoadClipFromWav {
                clip_id,
                path,
                start_sample,
                ..
            } if clip_id == CLIP => Some((path, start_sample)),
            _ => None,
        })
        .expect("the clip is reloaded");
    assert_eq!(reload.0, dir.path().join("audio/asset_900.wav"));
    assert_eq!(reload.1, 96_000, "it keeps its place");

    // One undo takes the relink back.
    let _ = app.update(Message::Undo);
    assert!(app.test_pool_asset(900).is_none());
    assert_eq!(app.test_clips()[0].asset_ref, None);
}

#[test]
fn a_failed_clip_import_reports_and_clears_the_in_flight_mark() {
    let dir = tempfile::tempdir().expect("temp dir");
    let (mut app, _rx) = app_with_missing_clip(dir.path());
    let _ = app.update(Message::Relink(RelinkMessage::ClipLocated(
        CLIP,
        Some(dir.path().join("nope.wav")),
    )));
    let _ = app.update(Message::Relink(RelinkMessage::ClipImported(
        CLIP,
        Err(resonance_app::message::RelinkError {
            asset_id: 900,
            path: "nope.wav".into(),
            reason: "no such file".into(),
        }),
    )));

    assert!(!app.test_relink().any_in_flight());
    assert!(app.test_relink().missing_clips.contains(&CLIP), "still missing");
    assert!(app
        .test_relink()
        .last_error
        .as_deref()
        .is_some_and(|e| e.contains("no such file")));
}

fn sim_settings() -> iced::Settings {
    let mut fonts: Vec<std::borrow::Cow<'static, [u8]>> = Vec::new();
    fonts.push(theme::ICON_FONT_BYTES.into());
    for face in theme::UI_FONT_FACES {
        fonts.push((*face).into());
    }
    iced::Settings {
        fonts,
        default_font: theme::UI_FONT,
        ..iced::Settings::default()
    }
}

/// The modal with one missing asset and one missing clip: the clip row
/// names the clip, its track and its WAV, with its own `Locate…`.
#[test]
fn relink_modal_with_a_missing_clip() {
    // A fixed, empty folder: its name is the window title in the golden.
    let dir = std::env::temp_dir().join("resonance-relink-demo");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("project dir");
    let (mut app, _task) = Resonance::new_for_test_on(ViewMode::Arrange);
    demo::seed_demo_content(&mut app);
    app.test_set_project_path(dir.clone());
    app.test_add_pool_asset(PoolAsset {
        id: 1,
        project_relative_path: "audio/asset_1.wav".into(),
        original_path: "/Users/max/Samples/Guitars/Old Guitar Loop.wav".into(),
        format: resonance_common::AudioFormat::Wav,
        channels: 2,
        source_sample_rate: RATE,
        duration_frames: 0,
        thumbnail_peaks: Vec::new(),
        missing: true,
    });
    // The demo's audio clip, restored without its WAV.
    let snapshot = app.test_snapshot_for_undo();
    let audio_clip = app.test_clips()[0].id;
    app.test_apply_engine_event(AudioEvent::ClipDeleted { clip_id: audio_clip });
    app.test_begin_restore_from_snapshot(snapshot);
    assert!(app.test_relink().missing_clips.contains(&audio_clip));
    app.test_dispatch(Message::Relink(RelinkMessage::ShowModal));

    let mut ui = Simulator::with_size(
        sim_settings(),
        Size::new(1440.0, 900.0),
        app.view(),
    );
    let snap = ui
        .snapshot(&theme::resonance_theme())
        .expect("snapshot should render");
    let _ = std::fs::remove_dir_all(&dir);
    common::assert_golden(&snap, "tests/snapshots/relink_modal_missing_clip.png");
}
