//! Tests for the `AudioCommand::DetectClipTempo` engine boundary
//! (todo #420).
//!
//! Drives the engine-internal pure helper `detect_clip_tempo_in_place`
//! directly via its `#[doc(hidden)]` re-export. That keeps the test
//! headless — no cpal stream, no engine thread, no audio device — while
//! exercising the exact stereo→mono downmix, DSP tempo detection and
//! `AudioEvent::ClipTempoDetected` emission the dispatch path runs. The
//! detector's BPM accuracy across tempi/rates is covered by
//! `resonance-dsp/tests/tempo.rs`; here we verify the clip boundary:
//! the event carries a plausible BPM for a loaded loop, the clip is not
//! mutated, and a missing clip emits no ghost event.

use std::sync::Arc;

use crossbeam_channel::unbounded;
use parking_lot::RwLock;

use resonance_audio::detect_clip_tempo_in_place;
use resonance_audio::types::{AudioClip, AudioEvent, ClipSource, WarpAlgorithm};

const SR: u32 = 48_000;

/// A click train at `bpm`: a short decaying 2 kHz blip on each beat,
/// `duration_secs` long, silence between. Mirrors the DSP test signal so
/// the detector recovers the tempo within tolerance.
fn click_train(bpm: f32, sample_rate: f32, duration_secs: f32) -> Vec<f32> {
    let total = (sample_rate * duration_secs) as usize;
    let period = (sample_rate * 60.0 / bpm) as usize;
    let blip = (sample_rate * 0.01) as usize; // 10 ms decay
    let mut buf = vec![0.0f32; total];
    let mut beat = 0;
    while beat * period < total {
        let start = beat * period;
        for j in 0..blip {
            if start + j >= total {
                break;
            }
            let t = j as f32 / sample_rate;
            let env = (-t * 400.0).exp();
            buf[start + j] += env * (std::f32::consts::TAU * 2_000.0 * t).sin();
        }
        beat += 1;
    }
    buf
}

/// Interleave a mono signal into stereo `[l, r]` frames (same sample in
/// both channels) — the shape `ClipSource::as_frames` yields.
fn to_stereo(mono: &[f32]) -> Vec<f32> {
    let mut out = Vec::with_capacity(mono.len() * 2);
    for &s in mono {
        out.push(s);
        out.push(s);
    }
    out
}

/// Build an in-RAM audio clip from interleaved stereo samples, with the
/// default (no-warp) settings.
fn sample_clip(id: u64, track_id: u64, stereo: Vec<f32>) -> AudioClip {
    AudioClip {
        id,
        track_id,
        start_sample: 0,
        source: ClipSource::Memory(stereo),
        name: "loop".into(),
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
fn detect_tempo_emits_plausible_bpm_for_loaded_loop() {
    let stereo = to_stereo(&click_train(120.0, SR as f32, 12.0));
    let clips: Arc<RwLock<Vec<AudioClip>>> = Arc::new(RwLock::new(vec![sample_clip(7, 100, stereo)]));
    let (event_tx, event_rx) = unbounded::<AudioEvent>();

    detect_clip_tempo_in_place(&clips, &event_tx, SR, /* clip_id */ 7);

    match event_rx.try_recv() {
        Ok(AudioEvent::ClipTempoDetected {
            clip_id,
            bpm,
            confidence,
        }) => {
            assert_eq!(clip_id, 7);
            assert!(
                (bpm - 120.0).abs() <= 2.0,
                "expected ~120 BPM, got {bpm} (conf {confidence})"
            );
            assert!(confidence > 0.3, "low confidence for a steady pulse: {confidence}");
        }
        other => panic!("expected ClipTempoDetected, got {other:?}"),
    }
    assert!(
        event_rx.try_recv().is_err(),
        "exactly one event should be emitted"
    );

    // Analysis only: the clip is never mutated.
    let clips = clips.read();
    assert!(!clips[0].warp_enabled);
    assert_eq!(clips[0].original_bpm, None);
}

#[test]
fn detect_tempo_missing_clip_emits_no_event() {
    let stereo = to_stereo(&click_train(120.0, SR as f32, 4.0));
    let clips: Arc<RwLock<Vec<AudioClip>>> = Arc::new(RwLock::new(vec![sample_clip(1, 100, stereo)]));
    let (event_tx, event_rx) = unbounded::<AudioEvent>();

    detect_clip_tempo_in_place(&clips, &event_tx, SR, /* clip_id */ 999);

    assert!(
        event_rx.try_recv().is_err(),
        "ClipTempoDetected must not be emitted when the clip lookup misses"
    );
}

#[test]
fn detect_tempo_on_silence_emits_zero() {
    // A silent clip still emits an event (the clip was found); the
    // detector reports a zero estimate for material with no onsets.
    let silent = vec![0.0f32; (SR as usize) * 5 * 2];
    let clips: Arc<RwLock<Vec<AudioClip>>> = Arc::new(RwLock::new(vec![sample_clip(3, 100, silent)]));
    let (event_tx, event_rx) = unbounded::<AudioEvent>();

    detect_clip_tempo_in_place(&clips, &event_tx, SR, /* clip_id */ 3);

    match event_rx.try_recv() {
        Ok(AudioEvent::ClipTempoDetected { clip_id, bpm, confidence }) => {
            assert_eq!(clip_id, 3);
            assert_eq!(bpm, 0.0);
            assert_eq!(confidence, 0.0);
        }
        other => panic!("expected ClipTempoDetected, got {other:?}"),
    }
}
