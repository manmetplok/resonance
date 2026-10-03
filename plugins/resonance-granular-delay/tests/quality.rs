//! Quality tiers (ba todo #1083, doc #252 §3/§9): Lo-fi / Normal / HQ.
//!
//! Provable tier semantics: on a +12 st transposed sine the aliased
//! energy orders HQ < Normal < Lo-fi (6-pt Lagrange + forced AA vs
//! Hermite vs linear + µ-law); Lo-fi shows bounded-SNR µ-law
//! quantization; Lo-fi caps the grain pool at half; and tier switches
//! mid-stream are click-free and allocation-free.

use resonance_granular_delay::dsp::LOFI_MAX_GRAINS;
use resonance_granular_delay::ResonanceGranularDelay;
use resonance_plugin::{EventIterator, OutputBuffer, Param, ResonancePlugin};
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

const SR: f32 = 48_000.0;

// --- Per-thread allocation counter for the no-allocation guard --------
// (pattern from tests/plugin.rs).

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

/// Deterministic wet-only plugin at the given quality tier: Sync
/// scheduler, no jitters, no feedback, Per-Grain time mode.
fn tier_plugin(quality: i32, time_ms: f32, density_hz: f32, pitch_st: f32) -> ResonanceGranularDelay {
    let plugin = ResonanceGranularDelay::new();
    plugin.params.sync.set_plain(0.0);
    plugin.params.time_ms.set_value(time_ms);
    plugin.params.grain_size_ms.set_value(90.0);
    plugin.params.density_hz.set_value(density_hz);
    plugin.params.scheduler.set_value(0); // Sync (deterministic)
    plugin.params.pitch.set_value(pitch_st);
    plugin.params.spray_ms.set_value(0.0);
    plugin.params.size_jitter.set_value(0.0);
    plugin.params.level_jitter.set_value(0.0);
    plugin.params.pan_spread.set_value(0.0);
    plugin.params.reverse_prob.set_value(0.0);
    plugin.params.spread_cents.set_value(0.0);
    plugin.params.feedback.set_value(0.0);
    plugin.params.mix.set_value(1.0); // wet only
    plugin.params.quality.set_value(quality);
    plugin
}

fn sine(frames: usize, freq: f32, amp: f32) -> Vec<f32> {
    (0..frames)
        .map(|i| (std::f32::consts::TAU * freq * i as f32 / SR).sin() * amp)
        .collect()
}

/// Hann-windowed single-bin DFT magnitude at `freq` (pattern from
/// resonance-dsp/tests/granular_pitch.rs).
fn dft_mag(x: &[f32], freq: f32) -> f32 {
    let tau = std::f64::consts::TAU;
    let w = tau * freq as f64 / SR as f64;
    let n_max = (x.len() - 1) as f64;
    let (mut re, mut im) = (0.0_f64, 0.0_f64);
    for (n, &s) in x.iter().enumerate() {
        let win = 0.5 - 0.5 * (tau * n as f64 / n_max).cos();
        let v = s as f64 * win;
        let phase = w * n as f64;
        re += v * phase.cos();
        im -= v * phase.sin();
    }
    ((re * re + im * im).sqrt() / x.len() as f64) as f32
}

fn max_second_difference(x: &[f32]) -> f32 {
    x.windows(3)
        .map(|w| (w[2] - 2.0 * w[1] + w[0]).abs())
        .fold(0.0_f32, f32::max)
}

fn rms(x: &[f32]) -> f32 {
    (x.iter().map(|&v| v as f64 * v as f64).sum::<f64>() / x.len().max(1) as f64).sqrt() as f32
}

/// Render 2 s of a 15 kHz sine through a wet-only +12 st (+7 ct) cloud
/// at the given tier and return the mean spectral magnitude over a
/// 1–23 kHz grid of the steady second.
///
/// The transposed partial lands at ~30.1 kHz — beyond Nyquist — so
/// *everything* measurable in the wet output is aliased energy: the
/// folded partial skirt near 18 kHz plus the interpolation images and
/// (Lo-fi) µ-law distortion products spread across the band. The +7
/// cent detune matters: at exactly +12 st the playback rate is 2.0 and
/// (with integer delay/IOT) every read lands on integer sample
/// positions, where all three kernels return the identical exact
/// sample and interpolation aliasing vanishes by construction. The
/// detune sweeps the fractional read phase so the kernels actually
/// engage.
fn aliased_energy(quality: i32) -> f32 {
    let mut plugin = tier_plugin(quality, 250.0, 25.0, 12.07);
    plugin.initialize(SR, 4096);
    let frames = (2.0 * SR) as usize;
    let input = sine(frames, 15_000.0, 0.8);
    let mut left = input.clone();
    let mut right = input;
    run_blocks(&mut plugin, &mut left, &mut right, 512);
    let seg = &left[SR as usize..];
    let mut sum = 0.0_f32;
    let mut n = 0;
    for i in 5..116 {
        sum += dft_mag(seg, i as f32 * 200.0);
        n += 1;
    }
    sum / n as f32
}

// --- Tests ------------------------------------------------------------

/// The tier ladder is audible where it claims to be: on a +12 st
/// transposed 15 kHz sine (whose transposed partial folds — see
/// [`aliased_energy`]), HQ (Lagrange + forced AA) has measurably less
/// aliased energy than Normal (Hermite, no AA), and Normal less than
/// Lo-fi (linear reads + µ-law distortion). Empirical margins are ~4×
/// per step; the asserts require 2×.
#[test]
fn aliased_energy_orders_hq_below_normal_below_lofi() {
    let e_lofi = aliased_energy(0);
    let e_normal = aliased_energy(1);
    let e_hq = aliased_energy(2);
    assert!(
        e_normal > 1e-7,
        "no measurable aliased energy at Normal ({e_normal:.3e}) — test misconfigured"
    );
    assert!(
        e_hq < e_normal / 2.0,
        "HQ aliased energy {e_hq:.3e} not clearly below Normal {e_normal:.3e}"
    );
    assert!(
        e_lofi > e_normal * 2.0,
        "Lo-fi aliased energy {e_lofi:.3e} not clearly above Normal {e_normal:.3e}"
    );
}

/// Lo-fi µ-law quantization is present and bounded. Setup pins every
/// grain read to integer positions (delay 500 ms = 24000 samples, Sync
/// IOT 48000/25 = 1920 samples, unity rate), where linear and Hermite
/// reads are both exact — so the Lo-fi-vs-Normal difference is purely
/// the windowed 8-bit µ-law error, and its SNR must land in the
/// plausible µ-law band (neither bit-exact nor broken).
#[test]
fn lofi_quantization_snr_is_bounded() {
    let render = |quality: i32| {
        let mut plugin = tier_plugin(quality, 500.0, 25.0, 0.0);
        plugin.initialize(SR, 4096);
        let frames = (2.0 * SR) as usize;
        let input = sine(frames, 440.0, 0.5);
        let mut left = input.clone();
        let mut right = input;
        run_blocks(&mut plugin, &mut left, &mut right, 512);
        left
    };
    let out_normal = render(1);
    let out_lofi = render(0);
    let steady = SR as usize..(2.0 * SR) as usize;
    let signal = rms(&out_normal[steady.clone()]);
    let noise: Vec<f32> = out_lofi[steady.clone()]
        .iter()
        .zip(&out_normal[steady])
        .map(|(l, n)| l - n)
        .collect();
    let noise_rms = rms(&noise);
    assert!(signal > 0.05, "no steady wet signal ({signal:.3e})");
    assert!(
        noise_rms > 0.0,
        "Lo-fi output is bit-identical to Normal — no quantization applied"
    );
    let snr_db = 20.0 * (signal / noise_rms).log10();
    assert!(
        (20.0..=55.0).contains(&snr_db),
        "Lo-fi quantization SNR {snr_db:.1} dB outside the plausible 8-bit µ-law band"
    );
}

/// Lo-fi reduces the grain pool: with settings that sustain ~50
/// overlapping grains, Normal exceeds the Lo-fi cap while Lo-fi never
/// crosses it.
#[test]
fn lofi_caps_the_grain_pool_at_half() {
    let count_active = |quality: i32| {
        let mut plugin = tier_plugin(quality, 500.0, 100.0, 0.0);
        plugin.params.grain_size_ms.set_value(500.0);
        plugin.initialize(SR, 4096);
        let frames = (2.0 * SR) as usize;
        let input = sine(frames, 440.0, 0.5);
        let mut max_active = 0usize;
        let mut pos = 0;
        while pos < frames {
            let n = 512.min(frames - pos);
            let mut left = input[pos..pos + n].to_vec();
            let mut right = input[pos..pos + n].to_vec();
            run_blocks(&mut plugin, &mut left, &mut right, n);
            if pos >= SR as usize {
                max_active = max_active.max(plugin.active_grains());
            }
            pos += n;
        }
        max_active
    };
    let normal_max = count_active(1);
    let lofi_max = count_active(0);
    assert!(
        normal_max > LOFI_MAX_GRAINS,
        "Normal never exceeded the Lo-fi cap ({normal_max} <= {LOFI_MAX_GRAINS}) — \
         test misconfigured"
    );
    assert!(
        lofi_max <= LOFI_MAX_GRAINS,
        "Lo-fi pool exceeded its cap: {lofi_max} > {LOFI_MAX_GRAINS}"
    );
}

/// Switching tiers mid-stream is click-free: cycling
/// Normal → Lo-fi → HQ → Normal over a steady sine produces no second
/// difference beyond the worst steady-state tier floor (grain-latched
/// kernels — a kernel swap under a sounding grain would step the
/// waveform).
#[test]
fn tier_switches_are_click_free() {
    let second = SR as usize;
    let render = |switches: bool| {
        let mut plugin = tier_plugin(1, 250.0, 25.0, 0.0);
        plugin.initialize(SR, 4096);
        let frames = 3 * second;
        let input = sine(frames, 440.0, 0.5);
        let mut left = input.clone();
        let mut right = input;
        let schedule: &[(usize, i32)] = if switches {
            // Quarter-second cadence through all tier transitions.
            &[
                (second, 0),
                (second + second / 4, 2),
                (second + second / 2, 1),
                (second + 3 * second / 4, 0),
                (2 * second, 1),
            ]
        } else {
            &[]
        };
        let mut pos = 0;
        let mut next = 0;
        while pos < frames {
            while next < schedule.len() && schedule[next].0 <= pos {
                plugin.params.quality.set_value(schedule[next].1);
                next += 1;
            }
            let n = 512.min(frames - pos);
            let (l, r) = (&mut left[pos..pos + n], &mut right[pos..pos + n]);
            run_blocks(&mut plugin, l, r, n);
            pos += n;
        }
        left
    };
    // Steady-state per-tier floors (Lo-fi's µ-law roughness dominates).
    let steady_floor = [0, 1, 2]
        .iter()
        .map(|&q| {
            let mut plugin = tier_plugin(q, 250.0, 25.0, 0.0);
            plugin.initialize(SR, 4096);
            let frames = 2 * second;
            let input = sine(frames, 440.0, 0.5);
            let mut left = input.clone();
            let mut right = input;
            run_blocks(&mut plugin, &mut left, &mut right, 512);
            max_second_difference(&left[second..])
        })
        .fold(0.0_f32, f32::max);
    let switched = render(true);
    let d2 = max_second_difference(&switched[second..]);
    assert!(
        d2 < (1.5 * steady_floor).max(0.02),
        "tier switching clicked: max d2 {d2} vs steady floor {steady_floor}"
    );
}

/// Tier switches allocate nothing: after warmup, cycling the quality
/// param every block through all three tiers keeps the audio path
/// allocation-free.
#[test]
fn tier_switching_does_not_allocate() {
    let mut plugin = tier_plugin(1, 250.0, 25.0, 12.0);
    plugin.initialize(SR, 4096);
    let block = 512;
    let mut left = vec![0.25_f32; block];
    let mut right = vec![0.25_f32; block];
    // Warmup: visit every tier once.
    for q in [0, 1, 2] {
        plugin.params.quality.set_value(q);
        run_blocks(&mut plugin, &mut left, &mut right, block);
    }
    let before = thread_allocs();
    for i in 0..200 {
        plugin.params.quality.set_value(i % 3);
        run_blocks(&mut plugin, &mut left, &mut right, block);
    }
    let after = thread_allocs();
    assert_eq!(
        after - before,
        0,
        "tier switching allocated {} times on the audio path",
        after - before
    );
}

/// DSP2-02: HQ must not be darker than Normal. At pitch 0 (and a hair
/// below, which sweeps the fractional read phase without engaging the
/// upward anti-alias read) a 5 and a 10 kHz partial through HQ must land
/// within 0.5 dB of Normal. The old quintic B-spline approximated rather
/// than interpolated — `[1,26,66,26,1]/120` even at integer positions —
/// and lost ~3.8 dB at 10 kHz, compounding on every recirculation.
#[test]
fn hq_keeps_the_passband_of_normal() {
    let level = |quality: i32, pitch: f32, hz: f32| {
        let mut plugin = tier_plugin(quality, 250.0, 25.0, pitch);
        plugin.initialize(SR, 4096);
        let frames = (2.0 * SR) as usize;
        let input = sine(frames, hz, 0.5);
        let mut left = input.clone();
        let mut right = input;
        run_blocks(&mut plugin, &mut left, &mut right, 512);
        let out_hz = hz * 2f32.powf(pitch / 12.0);
        20.0 * dft_mag(&left[SR as usize..], out_hz).log10()
    };
    for pitch in [0.0, -0.07] {
        for hz in [5_000.0, 10_000.0] {
            let normal = level(1, pitch, hz);
            let hq = level(2, pitch, hz);
            assert!(
                (hq - normal).abs() < 0.5,
                "{hz} Hz at {pitch} st: HQ {hq:.2} dB vs Normal {normal:.2} dB"
            );
        }
    }
}
