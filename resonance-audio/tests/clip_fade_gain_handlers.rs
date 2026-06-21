//! Tests for the audio-clip fade/gain command boundary (todo #314).
//!
//! Drives the engine-internal pure helpers `set_clip_fade_in_place` /
//! `set_clip_gain_in_place` directly via the `#[doc(hidden)]` re-exports.
//! That keeps the test headless — no cpal stream, no engine thread, no
//! audio device — while exercising the exact mutation + clamping + event
//! emission the `AudioCommand::SetClipFade` / `AudioCommand::SetClipGain`
//! dispatch path runs.

use std::sync::Arc;

use crossbeam_channel::unbounded;
use parking_lot::RwLock;

use resonance_audio::types::{AudioClip, AudioEvent, ClipSource, FadeCurve};
use resonance_audio::{
    set_clip_fade_in_place, set_clip_gain_in_place, MAX_CLIP_GAIN_DB, MIN_CLIP_GAIN_DB,
};

/// Build an in-RAM audio clip with `frames` stereo frames of silence.
fn sample_clip(id: u64, track_id: u64, frames: usize) -> AudioClip {
    AudioClip {
        id,
        track_id,
        start_sample: 0,
        source: ClipSource::Memory(vec![0.0f32; frames * 2]),
        name: "clip".into(),
        trim_start_frames: 0,
        trim_end_frames: 0,
        fade_in_frames: 0,
        fade_in_curve: FadeCurve::default(),
        fade_out_frames: 0,
        fade_out_curve: FadeCurve::default(),
        gain_db: 0.0,
        vocal_tuning: None,
    }
}

#[test]
fn set_fade_mutates_and_emits_event() {
    let clips: Arc<RwLock<Vec<AudioClip>>> = Arc::new(RwLock::new(vec![sample_clip(7, 100, 1000)]));
    let (event_tx, event_rx) = unbounded::<AudioEvent>();

    set_clip_fade_in_place(
        &clips,
        &event_tx,
        /* clip_id */ 7,
        /* fade_in_frames */ 200,
        /* fade_in_curve */ FadeCurve::Linear,
        /* fade_out_frames */ 300,
        /* fade_out_curve */ FadeCurve::Exp,
    );

    match event_rx.try_recv() {
        Ok(AudioEvent::ClipFadeChanged {
            clip_id,
            fade_in_frames,
            fade_in_curve,
            fade_out_frames,
            fade_out_curve,
        }) => {
            assert_eq!(clip_id, 7);
            assert_eq!(fade_in_frames, 200);
            assert_eq!(fade_in_curve, FadeCurve::Linear);
            assert_eq!(fade_out_frames, 300);
            assert_eq!(fade_out_curve, FadeCurve::Exp);
        }
        other => panic!("expected ClipFadeChanged, got {other:?}"),
    }
    assert!(
        event_rx.try_recv().is_err(),
        "exactly one event should be emitted"
    );

    let clips = clips.read();
    assert_eq!(clips[0].fade_in_frames, 200);
    assert_eq!(clips[0].fade_in_curve, FadeCurve::Linear);
    assert_eq!(clips[0].fade_out_frames, 300);
    assert_eq!(clips[0].fade_out_curve, FadeCurve::Exp);
}

#[test]
fn set_fade_clamps_to_clip_duration() {
    // 500-frame clip; ask for fades far longer than the audible region.
    let clips: Arc<RwLock<Vec<AudioClip>>> = Arc::new(RwLock::new(vec![sample_clip(1, 100, 500)]));
    let (event_tx, event_rx) = unbounded::<AudioEvent>();

    set_clip_fade_in_place(
        &clips,
        &event_tx,
        1,
        /* fade_in_frames */ 10_000,
        FadeCurve::EqualPower,
        /* fade_out_frames */ 10_000,
        FadeCurve::EqualPower,
    );

    match event_rx.try_recv() {
        Ok(AudioEvent::ClipFadeChanged {
            fade_in_frames,
            fade_out_frames,
            ..
        }) => {
            assert_eq!(fade_in_frames, 500, "fade-in clamped to clip duration");
            assert_eq!(fade_out_frames, 500, "fade-out clamped to clip duration");
        }
        other => panic!("expected ClipFadeChanged, got {other:?}"),
    }

    let clips = clips.read();
    assert_eq!(clips[0].fade_in_frames, 500);
    assert_eq!(clips[0].fade_out_frames, 500);
}

#[test]
fn set_fade_missing_clip_emits_no_event() {
    let clips: Arc<RwLock<Vec<AudioClip>>> = Arc::new(RwLock::new(vec![sample_clip(1, 100, 1000)]));
    let (event_tx, event_rx) = unbounded::<AudioEvent>();

    set_clip_fade_in_place(
        &clips,
        &event_tx,
        /* clip_id */ 999,
        100,
        FadeCurve::Linear,
        100,
        FadeCurve::Linear,
    );

    assert!(
        event_rx.try_recv().is_err(),
        "ClipFadeChanged must not be emitted when the clip lookup misses"
    );
    let clips = clips.read();
    assert_eq!(clips[0].fade_in_frames, 0);
    assert_eq!(clips[0].fade_out_frames, 0);
}

#[test]
fn set_gain_mutates_and_emits_event() {
    let clips: Arc<RwLock<Vec<AudioClip>>> = Arc::new(RwLock::new(vec![sample_clip(7, 100, 1000)]));
    let (event_tx, event_rx) = unbounded::<AudioEvent>();

    set_clip_gain_in_place(&clips, &event_tx, 7, -6.0);

    match event_rx.try_recv() {
        Ok(AudioEvent::ClipGainChanged { clip_id, gain_db }) => {
            assert_eq!(clip_id, 7);
            assert_eq!(gain_db, -6.0);
        }
        other => panic!("expected ClipGainChanged, got {other:?}"),
    }
    assert!(
        event_rx.try_recv().is_err(),
        "exactly one event should be emitted"
    );
    assert_eq!(clips.read()[0].gain_db, -6.0);
}

#[test]
fn set_gain_clamps_to_range() {
    let clips: Arc<RwLock<Vec<AudioClip>>> = Arc::new(RwLock::new(vec![sample_clip(1, 100, 1000)]));
    let (event_tx, event_rx) = unbounded::<AudioEvent>();

    // Way over the ceiling.
    set_clip_gain_in_place(&clips, &event_tx, 1, 1000.0);
    match event_rx.try_recv() {
        Ok(AudioEvent::ClipGainChanged { gain_db, .. }) => {
            assert_eq!(gain_db, MAX_CLIP_GAIN_DB);
        }
        other => panic!("expected ClipGainChanged, got {other:?}"),
    }
    assert_eq!(clips.read()[0].gain_db, MAX_CLIP_GAIN_DB);

    // Way under the floor.
    set_clip_gain_in_place(&clips, &event_tx, 1, -1000.0);
    match event_rx.try_recv() {
        Ok(AudioEvent::ClipGainChanged { gain_db, .. }) => {
            assert_eq!(gain_db, MIN_CLIP_GAIN_DB);
        }
        other => panic!("expected ClipGainChanged, got {other:?}"),
    }
    assert_eq!(clips.read()[0].gain_db, MIN_CLIP_GAIN_DB);
}

#[test]
fn set_gain_nan_falls_back_to_unity() {
    let clips: Arc<RwLock<Vec<AudioClip>>> = Arc::new(RwLock::new(vec![sample_clip(1, 100, 1000)]));
    let (event_tx, event_rx) = unbounded::<AudioEvent>();

    set_clip_gain_in_place(&clips, &event_tx, 1, f32::NAN);
    match event_rx.try_recv() {
        Ok(AudioEvent::ClipGainChanged { gain_db, .. }) => {
            assert_eq!(gain_db, 0.0, "NaN gain falls back to unity (0 dB)");
        }
        other => panic!("expected ClipGainChanged, got {other:?}"),
    }
    assert_eq!(clips.read()[0].gain_db, 0.0);
}

#[test]
fn set_gain_missing_clip_emits_no_event() {
    let clips: Arc<RwLock<Vec<AudioClip>>> = Arc::new(RwLock::new(vec![sample_clip(1, 100, 1000)]));
    let (event_tx, event_rx) = unbounded::<AudioEvent>();

    set_clip_gain_in_place(&clips, &event_tx, /* clip_id */ 999, -3.0);

    assert!(
        event_rx.try_recv().is_err(),
        "ClipGainChanged must not be emitted when the clip lookup misses"
    );
    assert_eq!(clips.read()[0].gain_db, 0.0);
}
