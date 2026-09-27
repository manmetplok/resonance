//! The clip hammer (code review ARCH-02 A2-8, refactor todo B-5): the
//! whole audio callback rendering on its own thread while the real clip
//! handlers — loads through the import pool, deletes, moves, trims,
//! splits, fades, gain — edit the same engine's clip list as fast as they
//! can.
//!
//! The clip list was the last map the playing branch `try_read`, so any
//! of those edits (or a worker's write queued behind an offline render's
//! read guard) used to make a block render silence. It is in the render
//! graph now: the callback does one wait-free load, and every edit is an
//! engine-thread publish whose replaced graph the retire sweep frees.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use resonance_audio::test_support::{EngineHandlerHarness, MixAudioHarness};
use resonance_audio::transcode_to_wav;
use resonance_audio::types::*;

const SR: u32 = 48_000;
const BLOCK: usize = 128;
const BLOCKS: usize = 3_000;
/// The clip no edit touches, so every block has something to play.
const BASE_CLIP: ClipId = 1;

/// Ten seconds of a constant `level` on `track_id`, from the top — longer
/// than the whole render, so a block is never silent for lack of audio.
fn clip(id: ClipId, track_id: TrackId, level: f32) -> AudioClip {
    AudioClip {
        id,
        track_id,
        start_sample: 0,
        source: ClipSource::memory(vec![level; 2 * SR as usize * 10]),
        name: format!("clip {id}"),
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

fn wav(tag: &str) -> (PathBuf, PathBuf) {
    let dir = std::env::temp_dir().join(format!(
        "resonance-clip-hammer-{tag}-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("load.wav");
    transcode_to_wav(&path, &vec![0.05; 2 * SR as usize], SR).expect("write test wav");
    (dir, path)
}

#[test]
fn clip_loads_and_edits_under_render_never_skip_a_block() {
    let (dir, wav) = wav("hammer");
    let mut h = EngineHandlerHarness::new();
    for id in 1..=4 {
        let mut t = Track::new(id, format!("t{id}"));
        t.set_output(TrackOutput::Master);
        h.push_track(t);
    }
    h.push_clip(clip(BASE_CLIP, 1, 0.1));
    for id in 2..=9 {
        h.push_clip(clip(id, 1 + id % 4, 0.01));
    }
    let shared = h.shared_arc();
    shared.playing.store(true, Ordering::Relaxed);

    // The audio thread: the real callback over the engine's own state.
    let done = Arc::new(AtomicBool::new(false));
    let renderer = {
        let shared = Arc::clone(&shared);
        let done = Arc::clone(&done);
        std::thread::spawn(move || {
            let mut cb = MixAudioHarness::on_shared(shared, BLOCK, 2, SR);
            for block in 0..BLOCKS {
                let out = cb.render();
                assert!(out.iter().any(|&s| s != 0.0), "block {block} rendered silence");
            }
            done.store(true, Ordering::Release);
        })
    };

    // The engine thread: hammer the clip list through the real handlers.
    let mut next_id: ClipId = 100;
    let mut edits = 0u64;
    let mut loads = 0u64;
    while !done.load(Ordering::Acquire) {
        let live: Vec<ClipId> = h.clip_ids().into_iter().filter(|&id| id != BASE_CLIP).collect();
        let pick = |k: u64| live.get((k as usize) % live.len().max(1)).copied();
        let cmd = match (edits % 7, pick(edits)) {
            (0, Some(clip_id)) => Some(AudioCommand::MoveClip {
                clip_id,
                new_start_sample: (edits % 10) * 100,
                new_track_id: 1 + (edits % 4),
            }),
            (1, Some(clip_id)) => Some(AudioCommand::TrimClip {
                clip_id,
                new_start_sample: 0,
                trim_start_frames: edits % 1_000,
                trim_end_frames: 0,
            }),
            (2, Some(clip_id)) => Some(AudioCommand::SetClipFade {
                clip_id,
                fade_in_frames: edits % 500,
                fade_in_curve: FadeCurve::EqualPower,
                fade_out_frames: edits % 700,
                fade_out_curve: FadeCurve::Linear,
            }),
            (3, Some(clip_id)) => Some(AudioCommand::SetClipGain {
                clip_id,
                gain_db: -((edits % 12) as f32),
            }),
            (4, Some(clip_id)) => {
                next_id += 1;
                Some(AudioCommand::SplitClip {
                    clip_id,
                    new_clip_id: next_id,
                    at_sample: 1_000 + edits % 5_000,
                })
            }
            (5, Some(clip_id)) if live.len() > 4 => Some(AudioCommand::DeleteClip { clip_id }),
            _ => {
                next_id += 1;
                loads += 1;
                h.load_clip_from_wav(next_id, 1 + next_id % 4, 0, wav.clone(), "load".into());
                None
            }
        };
        if let Some(cmd) = cmd {
            h.dispatch(cmd);
        }
        edits += 1;
        // One engine-loop pass: worker results, parked edits, the sweep.
        h.poll_deferred_clip_commands();
        if edits.is_multiple_of(16) {
            h.sweep_retired();
        }
        h.drain_events();
    }
    renderer.join().expect("renderer");

    assert!(edits > 100, "the handlers actually raced the callback ({edits} edits)");
    assert!(loads > 0, "and loaded clips through the pool");

    // Every load lands once the pool drains; nothing is left for a reader
    // to free.
    h.settle_imports(Duration::from_secs(30));
    h.poll_deferred_clip_commands();
    assert!(h.clip_ids().contains(&BASE_CLIP));
    h.sweep_retired();
    assert!(h.shared().retired.is_empty(), "the sweep freed every replaced graph");
    let _ = std::fs::remove_dir_all(&dir);
}
