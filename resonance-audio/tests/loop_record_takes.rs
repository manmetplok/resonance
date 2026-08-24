//! Proves the core cycle-record invariant of todo #408: looping over a
//! region while recording yields one distinct take per pass, and rolling
//! the capture at each loop seam never drops an input frame.
//!
//! These drive [`RecordingState::roll_audio_pass`] directly — the same
//! engine-thread routine the loop-seam poll calls — through the
//! `#[doc(hidden)] pub use` test surface in `lib.rs`, so the capture
//! mechanism is exercised without spinning up the audio engine or a real
//! input device.

use std::path::PathBuf;

use ringbuf::traits::{Producer, Split};
use ringbuf::HeapRb;

use resonance_audio::RecordingState;

fn make_tempdir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "resonance-looprec-test-{}-{}",
        tag,
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Push `frames` stereo frames into `prod`, encoding each channel as the
/// running global frame index so a later read can prove no frame was
/// dropped or reordered across a seam.
fn push_ramp(prod: &mut ringbuf::HeapProd<f32>, start_frame: u64, frames: u64) {
    let mut chunk = vec![0.0f32; 1000 * 2];
    let mut written = 0u64;
    while written < frames {
        let n = (frames - written).min(1000) as usize;
        for f in 0..n {
            let v = (start_frame + written + f as u64) as f32;
            chunk[f * 2] = v;
            chunk[f * 2 + 1] = v;
        }
        prod.push_slice(&chunk[..n * 2]);
        written += n as u64;
    }
}

#[test]
fn three_loop_passes_yield_three_takes_with_no_dropped_frames() {
    let project_dir = make_tempdir("three-passes");
    let audio_dir = project_dir.join("audio");
    let sr = 48_000u32;
    let loop_frames = 24_000u64; // half-second loop region
    let passes = 3u64;

    let mut rec = RecordingState::new(sr);
    let ring: HeapRb<f32> = HeapRb::new((loop_frames as usize) * 2 * 2);
    let (mut prod, cons) = ring.split();
    rec.ring_consumer = Some(cons);
    rec.input_channels = 2;
    rec.input_sample_rate = sr;
    rec.start_sample = 0;

    // The first pass's writer is the one created at record start.
    let buf = RecordingState::create_track_buf(
        &project_dir, /* track */ 7, /* clip */ 1, sr, sr, /* port */ 0, /* mono */ false,
    )
    .unwrap();
    rec.buffers.insert(7, buf);

    let clips = parking_lot::RwLock::new(Vec::new());
    let mut next_clip_id = 2u64; // pass 0 already holds clip id 1

    // Feed one loop's worth of audio and roll at the seam, three times.
    for pass in 0..passes {
        push_ramp(&mut prod, pass * loop_frames, loop_frames);
        let rolled = rec.roll_audio_pass(
            sr,
            /* clip_start_sample */ 0,
            &clips,
            &audio_dir,
            &mut next_clip_id,
            /* reopen */ true,
        );
        assert_eq!(rolled.len(), 1, "pass {pass} should produce exactly one take");
        assert_eq!(
            rolled[0].duration_samples, loop_frames,
            "pass {pass} take has the wrong length"
        );
    }

    // Exactly three retained takes — the fourth writer is open but empty.
    let guard = clips.read();
    assert_eq!(guard.len(), passes as usize, "expected one clip per pass");

    // Every take is a distinct clip with its own on-disk WAV holding a full
    // loop's worth of frames, and the concatenation of all three reproduces
    // the continuous input ramp — i.e. no frame was dropped at any seam.
    let mut ids: Vec<u64> = guard.iter().map(|c| c.id).collect();
    ids.sort_unstable();
    assert_eq!(ids, vec![1, 2, 3], "takes should use fresh clip ids per pass");

    let mut expected_frame = 0u64;
    for id in [1u64, 2, 3] {
        let clip = guard.iter().find(|c| c.id == id).unwrap();
        assert_eq!(
            clip.source.frame_count(),
            loop_frames,
            "take {id} should hold one loop of frames"
        );
        let frames = clip.source.as_frames();
        for f in 0..loop_frames {
            assert_eq!(
                frames[f as usize * 2],
                expected_frame as f32,
                "take {id} dropped or reordered a frame at offset {f}"
            );
            expected_frame += 1;
        }
    }
    assert_eq!(
        expected_frame,
        passes * loop_frames,
        "total captured frames must equal everything fed in"
    );

    drop(guard);
    let _ = std::fs::remove_dir_all(&project_dir);
}

#[test]
fn trailing_pass_rolls_without_reopening_and_clears_buffers() {
    let project_dir = make_tempdir("trailing");
    let audio_dir = project_dir.join("audio");
    let sr = 48_000u32;
    let loop_frames = 12_000u64;

    let mut rec = RecordingState::new(sr);
    let ring: HeapRb<f32> = HeapRb::new((loop_frames as usize) * 2 * 2);
    let (mut prod, cons) = ring.split();
    rec.ring_consumer = Some(cons);
    rec.input_channels = 2;
    rec.input_sample_rate = sr;
    rec.start_sample = 0;

    let buf =
        RecordingState::create_track_buf(&project_dir, 1, 1, sr, sr, 0, false).unwrap();
    rec.buffers.insert(1, buf);

    let clips = parking_lot::RwLock::new(Vec::new());
    let mut next_clip_id = 2u64;

    // One seam roll (reopen) then a final trailing roll at stop (no reopen).
    push_ramp(&mut prod, 0, loop_frames);
    let _ = rec.roll_audio_pass(sr, 0, &clips, &audio_dir, &mut next_clip_id, true);
    push_ramp(&mut prod, loop_frames, loop_frames);
    drop(prod); // emulate the input stream closing on stop
    let trailing = rec.roll_audio_pass(sr, 0, &clips, &audio_dir, &mut next_clip_id, false);

    assert_eq!(trailing.len(), 1, "trailing pass should emit one take");
    assert_eq!(clips.read().len(), 2, "two passes -> two takes");
    assert!(
        rec.buffers.is_empty(),
        "the trailing (no-reopen) roll must close out the per-track buffers"
    );

    let _ = std::fs::remove_dir_all(&project_dir);
}

// ---------------------------------------------------------------------------
// Take-id allocation and the audio/MIDI capture split (todo #409, doc #292)
// ---------------------------------------------------------------------------

/// A take id must be unique within its group. Deriving it from
/// `pass_index` was only sound while a `(group, pass)` pair could emit at
/// most once — and it cannot be relied on to: `finalize_loop_record_pass`
/// runs an audio loop and a MIDI loop that resolve to the *same* group for
/// the same track, so a track present in both emits twice at one
/// `pass_index`.
///
/// This drives the real allocator with exactly that shape. Under the old
/// `take_id = pass_index` rule both takes below would come back as `0`.
#[test]
fn two_captures_in_one_pass_get_distinct_take_ids() {
    use resonance_audio::__test_support::push_take;
    use resonance_common::{TakeContent, TakeGroup, TimelineRange};

    let slot = TimelineRange::new(0, 48_000);
    let mut group = TakeGroup::new(1, 7, slot);

    // Pass 0 emits twice for the same track: the audio roll, then the MIDI
    // capture — the exact sequence `finalize_loop_record_pass` produces.
    let audio_id = push_take(&mut group, slot, 0, &TakeContent::Audio { clip_ref: 100 });
    let midi_id = push_take(
        &mut group,
        slot,
        0,
        &TakeContent::Midi { notes: Vec::new() },
    );

    assert_ne!(
        audio_id, midi_id,
        "two takes captured in one pass must not share an id"
    );
    assert_eq!(group.takes.len(), 2, "both takes must be retained");
    assert!(group.take(audio_id).is_some());
    assert!(group.take(midi_id).is_some());
}

/// Ids stay unique — and resolvable — across several passes, including
/// when a pass emits twice. The group is the id's scope, so the sequence
/// is dense and monotonic regardless of how the passes broke down.
#[test]
fn take_ids_stay_unique_across_passes() {
    use resonance_audio::__test_support::push_take;
    use resonance_common::{TakeContent, TakeGroup, TimelineRange};

    let slot = TimelineRange::new(0, 48_000);
    let mut group = TakeGroup::new(1, 7, slot);

    let mut ids = Vec::new();
    for pass in 0..3u32 {
        ids.push(push_take(
            &mut group,
            slot,
            pass,
            &TakeContent::Audio {
                clip_ref: 100 + u64::from(pass),
            },
        ));
        // Pass 1 also yields a MIDI take, as a dual-armed track would.
        if pass == 1 {
            ids.push(push_take(
                &mut group,
                slot,
                pass,
                &TakeContent::Midi { notes: Vec::new() },
            ));
        }
    }

    let mut sorted = ids.clone();
    sorted.sort_unstable();
    sorted.dedup();
    assert_eq!(sorted.len(), ids.len(), "take ids must all be distinct: {ids:?}");
    assert_eq!(sorted, vec![0, 1, 2, 3], "ids should be dense and monotonic");
    for id in ids {
        assert!(group.take(id).is_some(), "take {id} must be resolvable");
    }
}

/// The capture-side predicate: which armed tracks get an audio recording
/// buffer. A track whose instrument runs in-process records a MIDI
/// performance, not audio — giving it a buffer wrote a junk WAV per pass
/// and filed a spurious second take (ba doc #292).
#[test]
fn only_tracks_without_an_in_process_instrument_capture_audio() {
    use resonance_audio::types::{Track, TrackType};

    let audio = Track::new(1, "audio".into());
    assert!(
        !audio.runs_internal_instrument(),
        "an audio track captures audio"
    );

    let instrument = Track::with_type(2, "synth".into(), TrackType::Instrument);
    assert!(
        instrument.runs_internal_instrument(),
        "an in-process instrument track must NOT open an audio recording buffer"
    );
}

/// External-instrument tracks (epic #39) are `Instrument`-typed and accept
/// MIDI, but their synth is outboard: the audio comes back on the return
/// input and recording it is the entire point of the feature. Excluding
/// them — as a blanket `accepts_midi()` test would — would break it, so
/// pin the distinction rather than leaving it to the predicate's shape.
#[test]
fn external_instrument_tracks_still_capture_their_audio_return() {
    use resonance_audio::types::{Track, TrackType};

    let external = Track::with_type(3, "outboard".into(), TrackType::Instrument);
    external.set_external(true);

    assert!(external.track_type.accepts_midi(), "precondition: accepts MIDI");
    assert!(
        !external.runs_internal_instrument(),
        "an external instrument's audio return must still be recorded"
    );
}

/// `Vocal` tracks also accept MIDI, but they render through the audio path
/// rather than an in-process instrument, so they keep audio capture too.
#[test]
fn vocal_tracks_still_capture_audio() {
    use resonance_audio::types::{Track, TrackType};

    let vocal = Track::with_type(4, "vox".into(), TrackType::Vocal);
    assert!(vocal.track_type.accepts_midi(), "precondition: accepts MIDI");
    assert!(
        !vocal.runs_internal_instrument(),
        "a vocal track renders through the audio path and keeps audio capture"
    );
}
