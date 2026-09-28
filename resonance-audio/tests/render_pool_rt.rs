//! The render pool's dispatch and join never allocate — on any thread
//! (realtime-multithreading.md §7).
//!
//! A standalone binary because it installs a `#[global_allocator]`, which
//! is process-wide: it counts heap allocations on EVERY thread, the render
//! workers included, while armed. The one test here drives the whole
//! audio callback on a four-thread pool through spinning and parked
//! stretches (so the wake path runs too) and asserts that, once warm, not
//! a single allocation happens anywhere in the process.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

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
    h.set_render_threads(4, 0x5eed);

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
    // Warm-up: first-use thread-locals, the scheduling hand-off, lazily
    // initialised statics. It cannot promise that every worker has run a
    // job — under load the caller and the others may claim them all — so
    // a worker's per-thread first-use state must be set up at spawn, not
    // here (render_pool's `claim_arc_swap_node`; this test caught it).
    run(&mut h, 48);

    ARMED.store(true, Ordering::SeqCst);
    run(&mut h, 128);
    ARMED.store(false, Ordering::SeqCst);

    let stats = h.take_pass_stats();
    assert_eq!(stats.threads, 4, "the jobs really ran on the pool");
    assert_eq!(
        ALLOCS.load(Ordering::SeqCst),
        0,
        "the render pool path allocated on some thread"
    );
}
