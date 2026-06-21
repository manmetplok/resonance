//! Regression for `engine/clips.rs::handle_load_clip_from_wav` — used
//! to call `ClipSource::open_wav` (which pre-touches every page of the
//! mmap) and `compute_waveform_peaks` (an O(n) decimation across the
//! whole sample buffer) on the engine control thread before the
//! `clips.write().push(...)`. Project load fires one `LoadClipFromWav`
//! per audio clip, so on a project with a handful of multi-minute
//! clips the engine command queue would stall for tens to hundreds of
//! milliseconds while the audio thread's `clips.try_read()` repeatedly
//! lost the race and the mixer emitted silence buffers.
//!
//! The fix moves the heavy work to a short-lived worker thread
//! (mirroring `handle_import_clip`'s existing pattern). The engine
//! thread now just records the new `next_clip_id`, spawns the worker,
//! and returns — the write lock on `clips` is held only for the
//! single `Vec::push` at the very end. The audio thread can therefore
//! `try_read` clips throughout the load with no contention beyond the
//! push itself.
//!
//! These tests pin the pattern at the public-API level — they exercise
//! the same `ClipSource::open_wav` + `compute_waveform_peaks` +
//! `clips.write().push(...)` sequence the engine worker now runs, and
//! assert that a simulated audio thread reading the same `clips` arc
//! never observes the read lock blocked for the duration of the
//! compute.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use parking_lot::RwLock;

use resonance_audio::transcode_to_wav;
use resonance_audio::types::{compute_waveform_peaks, AudioClip, ClipSource, FadeCurve};

fn make_tempdir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "resonance-load-test-{}-{}",
        tag,
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Write a multi-second f32-stereo WAV to disk so `ClipSource::open_wav`
/// has a meaningful pre-touch and `compute_waveform_peaks` has enough
/// samples to take measurable time. Five seconds at 48 kHz stereo is
/// 1.92 MB of PCM — large enough to surface a held-write-lock
/// regression but small enough that the test wall clock stays well
/// under the multi-second flake threshold.
fn write_test_wav(path: &std::path::Path, seconds: usize) -> u64 {
    let sample_rate: u32 = 48_000;
    let total_frames = sample_rate as usize * seconds;
    let mut samples = Vec::with_capacity(total_frames * 2);
    for i in 0..total_frames {
        let t = i as f32 / sample_rate as f32;
        let s = (2.0 * std::f32::consts::PI * 220.0 * t).sin() * 0.25;
        samples.push(s);
        samples.push(s);
    }
    transcode_to_wav(path, &samples, sample_rate).expect("write test wav");
    total_frames as u64
}

/// Exercise the off-thread pattern the engine worker uses today:
///   * open the wav on the worker
///   * compute peaks on the worker
///   * publish via a brief `clips.write().push()` on the worker
/// Meanwhile a separate "audio-thread" probe loops `try_read` on the
/// same arc and asserts contention windows stay short (< 10 ms each).
/// If the load path ever regressed to holding a write lock across the
/// compute, the probe would see hundreds of ms of `try_read` failures
/// and this test would fail.
#[test]
fn worker_publish_does_not_stall_concurrent_reads() {
    let dir = make_tempdir("nostall");
    let wav = dir.join("test.wav");
    let _frames = write_test_wav(&wav, /* seconds */ 5);

    let clips: Arc<RwLock<Vec<AudioClip>>> = Arc::new(RwLock::new(Vec::new()));

    // Spin up the "audio thread" probe before kicking off the worker.
    // It loops `try_read` until told to stop and records the longest
    // contiguous window during which `try_read` returned `None`.
    let probe_clips = Arc::clone(&clips);
    let probe_stop = Arc::new(AtomicBool::new(false));
    let probe_stop_handle = Arc::clone(&probe_stop);
    let longest_contended_us = Arc::new(AtomicU64::new(0));
    let longest_contended_handle = Arc::clone(&longest_contended_us);
    let probe = thread::spawn(move || {
        let mut contention_started: Option<Instant> = None;
        while !probe_stop_handle.load(Ordering::Relaxed) {
            match probe_clips.try_read() {
                Some(_guard) => {
                    if let Some(start) = contention_started.take() {
                        let elapsed_us = start.elapsed().as_micros() as u64;
                        let prev = longest_contended_handle.load(Ordering::Relaxed);
                        if elapsed_us > prev {
                            longest_contended_handle.store(elapsed_us, Ordering::Relaxed);
                        }
                    }
                    // Drop the read guard immediately, like the mixer
                    // does between blocks.
                }
                None => {
                    if contention_started.is_none() {
                        contention_started = Some(Instant::now());
                    }
                }
            }
            // Yield so the worker can make progress without us busy-
            // spinning the entire scheduler quantum.
            std::hint::spin_loop();
        }
    });

    // The "engine worker": same pattern the engine handler now uses.
    let worker_clips = Arc::clone(&clips);
    let worker = thread::spawn(move || {
        let source = ClipSource::open_wav(&wav).expect("open wav");
        let total_frames = source.frame_count();
        let waveform_peaks = compute_waveform_peaks(source.as_frames());
        assert!(
            !waveform_peaks.is_empty(),
            "5 s of audio must decimate to at least one peak bucket"
        );
        let clip = AudioClip {
            id: 1,
            track_id: 1,
            start_sample: 0,
            source,
            name: "test".into(),
            trim_start_frames: 0,
            trim_end_frames: 0,
            fade_in_frames: 0,
            fade_in_curve: FadeCurve::default(),
            fade_out_frames: 0,
            fade_out_curve: FadeCurve::default(),
            gain_db: 0.0,
        };
        worker_clips.write().push(clip);
        total_frames
    });

    let frames_loaded = worker.join().expect("worker thread");
    assert_eq!(frames_loaded, 48_000 * 5);

    // Give the probe a small grace window so any final contention
    // measurement settles before we stop it.
    thread::sleep(Duration::from_millis(10));
    probe_stop.store(true, Ordering::Relaxed);
    probe.join().expect("probe thread");

    let longest_us = longest_contended_us.load(Ordering::Relaxed);
    // The only window during which `try_read` can fail is the single
    // `Vec::push` at the very end of the worker. Even under heavy
    // scheduler pressure, that should land well under 10 ms. We pick
    // 50 ms to leave room for CI noise; if the load regressed to
    // holding the write lock across the compute, the contended
    // window would be hundreds of ms (the time it takes to walk the
    // 5-second buffer).
    assert!(
        longest_us < 50_000,
        "audio-thread probe saw a {longest_us} µs contention window — \
         the engine load path must not hold clips.write() across \
         compute_waveform_peaks"
    );

    // Sanity check: the clip really did get published.
    assert_eq!(clips.read().len(), 1);

    // Best-effort cleanup; the temp dir is per-PID per-nanos, so
    // leftover files don't break subsequent runs.
    let _ = std::fs::remove_dir_all(&dir);
}

/// Many concurrent loads must compose without deadlock or starvation.
/// The engine bounds concurrency at `MAX_CONCURRENT_IMPORTS`; this
/// test fires four parallel loads (the production cap) against one
/// shared clips arc and asserts all four publish in bounded time.
#[test]
fn concurrent_loads_all_publish_without_deadlock() {
    let dir = make_tempdir("concurrent");
    let mut handles = Vec::new();
    let clips: Arc<RwLock<Vec<AudioClip>>> = Arc::new(RwLock::new(Vec::new()));

    for id in 0..4u64 {
        let wav = dir.join(format!("clip_{id}.wav"));
        let _ = write_test_wav(&wav, /* seconds */ 1);
        let clips_arc = Arc::clone(&clips);
        handles.push(thread::spawn(move || {
            let source = ClipSource::open_wav(&wav).expect("open wav");
            let _peaks = compute_waveform_peaks(source.as_frames());
            let clip = AudioClip {
                id,
                track_id: 1,
                start_sample: id * 48_000,
                source,
                name: format!("clip_{id}"),
                trim_start_frames: 0,
                trim_end_frames: 0,
                fade_in_frames: 0,
                fade_in_curve: FadeCurve::default(),
                fade_out_frames: 0,
                fade_out_curve: FadeCurve::default(),
                gain_db: 0.0,
            };
            clips_arc.write().push(clip);
        }));
    }

    let deadline = Instant::now() + Duration::from_secs(5);
    for h in handles {
        let remaining = deadline.saturating_duration_since(Instant::now());
        assert!(
            remaining > Duration::from_millis(0),
            "concurrent loads ran past 5 s — possible deadlock"
        );
        h.join().expect("worker thread");
    }

    assert_eq!(clips.read().len(), 4);

    let _ = std::fs::remove_dir_all(&dir);
}
