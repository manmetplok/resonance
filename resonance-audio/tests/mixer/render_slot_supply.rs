//! The live callback's per-track render slots grow with the project
//! (realtime-multithreading.md §4.2).
//!
//! Every track job renders into its own slot, and the audio thread cannot
//! allocate, so the engine thread offers a larger pool whenever it
//! publishes a graph with more tracks than the callback's pool holds. A
//! track without a slot would simply not be rendered — so the proof the
//! hand-over works is that tracks added far past the initial pool are all
//! heard, exactly.

use std::sync::atomic::Ordering;
use std::sync::Arc;

use resonance_audio::test_support::MixAudioHarness;
use resonance_audio::types::*;

const SR: u32 = 48_000;
const BLOCK: usize = 128;
/// A power of two, so any sum of up to 2^20 copies is exact in f32.
const LEVEL: f32 = 1.0 / 1024.0;

fn dc_clip(id: ClipId, track_id: TrackId) -> AudioClip {
    AudioClip {
        id,
        track_id,
        start_sample: 0,
        source: ClipSource::memory(vec![LEVEL; BLOCK * 64 * 2]),
        name: format!("c{id}"),
        trim_start_frames: 0,
        trim_end_frames: 0,
        fade_in_frames: 0,
        fade_in_curve: FadeCurve::Linear,
        fade_out_frames: 0,
        fade_out_curve: FadeCurve::Linear,
        gain_db: 0.0,
        vocal_tuning: None,
        warp_enabled: false,
        original_bpm: None,
        transpose_semitones: 0.0,
        warp_algorithm: WarpAlgorithm::default(),
        warp_markers: Vec::new(),
        tuning_render_cache: None,
    }
}

/// Render until the fader ramps have settled and return the last block.
fn settled_block(h: &mut MixAudioHarness) -> Vec<f32> {
    h.render();
    h.render().to_vec()
}

#[test]
fn tracks_added_past_the_initial_pool_are_all_rendered() {
    let mut h = MixAudioHarness::new(
        vec![Track::new(1, "t1".into())],
        Vec::new(),
        vec![dc_clip(1, 1)],
        Vec::new(),
        Vec::new(),
        TempoMap::default(),
        BLOCK,
        2,
        SR,
        true,
    );
    h.shared().playing.store(true, Ordering::Relaxed);
    let out = settled_block(&mut h);
    assert!(
        out.iter().all(|&s| s == LEVEL),
        "one track: {:?}",
        &out[..4]
    );

    // Far past the 64-slot starting pool, in two publishes so the second
    // growth supersedes the first.
    for total in [100u64, 300] {
        let first_new = h.tracks().len() as u64 + 1;
        h.shared().edit_clips(|clips| {
            for id in first_new..=total {
                clips.push(Arc::new(dc_clip(id, id)));
            }
        });
        h.edit_tracks(|tracks| {
            for id in first_new..=total {
                tracks.insert(id, Arc::new(Track::new(id, format!("t{id}"))));
            }
        });
        let out = settled_block(&mut h);
        let expected = LEVEL * total as f32;
        assert!(
            out.iter().all(|&s| s == expected),
            "{total} tracks: expected {expected}, got {:?}",
            &out[..4]
        );
    }
}
