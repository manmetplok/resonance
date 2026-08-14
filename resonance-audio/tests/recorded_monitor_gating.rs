//! Integration tests for the Recorded-playback monitor gate in the
//! shared mix loop (`mixer/render_core.rs::recorded_monitor_gate`, doc
//! #257, todo #1099) and the covered-span predicate it is built on
//! (`types::audio_clip_covers`).
//!
//! While a track's playback source is `Recorded` and a recorded take
//! covers the block, the live input monitor must be skipped so the
//! hardware return is not layered on top of the take (which plays via
//! the normal clip mix). Outside covered spans, in `Live` mode, and on a
//! record-armed track (punch-in) the monitor mixes exactly as before.
//! The stopped-transport monitor path (`mixer/monitor.rs`) never runs
//! through this gate, so monitoring while preparing a take is untouched
//! by construction.

use resonance_audio::__test_support::recorded_monitor_gate;
use resonance_audio::types::*;
use resonance_common::PlaybackSource;

const TRACK: TrackId = 1;

/// A recorded take on `track_id` spanning `[start, start + frames)`.
fn take_on(track_id: TrackId, start: u64, frames: usize) -> AudioClip {
    AudioClip {
        id: 100,
        track_id,
        start_sample: start,
        source: ClipSource::Memory(vec![0.0; frames * 2]),
        name: "take".into(),
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

fn monitored_track() -> Track {
    let t = Track::new(TRACK, "ext".into());
    t.set_monitor_enabled(true);
    t
}

// -- recorded_monitor_gate ----------------------------------------------

#[test]
fn live_mode_never_gates() {
    let track = monitored_track(); // default source: Live
    let takes = vec![take_on(TRACK, 4_000, 8_000)];
    assert!(!recorded_monitor_gate(&track, &takes, 5_000, 512));
}

#[test]
fn recorded_mode_gates_covered_block() {
    let track = monitored_track();
    track.set_playback_source(PlaybackSource::Recorded);
    let takes = vec![take_on(TRACK, 4_000, 8_000)]; // [4_000, 12_000)

    // Block fully inside the take.
    assert!(recorded_monitor_gate(&track, &takes, 5_000, 512));
    // Block straddling the take's start.
    assert!(recorded_monitor_gate(&track, &takes, 3_800, 512));
}

#[test]
fn recorded_mode_stays_live_outside_covered_spans() {
    let track = monitored_track();
    track.set_playback_source(PlaybackSource::Recorded);
    let takes = vec![take_on(TRACK, 4_000, 8_000)];

    // Before the take: live fallback keeps monitoring.
    assert!(!recorded_monitor_gate(&track, &takes, 0, 512));
    // Block ending exactly at the take's start (half-open) is uncovered.
    assert!(!recorded_monitor_gate(&track, &takes, 3_488, 512));
    // At/after the take's end.
    assert!(!recorded_monitor_gate(&track, &takes, 12_000, 512));
    // No takes at all — nothing gates.
    assert!(!recorded_monitor_gate(&track, &[], 5_000, 512));
}

#[test]
fn armed_track_keeps_monitor_over_covered_span() {
    let track = monitored_track();
    track.set_playback_source(PlaybackSource::Recorded);
    track.set_record_armed(true); // punch-in over the existing take
    let takes = vec![take_on(TRACK, 4_000, 8_000)];
    assert!(!recorded_monitor_gate(&track, &takes, 5_000, 512));
}

#[test]
fn takes_on_other_tracks_do_not_gate() {
    let track = monitored_track();
    track.set_playback_source(PlaybackSource::Recorded);
    let takes = vec![take_on(2, 4_000, 8_000)]; // different track
    assert!(!recorded_monitor_gate(&track, &takes, 5_000, 512));
}

// -- audio_clip_covers ---------------------------------------------------

#[test]
fn covers_uses_half_open_clip_extent() {
    let takes = vec![take_on(TRACK, 1_000, 500)]; // [1_000, 1_500)

    // Window overlapping the interior.
    assert!(audio_clip_covers(&takes, TRACK, 1_200, 1_300));
    // First covered sample.
    assert!(audio_clip_covers(&takes, TRACK, 1_000, 1_001));
    // Last covered sample.
    assert!(audio_clip_covers(&takes, TRACK, 1_499, 1_500));
    // Window ending exactly at the clip start: not covered.
    assert!(!audio_clip_covers(&takes, TRACK, 500, 1_000));
    // Window starting exactly at the clip end: not covered.
    assert!(!audio_clip_covers(&takes, TRACK, 1_500, 2_000));
    // Zero-length window covers nothing.
    assert!(!audio_clip_covers(&takes, TRACK, 1_200, 1_200));
}

#[test]
fn covers_respects_trim_and_track_scoping() {
    // 500-frame source trimmed to the visible extent [1_100, 1_400).
    let mut clip = take_on(TRACK, 1_100, 500);
    clip.trim_start_frames = 100;
    clip.trim_end_frames = 100;
    let takes = vec![clip];

    assert!(audio_clip_covers(&takes, TRACK, 1_100, 1_101));
    assert!(!audio_clip_covers(&takes, TRACK, 1_400, 1_500));
    // Same window, different track: no coverage.
    assert!(!audio_clip_covers(&takes, 2, 1_100, 1_101));
}
