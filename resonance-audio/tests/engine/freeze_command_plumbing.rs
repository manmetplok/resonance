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
use parking_lot::RwLock;

use resonance_audio::test_support::{AutomationSnapshot, FreezeError, SharedState, freeze_terminal_event, to_freeze_cache_spawn};
use resonance_audio::types::*;
use resonance_common::{FreezeCacheRef, FreezeCacheStatus};

const SR: u32 = 48_000;

struct EngineState {
    shared: Arc<SharedState>,
    clips: Arc<RwLock<Vec<AudioClip>>>,
    tempo_map: Arc<arc_swap::ArcSwap<TempoMap>>,
}

fn empty_engine_state() -> EngineState {
    EngineState {
        shared: Arc::new(SharedState::default()),
        clips: Arc::new(RwLock::new(Vec::new())),
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
        warp_enabled: false,
        original_bpm: None,
        transpose_semitones: 0.0,
        warp_algorithm: Default::default(),
        warp_markers: Vec::new(),
        tuning_render_cache: None,
    }
}

/// Engine state with a single audio track (id 1) carrying a 1-second tone.
fn state_with_tone_track() -> EngineState {
    let state = empty_engine_state();
    state.shared.edit_tracks(|m| {
        m.insert(1, std::sync::Arc::new(Track::with_type(1, "track".into(), TrackType::Audio)));
    });
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
        Arc::clone(&state.clips),
        Arc::clone(&state.tempo_map),
        Arc::new(AutomationSnapshot::default()),
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
    // The renderer returns `FreezeError::Cancelled` on cooperative cancel
    // (proven end-to-end in freeze_render_core.rs); the worker must turn
    // that into `FreezeCancelled`, not `AudioEvent::FreezeError`.
    let ev = freeze_terminal_event(7, Err(FreezeError::Cancelled));
    assert!(
        matches!(ev, AudioEvent::FreezeCancelled { track_id: 7 }),
        "cancel sentinel must map to FreezeCancelled, got {ev:?}"
    );
}

#[test]
fn terminal_event_maps_other_errors_to_freeze_error() {
    let ev = freeze_terminal_event(9, Err(FreezeError::NothingToFreeze));
    match ev {
        AudioEvent::FreezeError { track_id, message } => {
            assert_eq!(track_id, 9);
            assert_eq!(message, "Nothing to freeze");
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

/// A cache rendered at another rate is converted by the shared
/// band-limited resampler when the engine publishes it, so the mixer
/// reads it frame for frame at the engine rate (code review FU-G3a).
/// The conversion runs on a worker, not the engine command thread; it
/// attaches when it lands — at the latest before an offline render
/// (FU-A4c).
#[test]
fn a_frozen_source_at_another_rate_is_converted_on_publish() {
    let mut h = resonance_audio::test_support::EngineHandlerHarness::new();
    h.push_track(Track::new(1, "frozen".into()));
    let cache_ref = FreezeCacheRef::new("c.wav".into(), 44_100, 32, 1, FreezeCacheStatus::Frozen);
    let source = FrozenSource::new(cache_ref.clone(), Arc::new(vec![0.25; 44_100 * 2]), 44_100, 44_100);

    h.set_track_frozen_source(1, Some(source));
    h.settle_frozen_conversions(true);

    let published = h.frozen_source(1).expect("attached");
    assert_eq!(published.sample_rate, SR, "published at the engine rate");
    assert_eq!(published.frame_count, SR as u64, "one second stays one second");
    assert_eq!(published.samples.len(), SR as usize * 2);
    assert!(published.samples.iter().all(|&s| (s - 0.25).abs() < 1e-4), "DC stays DC");
    assert_eq!(published.cache_ref, cache_ref, "the file's metadata is kept");
}

fn frozen_44k(seconds: usize) -> FrozenSource {
    let cache_ref = FreezeCacheRef::new("c.wav".into(), 44_100, 32, 1, FreezeCacheStatus::Frozen);
    let frames = 44_100 * seconds;
    FrozenSource::new(cache_ref, Arc::new(vec![0.25; frames * 2]), 44_100, frames as u64)
}

/// A conversion superseded before it lands never attaches: an unfreeze
/// (or a newer cache) wins, and so does a project clear, since track ids
/// are reused by the next project (FU-A4c).
#[test]
fn a_superseded_frozen_conversion_never_attaches() {
    let mut h = resonance_audio::test_support::EngineHandlerHarness::new();
    h.push_track(Track::new(1, "frozen".into()));

    h.set_track_frozen_source(1, Some(frozen_44k(1)));
    h.set_track_frozen_source(1, None);
    h.settle_frozen_conversions(true);
    assert!(h.frozen_source(1).is_none(), "the unfreeze wins");

    h.set_track_frozen_source(1, Some(frozen_44k(1)));
    h.clear_all();
    h.push_track(Track::new(1, "next project".into()));
    h.settle_frozen_conversions(true);
    assert!(h.frozen_source(1).is_none(), "the old project's cache stays out");
}
