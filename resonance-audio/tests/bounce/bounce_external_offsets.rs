//! External-instrument latency offsets in offline bounce/export and the
//! realtime-bounce take shift (doc #260 finding #4, ba todo #1122).
//!
//! Offline: the bounce paths build their own latency-comp table off the
//! engine thread; it must fold the published `shared.external_offsets`
//! exactly like the live refresh does, so a recorded external take
//! (whose content is baked one round trip late) renders at the same
//! relative position as live playback. Driven through the real
//! `render_stem` path with a synthetic offset — no hardware needed.
//!
//! Realtime bounce: the captured return is uniformly late by the source
//! track's round trip; `apply_take_shift` places the finalized clip
//! that much earlier (converting into leading trim at timeline 0) so
//! the bounced take lands where the live-monitored return sounded.

use std::sync::Arc;

use parking_lot::RwLock;

use resonance_audio::test_support::{SharedState, StemSource, apply_take_shift, render_stem};
use resonance_audio::types::*;

const SR: u32 = 48_000;

struct EngineState {
    shared: Arc<SharedState>,
    clips: Arc<RwLock<Vec<AudioClip>>>,
    tempo_map: Arc<arc_swap::ArcSwap<TempoMap>>,
}

impl EngineState {
    fn new() -> Self {
        Self {
            shared: Arc::new(SharedState::default()),
            clips: Arc::new(RwLock::new(Vec::new())),
            tempo_map: Arc::new(arc_swap::ArcSwap::from_pointee(TempoMap::default())),
        }
    }

    /// Impulse clip: silence with a single `1.0` stereo frame at
    /// `clip_start + at`, so the rendered position is exactly assertable.
    fn add_impulse_clip(&self, id: ClipId, track: TrackId, start: u64, frames: usize, at: usize) {
        let mut samples = vec![0.0f32; frames * 2];
        samples[at * 2] = 1.0;
        samples[at * 2 + 1] = 1.0;
        self.clips.write().push(AudioClip {
            id,
            track_id: track,
            start_sample: start,
            source: ClipSource::Memory(samples),
            name: "impulse".into(),
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
            warp_algorithm: WarpAlgorithm::default(),
            warp_markers: Vec::new(),
            tuning_render_cache: None,
        });
    }

    fn render_track_stem(&self, track: TrackId, frames: u64) -> Vec<f32> {
        render_stem(
            StemSource::Track(track),
            0,
            frames,
            &self.shared,
            &self.clips,
            &self.tempo_map,
            SR,
        )
        .expect("stem render")
    }
}

/// Frames of the interleaved-stereo output that carry any signal.
fn nonzero_frames(data: &[f32]) -> Vec<usize> {
    data.chunks(2)
        .enumerate()
        .filter(|(_, f)| f.iter().any(|&s| s != 0.0))
        .map(|(i, _)| i)
        .collect()
}

#[test]
fn offline_render_folds_external_offset_like_live_playback() {
    const OFFSET: i64 = 40;
    let state = EngineState::new();
    let mut t = Track::new(1, "ext".into());
    t.set_output(TrackOutput::Master);
    state.shared.edit_tracks(|m| { m.insert(1, std::sync::Arc::new(t)); });
    // Recorded external take: the true event was at frame 60, captured
    // one round trip (40 smp) late — content sits at frame 100.
    state.add_impulse_clip(10, 1, 0, 256, 100);

    // Without the published offset the take renders where its content
    // sits: late.
    let out = state.render_track_stem(1, 256);
    assert_eq!(nonzero_frames(&out), vec![100], "no offset: content position");

    // Publish the round-trip offset (what refresh_latency_comp does on
    // the engine thread) — the offline comp must now pre-roll/trim by
    // it, re-aligning the take with where the live return sounded.
    state
        .shared
        .external_offsets
        .store(Arc::new([(1u64, OFFSET)].into_iter().collect()));
    let out = state.render_track_stem(1, 256);
    assert_eq!(
        nonzero_frames(&out),
        vec![100 - OFFSET as usize],
        "offset folded: take re-aligned to the true event position"
    );
}

#[test]
fn offline_offset_delays_sibling_tracks_to_match() {
    // Two tracks, same impulse position in their clips. Track 1 is the
    // external instrument (offset 40, content recorded late); track 2
    // is a plain track. With the offset folded, track 2 must be
    // *delayed* by 40 so both land together — the same "rest of the mix
    // waits for the return" behaviour as live playback.
    const OFFSET: i64 = 40;
    let state = EngineState::new();
    for id in [1u64, 2] {
        let mut t = Track::new(id, format!("t{id}"));
        t.set_output(TrackOutput::Master);
        state.shared.edit_tracks(|m| { m.insert(id, std::sync::Arc::new(t)); });
    }
    state.add_impulse_clip(10, 1, 0, 256, 100);
    state.add_impulse_clip(11, 2, 0, 256, 60);
    state
        .shared
        .external_offsets
        .store(Arc::new([(1u64, OFFSET)].into_iter().collect()));

    // Full-mix comp applies to both stems over the same range, so each
    // renders through the same table; track 1 trimmed to 60, track 2
    // delayed 40 then trimmed 40 => stays at 60.
    let ext = state.render_track_stem(1, 256);
    let plain = state.render_track_stem(2, 256);
    assert_eq!(nonzero_frames(&ext), vec![60]);
    assert_eq!(nonzero_frames(&plain), vec![60]);
}

// -- Realtime-bounce take shift ---------------------------------------------

#[test]
fn take_shift_moves_clip_earlier_by_the_round_trip() {
    assert_eq!(apply_take_shift(1000, 0, 40), (960, 0));
    // Existing loop trim is preserved.
    assert_eq!(apply_take_shift(1000, 25, 40), (960, 25));
}

#[test]
fn take_shift_pins_at_zero_and_trims_the_overflow() {
    // A take that started 10 samples into the timeline with a 40-sample
    // round trip cannot start at -30: it pins at 0 and drops the 30
    // leading samples instead, preserving alignment of the remainder.
    assert_eq!(apply_take_shift(10, 0, 40), (0, 30));
    assert_eq!(apply_take_shift(10, 5, 40), (0, 35));
}

#[test]
fn take_shift_ignores_non_positive_shifts() {
    assert_eq!(apply_take_shift(1000, 7, 0), (1000, 7));
    assert_eq!(apply_take_shift(1000, 7, -480), (1000, 7));
}
