//! The audio thread's playhead publish must never overwrite a reposition
//! the engine control thread made while the block was rendering
//! (code review MIX-01).
//!
//! `render_playing_block` observes the playhead at the top of the block,
//! renders, and publishes `observed + frames` at the bottom. A Seek /
//! Stop / MIDI-clock song-position store that lands in between used to
//! be clobbered by that publish — the window is the block's whole render
//! time, so a seek during playback was lost with a probability of
//! roughly render_time / period.
//!
//! The first test is the race itself, made reproducible: one thread runs
//! the real callback back to back (so the "render in flight" window is
//! nearly the whole loop), the other plays the control thread and seeks
//! between blocks. Every seek target carries a residue mod block size
//! that identifies it, so a clobbered seek shows up as the wrong residue
//! a few blocks later. The remaining tests pin the publish primitive's
//! contract directly.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

use resonance_audio::test_support::{commit_playhead, MixAudioHarness, SharedState};
use resonance_audio::types::*;

const SR: u32 = 48_000;
const BLOCK: usize = 128;
const CH: usize = 2;
/// Blocks of audio in the fixture clip; every seek target lands inside it
/// so the playing branch always has real work to do.
const CLIP_BLOCKS: usize = 400;

fn fixture() -> MixAudioHarness {
    let track = Track::new(1, "clips".into());
    track.set_output(TrackOutput::Master);
    let clip = AudioClip {
        id: 1,
        track_id: 1,
        start_sample: 0,
        source: ClipSource::Memory(vec![0.1; CLIP_BLOCKS * BLOCK * CH]),
        name: "c1".into(),
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
    };
    let mut tempo = TempoMap::default();
    tempo.rebuild_bar_table(SR);
    MixAudioHarness::new(
        vec![track],
        Vec::new(),
        vec![clip],
        Vec::new(),
        Vec::new(),
        tempo,
        BLOCK,
        CH,
        SR,
        true,
    )
}

/// Seek target for `attempt`: somewhere inside the clip, with a residue
/// mod `BLOCK` (`1..=BLOCK-1`) that differs from the previous attempt's,
/// so "the playhead kept advancing from where it was" and "the playhead
/// advanced from the seek target" are distinguishable a few blocks later.
fn seek_target(attempt: u64) -> u64 {
    let residue = attempt % (BLOCK as u64 - 1) + 1;
    (attempt % 100) * (BLOCK as u64) * 3 + residue
}

#[test]
fn seek_during_a_rendering_block_is_never_lost() {
    let mut h = fixture();
    let shared = h.shared_arc();
    shared.playing.store(true, Ordering::SeqCst);

    let blocks = Arc::new(AtomicU64::new(0));
    let stop = Arc::new(AtomicBool::new(false));
    let render = {
        let blocks = Arc::clone(&blocks);
        let stop = Arc::clone(&stop);
        std::thread::spawn(move || {
            while !stop.load(Ordering::Relaxed) {
                h.render();
                blocks.fetch_add(1, Ordering::Release);
            }
            h
        })
    };

    let mut lost = Vec::new();
    for attempt in 1..=400u64 {
        let target = seek_target(attempt);
        let before = blocks.load(Ordering::Acquire);
        // The control thread's Seek.
        shared.playhead.store(target, Ordering::SeqCst);
        // Let a few blocks go by, then look at where playback continued.
        while blocks.load(Ordering::Acquire) < before + 4 {
            std::hint::spin_loop();
        }
        let now = shared.playhead.load(Ordering::SeqCst);
        let honoured = now >= target && now % BLOCK as u64 == target % BLOCK as u64;
        if !honoured {
            lost.push((attempt, target, now));
        }
    }
    stop.store(true, Ordering::Relaxed);
    let _h = render.join().expect("render thread");

    assert!(
        lost.is_empty(),
        "{} of 400 seeks were overwritten by the audio thread's playhead publish \
         (attempt, seek target, playhead a few blocks later): {:?}",
        lost.len(),
        &lost[..lost.len().min(8)]
    );
}

#[test]
fn commit_publishes_the_advance_when_nobody_moved_the_playhead() {
    let shared = SharedState::default();
    shared.playhead.store(1_000, Ordering::SeqCst);
    assert!(commit_playhead(&shared, 1_000, 1_128));
    assert_eq!(shared.playhead.load(Ordering::SeqCst), 1_128);
}

#[test]
fn commit_yields_to_a_reposition_made_while_the_block_rendered() {
    let shared = SharedState::default();
    shared.playhead.store(1_000, Ordering::SeqCst);
    let observed = shared.playhead.load(Ordering::SeqCst);
    // Seek (or Stop-to-zero, or a MIDI-clock song position) lands while
    // the block is rendering.
    shared.playhead.store(50_000, Ordering::SeqCst);
    assert!(
        !commit_playhead(&shared, observed, observed + 128),
        "a lost CAS is the signal that someone repositioned the transport"
    );
    assert_eq!(
        shared.playhead.load(Ordering::SeqCst),
        50_000,
        "the reposition wins; the stale advance must not clobber it"
    );
}

#[test]
fn stop_to_zero_during_a_block_leaves_the_transport_at_zero() {
    let mut h = fixture();
    let shared = h.shared_arc();
    shared.playing.store(true, Ordering::SeqCst);
    shared.playhead.store(BLOCK as u64 * 10, Ordering::SeqCst);
    h.render();
    assert_eq!(shared.playhead.load(Ordering::SeqCst), BLOCK as u64 * 11);

    // The engine's Stop: playing off, playhead parked at 0 — observed by
    // the callback only through its CAS, since `playing` was already
    // read for this block.
    let observed = shared.playhead.load(Ordering::SeqCst);
    shared.playing.store(false, Ordering::SeqCst);
    shared.playhead.store(0, Ordering::SeqCst);
    assert!(!commit_playhead(&shared, observed, observed + BLOCK as u64));
    assert_eq!(shared.playhead.load(Ordering::SeqCst), 0);

    // The next (stopped) block leaves it there.
    h.render();
    assert_eq!(shared.playhead.load(Ordering::SeqCst), 0);
}
