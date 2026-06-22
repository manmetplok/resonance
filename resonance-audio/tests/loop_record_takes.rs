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
