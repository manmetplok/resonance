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
    count_in_arm, EngineHandlerHarness, LatencyComp, LiveMidiEvent, MixAudioHarness,
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
// Loop on, distinct takes off: the single clip's loop trim
// ---------------------------------------------------------------------------

/// Record pressed inside the loop, distinct takes off: nothing comes
/// before the punch-in to trim, so the take must sit where it was played,
/// not at `loop_in` (BUG-69). It used to be placed at `loop_in` with no
/// start trim — early by `start − loop_in`.
#[test]
fn a_take_punched_in_mid_loop_lands_where_it_was_played() {
    const PUNCH_IN: u64 = LOOP_IN + 6_000;
    let (mut h, dir) = armed("loop-mid");
    h.set_loop_range(true, LOOP_IN, LOOP_IN + LOOP_LEN);
    h.shared().playhead.store(PUNCH_IN, Ordering::Relaxed);
    let mut input = h.record_with_fake_input(0, 2).expect("session opened");
    h.shared().latch_recording_start();
    push_ramp(&mut input, 0, 4_000);
    h.recording_tick();
    h.stop();

    let takes = finished(&h.drain_events());
    assert_eq!(takes.len(), 1, "one take");
    let (id, start, duration) = takes[0];
    assert_eq!(start, PUNCH_IN, "the take sits on the punch-in");
    assert_eq!(duration, 4_000);
    let c = clip(&h, id);
    assert_eq!(c.start_sample, PUNCH_IN);
    assert_eq!(c.trim_start_frames, 0, "nothing trimmed off the front");
    let _ = std::fs::remove_dir_all(dir);
}

/// Record pressed before the loop still trims to it: the take starts at
/// `loop_in`, its pre-loop audio trimmed off.
#[test]
fn a_take_punched_in_before_the_loop_is_trimmed_to_loop_in() {
    const PUNCH_IN: u64 = LOOP_IN - 2_000;
    let (mut h, dir) = armed("loop-before");
    h.set_loop_range(true, LOOP_IN, LOOP_IN + LOOP_LEN);
    h.shared().playhead.store(PUNCH_IN, Ordering::Relaxed);
    let mut input = h.record_with_fake_input(0, 2).expect("session opened");
    h.shared().latch_recording_start();
    push_ramp(&mut input, 0, 4_000);
    h.recording_tick();
    h.stop();

    let takes = finished(&h.drain_events());
    assert_eq!(takes.len(), 1, "one take");
    let (id, start, duration) = takes[0];
    assert_eq!(start, LOOP_IN);
    assert_eq!(duration, 2_000);
    assert_eq!(clip(&h, id).trim_start_frames, 2_000);
    let _ = std::fs::remove_dir_all(dir);
}

/// Record pressed at or after `loop_out` with the loop on: the playhead
/// never reaches the seam, so there is no loop pass to trim to and the
/// take is kept whole (BUG-74). The trim used to compute an empty range
/// and delete the take's file — the whole take silently lost.
#[test]
fn a_take_started_past_the_loop_is_kept_whole() {
    for punch_in in [LOOP_IN + LOOP_LEN, LOOP_IN + LOOP_LEN + 10_000] {
        let (mut h, dir) = armed("loop-past");
        h.set_loop_range(true, LOOP_IN, LOOP_IN + LOOP_LEN);
        h.shared().playhead.store(punch_in, Ordering::Relaxed);
        let mut input = h.record_with_fake_input(0, 2).expect("session opened");
        h.shared().latch_recording_start();
        push_ramp(&mut input, 0, 4_000);
        h.recording_tick();
        h.stop();

        let takes = finished(&h.drain_events());
        assert_eq!(takes.len(), 1, "punch-in {punch_in}: the take must survive");
        let (id, start, duration) = takes[0];
        assert_eq!(start, punch_in);
        assert_eq!(duration, 4_000);
        let c = clip(&h, id);
        assert_eq!((c.trim_start_frames, c.trim_end_frames), (0, 0));
        let _ = std::fs::remove_dir_all(dir);
    }
}

/// Latency compensation can place a take that was latched past
/// `loop_out` back inside the loop; the raw playhead still never wraps,
/// so it is kept whole too.
#[test]
fn a_take_latched_past_the_loop_but_compensated_into_it_is_kept_whole() {
    const PDC: u64 = 2_000;
    const PUNCH_IN: u64 = LOOP_IN + LOOP_LEN + 500;
    let (mut h, dir) = armed("loop-past-pdc");
    h.set_latency_comp(latent_plugin_comp(PDC));
    h.set_loop_range(true, LOOP_IN, LOOP_IN + LOOP_LEN);
    h.shared().playhead.store(PUNCH_IN, Ordering::Relaxed);
    let mut input = h.record_with_fake_input(0, 2).expect("session opened");
    h.shared().latch_recording_start();
    push_ramp(&mut input, 0, 4_000);
    h.recording_tick();
    h.stop();

    let takes = finished(&h.drain_events());
    assert_eq!(takes.len(), 1, "one take");
    let (_, start, duration) = takes[0];
    assert_eq!(start, PUNCH_IN - PDC);
    assert_eq!(duration, 4_000, "nothing cut at loop_out");
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

// ---------------------------------------------------------------------------
// BUG-20: a seek or a loop move during cycle-record
// ---------------------------------------------------------------------------

/// Every `TakeCaptured` as `(content, slot, extent)`.
fn captured(events: &[AudioEvent]) -> Vec<(TakeContent, TimelineRange, TimelineRange)> {
    events
        .iter()
        .filter_map(|e| match e {
            AudioEvent::TakeCaptured {
                content,
                slot,
                extent,
                ..
            } => Some((content.clone(), *slot, *extent)),
            _ => None,
        })
        .collect()
}

/// A seek mid-pass ends the pass there and starts the next one where the
/// playhead landed: the pass cut afterwards falls at the loop end the
/// playhead really reaches, and every pass after that is a whole loop. The
/// session used to ignore the seek and keep cutting one loop length from
/// the punch-in, so every later pass straddled the real seam.
#[test]
fn a_seek_during_cycle_record_rolls_the_pass_and_recuts_from_the_new_playhead() {
    const SEEK_TO: u64 = LOOP_IN + LOOP_LEN / 4;
    let (mut h, dir, mut input) = cycle_session("seek", 0);

    push_ramp(&mut input, 0, LOOP_LEN / 2);
    h.recording_tick();
    assert!(audio_takes(&h.drain_events()).is_empty(), "half a pass");

    h.dispatch(AudioCommand::SeekTo(SEEK_TO));
    let at_seek = audio_takes(&h.drain_events());
    assert_eq!(at_seek.len(), 1, "the seek rolls the pass it interrupted");
    assert_eq!(at_seek[0].1, TimelineRange::new(LOOP_IN, LOOP_LEN / 2));

    // The input callback's next push latches the post-seek playhead.
    h.shared().latch_recording_start();
    push_ramp(&mut input, LOOP_LEN / 2, LOOP_LEN - LOOP_LEN / 4);
    h.recording_tick();
    let after = audio_takes(&h.drain_events());
    assert_eq!(after.len(), 1, "the pass from the seek ends at loop_out");
    let (resumed, extent) = after[0];
    assert_eq!(extent, TimelineRange::new(SEEK_TO, LOOP_LEN - LOOP_LEN / 4));
    assert_eq!(clip(&h, resumed).start_sample, SEEK_TO);
    assert_eq!(clip(&h, resumed).source.as_frames()[0], (LOOP_LEN / 2) as f32);

    let next = LOOP_LEN / 2 + LOOP_LEN - LOOP_LEN / 4;
    push_ramp(&mut input, next, LOOP_LEN);
    h.recording_tick();
    let whole = audio_takes(&h.drain_events());
    assert_eq!(whole.len(), 1);
    let (id, extent) = whole[0];
    assert_eq!(extent, TimelineRange::new(LOOP_IN, LOOP_LEN), "a whole loop again");
    assert_eq!(clip(&h, id).source.as_frames()[0], next as f32);
    h.stop();
    let _ = std::fs::remove_dir_all(dir);
}

/// Moving the loop mid-run rolls the pass under the old slot, and the
/// passes after it are cut and filed against the new range.
#[test]
fn moving_the_loop_during_cycle_record_cuts_later_passes_at_the_new_seams() {
    const NEW_IN: u64 = 200_000;
    const NEW_LEN: u64 = 30_000;
    const LANDED: u64 = NEW_IN + 1_000;
    let (mut h, dir, mut input) = cycle_session("move-loop", 0);

    push_ramp(&mut input, 0, LOOP_LEN / 2);
    h.recording_tick();
    h.drain_events();

    // The playhead is inside the new range when it moves.
    h.set_playhead(LANDED);
    h.set_loop_range(true, NEW_IN, NEW_IN + NEW_LEN);
    let rolled = captured(&h.drain_events());
    assert_eq!(rolled.len(), 1, "the interrupted pass is rolled: {rolled:?}");
    assert_eq!(rolled[0].1, TimelineRange::new(LOOP_IN, LOOP_LEN), "under the old slot");
    assert_eq!(rolled[0].2, TimelineRange::new(LOOP_IN, LOOP_LEN / 2));

    h.shared().latch_recording_start();
    push_ramp(&mut input, LOOP_LEN / 2, NEW_LEN - 1_000 + NEW_LEN);
    h.recording_tick();
    let later = captured(&h.drain_events());
    let new_slot = TimelineRange::new(NEW_IN, NEW_LEN);
    let slots_extents: Vec<_> = later.iter().map(|(_, s, e)| (*s, *e)).collect();
    assert_eq!(
        slots_extents,
        vec![
            (new_slot, TimelineRange::new(LANDED, NEW_LEN - 1_000)),
            (new_slot, new_slot),
        ],
    );
    h.stop();
    let _ = std::fs::remove_dir_all(dir);
}

/// Re-sending the loop range the run already cycles over is not a move.
#[test]
fn re_sending_the_same_loop_range_does_not_roll_a_pass() {
    let (mut h, dir, mut input) = cycle_session("same-loop", 0);
    push_ramp(&mut input, 0, LOOP_LEN / 2);
    h.recording_tick();
    h.set_loop_range(true, LOOP_IN, LOOP_IN + LOOP_LEN);
    assert!(captured(&h.drain_events()).is_empty());
    push_ramp(&mut input, LOOP_LEN / 2, LOOP_LEN / 2);
    h.recording_tick();
    let takes = audio_takes(&h.drain_events());
    assert_eq!(takes.len(), 1);
    assert_eq!(takes[0].1, TimelineRange::new(LOOP_IN, LOOP_LEN));
    h.stop();
    let _ = std::fs::remove_dir_all(dir);
}

/// The MIDI half of a pass rolls with its audio half at a seek, so the
/// two stay paired: a note played before the seek is in the pass the seek
/// ends, one played after it in the next.
#[test]
fn a_seek_rolls_the_midi_pass_with_the_audio_pass() {
    const KEYS: TrackId = 2;
    let (mut h, dir) = armed("seek-midi");
    let keys = Track::with_type(KEYS, "keys".into(), TrackType::Instrument);
    keys.set_record_armed(true);
    h.push_track(keys);
    h.set_loop_range(true, LOOP_IN, LOOP_IN + LOOP_LEN);
    h.set_loop_record_mode(true);
    h.shared().playhead.store(LOOP_IN, Ordering::Relaxed);
    let mut input = h.record_with_fake_input(0, 2).expect("session opened");
    h.shared().latch_recording_start();

    let note = |note: u8| LiveMidiEvent::InboundNoteOn {
        track_id: KEYS,
        note,
        velocity: 0.8,
        arrival: std::time::Instant::now(),
    };
    let notes_of = |c: &TakeContent| match c {
        TakeContent::Midi { notes } => Some(notes.iter().map(|n| n.note).collect::<Vec<u8>>()),
        TakeContent::Audio { .. } => None,
    };
    push_ramp(&mut input, 0, 6_000);
    h.set_playhead(LOOP_IN + 5_000);
    h.live_midi_event(note(60));
    h.recording_tick();
    h.drain_events();

    h.dispatch(AudioCommand::SeekTo(LOOP_IN + 2_000));
    let at_seek = captured(&h.drain_events());
    let audio = at_seek.iter().filter(|(c, _, _)| notes_of(c).is_none()).count();
    let midi: Vec<_> = at_seek.iter().filter_map(|(c, _, _)| notes_of(c)).collect();
    assert_eq!(audio, 1, "the audio half rolls at the seek");
    assert_eq!(midi, vec![vec![60]], "and the MIDI half with it");

    h.shared().latch_recording_start();
    push_ramp(&mut input, 6_000, 1_000);
    h.live_midi_event(note(64));
    h.stop();
    let trailing = captured(&h.drain_events());
    let midi: Vec<_> = trailing.iter().filter_map(|(c, _, _)| notes_of(c)).collect();
    assert_eq!(midi, vec![vec![64]], "the note after the seek is the next pass's");
    let _ = std::fs::remove_dir_all(dir);
}
