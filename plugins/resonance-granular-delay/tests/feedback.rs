//! Feedback-path behaviour through the full plugin stack (ba todo
//! #1074, doc #252 §1/§5): decaying repeat trains that track the loop
//! coefficient, bounded over-unity (110 %) feedback, DC-blocker
//! hygiene, in-loop damping, the Wet→Buffer vs Output-only topologies
//! and the no-allocation guarantee with the loop active.

use resonance_granular_delay::ResonanceGranularDelay;
use resonance_plugin::{EventIterator, OutputBuffer, Param, ResonancePlugin};
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

const SR: f32 = 48_000.0;

// --- Per-thread allocation counter for the no-allocation guard --------
// (pattern from tests/plugin.rs / resonance-dsp/tests/granular.rs).

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

/// Deterministic wet-only plugin (Sync scheduler, zero jitters) with an
/// explicit feedback setting; damping wide open unless a test narrows
/// it.
fn feedback_plugin(time_ms: f32, feedback: f32) -> ResonanceGranularDelay {
    let plugin = ResonanceGranularDelay::new();
    plugin.params.sync.set_plain(0.0);
    plugin.params.time_ms.set_value(time_ms);
    plugin.params.grain_size_ms.set_value(80.0);
    plugin.params.density_hz.set_value(30.0);
    plugin.params.scheduler.set_value(0); // Sync (deterministic)
    plugin.params.spray_ms.set_value(0.0);
    plugin.params.size_jitter.set_value(0.0);
    plugin.params.level_jitter.set_value(0.0);
    plugin.params.pan_spread.set_value(0.0);
    plugin.params.reverse_prob.set_value(0.0);
    plugin.params.spread_cents.set_value(0.0);
    plugin.params.mix.set_value(1.0); // wet only
    plugin.params.feedback.set_value(feedback);
    plugin.params.fb_route.set_value(0); // Wet -> Buffer
    plugin.params.filter_hz.set_value(20_000.0); // damping wide open
    plugin
}

fn rms(x: &[f32]) -> f32 {
    (x.iter().map(|v| v * v).sum::<f32>() / x.len().max(1) as f32).sqrt()
}

fn mean(x: &[f32]) -> f32 {
    x.iter().sum::<f32>() / x.len().max(1) as f32
}

/// Crude normalized high-frequency measure: first-difference energy
/// over signal energy (0 for DC, 4 for Nyquist-rate alternation).
fn hf_ratio(x: &[f32]) -> f32 {
    let e: f32 = x.iter().map(|v| v * v).sum();
    if e < 1e-12 {
        return 0.0;
    }
    let d: f32 = x.windows(2).map(|w| (w[1] - w[0]) * (w[1] - w[0])).sum();
    d / e
}

/// Amplitude ratio between consecutive impulse repeats. The loop delay
/// per pass is `delay + block` (the Wet→Buffer bus is applied at the
/// next block's write point), so repeat `k` centres near
/// `k * (delay_samples + block)`.
fn repeat_amplitude_ratio(feedback: f32, block: usize) -> f32 {
    let mut plugin = feedback_plugin(250.0, feedback);
    plugin.initialize(SR, block as u32);

    let frames = 2 * SR as usize;
    let mut left = vec![0.0f32; frames];
    let mut right = vec![0.0f32; frames];
    left[0] = 1.0;
    right[0] = 1.0;
    run_blocks(&mut plugin, &mut left, &mut right, block);

    let period = (0.250 * SR) as usize + block;
    let window = |k: usize| -> &[f32] {
        let c = k * period;
        &left[c - 2_000..c + 8_000]
    };
    rms(window(2)) / rms(window(1)).max(1e-9)
}

// --- Tests ------------------------------------------------------------

/// DoD: an impulse at 50 % feedback yields a decaying repeat train —
/// successive repeats lose amplitude monotonically and the train is
/// still audible at the fourth pass.
#[test]
fn impulse_at_half_feedback_yields_decaying_repeat_train() {
    let block = 512;
    let mut plugin = feedback_plugin(250.0, 0.5);
    plugin.initialize(SR, block as u32);

    let frames = 3 * SR as usize;
    let mut left = vec![0.0f32; frames];
    let mut right = vec![0.0f32; frames];
    left[0] = 1.0;
    right[0] = 1.0;
    run_blocks(&mut plugin, &mut left, &mut right, block);

    for &x in &left {
        assert!(x.is_finite(), "non-finite sample in repeat train: {x}");
    }

    let period = (0.250 * SR) as usize + block;
    let energy: Vec<f32> = (1..=4)
        .map(|k| rms(&left[k * period - 2_000..k * period + 8_000]))
        .collect();
    for k in 1..energy.len() {
        assert!(
            energy[k] < energy[k - 1],
            "repeat {} did not decay: {:?}",
            k + 1,
            energy
        );
    }
    assert!(
        energy[3] > 1e-5,
        "repeat train died too early at 50% feedback: {energy:?}"
    );
}

/// The decay rate tracks the feedback coefficient: halving the loop
/// gain halves the per-repeat amplitude ratio (up to the granulation
/// loop's own, feedback-independent gain).
#[test]
fn repeat_decay_rate_tracks_the_feedback_coefficient() {
    let r_half = repeat_amplitude_ratio(0.5, 512);
    let r_quarter = repeat_amplitude_ratio(0.25, 512);
    assert!(
        r_half > 0.0 && r_half < 1.0,
        "50% feedback repeat ratio out of range: {r_half}"
    );
    let coeff_ratio = r_quarter / r_half.max(1e-9);
    assert!(
        (0.3..=0.7).contains(&coeff_ratio),
        "per-repeat decay does not track the coefficient: \
         r(0.25)={r_quarter}, r(0.5)={r_half}, ratio {coeff_ratio} (ideal 0.5)"
    );
}

/// DoD: 110 % feedback with the *default* damping (LP 8 kHz) stays
/// bounded over a 30 s render — the tanh soft clip catches runaway.
/// Peak must stay below +6 dBFS and the late RMS below unity.
#[test]
fn feedback_110_percent_with_default_damping_stays_bounded() {
    let block = 1024;
    let mut plugin = feedback_plugin(250.0, 1.1);
    plugin.params.filter_hz.set_value(8_000.0); // default damping
    plugin.initialize(SR, block as u32);

    let frames = 30 * SR as usize;
    let mut left = vec![0.0f32; frames];
    let mut right = vec![0.0f32; frames];
    left[0] = 1.0;
    right[0] = 1.0;
    run_blocks(&mut plugin, &mut left, &mut right, block);

    let mut peak = 0.0f32;
    for &x in left.iter().chain(right.iter()) {
        assert!(x.is_finite(), "non-finite sample at 110% feedback: {x}");
        peak = peak.max(x.abs());
    }
    assert!(
        peak < 2.0,
        "110% feedback exceeded +6 dBFS: peak {peak}"
    );
    let late = &left[25 * SR as usize..];
    assert!(
        rms(late) < 1.0,
        "110% feedback late RMS above unity: {}",
        rms(late)
    );
}

/// DoD: a constant DC-offset input does not accumulate DC across
/// repeats. At 100 % feedback with the damping wide open, the late
/// output mean stays at the single-pass (feedback 0) level because the
/// in-loop DC blocker strips offset from every recirculation.
#[test]
fn dc_offset_input_does_not_accumulate_across_repeats() {
    let render_mean = |feedback: f32, seconds: usize| -> f32 {
        let mut plugin = feedback_plugin(250.0, feedback);
        plugin.initialize(SR, 1024);
        let frames = seconds * SR as usize;
        let mut left = vec![0.3f32; frames];
        let mut right = vec![0.3f32; frames];
        run_blocks(&mut plugin, &mut left, &mut right, 1024);
        mean(&left[frames - SR as usize..])
    };

    let single_pass = render_mean(0.0, 6);
    let recirculated = render_mean(1.0, 12);
    assert!(
        single_pass.abs() > 0.01,
        "DC input produced no single-pass wet DC to compare against: {single_pass}"
    );
    assert!(
        recirculated.abs() < single_pass.abs() * 1.5 + 0.02,
        "DC accumulated across repeats: single-pass mean {single_pass}, \
         100% feedback mean {recirculated}"
    );
}

/// DoD: with FB Route = Output-only the grain source buffer receives
/// the clean dry input only — bit-exact — even at high feedback.
#[test]
fn output_only_route_keeps_the_buffer_clean() {
    let mut plugin = feedback_plugin(250.0, 0.9);
    plugin.params.fb_route.set_value(1); // Output-only
    plugin.initialize(SR, 512);

    let frames = SR as usize;
    let mut left = vec![0.0f32; frames];
    let mut right = vec![0.0f32; frames];
    for i in 0..frames {
        let s = (std::f32::consts::TAU * 220.0 * i as f32 / SR).sin() * 0.7;
        left[i] = s;
        right[i] = s * 0.5;
    }
    let dry_l = left.clone();
    let dry_r = right.clone();

    run_blocks(&mut plugin, &mut left, &mut right, 512);

    let ring_l = plugin.ring_l();
    let ring_r = plugin.ring_r();
    let mask = ring_l.len() - 1;
    for n in 0..frames {
        assert!(
            ring_l[n & mask] == dry_l[n] && ring_r[n & mask] == dry_r[n],
            "Output-only route wrote wet material into the buffer at sample {n}: \
             ring ({}, {}) vs dry ({}, {})",
            ring_l[n & mask],
            ring_r[n & mask],
            dry_l[n],
            dry_r[n]
        );
    }
}

/// The two topologies measurably differ in re-granulation: with +12 st
/// grains, FB Pitch on and a sine burst, Wet→Buffer re-granulates
/// every recirculation so the second repeat climbs another octave,
/// while Output-only recirculates the once-granulated wet unchanged.
/// The dominant frequency of repeat 2 (zero-crossing rate) proves it,
/// and the Wet→Buffer buffer visibly contains written-back wet
/// material. (Since ba todo #1078 the cumulative climb is gated behind
/// FB Pitch; tests/shimmer.rs covers the constant-pitch off state.)
#[test]
fn topologies_differ_in_recirculated_regranulation() {
    let render = |route: i32| -> (Vec<f32>, Vec<f32>) {
        let mut plugin = feedback_plugin(400.0, 0.9);
        plugin.params.fb_route.set_value(route);
        plugin.params.pitch.set_value(12.0); // each granulation pass: +1 octave
        plugin.params.fb_pitch.set_plain(1.0); // shimmer: transpose recirculates
        plugin.initialize(SR, 512);

        let frames = 2 * SR as usize;
        let mut left = vec![0.0f32; frames];
        let mut right = vec![0.0f32; frames];
        // 150 ms, 220 Hz burst: pass k occupies ~[k*400ms, k*400ms+230ms].
        for i in 0..(0.15 * SR) as usize {
            let s = (std::f32::consts::TAU * 220.0 * i as f32 / SR).sin() * 0.8;
            left[i] = s;
            right[i] = s;
        }
        let dry = left.clone();
        run_blocks(&mut plugin, &mut left, &mut right, 512);
        (left, dry)
    };

    let zero_crossings = |x: &[f32]| -> usize {
        x.windows(2)
            .filter(|w| (w[0] >= 0.0) != (w[1] >= 0.0))
            .count()
    };

    let (wet_to_buffer, dry) = render(0);
    let (output_only, _) = render(1);

    // Repeat 2 window: 2 loop passes of (400 ms + 512-sample bus delay).
    let period = (0.400 * SR) as usize + 512;
    let w0 = 2 * period;
    let w1 = w0 + (0.15 * SR) as usize;
    let zc_wtb = zero_crossings(&wet_to_buffer[w0..w1]);
    let zc_oo = zero_crossings(&output_only[w0..w1]);
    assert!(
        rms(&wet_to_buffer[w0..w1]) > 1e-4 && rms(&output_only[w0..w1]) > 1e-4,
        "no second repeat to compare: WtB rms {}, OO rms {}",
        rms(&wet_to_buffer[w0..w1]),
        rms(&output_only[w0..w1])
    );
    assert!(
        zc_wtb as f32 > 1.5 * zc_oo as f32,
        "Wet→Buffer repeat 2 was not re-granulated up an extra octave: \
         {zc_wtb} vs {zc_oo} zero crossings"
    );

    // And the Wet→Buffer route demonstrably writes wet into the buffer:
    // around the first repeat the ring differs from the dry input.
    let mut plugin = feedback_plugin(400.0, 0.9);
    plugin.initialize(SR, 512);
    let frames = SR as usize;
    let mut left = dry[..frames].to_vec();
    let mut right = dry[..frames].to_vec();
    run_blocks(&mut plugin, &mut left, &mut right, 512);
    let ring = plugin.ring_l();
    let mask = ring.len() - 1;
    let echo = (0.400 * SR) as usize + 512;
    let max_diff = (echo..echo + (0.2 * SR) as usize)
        .map(|n| (ring[n & mask] - if n < frames { dry[n] } else { 0.0 }).abs())
        .fold(0.0f32, f32::max);
    assert!(
        max_diff > 1e-3,
        "Wet→Buffer route wrote nothing back into the buffer: max diff {max_diff}"
    );
}

/// The in-loop damping filter shapes the recirculating spectrum: after
/// many passes a dark lowpass leaves relatively less high-frequency
/// energy than a wide-open one, and highpass damping leaves relatively
/// more than lowpass at the same cutoff.
#[test]
fn damping_filter_shapes_the_loop_spectrum() {
    let late_hf = |filter_type: i32, cutoff: f32| -> f32 {
        let mut plugin = feedback_plugin(250.0, 1.05);
        plugin.params.filter_type.set_value(filter_type);
        plugin.params.filter_hz.set_value(cutoff);
        plugin.initialize(SR, 1024);
        let frames = 3 * SR as usize;
        let mut left = vec![0.0f32; frames];
        let mut right = vec![0.0f32; frames];
        left[0] = 1.0;
        right[0] = 1.0;
        run_blocks(&mut plugin, &mut left, &mut right, 1024);
        hf_ratio(&left[SR as usize..])
    };

    let dark = late_hf(0, 1_500.0);
    let bright = late_hf(0, 16_000.0);
    assert!(
        dark < 0.6 * bright,
        "lowpass damping did not darken the loop: LP1.5k hf {dark}, LP16k hf {bright}"
    );

    let highpassed = late_hf(1, 1_500.0);
    assert!(
        highpassed > 1.5 * dark,
        "highpass damping did not brighten the loop relative to lowpass: \
         HP1.5k hf {highpassed}, LP1.5k hf {dark}"
    );
}

/// The audio path stays allocation-free with the feedback loop active,
/// across both topologies and a filter-type flip mid-run.
#[test]
fn audio_path_with_feedback_active_does_not_allocate() {
    let mut plugin = feedback_plugin(200.0, 1.0);
    plugin.params.mix.set_value(0.5);
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
    plugin.params.fb_route.set_value(1); // Output-only
    plugin.params.filter_type.set_value(1); // HP
    run_blocks(&mut plugin, &mut left, &mut right, block);
    let after = thread_allocs();
    assert_eq!(
        after - before,
        0,
        "feedback path allocated {} times on the audio path",
        after - before
    );
}
