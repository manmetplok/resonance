//! The render pool's dispatch and join never make a Rust heap
//! allocation, on any thread (realtime-multithreading.md §7).
//!
//! A standalone binary because it installs a `#[global_allocator]`, which
//! is process-wide: it counts Rust heap allocations on EVERY thread, the
//! render workers included, while armed. The one test here drives the
//! whole audio callback on a four-thread pool through spinning and parked
//! stretches (so the wake path runs too) and asserts that, from the
//! pool's first block until every worker has run a job, no Rust
//! allocation happens anywhere in the process.
//!
//! What the counter cannot see: allocations made below Rust's allocator,
//! by libc or another C library on its own behalf — glibc `calloc`ing a
//! thread-local destructor entry (`rt_prep.rs`), for one. This guards
//! the Rust side of the path, not the process's whole heap.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use resonance_audio::test_support::MixAudioHarness;
use resonance_audio::types::*;

struct CountingAllocator;

static ARMED: AtomicBool = AtomicBool::new(false);
static ALLOCS: AtomicU64 = AtomicU64::new(0);

unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if ARMED.load(Ordering::Relaxed) {
            ALLOCS.fetch_add(1, Ordering::Relaxed);
        }
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }
}

#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;

const SR: u32 = 48_000;
const BLOCK: usize = 128;
const TRACKS: u64 = 16;
const BUS: BusId = 100;

fn clip(id: ClipId, track_id: TrackId) -> AudioClip {
    let samples: Vec<f32> = (0..BLOCK * 256 * 2)
        .map(|i| ((i as u64 * (id + 3)) % 97) as f32 / 97.0 - 0.5)
        .collect();
    AudioClip {
        id,
        track_id,
        start_sample: 0,
        source: ClipSource::memory(samples),
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

#[test]
fn parallel_callbacks_never_allocate_on_any_thread() {
    let tracks: Vec<Track> = (1..=TRACKS)
        .map(|id| {
            let mut t = Track::new(id, format!("t{id}"));
            if id % 3 == 0 {
                t.set_output(TrackOutput::Bus(BUS));
            }
            t
        })
        .collect();
    let clips = (1..=TRACKS).map(|id| clip(id, id)).collect();
    let sends = vec![AuxSend {
        id: 1,
        source: SendSource::Track(1),
        dest: BUS,
        level_db: -6.0,
        pre_fader: false,
        enabled: true,
    }];
    resonance_audio::test_support::override_threads_on_this_thread(Some(1));
    let mut h = MixAudioHarness::new(
        tracks,
        vec![Bus::new(BUS, "bus".into())],
        clips,
        Vec::new(),
        sends,
        TempoMap::default(),
        BLOCK,
        2,
        SR,
        true,
    );
    h.shared()
        .sidechain_routes
        .store(Arc::new(vec![SidechainRoute {
            plugin: 999,
            source: SendSource::Track(2),
            enabled: true,
        }]));
    h.shared().playing.store(true, Ordering::Relaxed);

    let run = |h: &mut MixAudioHarness, blocks: usize| {
        for block in 0..blocks {
            if block % 16 == 15 {
                // Past the workers' idle spin: they park, and the next
                // block has to wake them.
                std::thread::sleep(Duration::from_millis(1));
            }
            h.render();
        }
    };
    // Warm-up, serial: the caller's first-use thread-locals and the lazily
    // initialised statics. No worker exists yet (the harness was built
    // serial whatever RESONANCE_RENDER_THREADS says), so each worker's
    // whole life runs under the counter below.
    run(&mut h, 48);

    // Armed from the pool's first block: the caller's first parallel run
    // and every worker's first job. When a worker gets its first job is up
    // to scheduling (under load the caller and the others may claim every
    // job for many blocks), so render until each has run one: a worker
    // whose per-thread first-use state is not set up at spawn (the
    // arc-swap node, `rt_prep`) then fails this every time, not by luck.
    h.set_render_threads(4, 0x5eed);
    ARMED.store(true, Ordering::SeqCst);
    let deadline = Instant::now() + Duration::from_secs(30);
    let mut blocks = 0usize;
    while blocks < 128 || (h.render_pool_min_worker_jobs() == 0 && Instant::now() < deadline) {
        run(&mut h, 16);
        blocks += 16;
    }
    ARMED.store(false, Ordering::SeqCst);

    assert!(
        h.render_pool_min_worker_jobs() > 0,
        "a worker never ran a job in {blocks} blocks"
    );
    let stats = h.take_pass_stats();
    assert_eq!(stats.threads, 4, "the jobs really ran on the pool");
    assert_eq!(
        ALLOCS.load(Ordering::SeqCst),
        0,
        "the render pool path allocated on some thread"
    );
}
