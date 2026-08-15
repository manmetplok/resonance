//! The diffusion stage (ba todo #1321): an allpass smear of the wet
//! path that the Diffuse knob has promised since the plugin shipped.
//!
//! What is pinned here: 0 is an exact bypass (the stage never even
//! touches the wet buses), turning it up smears transients in time
//! while an allpass chain's unity magnitude keeps the level, the smear
//! decorrelates the two channels rather than collapsing them, sweeping
//! the knob is click-free, and nothing ever goes non-finite. The
//! bit-identical-at-0 claim across the whole parameter space is pinned
//! by tests/dsp_regression.rs, whose 14 earlier scenarios all run
//! Diffusion 0 and whose golden words for them did not move.

use resonance_granular_delay::ResonanceGranularDelay;
use resonance_plugin::{EventIterator, OutputBuffer, Param, ResonancePlugin};
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

// --- Per-thread allocation counter for the no-allocation guard --------
// (pattern from tests/plugin.rs / tests/feedback.rs / tests/shimmer.rs).

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

const SR: f32 = 48_000.0;
const BLOCK: usize = 128;
const BLOCKS: usize = 240;

/// A short click every 200 ms on an otherwise silent input: the signal
/// a diffuser is judged on, since smearing shows up directly as the
/// transient spreading out in time.
fn click(n: u64) -> f32 {
    let period = (SR * 0.2) as u64;
    let phase = n % period;
    if phase < 8 {
        0.8 * (1.0 - phase as f32 / 8.0)
    } else {
        0.0
    }
}

fn plugin(diffusion: f32) -> ResonanceGranularDelay {
    let mut p = ResonanceGranularDelay::new();
    p.params.sync.set_plain(0.0);
    p.params.time_ms.set_value(60.0);
    p.params.grain_size_ms.set_value(40.0);
    p.params.density_hz.set_value(40.0);
    p.params.scheduler.set_value(0); // Sync
    p.params.mix.set_value(1.0); // wet only, so the stage is the output
    p.params.feedback.set_value(0.0);
    p.params.spray_ms.set_value(0.0);
    p.params.size_jitter.set_value(0.0);
    p.params.level_jitter.set_value(0.0);
    p.params.reverse_prob.set_value(0.0);
    p.params.pan_spread.set_value(0.0);
    p.params.diffusion.set_value(diffusion);
    p.initialize(SR, BLOCK as u32);
    p
}

/// Render the click train; returns the stereo output.
fn render(p: &mut ResonanceGranularDelay) -> (Vec<f32>, Vec<f32>) {
    let (mut ol, mut or) = (Vec::new(), Vec::new());
    let mut left = [0.0f32; BLOCK];
    let mut right = [0.0f32; BLOCK];
    let mut n = 0u64;
    for _ in 0..BLOCKS {
        for i in 0..BLOCK {
            let v = click(n + i as u64);
            left[i] = v;
            right[i] = v;
        }
        {
            let mut outs = [OutputBuffer {
                left: &mut left[..],
                right: &mut right[..],
            }];
            let mut ev = EventIterator::empty();
            p.process(&mut outs, BLOCK, &mut ev, None);
        }
        n += BLOCK as u64;
        ol.extend_from_slice(&left);
        or.extend_from_slice(&right);
    }
    (ol, or)
}

fn rms(x: &[f32]) -> f32 {
    (x.iter().map(|v| v * v).sum::<f32>() / x.len() as f32).sqrt()
}

/// Samples needed to accumulate 90 % of the signal's energy — the
/// bluntest possible measure of "smeared in time".
fn energy_spread(x: &[f32]) -> usize {
    let total: f32 = x.iter().map(|v| v * v).sum();
    let mut acc = 0.0;
    for (i, v) in x.iter().enumerate() {
        acc += v * v;
        if acc >= 0.9 * total {
            return i;
        }
    }
    x.len()
}

/// The parameter is read by the DSP at all — the finding was that it
/// was not. `diffusion_engaged` reports whether the stage touched the
/// wet buses on the last block.
#[test]
fn the_stage_engages_only_when_the_knob_leaves_zero() {
    let mut off = plugin(0.0);
    render(&mut off);
    assert!(
        !off.diffusion_engaged(),
        "Diffusion 0 must skip the stage entirely"
    );

    let mut on = plugin(0.6);
    render(&mut on);
    assert!(on.diffusion_engaged(), "the Diffuse knob is still inert");
}

/// At 0 the wet path is untouched; above 0 it audibly is.
#[test]
fn zero_is_a_bypass_and_nonzero_is_not() {
    let (a, _) = render(&mut plugin(0.0));
    let (b, _) = render(&mut plugin(0.0));
    assert_eq!(
        a.iter().map(|s| s.to_bits()).collect::<Vec<_>>(),
        b.iter().map(|s| s.to_bits()).collect::<Vec<_>>(),
        "the bypassed path must be deterministic"
    );

    let (c, _) = render(&mut plugin(1.0));
    let departure = a
        .iter()
        .zip(&c)
        .map(|(x, y)| (x - y).abs())
        .fold(0.0f32, f32::max);
    assert!(
        departure > 1e-3,
        "Diffusion 1 rendered the same audio as Diffusion 0 ({departure})"
    );
}

/// An allpass chain has unity magnitude response: it moves energy in
/// time, it does not add or remove it. So the smear must not change the
/// wet level appreciably — a diffuser that did would be a gain control.
#[test]
fn diffusion_smears_in_time_without_changing_the_level() {
    let (dry, _) = render(&mut plugin(0.0));
    let (wet, _) = render(&mut plugin(1.0));

    let (r_dry, r_wet) = (rms(&dry), rms(&wet));
    let ratio = r_wet / r_dry;
    assert!(
        (0.75..1.35).contains(&ratio),
        "diffusion changed the wet level by {ratio}x (allpass must be level-neutral)"
    );

    // Same signal, spread over more time.
    let (s_dry, s_wet) = (energy_spread(&dry), energy_spread(&wet));
    assert!(
        s_wet > s_dry,
        "diffusion did not spread the transient energy ({s_dry} -> {s_wet} samples)"
    );
}

/// The two channels run their own allpass lengths, so the smear widens
/// the wet image instead of collapsing it to mono.
#[test]
fn diffusion_decorrelates_the_two_channels() {
    let (l0, r0) = render(&mut plugin(0.0));
    let (l1, r1) = render(&mut plugin(1.0));

    let side = |l: &[f32], r: &[f32]| {
        rms(&l.iter().zip(r).map(|(a, b)| a - b).collect::<Vec<_>>())
    };
    assert!(
        side(&l0, &r0) <= 1e-6,
        "the undiffused mono case should have no side signal"
    );
    assert!(
        side(&l1, &r1) > 1e-4,
        "diffusion should decorrelate the channels"
    );
}

/// The allpass chains are pre-allocated at activation, so the render
/// path keeps the crate's no-allocation guarantee with the smear on.
#[test]
fn the_diffusion_stage_does_not_allocate() {
    let mut p = plugin(0.8);
    let mut left = [0.0f32; BLOCK];
    let mut right = [0.0f32; BLOCK];
    // Warm up: prime the ring and arm the chain outside the guard.
    for _ in 0..16 {
        let mut outs = [OutputBuffer {
            left: &mut left[..],
            right: &mut right[..],
        }];
        let mut ev = EventIterator::empty();
        p.process(&mut outs, BLOCK, &mut ev, None);
    }

    let before = thread_allocs();
    for _ in 0..64 {
        let mut outs = [OutputBuffer {
            left: &mut left[..],
            right: &mut right[..],
        }];
        let mut ev = EventIterator::empty();
        p.process(&mut outs, BLOCK, &mut ev, None);
    }
    assert_eq!(
        thread_allocs(),
        before,
        "the diffusion render path allocated"
    );
}

/// Sweeping the knob (including in and out of the bypass, which arms
/// and disarms the chain) stays click-free and finite.
#[test]
fn sweeping_the_knob_is_click_free() {
    let mut p = plugin(0.0);
    let mut left = [0.0f32; BLOCK];
    let mut right = [0.0f32; BLOCK];
    let mut n = 0u64;
    let mut prev = 0.0f32;
    let mut max_step = 0.0f32;

    for block in 0..BLOCKS {
        // 0 -> 1 -> 0, crossing the bypass boundary in both directions.
        let t = block as f32 / BLOCKS as f32;
        let amount = if t < 0.5 { 2.0 * t } else { 2.0 * (1.0 - t) };
        p.params.diffusion.set_value(amount);
        for i in 0..BLOCK {
            let v = 0.4
                * ((330.0 * (n + i as u64) as f32 / SR) * std::f32::consts::TAU).sin();
            left[i] = v;
            right[i] = v;
        }
        {
            let mut outs = [OutputBuffer {
                left: &mut left[..],
                right: &mut right[..],
            }];
            let mut ev = EventIterator::empty();
            p.process(&mut outs, BLOCK, &mut ev, None);
        }
        n += BLOCK as u64;
        for &s in left.iter() {
            assert!(s.is_finite(), "diffusion produced a non-finite sample");
            max_step = max_step.max((s - prev).abs());
            prev = s;
        }
    }
    // The material itself steps by well under 0.1 per sample at 330 Hz;
    // a splice would show up as a jump far larger than the signal.
    assert!(
        max_step < 0.3,
        "the diffusion sweep clicked: {max_step} sample-to-sample jump"
    );
}
