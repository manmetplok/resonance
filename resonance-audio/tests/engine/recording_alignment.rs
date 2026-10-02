//! Where a recorded take lands, end to end through the real Record / Stop
//! handlers and the engine loop's recording step, over a fake input
//! stream (code review RT-01, RT-02, RT-08).
//!
//! The fake input is the recording ring's producer, handed back by
//! `EngineHandlerHarness::record_with_fake_input`: a test plays the input
//! callback by latching the start (`SharedState::latch_recording_start`)
//! and pushing frames, then runs the engine loop's tick
//! (`recording_tick`). Every pushed frame carries its own index, so a
//! take's first sample says exactly which input frame it starts on.

use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::Arc;

use ringbuf::traits::Producer;

use resonance_audio::test_support::{
    count_in_arm, EngineHandlerHarness, LatencyComp, MixAudioHarness,
};
use resonance_audio::types::*;
use resonance_common::{TakeContent, TimelineRange};

const TRACK: TrackId = 1;

fn tempdir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "resonance-rec-align-{tag}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// A harness with a project dir and one armed audio track.
fn armed(tag: &str) -> (EngineHandlerHarness, PathBuf) {
    let mut h = EngineHandlerHarness::new();
    let dir = tempdir(tag);
    h.set_project_dir(dir.clone());
    let track = Track::new(TRACK, "Guitar".into());
    track.set_record_armed(true);
    h.push_track(track);
    (h, dir)
}

/// Push `frames` stereo frames whose samples are their global input
/// frame index, starting at `start`.
fn push_ramp(prod: &mut ringbuf::HeapProd<f32>, start: u64, frames: u64) {
    let mut chunk = Vec::with_capacity(frames as usize * 2);
    for f in start..start + frames {
        chunk.push(f as f32);
        chunk.push(f as f32);
    }
    assert_eq!(prod.push_slice(&chunk), chunk.len(), "test ring too small");
}

/// A comp table whose whole pipeline latency is `samples` — what the
/// engine publishes once a plugin reporting that latency sits on a track.
fn latent_plugin_comp(samples: u64) -> LatencyComp {
    LatencyComp::new(samples, &[], 0, &[])
}

fn clip(h: &EngineHandlerHarness, id: ClipId) -> Arc<AudioClip> {
    h.shared()
        .clips()
        .iter()
        .find(|c| c.id == id)
        .cloned()
        .unwrap_or_else(|| panic!("clip {id} not in the render graph"))
}

fn finished(events: &[AudioEvent]) -> Vec<(ClipId, SamplePos, u64)> {
    events
        .iter()
        .filter_map(|e| match e {
            AudioEvent::RecordingFinished {
                clip_id,
                start_sample,
                duration_samples,
                ..
            } => Some((*clip_id, *start_sample, *duration_samples)),
            _ => None,
        })
        .collect()
}

/// `(clip id, extent)` of every audio take captured.
fn audio_takes(events: &[AudioEvent]) -> Vec<(ClipId, TimelineRange)> {
    events
        .iter()
        .filter_map(|e| match e {
            AudioEvent::TakeCaptured {
                extent,
                content: TakeContent::Audio { clip_ref },
                ..
            } => Some((*clip_ref, *extent)),
            _ => None,
        })
        .collect()
}

// ---------------------------------------------------------------------------
// RT-01: PDC + master latency
// ---------------------------------------------------------------------------

/// The performer plays against a mix that PDC and the master chain hold
/// `PDC + MASTER` samples behind the raw playhead, so the take must land
/// that much earlier than the latch. It used to land exactly on the latch
/// (only device I/O latency was subtracted): every overdub late by the
/// project's plugin latency.
#[test]
fn a_take_is_placed_behind_the_plugin_and_master_latency_the_performer_heard() {
    const PDC: u64 = 2_048;
    const MASTER: u64 = 512;
    const PUNCH_IN: u64 = 96_000;
    let (mut h, dir) = armed("pdc");
    h.set_latency_comp(latent_plugin_comp(PDC));
    h.shared()
        .master_latency_samples
        .store(MASTER, Ordering::Relaxed);
    h.shared().playhead.store(PUNCH_IN, Ordering::Relaxed);

    let mut input = h.record_with_fake_input(0, 2).expect("session opened");
    assert!(h.shared().recording.load(Ordering::SeqCst));

    // The plugin goes away mid-take: the take was latched against the
    // latency the performer heard when it started, and stays there.
    h.set_latency_comp(LatencyComp::empty());
    h.shared().master_latency_samples.store(0, Ordering::Relaxed);

    h.shared().latch_recording_start();
    push_ramp(&mut input, 0, 4_800);
    h.recording_tick();
    h.stop();

    let takes = finished(&h.drain_events());
    assert_eq!(takes.len(), 1, "one take");
    let (_, start, duration) = takes[0];
    assert_eq!(
        start,
        PUNCH_IN - PDC - MASTER,
        "the take must sit PDC + master latency before the latch"
    );
    assert_eq!(duration, 4_800);
    let _ = std::fs::remove_dir_all(dir);
}

/// Compensation that would start the take before sample 0 pins it at 0
/// and drops the input frames from before 0, so what is kept is still
/// aligned.
#[test]
fn a_take_compensated_past_zero_starts_at_zero_still_aligned() {
    const PDC: u64 = 3_000;
    const PUNCH_IN: u64 = 1_000;
    let (mut h, dir) = armed("pdc-zero");
    h.set_latency_comp(latent_plugin_comp(PDC));
    h.shared().playhead.store(PUNCH_IN, Ordering::Relaxed);

    let mut input = h.record_with_fake_input(0, 2).expect("session opened");
    h.shared().latch_recording_start();
    push_ramp(&mut input, 0, 6_000);
    h.recording_tick();
    h.stop();

    let takes = finished(&h.drain_events());
    let (id, start, duration) = takes[0];
    assert_eq!(start, 0);
    let dropped = PDC - PUNCH_IN;
    assert_eq!(duration, 6_000 - dropped);
    assert_eq!(
        clip(&h, id).source.as_frames()[0],
        dropped as f32,
        "the kept audio starts on the input frame that belongs at sample 0"
    );
    let _ = std::fs::remove_dir_all(dir);
}

// ---------------------------------------------------------------------------
// RT-02: cycle-record passes cut by sample count
// ---------------------------------------------------------------------------

const LOOP_IN: u64 = 48_000;
const LOOP_LEN: u64 = 24_000;

fn cycle_session(tag: &str, pdc: u64) -> (EngineHandlerHarness, PathBuf, ringbuf::HeapProd<f32>) {
    let (mut h, dir) = armed(tag);
    h.set_latency_comp(latent_plugin_comp(pdc));
    h.set_loop_range(true, LOOP_IN, LOOP_IN + LOOP_LEN);
    h.set_loop_record_mode(true);
    h.shared().playhead.store(LOOP_IN, Ordering::Relaxed);
    let input = h.record_with_fake_input(0, 2).expect("session opened");
    h.shared().latch_recording_start();
    (h, dir, input)
}

/// 1.5 loops of input arrive before the engine thread gets to look — the
/// tick ran late, or the wrap landed just after the last poll. Pass 0
/// must still end exactly on the loop boundary and pass 1 start on input
/// frame `LOOP_LEN`; the cut used to fall wherever the ring happened to be
/// when the tick noticed the wrap.
#[test]
fn loop_passes_are_cut_at_the_seam_not_at_the_engine_tick() {
    let (mut h, dir, mut input) = cycle_session("cycle", 0);

    push_ramp(&mut input, 0, LOOP_LEN * 3 / 2);
    h.recording_tick();
    let first = audio_takes(&h.drain_events());
    assert_eq!(first.len(), 1, "exactly one pass is complete");
    let (pass0, extent0) = first[0];
    assert_eq!(extent0, TimelineRange::new(LOOP_IN, LOOP_LEN));
    let frames0 = clip(&h, pass0).source.as_frames().to_vec();
    assert_eq!(frames0.len() as u64, LOOP_LEN * 2);
    assert_eq!(frames0[frames0.len() - 2], (LOOP_LEN - 1) as f32);

    push_ramp(&mut input, LOOP_LEN * 3 / 2, LOOP_LEN / 2);
    h.recording_tick();
    let second = audio_takes(&h.drain_events());
    assert_eq!(second.len(), 1, "the second pass completes on its own count");
    let (pass1, extent1) = second[0];
    assert_eq!(extent1, TimelineRange::new(LOOP_IN, LOOP_LEN));
    let frames1 = clip(&h, pass1).source.as_frames().to_vec();
    assert_eq!(frames1[0], LOOP_LEN as f32, "pass 1 starts on input frame LOOP_LEN");
    assert_eq!(frames1[frames1.len() - 2], (2 * LOOP_LEN - 1) as f32);

    // The stop's trailing pass is whatever came after the last cut.
    push_ramp(&mut input, 2 * LOOP_LEN, 1_000);
    h.stop();
    let trailing = audio_takes(&h.drain_events());
    assert_eq!(trailing.len(), 1);
    let (pass2, extent2) = trailing[0];
    assert_eq!(extent2, TimelineRange::new(LOOP_IN, 1_000));
    assert_eq!(clip(&h, pass2).source.as_frames()[0], (2 * LOOP_LEN) as f32);
    let _ = std::fs::remove_dir_all(dir);
}

/// With latency, the performer hears the loop wrap `PDC` samples after the
/// raw playhead does. The punch-in pass is placed `PDC` early (RT-01) and
/// runs to timeline `loop_out`; every later pass then sits exactly on the
/// loop — placed at `loop_in`, cut on the same sample clock — where it
/// used to be placed at `loop_in` with no compensation at all.
#[test]
fn later_loop_passes_carry_the_same_compensation_as_the_first() {
    const PDC: u64 = 1_000;
    let (mut h, dir, mut input) = cycle_session("cycle-pdc", PDC);

    push_ramp(&mut input, 0, 2 * LOOP_LEN + PDC);
    h.recording_tick();
    let takes = audio_takes(&h.drain_events());
    assert_eq!(takes.len(), 2, "two complete passes");

    let (pass0, extent0) = takes[0];
    assert_eq!(extent0, TimelineRange::new(LOOP_IN - PDC, LOOP_LEN + PDC));
    let (pass1, extent1) = takes[1];
    assert_eq!(extent1, TimelineRange::new(LOOP_IN, LOOP_LEN));
    assert_eq!(
        clip(&h, pass1).source.as_frames()[0],
        (LOOP_LEN + PDC) as f32,
        "pass 1 starts where the performer heard the loop start again"
    );
    assert_eq!(clip(&h, pass0).start_sample, LOOP_IN - PDC);
    assert_eq!(clip(&h, pass1).start_sample, LOOP_IN);
    h.stop();
    let _ = std::fs::remove_dir_all(dir);
}

// ---------------------------------------------------------------------------
// RT-08: count-in → record starts on the exact frame
// ---------------------------------------------------------------------------

/// Not a divisor of the one-bar count-in (96 000 frames at 120 bpm, 4/4),
/// so the count-in ends mid-block.
const BLOCK: usize = 112;

/// The session — stream, take file — is open before the first click, and
/// the audio thread starts the take in the block where the count-in ends:
/// the playhead moves on by the frames past the count-in's end, so the
/// first playing block starts exactly `count_in_total` frames after the
/// count-in started. It used to idle until the engine thread's next tick
/// opened the input stream (up to 500 ms), with the performer already
/// playing.
#[test]
fn a_count_in_starts_the_take_on_its_last_frame() {
    const PUNCH_IN: u64 = 10_000;
    let (mut h, dir) = armed("count-in");
    h.shared().playhead.store(PUNCH_IN, Ordering::Relaxed);
    let mut input = h.record_with_fake_input(1, 2).expect("session opened at count-in start");

    let shared = h.shared_arc();
    assert!(shared.count_in_active.load(Ordering::SeqCst));
    assert!(!shared.recording.load(Ordering::SeqCst), "not capturing during the count-in");
    assert_eq!(shared.count_in_record_arm.load(Ordering::SeqCst), count_in_arm::ARMED);
    assert_eq!(h.recording_buffers_open(), 1, "take file open before the first click");
    let total = shared.count_in_total.load(Ordering::SeqCst);
    assert_eq!(total, 96_000);

    let mut cb = MixAudioHarness::on_shared(Arc::clone(&shared), BLOCK, 2, 48_000);
    let mut elapsed = 0u64;
    while !shared.recording.load(Ordering::SeqCst) {
        assert!(elapsed < total + BLOCK as u64, "the take never started");
        assert_eq!(shared.playhead.load(Ordering::SeqCst), PUNCH_IN, "pinned while counting in");
        cb.render();
        elapsed += BLOCK as u64;
    }
    assert!(elapsed >= total && elapsed < total + BLOCK as u64, "flipped in the block the count-in ended");
    assert!(!shared.count_in_active.load(Ordering::SeqCst));
    assert_eq!(
        shared.playhead.load(Ordering::SeqCst),
        PUNCH_IN + (elapsed - total),
        "the timeline started on the count-in's last frame"
    );
    // The very next block plays, from there.
    cb.render();
    elapsed += BLOCK as u64;
    assert_eq!(shared.playhead.load(Ordering::SeqCst), PUNCH_IN + (elapsed - total));

    // The engine loop only reports it.
    h.recording_tick();
    let events = h.drain_events();
    assert!(
        events
            .iter()
            .any(|e| matches!(e, AudioEvent::RecordingStarted { start_sample } if *start_sample == PUNCH_IN)),
        "RecordingStarted at the punch-in, got {events:?}"
    );
    assert_eq!(shared.count_in_record_arm.load(Ordering::SeqCst), count_in_arm::IDLE);

    // And the take records like any other.
    shared.latch_recording_start();
    push_ramp(&mut input, 0, 2_000);
    h.recording_tick();
    h.stop();
    assert_eq!(finished(&h.drain_events()).len(), 1);
    let _ = std::fs::remove_dir_all(dir);
}

/// Stop during the count-in: the take never started, so nothing is
/// finalized and the open take file is removed — and the armed flip can
/// no longer fire.
#[test]
fn stopping_during_the_count_in_discards_the_unstarted_take() {
    let (mut h, dir) = armed("count-in-stop");
    let _input = h.record_with_fake_input(1, 2).expect("session opened");
    let shared = h.shared_arc();
    let mut cb = MixAudioHarness::on_shared(Arc::clone(&shared), BLOCK, 2, 48_000);
    for _ in 0..4 {
        cb.render();
    }
    h.stop();

    let events = h.drain_events();
    assert!(finished(&events).is_empty(), "no take: {events:?}");
    assert_eq!(h.recording_buffers_open(), 0);
    assert_eq!(shared.count_in_record_arm.load(Ordering::SeqCst), count_in_arm::IDLE);
    assert!(!shared.count_in_active.load(Ordering::SeqCst));
    assert!(!shared.recording.load(Ordering::SeqCst));
    let wavs = std::fs::read_dir(dir.join("audio"))
        .map(|d| d.count())
        .unwrap_or(0);
    assert_eq!(wavs, 0, "the unstarted take's file is gone");
    let _ = std::fs::remove_dir_all(dir);
}

// ---------------------------------------------------------------------------
// RT-17: ring sized in frames
// ---------------------------------------------------------------------------

#[test]
fn the_recording_ring_holds_the_same_time_at_any_channel_count() {
    use resonance_audio::test_support::{recording_ring_len, RECORDING_RING_SECONDS};
    for channels in [2u16, 8, 18, 32] {
        let len = recording_ring_len(48_000, channels);
        assert!(
            len >= RECORDING_RING_SECONDS * 48_000 * channels as usize,
            "{channels} channels: {len} samples is under {RECORDING_RING_SECONDS} s"
        );
    }
}
