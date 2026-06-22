//! Tests for the audio-clip warp command boundary (todo #418).
//!
//! Drives the engine-internal pure helpers `set_clip_warp_in_place` /
//! `set_clip_warp_markers_in_place` directly via the `#[doc(hidden)]`
//! re-exports. That keeps the test headless — no cpal stream, no engine
//! thread, no audio device — while exercising the exact mutation + event
//! emission (and marker-sort invariant) the `AudioCommand::SetClipWarp` /
//! `AudioCommand::SetClipWarpMarkers` dispatch path runs.

use std::sync::Arc;

use crossbeam_channel::unbounded;
use parking_lot::RwLock;

use resonance_audio::types::{AudioClip, AudioEvent, ClipSource, WarpAlgorithm, WarpMarker};
use resonance_audio::{set_clip_warp_in_place, set_clip_warp_markers_in_place};

/// Build an in-RAM audio clip with `frames` stereo frames of silence and
/// the default (no-warp) settings.
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
        fade_in_curve: Default::default(),
        fade_out_frames: 0,
        fade_out_curve: Default::default(),
        gain_db: 0.0,
        vocal_tuning: None,
        warp_enabled: false,
        original_bpm: None,
        transpose_semitones: 0.0,
        warp_algorithm: WarpAlgorithm::Transient,
        warp_markers: Vec::new(),
    }
}

#[test]
fn set_warp_mutates_and_emits_event() {
    let clips: Arc<RwLock<Vec<AudioClip>>> = Arc::new(RwLock::new(vec![sample_clip(7, 100, 1000)]));
    let (event_tx, event_rx) = unbounded::<AudioEvent>();

    set_clip_warp_in_place(
        &clips,
        &event_tx,
        /* clip_id */ 7,
        /* warp_enabled */ true,
        /* original_bpm */ Some(120.0),
        /* transpose_semitones */ -3.0,
        /* warp_algorithm */ WarpAlgorithm::Tonal,
    );

    match event_rx.try_recv() {
        Ok(AudioEvent::ClipWarpChanged {
            clip_id,
            warp_enabled,
            original_bpm,
            transpose_semitones,
            warp_algorithm,
        }) => {
            assert_eq!(clip_id, 7);
            assert!(warp_enabled);
            assert_eq!(original_bpm, Some(120.0));
            assert_eq!(transpose_semitones, -3.0);
            assert_eq!(warp_algorithm, WarpAlgorithm::Tonal);
        }
        other => panic!("expected ClipWarpChanged, got {other:?}"),
    }
    assert!(
        event_rx.try_recv().is_err(),
        "exactly one event should be emitted"
    );

    let clips = clips.read();
    assert!(clips[0].warp_enabled);
    assert_eq!(clips[0].original_bpm, Some(120.0));
    assert_eq!(clips[0].transpose_semitones, -3.0);
    assert_eq!(clips[0].warp_algorithm, WarpAlgorithm::Tonal);
}

#[test]
fn set_warp_missing_clip_emits_no_event() {
    let clips: Arc<RwLock<Vec<AudioClip>>> = Arc::new(RwLock::new(vec![sample_clip(1, 100, 1000)]));
    let (event_tx, event_rx) = unbounded::<AudioEvent>();

    set_clip_warp_in_place(
        &clips,
        &event_tx,
        /* clip_id */ 999,
        true,
        Some(90.0),
        0.0,
        WarpAlgorithm::Tonal,
    );

    assert!(
        event_rx.try_recv().is_err(),
        "ClipWarpChanged must not be emitted when the clip lookup misses"
    );
    // The real clip is untouched at its defaults.
    let clips = clips.read();
    assert!(!clips[0].warp_enabled);
    assert_eq!(clips[0].original_bpm, None);
    assert_eq!(clips[0].warp_algorithm, WarpAlgorithm::Transient);
}

#[test]
fn set_warp_markers_mutates_and_emits_event() {
    let clips: Arc<RwLock<Vec<AudioClip>>> = Arc::new(RwLock::new(vec![sample_clip(7, 100, 1000)]));
    let (event_tx, event_rx) = unbounded::<AudioEvent>();

    let markers = vec![
        WarpMarker {
            source_frame: 0,
            timeline_beat: 0.0,
        },
        WarpMarker {
            source_frame: 44_100,
            timeline_beat: 1.0,
        },
    ];

    set_clip_warp_markers_in_place(&clips, &event_tx, 7, markers.clone());

    match event_rx.try_recv() {
        Ok(AudioEvent::ClipWarpMarkersChanged {
            clip_id,
            markers: emitted,
        }) => {
            assert_eq!(clip_id, 7);
            assert_eq!(emitted, markers);
        }
        other => panic!("expected ClipWarpMarkersChanged, got {other:?}"),
    }
    assert!(
        event_rx.try_recv().is_err(),
        "exactly one event should be emitted"
    );
    assert_eq!(clips.read()[0].warp_markers, markers);
}

#[test]
fn set_warp_markers_sorts_by_timeline_beat() {
    let clips: Arc<RwLock<Vec<AudioClip>>> = Arc::new(RwLock::new(vec![sample_clip(7, 100, 1000)]));
    let (event_tx, event_rx) = unbounded::<AudioEvent>();

    // Deliberately out of order on `timeline_beat`.
    let unsorted = vec![
        WarpMarker {
            source_frame: 88_200,
            timeline_beat: 2.0,
        },
        WarpMarker {
            source_frame: 0,
            timeline_beat: 0.0,
        },
        WarpMarker {
            source_frame: 44_100,
            timeline_beat: 1.0,
        },
    ];

    set_clip_warp_markers_in_place(&clips, &event_tx, 7, unsorted);

    let expected_beats = [0.0, 1.0, 2.0];
    match event_rx.try_recv() {
        Ok(AudioEvent::ClipWarpMarkersChanged { markers, .. }) => {
            let beats: Vec<f64> = markers.iter().map(|m| m.timeline_beat).collect();
            assert_eq!(beats, expected_beats, "emitted markers sorted ascending");
        }
        other => panic!("expected ClipWarpMarkersChanged, got {other:?}"),
    }

    let stored: Vec<f64> = clips.read()[0]
        .warp_markers
        .iter()
        .map(|m| m.timeline_beat)
        .collect();
    assert_eq!(stored, expected_beats, "stored markers sorted ascending");
}

#[test]
fn set_warp_markers_missing_clip_emits_no_event() {
    let clips: Arc<RwLock<Vec<AudioClip>>> = Arc::new(RwLock::new(vec![sample_clip(1, 100, 1000)]));
    let (event_tx, event_rx) = unbounded::<AudioEvent>();

    set_clip_warp_markers_in_place(
        &clips,
        &event_tx,
        /* clip_id */ 999,
        vec![WarpMarker {
            source_frame: 0,
            timeline_beat: 0.0,
        }],
    );

    assert!(
        event_rx.try_recv().is_err(),
        "ClipWarpMarkersChanged must not be emitted when the clip lookup misses"
    );
    assert!(clips.read()[0].warp_markers.is_empty());
}
