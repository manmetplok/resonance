//! The deferred-drop retire queue (code review MIX-04 / ARCH-02 A2-2).
//!
//! An `ArcSwap` snapshot the audio callback pins for a block must never
//! be *freed* by that callback: when the engine thread replaces it with
//! `store`, the engine's reference is gone and the callback's guard is
//! the last owner — the destructor (a `munmap` for a `LatencyComp` or a
//! frozen cache) runs on the realtime thread. `Retired` keeps the
//! replaced value on the engine side until a sweep finds no other owner.
//!
//! Own binary: the last test proves the render thread frees nothing
//! while the engine republishes, which needs a counting global
//! allocator (`tests/sidechain_taps.rs` has the precedent). The counter
//! is thread-local so the engine-side frees are not mistaken for
//! callback-side ones.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Barrier, Mutex};
use std::thread::{self, ThreadId};

use arc_swap::ArcSwap;
use resonance_audio::test_support::{
    publish_retiring, EngineHandlerHarness, MixAudioHarness, Retired, SharedState,
};
use resonance_audio::types::*;
use resonance_audio::{start_audition_in_place, AuditionSource};
use resonance_common::{FreezeCacheRef, FreezeCacheStatus, TakeContent, TakeGroup, TimelineRange};

/// Counts this thread's heap frees.
struct CountingAllocator;

thread_local! {
    static THREAD_DEALLOCS: Cell<u64> = const { Cell::new(0) };
}

unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        THREAD_DEALLOCS.with(|c| c.set(c.get() + 1));
        unsafe { System.dealloc(ptr, layout) }
    }
}

#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;

fn deallocs_here() -> u64 {
    THREAD_DEALLOCS.with(Cell::get)
}

/// A payload that records which thread dropped it. Carries a heap
/// buffer so its drop is a real free.
struct Tracked {
    _payload: Vec<f32>,
    dropped_on: Arc<Mutex<Option<ThreadId>>>,
}

impl Drop for Tracked {
    fn drop(&mut self) {
        *self.dropped_on.lock().unwrap() = Some(thread::current().id());
    }
}

fn tracked(payload_len: usize) -> (Arc<Tracked>, Arc<Mutex<Option<ThreadId>>>) {
    let dropped_on = Arc::new(Mutex::new(None));
    let t = Arc::new(Tracked {
        _payload: vec![0.0; payload_len],
        dropped_on: Arc::clone(&dropped_on),
    });
    (t, dropped_on)
}

/// A "callback" thread that pins the slot's current value for as long
/// as the test says, then drops its guard and reports the frees that
/// drop caused on its own thread.
struct Reader {
    pinned: Arc<Barrier>,
    release: Arc<Barrier>,
    handle: thread::JoinHandle<u64>,
}

fn pin_on_reader(slot: &Arc<ArcSwap<Tracked>>, full: bool) -> Reader {
    let pinned = Arc::new(Barrier::new(2));
    let release = Arc::new(Barrier::new(2));
    let handle = {
        let slot = Arc::clone(slot);
        let pinned = Arc::clone(&pinned);
        let release = Arc::clone(&release);
        thread::spawn(move || {
            // Both shapes the callback uses: a `load()` guard (the block-
            // long automation / tempo / comp snapshots) and a `load_full()`
            // Arc (`frozen_source`, the reference PCM).
            let guard = (!full).then(|| slot.load());
            let arc = full.then(|| slot.load_full());
            pinned.wait();
            release.wait();
            let before = deallocs_here();
            drop(guard);
            drop(arc);
            deallocs_here() - before
        })
    };
    Reader {
        pinned,
        release,
        handle,
    }
}

fn retire_primitive(full: bool) {
    let (a, a_dropped) = tracked(1 << 16);
    let slot = Arc::new(ArcSwap::new(a));
    let retired = Retired::new();

    let reader = pin_on_reader(&slot, full);
    reader.pinned.wait();

    // Publish B over A while the reader pins A: A is retired, not freed.
    let (b, b_dropped) = tracked(1 << 16);
    publish_retiring(&slot, b, &retired);
    assert_eq!(retired.len(), 1);
    assert_eq!(retired.sweep(), 0, "A is still pinned by the reader");
    assert!(a_dropped.lock().unwrap().is_none());

    // Publish C over B: nobody pins B, so the next sweep frees it — here,
    // on the publishing thread.
    let (c, _) = tracked(1 << 16);
    publish_retiring(&slot, c, &retired);
    assert_eq!(retired.len(), 2);
    assert_eq!(retired.sweep(), 1);
    assert_eq!(*b_dropped.lock().unwrap(), Some(thread::current().id()));
    assert_eq!(retired.len(), 1);

    // The reader lets go of A. Its drop frees nothing on the reader
    // thread: the queue still owns A.
    reader.release.wait();
    let reader_frees = reader.handle.join().expect("reader");
    assert_eq!(reader_frees, 0, "the callback's guard drop must free nothing");
    assert!(a_dropped.lock().unwrap().is_none(), "A still alive in the queue");

    // ... until this thread sweeps.
    assert_eq!(retired.sweep(), 1);
    assert_eq!(*a_dropped.lock().unwrap(), Some(thread::current().id()));
    assert!(retired.is_empty());
}

#[test]
fn retired_snapshot_is_freed_by_the_sweep_never_by_the_reader_guard() {
    retire_primitive(false);
}

#[test]
fn retired_snapshot_is_freed_by_the_sweep_never_by_the_reader_arc() {
    retire_primitive(true);
}

#[test]
fn plain_store_frees_on_the_reader_thread_which_is_the_bug() {
    // Negative control for the detector above: the pre-fix `store` shape
    // hands the reader the last reference, so the reader's guard drop
    // runs the destructor and the frees land on its thread.
    let (a, a_dropped) = tracked(1 << 16);
    let slot = Arc::new(ArcSwap::new(a));
    let reader = pin_on_reader(&slot, false);
    reader.pinned.wait();
    let (b, _) = tracked(1 << 16);
    slot.store(b);
    reader.release.wait();
    let reader_frees = reader.handle.join().expect("reader");
    let reader_id = *a_dropped.lock().unwrap();
    assert!(reader_frees > 0, "the plain store must be observable as reader-side frees");
    assert!(reader_id.is_some() && reader_id != Some(thread::current().id()));
}

#[test]
fn sweep_keeps_entries_pinned_elsewhere_and_frees_the_rest() {
    let retired = Retired::new();
    let (a, _) = tracked(8);
    let (b, _) = tracked(8);
    let a2 = Arc::clone(&a);
    retired.retire(a);
    retired.retire(b);
    retired.retire_opt::<Tracked>(None);
    assert_eq!(retired.len(), 2);
    assert!(retired.holds(&a2));
    assert_eq!(retired.sweep(), 1, "b freed, a pinned by a2");
    assert!(retired.holds(&a2));
    drop(a2);
    assert_eq!(retired.sweep(), 1);
    assert!(retired.is_empty());
}

fn frozen(frames: usize) -> FrozenSource {
    let cache_ref = FreezeCacheRef::new("retire.wav".into(), 48_000, 32, 1, FreezeCacheStatus::Frozen);
    FrozenSource::new(cache_ref, Arc::new(vec![0.25; frames * 2]), 48_000, frames as u64)
}

#[test]
fn engine_handlers_retire_every_replaced_snapshot() {
    let mut h = EngineHandlerHarness::new();
    let retired_len = |h: &EngineHandlerHarness| h.shared().retired.len();

    // take_comp: the callback's `Arc` (published_comp_table is a
    // load_full) survives two republishes in the queue.
    let held_comp = h.published_comp_table();
    let slot = TimelineRange::new(0, 48_000);
    let mut g1 = TakeGroup::new(1, 7, slot);
    resonance_audio::test_support::push_take(&mut g1, slot, &TakeContent::Audio { clip_ref: 100 });
    h.seed_take_group(g1);
    let mut g2 = TakeGroup::new(2, 7, slot);
    resonance_audio::test_support::push_take(&mut g2, slot, &TakeContent::Audio { clip_ref: 200 });
    h.seed_take_group(g2);
    assert_eq!(retired_len(&h), 2, "both replaced tables retired");
    assert!(h.shared().retired.holds(&held_comp));
    assert_eq!(h.shared().retired.sweep(), 1, "only the unpinned one freed");
    assert!(h.shared().retired.holds(&held_comp));
    drop(held_comp);
    assert_eq!(h.shared().retired.sweep(), 1);
    assert_eq!(retired_len(&h), 0);

    // frozen_source: freeze, hold the cache as the render pass does,
    // re-freeze, then remove the track. The held cache and the track's
    // last cache both go through the queue; neither is freed by the
    // holder.
    h.push_track(Track::new(1, "frozen".into()));
    h.set_track_frozen_source(1, Some(frozen(1 << 15)));
    let held_cache = h.frozen_source(1).expect("frozen");
    h.set_track_frozen_source(1, Some(frozen(1 << 15)));
    assert!(h.shared().retired.holds(&held_cache));
    h.remove_track(1);
    assert!(h.frozen_source(1).is_none());
    // Queue: the held cache + the removed track's cache + its (empty)
    // plugin chain.
    assert_eq!(retired_len(&h), 3);
    assert_eq!(h.shared().retired.sweep(), 2);
    assert!(h.shared().retired.holds(&held_cache));
    assert_eq!(Arc::strong_count(&held_cache), 2, "holder + queue");
    drop(held_cache);
    assert_eq!(h.shared().retired.sweep(), 1);

    // clear_all republishes the aux-send, take-comp and reference tables
    // and drains the tracks: everything replaced lands in the queue.
    h.push_track(Track::new(2, "t".into()));
    h.set_track_frozen_source(2, Some(frozen(64)));
    h.clear_all();
    assert!(retired_len(&h) >= 3, "aux sends, take comp, track cache, chain");
    h.shared().retired.sweep();
    assert_eq!(retired_len(&h), 0);
}

/// One track, one clip, playing: every block loads the automation,
/// comp, aux, sidechain and take-comp snapshots and the audition source.
fn playing_harness() -> MixAudioHarness {
    let track = Track::new(1, "clips".into());
    track.set_output(TrackOutput::Master);
    let clip = AudioClip {
        id: 1,
        track_id: 1,
        start_sample: 0,
        source: ClipSource::Memory(vec![0.1; 4096 * 2]),
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
    tempo.rebuild_bar_table(48_000);
    let h = MixAudioHarness::new(vec![track], Vec::new(), vec![clip], Vec::new(), Vec::new(), tempo, 128, 2, 48_000, true);
    h.shared().playing.store(true, Ordering::Relaxed);
    h.shared().loop_enabled.store(true, Ordering::Relaxed);
    h.shared().loop_in.store(0, Ordering::Relaxed);
    h.shared().loop_out.store(4096, Ordering::Relaxed);
    h
}

#[test]
fn render_thread_frees_nothing_while_the_engine_republishes() {
    // The integration shape: this thread is the audio callback, a second
    // thread is the engine republishing a large snapshot as fast as it
    // can (and sweeping, as the engine loop does). The callback pins the
    // audition source every block through the overlay; with `store` the
    // old sources would be freed here. With the retire queue the render
    // thread's free count stays exactly where it was.
    let mut h = playing_harness();
    let shared: Arc<SharedState> = h.shared_arc();
    let stop = Arc::new(AtomicBool::new(false));
    let big = || AuditionSource::from_samples(vec![0.05; 1 << 17], 48_000);
    start_audition_in_place(&shared, big(), 0, 120.0, true, false);

    let publisher = {
        let shared = Arc::clone(&shared);
        let stop = Arc::clone(&stop);
        thread::spawn(move || {
            let mut published = 0u64;
            while !stop.load(Ordering::Relaxed) {
                start_audition_in_place(&shared, big(), 0, 120.0, true, false);
                shared.retired.sweep();
                published += 1;
            }
            published
        })
    };

    let before = deallocs_here();
    for _ in 0..2_000 {
        h.render();
    }
    let render_frees = deallocs_here() - before;
    stop.store(true, Ordering::Relaxed);
    let published = publisher.join().expect("publisher");
    assert!(published > 0);
    assert_eq!(
        render_frees, 0,
        "the render thread freed {render_frees} allocations across {published} republishes"
    );
    // Whatever the publisher left behind is freed here, on the engine
    // side of the fence, once nothing pins it.
    drop(h);
    shared.retired.sweep();
}
