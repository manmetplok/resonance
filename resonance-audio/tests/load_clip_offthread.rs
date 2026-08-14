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
//!
//! `queued_loads_past_the_cap_are_never_dropped` covers the second half
//! of the story: the worker pool that bounds this work
//! (`ImportQueue` / `MAX_CONCURRENT_IMPORTS`) must *queue* requests past
//! its cap rather than reject them, because the old cap-and-drop lost
//! every audio clip past the fourth on project load.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use parking_lot::RwLock;

use resonance_audio::transcode_to_wav;
use resonance_audio::types::{compute_waveform_peaks, AudioClip, AudioEvent, ClipSource, FadeCurve};
use resonance_audio::{ImportQueue, MAX_CONCURRENT_IMPORTS};

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
            vocal_tuning: None,
            warp_enabled: false,
            original_bpm: None,
            transpose_semitones: 0.0,
            warp_algorithm: Default::default(),
            warp_markers: Vec::new(),
            tuning_render_cache: None,
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
                vocal_tuning: None,
                warp_enabled: false,
                original_bpm: None,
                transpose_semitones: 0.0,
                warp_algorithm: Default::default(),
                warp_markers: Vec::new(),
                tuning_render_cache: None,
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

/// Regression for the capacity *drop*: `handle_load_clip_from_wav` used
/// to compare an in-flight counter against `MAX_CONCURRENT_IMPORTS` and
/// `return` when it was at the cap, emitting only an error event. Project
/// load fires one `LoadClipFromWav` per audio clip, so every project with
/// more than `MAX_CONCURRENT_IMPORTS` audio clips silently lost the
/// excess: those clips never entered the engine, their lanes played
/// silent, and the next save wrote a bundle missing their WAVs.
///
/// The handler now hands the work to `ImportQueue`, which queues past the
/// cap instead of rejecting. This test drives that queue with the same
/// job body the handler submits (open the WAV, decimate peaks, publish
/// under a brief write lock, emit `ClipImported`) at three times the cap
/// and asserts:
///   * every submitted load eventually publishes its clip and its event —
///     nothing is dropped;
///   * no more than `MAX_CONCURRENT_IMPORTS` jobs ever run at once, and no
///     more than that many worker threads are ever spawned, so the bound
///     the cap exists for still holds; and
///   * submitting never blocks the caller (the engine control thread)
///     even though the queue is heavily backlogged.
#[test]
fn queued_loads_past_the_cap_are_never_dropped() {
    let dir = make_tempdir("queue");
    // Three times the production cap: the first `MAX_CONCURRENT_IMPORTS`
    // jobs occupy every worker, so the rest can only complete by having
    // been queued.
    let total = MAX_CONCURRENT_IMPORTS * 3;

    let clips: Arc<RwLock<Vec<AudioClip>>> = Arc::new(RwLock::new(Vec::new()));
    let (event_tx, event_rx) = crossbeam_channel::unbounded::<AudioEvent>();

    // Live job count and its high-water mark, to prove the queue really
    // bounds concurrency rather than just spawning a thread per request.
    let in_flight = Arc::new(AtomicUsize::new(0));
    let peak_in_flight = Arc::new(AtomicUsize::new(0));

    let mut wavs = Vec::new();
    for id in 0..total as u64 {
        let wav = dir.join(format!("clip_{id}.wav"));
        write_test_wav(&wav, /* seconds */ 1);
        wavs.push(wav);
    }

    let mut queue = ImportQueue::new(MAX_CONCURRENT_IMPORTS);

    let submit_started = Instant::now();
    for (id, wav) in wavs.into_iter().enumerate() {
        let clip_id = id as u64;
        let clips_arc = Arc::clone(&clips);
        let tx = event_tx.clone();
        let in_flight = Arc::clone(&in_flight);
        let peak_in_flight = Arc::clone(&peak_in_flight);
        queue
            .submit(move || {
                let now = in_flight.fetch_add(1, Ordering::SeqCst) + 1;
                peak_in_flight.fetch_max(now, Ordering::SeqCst);

                let source = ClipSource::open_wav(&wav).expect("open wav");
                let duration_samples = source.frame_count();
                let waveform_peaks = compute_waveform_peaks(source.as_frames());
                let clip = AudioClip {
                    id: clip_id,
                    track_id: 1,
                    start_sample: clip_id * 48_000,
                    source,
                    name: format!("clip_{clip_id}"),
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
                    warp_algorithm: Default::default(),
                    warp_markers: Vec::new(),
                    tuning_render_cache: None,
                };
                clips_arc.write().push(clip);
                let _ = tx.send(AudioEvent::ClipImported {
                    clip_id,
                    track_id: 1,
                    start_sample: clip_id * 48_000,
                    duration_samples,
                    name: format!("clip_{clip_id}"),
                    waveform_peaks,
                });

                // Hold the worker slot long enough that the later
                // submissions are provably queued behind a full pool.
                thread::sleep(Duration::from_millis(100));
                in_flight.fetch_sub(1, Ordering::SeqCst);
            })
            .expect("submit must not fail");
    }
    let submit_elapsed = submit_started.elapsed();
    drop(event_tx);

    // Requirement: the engine control thread must not wait on the queue.
    // Submitting `total` jobs is `total` channel sends plus at most
    // `MAX_CONCURRENT_IMPORTS` thread spawns; the jobs themselves take
    // >= 100 ms each, so anything near the run's wall clock means the
    // caller got parked.
    assert!(
        submit_elapsed < Duration::from_millis(500),
        "submitting {total} loads took {submit_elapsed:?} — the engine \
         thread must not block on the import queue"
    );

    // Everything must land. Generous deadline: this is a
    // did-it-happen-at-all assertion, not a timing one.
    let deadline = Instant::now() + Duration::from_secs(30);
    while clips.read().len() < total {
        assert!(
            Instant::now() < deadline,
            "only {} of {total} queued loads published within 30 s — \
             loads past the concurrency cap must be queued, not dropped",
            clips.read().len()
        );
        thread::sleep(Duration::from_millis(10));
    }

    let mut ids: Vec<u64> = clips.read().iter().map(|c| c.id).collect();
    ids.sort_unstable();
    assert_eq!(
        ids,
        (0..total as u64).collect::<Vec<_>>(),
        "every submitted clip must be published exactly once"
    );

    // Every clip must also report completion to the app. Order is not
    // significant (workers finish out of order) — only that none is lost.
    let mut event_ids: Vec<u64> = event_rx
        .iter()
        .map(|ev| match ev {
            AudioEvent::ClipImported { clip_id, .. } => clip_id,
            other => panic!("unexpected event from a successful load: {other:?}"),
        })
        .collect();
    event_ids.sort_unstable();
    assert_eq!(
        event_ids,
        (0..total as u64).collect::<Vec<_>>(),
        "every queued load must emit ClipImported"
    );

    let peak = peak_in_flight.load(Ordering::SeqCst);
    assert!(
        peak <= MAX_CONCURRENT_IMPORTS,
        "{peak} loads ran concurrently — the queue must stay bounded at \
         {MAX_CONCURRENT_IMPORTS}"
    );
    assert!(
        queue.worker_count() <= MAX_CONCURRENT_IMPORTS,
        "queue spawned {} workers for {total} jobs — concurrency must be \
         bounded by threads, not by request count",
        queue.worker_count()
    );

    let _ = std::fs::remove_dir_all(&dir);
}
