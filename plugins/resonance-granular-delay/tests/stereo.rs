//! Stereo-stage behaviour through the full plugin stack (ba todo #1077,
//! doc #252 §5): per-grain pan with decorrelated L/R scheduling behind
//! Pan Spread, M/S width on the wet sum, the ping-pong feedback route,
//! dry-path bit-exactness and the no-allocation guarantee with the
//! stereo processing active.

use resonance_granular_delay::ResonanceGranularDelay;
use resonance_plugin::{EventIterator, OutputBuffer, Param, ResonancePlugin};
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

const SR: f32 = 48_000.0;

// --- Per-thread allocation counter for the no-allocation guard --------
// (pattern from tests/plugin.rs / tests/feedback.rs).

thread_local! {
    static ALLOC_COUNT: Cell<usize> = const { Cell::new(0) };
}

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

/// Wet-only plugin with a decorrelation-friendly cloud (async
/// scheduler, position spray) and no feedback; individual tests set
/// pan spread and width.
fn stereo_plugin(pan_spread: f32, width: f32) -> ResonanceGranularDelay {
    let plugin = ResonanceGranularDelay::new();
    plugin.params.sync.set_plain(0.0);
    plugin.params.time_ms.set_value(200.0);
    plugin.params.grain_size_ms.set_value(60.0);
    plugin.params.density_hz.set_value(30.0);
    plugin.params.scheduler.set_value(1); // Async: RNG drives onsets
    plugin.params.spray_ms.set_value(30.0); // RNG drives positions
    plugin.params.size_jitter.set_value(0.5);
    plugin.params.level_jitter.set_value(0.0);
    plugin.params.reverse_prob.set_value(0.0);
    plugin.params.spread_cents.set_value(0.0);
    plugin.params.pan_spread.set_value(pan_spread);
    plugin.params.width.set_value(width);
    plugin.params.feedback.set_value(0.0);
    plugin.params.mix.set_value(1.0); // wet only
    plugin
}

/// Deterministic broadband mono test signal (xorshift32 noise).
fn mono_noise(frames: usize) -> Vec<f32> {
    let mut state = 0x1234_5679u32;
    (0..frames)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            (state >> 8) as f32 * (1.0 / (1 << 24) as f32) - 0.5
        })
        .collect()
}

fn rms(x: &[f32]) -> f32 {
    (x.iter().map(|v| v * v).sum::<f32>() / x.len().max(1) as f32).sqrt()
}

/// Normalized zero-lag cross-correlation coefficient.
fn correlation(a: &[f32], b: &[f32]) -> f64 {
    let (mut aa, mut bb, mut ab) = (0.0f64, 0.0f64, 0.0f64);
    for (&x, &y) in a.iter().zip(b.iter()) {
        aa += (x * x) as f64;
        bb += (y * y) as f64;
        ab += (x * y) as f64;
    }
    ab / (aa * bb).sqrt().max(1e-30)
}

/// Render 1.5 s of mono noise through a wet-only stereo plugin and
/// return the steady-state region of both channels.
fn render_steady(pan_spread: f32, width: f32) -> (Vec<f32>, Vec<f32>) {
    let mut plugin = stereo_plugin(pan_spread, width);
    plugin.initialize(SR, 512);
    let frames = (1.5 * SR) as usize;
    let noise = mono_noise(frames);
    let mut left = noise.clone();
    let mut right = noise;
    run_blocks(&mut plugin, &mut left, &mut right, 512);
    let steady = (0.4 * SR) as usize;
    (left[steady..].to_vec(), right[steady..].to_vec())
}

// --- Tests ------------------------------------------------------------

/// DoD: width 0 collapses the wet bus to its mid signal — left and
/// right are equal within epsilon even with the decorrelated engine
/// fully engaged (pan spread 100 %).
#[test]
fn width_zero_makes_wet_left_equal_right() {
    let (left, right) = render_steady(1.0, 0.0);
    assert!(rms(&left) > 0.02, "wet bus unexpectedly quiet: {}", rms(&left));
    for (i, (&l, &r)) in left.iter().zip(right.iter()).enumerate() {
        assert!(
            (l - r).abs() <= 1e-7,
            "width 0 left/right diverge at sample {i}: {l} vs {r}"
        );
    }
}

/// DoD: pan spread 100 % decorrelates the channels — the L/R
/// cross-correlation of the wet output is clearly lower than at spread
/// 0, where the lock-stepped engines render identical clouds over a
/// mono input (correlation ~1).
#[test]
fn pan_spread_decorrelates_left_and_right() {
    let (l0, r0) = render_steady(0.0, 1.0);
    let (l1, r1) = render_steady(1.0, 1.0);
    let corr_locked = correlation(&l0, &r0);
    let corr_spread = correlation(&l1, &r1);
    assert!(
        corr_locked > 0.99,
        "lock-stepped mono wet should be fully correlated: {corr_locked}"
    );
    assert!(
        corr_spread < 0.8,
        "pan spread 100% did not decorrelate the channels: {corr_spread}"
    );
    assert!(
        corr_spread < corr_locked - 0.25,
        "decorrelation not clearly lower than lock-stepped: \
         {corr_spread} vs {corr_locked}"
    );
}

/// M/S width scales the side channel linearly (the grain cloud is
/// seed-deterministic, so renders differ only in the width applied at
/// the wet mix point) and leaves the mid channel untouched.
#[test]
fn width_scales_side_energy_and_leaves_mid_alone() {
    let side = |l: &[f32], r: &[f32]| -> Vec<f32> {
        l.iter().zip(r.iter()).map(|(&a, &b)| 0.5 * (a - b)).collect()
    };
    let mid = |l: &[f32], r: &[f32]| -> Vec<f32> {
        l.iter().zip(r.iter()).map(|(&a, &b)| 0.5 * (a + b)).collect()
    };

    let (l02, r02) = render_steady(1.0, 0.2);
    let (l10, r10) = render_steady(1.0, 1.0);
    let (l15, r15) = render_steady(1.0, 1.5);

    let s02 = rms(&side(&l02, &r02));
    let s10 = rms(&side(&l10, &r10));
    let s15 = rms(&side(&l15, &r15));
    assert!(
        s10 > 0.005,
        "decorrelated cloud produced no side energy to scale: {s10}"
    );
    let narrow = s02 / s10;
    let wide = s15 / s10;
    assert!(
        (narrow - 0.2).abs() < 0.01,
        "side energy at width 0.2 is not 0.2x the width-1 side: ratio {narrow}"
    );
    assert!(
        (wide - 1.5).abs() < 0.01,
        "side energy at width 1.5 is not 1.5x the width-1 side: ratio {wide}"
    );

    let m0 = mid(&l02, &r02);
    let m1 = mid(&l15, &r15);
    let max_mid_diff = m0
        .iter()
        .zip(m1.iter())
        .map(|(&a, &b)| (a - b).abs())
        .fold(0.0f32, f32::max);
    assert!(
        max_mid_diff < 1e-6,
        "width changed the mid channel: max diff {max_mid_diff}"
    );
}

/// Mono compatibility: folding the full-width output down to mono
/// equals the width-0 render (the mid signal) — widening introduces no
/// combing into the mono sum beyond float rounding.
#[test]
fn mono_fold_down_of_full_width_matches_width_zero() {
    let (l1, r1) = render_steady(1.0, 1.0);
    let (l0, _r0) = render_steady(1.0, 0.0);
    let max_diff = l1
        .iter()
        .zip(r1.iter())
        .zip(l0.iter())
        .map(|((&l, &r), &m)| (0.5 * (l + r) - m).abs())
        .fold(0.0f32, f32::max);
    assert!(
        max_diff < 1e-6,
        "mono fold-down of width 1 combs against the width-0 collapse: \
         max diff {max_diff}"
    );
}

/// DoD: with FB Route = Ping-pong, an impulse fed to the left input
/// alternates repeat energy between the channels: repeat 1 lands left,
/// repeat 2 right, repeat 3 left again.
#[test]
fn ping_pong_alternates_repeat_energy_between_channels() {
    let block = 512usize;
    let plugin = ResonanceGranularDelay::new();
    plugin.params.sync.set_plain(0.0);
    plugin.params.time_ms.set_value(250.0);
    plugin.params.grain_size_ms.set_value(80.0);
    plugin.params.density_hz.set_value(30.0);
    plugin.params.scheduler.set_value(0); // Sync (deterministic)
    plugin.params.spray_ms.set_value(0.0);
    plugin.params.size_jitter.set_value(0.0);
    plugin.params.level_jitter.set_value(0.0);
    plugin.params.pan_spread.set_value(0.0);
    plugin.params.reverse_prob.set_value(0.0);
    plugin.params.spread_cents.set_value(0.0);
    plugin.params.mix.set_value(1.0);
    plugin.params.feedback.set_value(0.8);
    plugin.params.fb_route.set_value(2); // Ping-pong
    plugin.params.filter_hz.set_value(20_000.0);
    let mut plugin = plugin;
    plugin.initialize(SR, block as u32);

    let frames = 2 * SR as usize;
    let mut left = vec![0.0f32; frames];
    let mut right = vec![0.0f32; frames];
    left[0] = 1.0; // left input only

    run_blocks(&mut plugin, &mut left, &mut right, block);

    // Loop period per pass: exactly the delay (see tests/feedback.rs).
    let period = (0.250 * SR) as usize;
    let window = |x: &[f32], k: usize| rms(&x[k * period - 2_000..k * period + 8_000]);

    let (l1, r1) = (window(&left, 1), window(&right, 1));
    let (l2, r2) = (window(&left, 2), window(&right, 2));
    let (l3, r3) = (window(&left, 3), window(&right, 3));
    assert!(
        l1 > 10.0 * r1,
        "repeat 1 should land left: L {l1} vs R {r1}"
    );
    assert!(
        r2 > 10.0 * l2,
        "repeat 2 should cross to the right: L {l2} vs R {r2}"
    );
    assert!(
        l3 > 10.0 * r3,
        "repeat 3 should cross back to the left: L {l3} vs R {r3}"
    );
    assert!(
        r2 > 1e-4 && l3 > 1e-5,
        "ping-pong repeats died too early: r2 {r2}, l3 {l3}"
    );
}

/// DoD: with mix = 0 the output is bit-exact to the input even with
/// every stereo stage engaged (decorrelated spread, width 150 %,
/// ping-pong feedback).
#[test]
fn mix_zero_is_bit_exact_with_stereo_processing_active() {
    let mut plugin = stereo_plugin(1.0, 1.5);
    plugin.params.mix.set_value(0.0);
    plugin.params.feedback.set_value(0.9);
    plugin.params.fb_route.set_value(2); // Ping-pong
    plugin.initialize(SR, 512);

    let frames = SR as usize / 2;
    let mut left = vec![0.0f32; frames];
    let mut right = vec![0.0f32; frames];
    for i in 0..frames {
        let s = (std::f32::consts::TAU * 220.0 * i as f32 / SR).sin() * 0.6;
        left[i] = s;
        right[i] = -0.5 * s;
    }
    let dry_l = left.clone();
    let dry_r = right.clone();

    run_blocks(&mut plugin, &mut left, &mut right, 512);

    // Bit-exact up to IEEE zero-sign normalization: adding the zeroed
    // wet term (`x + 0.0`) turns a -0.0 dry sample into +0.0, which is
    // numerically the same value.
    let same = |out: f32, dry: f32| out.to_bits() == dry.to_bits() || (out == 0.0 && dry == 0.0);
    for i in 0..frames {
        assert!(
            same(left[i], dry_l[i]) && same(right[i], dry_r[i]),
            "dry path not bit-exact at sample {i}: ({}, {}) vs ({}, {})",
            left[i],
            right[i],
            dry_l[i],
            dry_r[i]
        );
    }
}

/// The audio path stays allocation-free with the full stereo stage
/// active, including toggling pan spread across zero mid-run (the
/// decorrelated engine's engage/drain paths).
#[test]
fn stereo_processing_does_not_allocate() {
    let mut plugin = stereo_plugin(1.0, 1.5);
    plugin.params.mix.set_value(0.5);
    plugin.params.feedback.set_value(0.9);
    plugin.params.fb_route.set_value(2); // Ping-pong
    plugin.initialize(SR, 512);

    let block = 512usize;
    let mut left = vec![0.3f32; block * 40];
    let mut right = vec![0.3f32; block * 40];

    // Warm-up block.
    {
        let (l, r) = (&mut left[..block], &mut right[..block]);
        let mut outs = [OutputBuffer { left: l, right: r }];
        let mut ev = EventIterator::empty();
        plugin.process(&mut outs, block, &mut ev, None);
    }

    let before = thread_allocs();
    run_blocks(&mut plugin, &mut left, &mut right, block);
    plugin.params.pan_spread.set_value(0.0); // decorrelated -> drain
    run_blocks(&mut plugin, &mut left, &mut right, block);
    plugin.params.pan_spread.set_value(1.0); // re-engage
    run_blocks(&mut plugin, &mut left, &mut right, block);
    let after = thread_allocs();
    assert_eq!(
        after - before,
        0,
        "stereo stage allocated {} times on the audio path",
        after - before
    );
}
