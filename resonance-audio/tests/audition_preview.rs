//! Integration coverage for audition preview playback (doc #175,
//! `engine/audition.rs` + `mixer/audition.rs`).
//!
//! Audition previews an arbitrary audio file through the engine — a pool
//! asset or an un-imported filesystem file — independently of the
//! arrangement, transport, and undo. These tests drive the command/state
//! boundary and the realtime overlay directly against a plain `SharedState`,
//! so they need neither the engine thread nor a real audio device:
//!
//! - `compute_sync_ratio`: the sync-to-tempo (varispeed) ratio math.
//! - `load_audition_source`: decode a real WAV off disk to an engine-rate
//!   source.
//! - `start_audition_in_place` / `stop_audition_in_place` /
//!   `set_audition_options_in_place`: the playback-state boundary.
//! - `mix_audition_overlay`: the allocation-free realtime mix — sample
//!   playback, linear-interpolation varispeed, loop wrap, and the
//!   natural-finish latch.

use std::path::PathBuf;
use std::sync::atomic::Ordering;

use hound::{SampleFormat, WavSpec, WavWriter};

use resonance_audio::__test_support::{mix_audition_overlay, SharedState};
use resonance_audio::{
    compute_sync_ratio, load_audition_source, set_audition_options_in_place,
    start_audition_in_place, stop_audition_in_place, AuditionSource,
};

const RATE: u32 = 48_000;

/// Stereo source from per-frame (l, r) pairs.
fn source(frames: &[(f32, f32)]) -> AuditionSource {
    let mut samples = Vec::with_capacity(frames.len() * 2);
    for (l, r) in frames {
        samples.push(*l);
        samples.push(*r);
    }
    AuditionSource::from_samples(samples, RATE)
}

fn ratio_of(shared: &SharedState) -> f32 {
    f32::from_bits(shared.audition_ratio_bits.load(Ordering::Relaxed))
}

fn pos_of(shared: &SharedState) -> f64 {
    f64::from_bits(shared.audition_pos_bits.load(Ordering::Relaxed))
}

// ---- compute_sync_ratio -------------------------------------------------

#[test]
fn sync_off_is_unity() {
    assert_eq!(compute_sync_ratio(24_000, RATE, 120.0, false), 1.0);
}

#[test]
fn sync_degenerate_inputs_are_unity() {
    assert_eq!(compute_sync_ratio(0, RATE, 120.0, true), 1.0);
    assert_eq!(compute_sync_ratio(24_000, 0, 120.0, true), 1.0);
    assert_eq!(compute_sync_ratio(24_000, RATE, 0.0, true), 1.0);
}

#[test]
fn sync_on_grid_loop_is_unity() {
    // 24_000 frames @ 48 kHz = 0.5 s = exactly one beat at 120 BPM.
    assert!((compute_sync_ratio(24_000, RATE, 120.0, true) - 1.0).abs() < 1e-6);
}

#[test]
fn sync_off_grid_loop_snaps_to_nearest_beat() {
    // 30_000 frames = 0.625 s = 1.25 beats @ 120 BPM → snaps to 1 beat,
    // so playback runs 1.25× faster to fit.
    assert!((compute_sync_ratio(30_000, RATE, 120.0, true) - 1.25).abs() < 1e-6);
    // 36_000 frames = 0.75 s = 1.5 beats → rounds up to 2 beats, so
    // playback slows to 0.75× to stretch across the longer span.
    assert!((compute_sync_ratio(36_000, RATE, 120.0, true) - 0.75).abs() < 1e-6);
}

// ---- start / stop / options boundary ------------------------------------

#[test]
fn start_seeds_playback_state() {
    let shared = SharedState::default();
    start_audition_in_place(&shared, source(&[(0.1, 0.2), (0.3, 0.4)]), 0, 120.0, true, false);

    assert!(shared.audition_playing.load(Ordering::Relaxed));
    assert!(shared.audition_loop.load(Ordering::Relaxed));
    assert!(!shared.audition_sync.load(Ordering::Relaxed));
    assert!(!shared.audition_finished.load(Ordering::Relaxed));
    assert_eq!(ratio_of(&shared), 1.0);
    assert_eq!(pos_of(&shared), 0.0);
    assert!(shared.audition_source.load().is_some());
}

#[test]
fn start_clamps_start_frame_to_length() {
    let shared = SharedState::default();
    start_audition_in_place(&shared, source(&[(0.0, 0.0), (0.0, 0.0)]), 99, 120.0, false, false);
    assert_eq!(pos_of(&shared), 2.0); // clamped to frame_count
}

#[test]
fn stop_reports_playing_and_clears_source() {
    let shared = SharedState::default();
    start_audition_in_place(&shared, source(&[(1.0, 1.0)]), 0, 120.0, false, false);

    assert!(stop_audition_in_place(&shared));
    assert!(!shared.audition_playing.load(Ordering::Relaxed));
    assert!(shared.audition_source.load().is_none());

    // Stopping an already-idle audition is a silent no-op (returns false).
    assert!(!stop_audition_in_place(&shared));
}

#[test]
fn set_options_recomputes_ratio_for_loaded_source() {
    let shared = SharedState::default();
    // 30_000-frame source, started with sync off (ratio 1.0).
    let frames: Vec<(f32, f32)> = (0..30_000).map(|_| (0.0, 0.0)).collect();
    start_audition_in_place(&shared, source(&frames), 0, 120.0, true, false);
    assert_eq!(ratio_of(&shared), 1.0);

    // Turning sync on recomputes against the 1.25-beat loop.
    set_audition_options_in_place(&shared, 120.0, true, true);
    assert!(shared.audition_sync.load(Ordering::Relaxed));
    assert!((ratio_of(&shared) - 1.25).abs() < 1e-6);
}

// ---- realtime overlay ---------------------------------------------------

#[test]
fn overlay_is_noop_when_idle() {
    let shared = SharedState::default();
    let mut data = vec![0.5f32; 8];
    mix_audition_overlay(&mut data, 2, &shared);
    assert_eq!(data, vec![0.5f32; 8]); // untouched
}

#[test]
fn overlay_plays_samples_and_advances_position() {
    let shared = SharedState::default();
    start_audition_in_place(
        &shared,
        source(&[(0.1, 0.2), (0.3, 0.4), (0.5, 0.6), (0.7, 0.8)]),
        0,
        120.0,
        false,
        false,
    );

    let mut data = vec![0.0f32; 2 * 2]; // 2 frames, stereo
    mix_audition_overlay(&mut data, 2, &shared);

    assert!((data[0] - 0.1).abs() < 1e-6);
    assert!((data[1] - 0.2).abs() < 1e-6);
    assert!((data[2] - 0.3).abs() < 1e-6);
    assert!((data[3] - 0.4).abs() < 1e-6);
    assert_eq!(pos_of(&shared), 2.0);
    assert!(shared.audition_playing.load(Ordering::Relaxed));
}

#[test]
fn overlay_sums_onto_existing_audio() {
    let shared = SharedState::default();
    start_audition_in_place(&shared, source(&[(0.25, -0.25)]), 0, 120.0, false, false);

    let mut data = vec![0.5f32, 0.5f32];
    mix_audition_overlay(&mut data, 2, &shared);
    assert!((data[0] - 0.75).abs() < 1e-6);
    assert!((data[1] - 0.25).abs() < 1e-6);
}

#[test]
fn overlay_clamps_to_unit_range() {
    let shared = SharedState::default();
    start_audition_in_place(&shared, source(&[(0.9, -0.9)]), 0, 120.0, false, false);

    let mut data = vec![0.5f32, -0.5f32];
    mix_audition_overlay(&mut data, 2, &shared);
    assert_eq!(data[0], 1.0); // 0.5 + 0.9 clamped
    assert_eq!(data[1], -1.0); // -0.5 - 0.9 clamped
}

#[test]
fn overlay_non_loop_finishes_at_end() {
    let shared = SharedState::default();
    start_audition_in_place(&shared, source(&[(1.0, 1.0), (1.0, 1.0)]), 0, 120.0, false, false);

    // Ask for more frames than the source holds.
    let mut data = vec![0.0f32; 4 * 2];
    mix_audition_overlay(&mut data, 2, &shared);

    // First two frames played, then it stopped + latched finished.
    assert_eq!(data[0], 1.0);
    assert_eq!(data[2], 1.0);
    assert_eq!(data[4], 0.0); // nothing past the end
    assert!(!shared.audition_playing.load(Ordering::Relaxed));
    assert!(shared.audition_finished.load(Ordering::Relaxed));
}

#[test]
fn overlay_loop_wraps_and_keeps_playing() {
    let shared = SharedState::default();
    start_audition_in_place(&shared, source(&[(0.1, 0.1), (0.2, 0.2)]), 0, 120.0, true, false);

    let mut data = vec![0.0f32; 4 * 2];
    mix_audition_overlay(&mut data, 2, &shared);

    // Frame 0,1 then wraps to 0,1 again.
    assert!((data[0] - 0.1).abs() < 1e-6);
    assert!((data[2] - 0.2).abs() < 1e-6);
    assert!((data[4] - 0.1).abs() < 1e-6);
    assert!((data[6] - 0.2).abs() < 1e-6);
    assert_eq!(pos_of(&shared), 2.0);
    assert!(shared.audition_playing.load(Ordering::Relaxed));
    assert!(!shared.audition_finished.load(Ordering::Relaxed));
}

#[test]
fn overlay_varispeed_linear_interpolation() {
    let shared = SharedState::default();
    // Sample values stay within unit range so the overlay's hard clamp
    // doesn't mask the interpolation result.
    start_audition_in_place(
        &shared,
        source(&[(0.0, 0.0), (0.2, 0.2), (0.4, 0.4), (0.6, 0.6)]),
        0,
        120.0,
        false,
        false,
    );
    // Force a half-speed ratio: positions 0.0, 0.5, 1.0, 1.5.
    shared
        .audition_ratio_bits
        .store(0.5f32.to_bits(), Ordering::Relaxed);

    let mut data = vec![0.0f32; 4 * 2];
    mix_audition_overlay(&mut data, 2, &shared);

    assert!((data[0] - 0.0).abs() < 1e-6);
    assert!((data[2] - 0.1).abs() < 1e-6); // interp 0.0↔0.2
    assert!((data[4] - 0.2).abs() < 1e-6);
    assert!((data[6] - 0.3).abs() < 1e-6); // interp 0.2↔0.4
    assert!((pos_of(&shared) - 2.0).abs() < 1e-6);
}

#[test]
fn overlay_empty_source_finishes_immediately() {
    let shared = SharedState::default();
    start_audition_in_place(&shared, AuditionSource::from_samples(vec![], RATE), 0, 120.0, false, false);

    let mut data = vec![0.0f32; 4];
    mix_audition_overlay(&mut data, 2, &shared);
    assert!(!shared.audition_playing.load(Ordering::Relaxed));
    assert!(shared.audition_finished.load(Ordering::Relaxed));
}

// ---- decode boundary ----------------------------------------------------

fn temp_wav(tag: &str, frames: usize) -> PathBuf {
    let path = std::env::temp_dir().join(format!(
        "resonance-audition-{}-{}.wav",
        tag,
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let spec = WavSpec {
        channels: 2,
        sample_rate: RATE,
        bits_per_sample: 32,
        sample_format: SampleFormat::Float,
    };
    let mut writer = WavWriter::create(&path, spec).expect("create wav");
    for i in 0..frames {
        let s = (i as f32 / frames as f32) - 0.5;
        writer.write_sample(s).expect("l");
        writer.write_sample(s).expect("r");
    }
    writer.finalize().expect("finalize");
    path
}

#[test]
fn load_decodes_wav_to_engine_rate_source() {
    let path = temp_wav("decode", 1000);
    let src = load_audition_source(&path, RATE).expect("decode");
    assert_eq!(src.frame_count, 1000);
    assert_eq!(src.sample_rate, RATE);
    assert_eq!(src.samples.len(), 2000);
    let _ = std::fs::remove_file(&path);
}

#[test]
fn load_missing_file_errors() {
    let path = PathBuf::from("/nonexistent/resonance-audition-missing.wav");
    assert!(load_audition_source(&path, RATE).is_err());
}
