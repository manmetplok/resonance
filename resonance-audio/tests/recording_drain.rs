//! Proves the core invariant of the streaming-recording refactor: the
//! drain path never accumulates audio samples in memory regardless of
//! how long the recording runs. A ten-minute take at 48 kHz stereo is
//! ~220 MB of PCM — if any of that lived in `TrackRecordingBuf` the
//! test would fail by showing a scratch capacity in the hundreds of
//! megabytes.
//!
//! `RecordingState` and `TrackRecordingBuf` are exposed via
//! `#[doc(hidden)] pub use` in `lib.rs` purely so this test can poke
//! at the engine-thread recording scratch without spinning up the
//! engine itself. They are not part of the public API.

use std::path::PathBuf;

use ringbuf::traits::{Producer, Split};
use ringbuf::HeapRb;

use resonance_audio::types::ClipSource;
use resonance_audio::RecordingState;

fn make_tempdir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "resonance-rec-test-{}-{}",
        tag,
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn drain_streams_to_disk_without_growing_memory() {
    let project_dir = make_tempdir("drain");

    let mut rec = RecordingState::new(48_000);
    let ring: HeapRb<f32> = HeapRb::new(48_000 * 2 * 2); // 2 s stereo
    let (mut prod, cons) = ring.split();
    rec.ring_consumer = Some(cons);
    rec.input_channels = 2;
    rec.input_sample_rate = 48_000;
    rec.start_sample = 0;

    let buf = RecordingState::create_track_buf(
        &project_dir,
        /* track_id */ 42,
        /* clip_id  */ 1,
        /* engine   */ 48_000,
        /* input    */ 48_000,
        /* port     */ 0,
        /* mono     */ false,
    )
    .unwrap();
    let wav_path = buf.path.clone();
    rec.buffers.insert(42, buf);

    // Feed the equivalent of 10 seconds of audio in 1000-frame
    // chunks (picked so the totals divide evenly), draining
    // between each push so the ring never overflows. Ten
    // seconds is plenty to catch any accidental `Vec::push` on
    // a hot path — the scratch budget is bounded at
    // `DRAIN_SCRATCH_LEN` regardless.
    let frames_per_chunk = 1000usize;
    let total_frames = 48_000 * 10;
    let chunks = total_frames / frames_per_chunk;
    let mut sample = vec![0.0f32; frames_per_chunk * 2];
    for i in 0..chunks {
        for f in 0..frames_per_chunk {
            let s = ((i * frames_per_chunk + f) as f32 * 0.001).sin();
            sample[f * 2] = s;
            sample[f * 2 + 1] = s;
        }
        prod.push_slice(&sample);
        rec.drain_ring_to_buffers();
    }

    // After draining 10 s of audio, the per-track scratch
    // capacity must remain bounded — it should be the size of
    // one drain chunk at most, not anywhere near the total
    // audio size.
    let track_buf = rec.buffers.get(&42).unwrap();
    let in_memory_bytes = track_buf.resample_scratch.capacity() * 4;
    assert!(
        in_memory_bytes <= 256 * 1024,
        "TrackRecordingBuf holds {} bytes of PCM in memory — the drain path should stream to disk",
        in_memory_bytes
    );
    assert_eq!(
        track_buf.frames_written,
        48_000 * 10,
        "wrong number of frames streamed to disk"
    );

    // Finalize and verify the WAV file on disk really does
    // contain all 10 seconds. Drop the producer first so the
    // ring's outstanding samples are all consumable.
    drop(prod);
    let (tx, _rx) = crossbeam_channel::unbounded();
    let clips = parking_lot::RwLock::new(Vec::new());
    rec.finalize_recording(48_000, &clips, &tx);

    let source = ClipSource::open_wav(&wav_path).expect("mmap finalized wav");
    assert_eq!(source.frame_count(), 48_000 * 10);
    let clip_count = clips.read().len();
    assert_eq!(clip_count, 1);

    let _ = std::fs::remove_dir_all(&project_dir);
}
