//! Integration tests for the freeze command/event plumbing (todo #572,
//! doc #187): the `to_freeze_cache_spawn` worker that drives the offline
//! freeze renderer behind `AudioCommand::FreezeTrack` and the
//! `freeze_terminal_event` mapping that classifies its outcome into the
//! `Freeze*` event family.
//!
//! These cover the boundary that the engine `dispatch` arms delegate to;
//! the full engine command loop needs a live audio device and so isn't
//! exercised headless (see `engine_send_disconnected.rs` for the pattern).
//! The frozen-source attach/detach commands are validated at the
//! `Track::frozen_source` field they mutate.

use std::sync::Arc;

use crossbeam_channel::{unbounded, Receiver};
use indexmap::IndexMap;
use parking_lot::{Mutex, RwLock};

use resonance_audio::__test_support::{
    freeze_terminal_event, to_freeze_cache_spawn, SharedState, SyncClapInstance,
    FREEZE_CANCELLED_MSG,
};
use resonance_audio::types::*;
use resonance_common::{FreezeCacheRef, FreezeCacheStatus};

const SR: u32 = 48_000;

struct EngineState {
    shared: Arc<SharedState>,
    tracks: Arc<RwLock<IndexMap<TrackId, Track>>>,
    busses: Arc<RwLock<IndexMap<BusId, Bus>>>,
    master: Arc<RwLock<MasterBus>>,
    clips: Arc<RwLock<Vec<AudioClip>>>,
    midi_clips: Arc<RwLock<Vec<MidiClip>>>,
    plugins: Arc<RwLock<IndexMap<PluginInstanceId, Mutex<SyncClapInstance>>>>,
    tempo_map: Arc<arc_swap::ArcSwap<TempoMap>>,
}

fn empty_engine_state() -> EngineState {
    EngineState {
        shared: Arc::new(SharedState::default()),
        tracks: Arc::new(RwLock::new(IndexMap::new())),
        busses: Arc::new(RwLock::new(IndexMap::new())),
        master: Arc::new(RwLock::new(MasterBus::new())),
        clips: Arc::new(RwLock::new(Vec::new())),
        midi_clips: Arc::new(RwLock::new(Vec::new())),
        plugins: Arc::new(RwLock::new(IndexMap::new())),
        tempo_map: Arc::new(arc_swap::ArcSwap::from_pointee(TempoMap::default())),
    }
}

/// Stereo interleaved 220 Hz sine, `frames` long at amplitude 0.5.
fn tone(frames: usize) -> Vec<f32> {
    let mut data = Vec::with_capacity(frames * 2);
    for i in 0..frames {
        let s = (i as f32 * 220.0 * std::f32::consts::TAU / SR as f32).sin() * 0.5;
        data.push(s);
        data.push(s);
    }
    data
}

fn audio_clip(id: ClipId, track_id: TrackId, data: Vec<f32>) -> AudioClip {
    AudioClip {
        id,
        track_id,
        start_sample: 0,
        source: ClipSource::Memory(data),
        name: "tone".into(),
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

/// Engine state with a single audio track (id 1) carrying a 1-second tone.
fn state_with_tone_track() -> EngineState {
    let state = empty_engine_state();
    state
        .tracks
        .write()
        .insert(1, Track::with_type(1, "track".into(), TrackType::Audio));
    state.clips.write().push(audio_clip(1, 1, tone(SR as usize)));
    state
}

fn tmp_path(name: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!("resonance_freeze_cmd_test_{name}.wav"))
}

/// Spawn a freeze and drain its event stream until the terminal event
/// (`FreezeCompleted` / `FreezeCancelled` / `FreezeError`) arrives,
/// returning the progress fractions seen and the terminal event.
fn drive_freeze(
    track_id: TrackId,
    path: &std::path::Path,
    state: &EngineState,
) -> (Vec<f32>, AudioEvent) {
    let (tx, rx): (_, Receiver<AudioEvent>) = unbounded();
    to_freeze_cache_spawn(
        track_id,
        path.to_string_lossy().into_owned(),
        Arc::clone(&state.shared),
        Arc::clone(&state.tracks),
        Arc::clone(&state.busses),
        Arc::clone(&state.master),
        Arc::clone(&state.clips),
        Arc::clone(&state.midi_clips),
        Arc::clone(&state.plugins),
        Arc::clone(&state.tempo_map),
        SR,
        tx,
    );

    let mut fractions = Vec::new();
    loop {
        let ev = rx
            .recv_timeout(std::time::Duration::from_secs(10))
            .expect("freeze worker must emit a terminal event");
        match ev {
            AudioEvent::FreezeProgress { track_id: tid, fraction } => {
                assert_eq!(tid, track_id, "progress must carry the frozen track id");
                fractions.push(fraction);
            }
            terminal => return (fractions, terminal),
        }
    }
}

#[test]
fn freeze_track_command_emits_progress_then_completed_with_cache_file() {
    let state = state_with_tone_track();
    let path = tmp_path("completed");
    let _ = std::fs::remove_file(&path);

    let (fractions, terminal) = drive_freeze(1, &path, &state);

    // Progress brackets the render: 0.0 up front, 1.0 at the end.
    assert_eq!(fractions.first().copied(), Some(0.0));
    assert_eq!(fractions.last().copied(), Some(1.0));

    match terminal {
        AudioEvent::FreezeCompleted { track_id, cache_ref } => {
            assert_eq!(track_id, 1);
            assert_eq!(cache_ref.sample_rate, SR);
            assert_eq!(cache_ref.bit_depth, 32);
            assert!(cache_ref.is_valid(), "fresh cache must be Frozen/valid");
            assert_ne!(cache_ref.render_fingerprint, 0);
            assert_eq!(
                cache_ref.cache_filename,
                path.file_name().unwrap().to_string_lossy()
            );
        }
        other => panic!("expected FreezeCompleted, got {other:?}"),
    }

    assert!(path.exists(), "completed freeze must leave the cache WAV");
    let _ = std::fs::remove_file(&path);
}

#[test]
fn freeze_missing_track_emits_freeze_error() {
    let state = empty_engine_state();
    let path = tmp_path("missing");
    let _ = std::fs::remove_file(&path);

    let (_fractions, terminal) = drive_freeze(42, &path, &state);

    match terminal {
        AudioEvent::FreezeError { track_id, message } => {
            assert_eq!(track_id, 42);
            assert!(message.contains("not found"), "got: {message}");
        }
        other => panic!("expected FreezeError, got {other:?}"),
    }
    assert!(!path.exists(), "errored freeze must not leave a file");
}

#[test]
fn terminal_event_maps_cancel_sentinel_to_freeze_cancelled() {
    // The renderer returns `FREEZE_CANCELLED_MSG` on cooperative cancel
    // (proven end-to-end in freeze_render_core.rs); the worker must turn
    // that into `FreezeCancelled`, not `FreezeError`.
    let ev = freeze_terminal_event(7, Err(FREEZE_CANCELLED_MSG.to_string()));
    assert!(
        matches!(ev, AudioEvent::FreezeCancelled { track_id: 7 }),
        "cancel sentinel must map to FreezeCancelled, got {ev:?}"
    );
}

#[test]
fn terminal_event_maps_other_errors_to_freeze_error() {
    let ev = freeze_terminal_event(9, Err("disk full".to_string()));
    match ev {
        AudioEvent::FreezeError { track_id, message } => {
            assert_eq!(track_id, 9);
            assert_eq!(message, "disk full");
        }
        other => panic!("expected FreezeError, got {other:?}"),
    }
}

#[test]
fn terminal_event_maps_ok_to_freeze_completed() {
    let cache_ref = FreezeCacheRef::new("t.wav".into(), SR, 32, 123, FreezeCacheStatus::Frozen);
    let ev = freeze_terminal_event(3, Ok(cache_ref.clone()));
    match ev {
        AudioEvent::FreezeCompleted { track_id, cache_ref: got } => {
            assert_eq!(track_id, 3);
            assert_eq!(got, cache_ref);
        }
        other => panic!("expected FreezeCompleted, got {other:?}"),
    }
}

#[test]
fn set_track_frozen_source_attaches_and_unfreeze_detaches() {
    // `SetTrackFrozenSource { source }` / `UnfreezeTrack` mutate this
    // `ArcSwapOption` field; validate the attach/detach contract on it.
    let track = Track::with_type(1, "track".into(), TrackType::Audio);
    assert!(
        track.frozen_source.load().is_none(),
        "a fresh track must start with no frozen source"
    );

    let cache_ref = FreezeCacheRef::new("c.wav".into(), SR, 32, 42, FreezeCacheStatus::Frozen);
    let samples = Arc::new(tone(SR as usize));
    let frame_count = samples.len() as u64 / 2;
    let source = FrozenSource::new(cache_ref.clone(), samples, SR, frame_count);

    // Attach (SetTrackFrozenSource { source: Some(..) }).
    track.frozen_source.store(Some(Arc::new(source)));
    let attached = track.frozen_source.load();
    let attached = attached.as_ref().expect("source must be attached");
    assert_eq!(attached.cache_ref, cache_ref);
    assert_eq!(attached.frame_count, frame_count);

    // Detach (UnfreezeTrack / SetTrackFrozenSource { source: None }).
    track.frozen_source.store(None);
    assert!(
        track.frozen_source.load().is_none(),
        "unfreeze must detach the frozen source"
    );
}
