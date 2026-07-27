//! Plugin-level tests: instantiation, parameter surface stability,
//! param plumbing round-trips and the no-allocation guarantee on the
//! audio path (ba todo #1073, doc #252 §8).

use resonance_granular_delay::params::PARAM_COUNT;
use resonance_granular_delay::ResonanceGranularDelay;
use resonance_plugin::{EventIterator, OutputBuffer, Param, ResonancePlugin};
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

// --- Per-thread allocation counter for the no-allocation guard --------
// (pattern from resonance-dsp/tests/granular.rs).

thread_local! {
    static ALLOC_COUNT: Cell<usize> = const { Cell::new(0) };
}

/// System allocator that counts alloc/realloc calls per thread, so the
/// no-allocation guard is not polluted by concurrently running tests.
struct CountingAlloc;

// SAFETY: delegates entirely to `System`; the const-initialised
// thread-local counter never allocates itself (`try_with` tolerates TLS
// teardown).
unsafe impl GlobalAlloc for CountingAlloc {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let _ = ALLOC_COUNT.try_with(|c| c.set(c.get() + 1));
        System.alloc(layout)
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        System.dealloc(ptr, layout)
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        let _ = ALLOC_COUNT.try_with(|c| c.set(c.get() + 1));
        System.realloc(ptr, layout, new_size)
    }
}

#[global_allocator]
static ALLOC: CountingAlloc = CountingAlloc;

fn thread_allocs() -> usize {
    ALLOC_COUNT.with(|c| c.get())
}

// --- Helpers ----------------------------------------------------------

fn run_blocks(
    plugin: &mut ResonanceGranularDelay,
    left: &mut [f32],
    right: &mut [f32],
    block: usize,
) {
    let frames = left.len();
    let mut pos = 0;
    while pos < frames {
        let n = (frames - pos).min(block);
        let mut outs = [OutputBuffer {
            left: &mut left[pos..pos + n],
            right: &mut right[pos..pos + n],
        }];
        let mut ev = EventIterator::empty();
        plugin.process(&mut outs, n, &mut ev, None);
        pos += n;
    }
}

// --- Tests ------------------------------------------------------------

#[test]
fn param_enumeration_covers_declared_count() {
    let plugin = ResonanceGranularDelay::new();
    assert_eq!(plugin.param_count(), PARAM_COUNT);
    let mut seen = std::collections::HashSet::new();
    for i in 0..plugin.param_count() {
        let id = plugin.param(i).id().to_string();
        assert!(seen.insert(id.clone()), "duplicate param id: {id}");
    }
}

#[test]
fn param_ids_and_count_stable_across_instantiations() {
    let a = ResonanceGranularDelay::new();
    let b = ResonanceGranularDelay::new();
    assert_eq!(a.param_count(), b.param_count());
    for i in 0..a.param_count() {
        assert_eq!(
            a.param(i).id(),
            b.param(i).id(),
            "param id at index {i} differs between instances"
        );
    }
}

#[test]
fn params_round_trip_set_and_read() {
    let plugin = ResonanceGranularDelay::new();
    let p = &plugin.params;

    p.time_ms.set_value(250.0);
    assert_eq!(p.time_ms.value(), 250.0);
    p.grain_size_ms.set_value(120.0);
    assert_eq!(p.grain_size_ms.value(), 120.0);
    p.density_hz.set_value(40.0);
    assert_eq!(p.density_hz.value(), 40.0);
    p.pitch.set_value(-12.0);
    assert_eq!(p.pitch.value(), -12.0);
    p.spread_cents.set_value(25.0);
    assert_eq!(p.spread_cents.value(), 25.0);
    p.texture.set_value(0.75);
    assert_eq!(p.texture.value(), 0.75);
    p.mix.set_value(1.0);
    assert_eq!(p.mix.value(), 1.0);
    p.scheduler.set_value(0);
    assert_eq!(p.scheduler.value(), 0);
    p.sync.set_plain(0.0);
    assert!(!p.sync.value());

    // Declared-but-inert params (later todos in epic #196) must still
    // hold values so state saved today round-trips tomorrow.
    p.feedback.set_value(0.8);
    assert_eq!(p.feedback.value(), 0.8);
    p.freeze.set_plain(1.0);
    assert!(p.freeze.value());
    p.width.set_value(1.25);
    assert_eq!(p.width.value(), 1.25);
    p.quality.set_value(2);
    assert_eq!(p.quality.value(), 2);
}

#[test]
fn processes_impulse_without_nans_and_produces_wet_energy() {
    let mut plugin = ResonanceGranularDelay::new();
    plugin.params.sync.set_plain(0.0);
    plugin.params.time_ms.set_value(250.0);
    plugin.params.mix.set_value(0.5);
    plugin.initialize(48_000.0, 4096);

    let frames = 48_000usize;
    let mut left = vec![0.0f32; frames];
    let mut right = vec![0.0f32; frames];
    left[0] = 1.0;
    right[0] = 1.0;

    run_blocks(&mut plugin, &mut left, &mut right, 4096);

    for &x in left.iter().chain(right.iter()) {
        assert!(x.is_finite(), "non-finite sample: {x}");
    }
    assert!(
        plugin.grains_spawned() > 0,
        "engine never spawned any grains"
    );
    // Granulated wet energy must appear around the 250 ms delay.
    let wet_energy: f32 = left[11_500..15_000].iter().map(|x| x.abs()).sum();
    assert!(
        wet_energy > 1e-4,
        "expected granulated energy near the delay, got {wet_energy}"
    );
}

#[test]
fn audio_path_does_not_allocate() {
    let mut plugin = ResonanceGranularDelay::new();
    plugin.params.sync.set_plain(0.0);
    plugin.params.time_ms.set_value(200.0);
    plugin.params.density_hz.set_value(80.0);
    plugin.params.spray_ms.set_value(50.0);
    plugin.params.pan_spread.set_value(0.8);
    plugin.params.mix.set_value(0.5);
    plugin.initialize(48_000.0, 512);

    let block = 512usize;
    let mut left = vec![0.3f32; block * 40];
    let mut right = vec![0.3f32; block * 40];

    // Warm-up block (first call may lazily initialise nothing today,
    // but keep the guard honest against future regressions).
    {
        let (l, r) = (&mut left[..block], &mut right[..block]);
        let mut outs = [OutputBuffer { left: l, right: r }];
        let mut ev = EventIterator::empty();
        plugin.process(&mut outs, block, &mut ev, None);
    }

    let before = thread_allocs();
    run_blocks(&mut plugin, &mut left, &mut right, block);
    let after = thread_allocs();
    assert_eq!(
        after - before,
        0,
        "process() allocated {} times on the audio path",
        after - before
    );
}
