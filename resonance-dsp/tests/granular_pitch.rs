//! Tests for per-grain pitch in the grain engine: playback-rate
//! transposition, symmetric detune spread, the rate-tracked anti-alias
//! lowpass and reverse grains (ba todo #1072, doc #252 §3).

use resonance_dsp::{GrainEngine, GrainParams, SchedulerMode};
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::f64::consts::TAU;

const SR: f32 = 48_000.0;
const BLOCK: usize = 256;
const BUF_LEN: usize = 1 << 16; // 65536 samples ≈ 1.37 s @ 48 kHz

// --- Per-thread allocation counter for the no-allocation guard. -------

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

// --- Helpers. ---------------------------------------------------------

/// Sine with an integer number of cycles over the buffer so the
/// circular wrap is seamless. Frequency = cycles · SR / len.
fn sine_buffer(len: usize, cycles: usize, amp: f32) -> Vec<f32> {
    (0..len)
        .map(|i| (TAU * cycles as f64 * i as f64 / len as f64).sin() as f32 * amp)
        .collect()
}

fn render_seconds(
    engine: &mut GrainEngine,
    source: &[f32],
    params: &GrainParams,
    seconds: f32,
) -> (Vec<f32>, Vec<f32>) {
    let total = (seconds * SR) as usize;
    let mut left = vec![0.0_f32; total];
    let mut right = vec![0.0_f32; total];
    let mut write_pos = 0.0_f64;
    let mut i = 0;
    while i < total {
        let n = BLOCK.min(total - i);
        engine.process(
            source,
            write_pos,
            params,
            &mut left[i..i + n],
            &mut right[i..i + n],
        );
        write_pos += n as f64 * params.head_advance as f64;
        i += n;
    }
    (left, right)
}

/// Hann-windowed single-bin DFT magnitude at `freq` (normalized by the
/// segment length).
fn dft_mag(x: &[f32], freq: f32) -> f32 {
    let w = TAU * freq as f64 / SR as f64;
    let n_max = (x.len() - 1) as f64;
    let (mut re, mut im) = (0.0_f64, 0.0_f64);
    for (n, &s) in x.iter().enumerate() {
        let win = 0.5 - 0.5 * (TAU * n as f64 / n_max).cos();
        let v = s as f64 * win;
        let phase = w * n as f64;
        re += v * phase.cos();
        im -= v * phase.sin();
    }
    ((re * re + im * im).sqrt() / x.len() as f64) as f32
}

fn rms(x: &[f32]) -> f32 {
    (x.iter().map(|&v| v as f64 * v as f64).sum::<f64>() / x.len() as f64).sqrt() as f32
}

fn max_second_difference(x: &[f32]) -> f32 {
    x.windows(3)
        .map(|w| (w[2] - 2.0 * w[1] + w[0]).abs())
        .fold(0.0_f32, f32::max)
}

// --- Playback-rate transposition. -------------------------------------

#[test]
fn transpose_plus_octave_moves_dominant_peak_to_double_f0() {
    // 512 cycles over 65536 samples = exactly 375 Hz.
    let source = sine_buffer(BUF_LEN, 512, 0.8);
    let mut engine = GrainEngine::new(SR, 5);
    let params = GrainParams {
        density_hz: 50.0,
        grain_seconds: 0.1,
        position_seconds: 0.5,
        position_jitter_seconds: 0.3,
        texture: 1.0,
        mode: SchedulerMode::Async,
        pitch_semitones: 12.0,
        ..GrainParams::default()
    };
    let (left, _) = render_seconds(&mut engine, &source, &params, 1.5);
    let segment = &left[24_000..24_000 + 16_384];

    let target = dft_mag(segment, 750.0);
    assert!(target > 1e-3, "no energy at 2·f0: {target}");
    let at_f0 = dft_mag(segment, 375.0);
    assert!(
        target > 5.0 * at_f0,
        "peak must move to 2·f0: |750 Hz| = {target}, |375 Hz| = {at_f0}"
    );
    // 2·f0 dominates the whole band (coarse scan, skipping the peak's
    // own window skirt around 650–850 Hz).
    let mut freq = 100.0_f32;
    while freq <= 5000.0 {
        if !(650.0..=850.0).contains(&freq) {
            let mag = dft_mag(segment, freq);
            assert!(
                target > 3.0 * mag,
                "spectral peak not dominant at 2·f0: |{freq} Hz| = {mag} vs {target}"
            );
        }
        freq += 50.0;
    }
}

// --- Symmetric detune spread. -----------------------------------------

#[test]
fn detune_spread_alternates_sign_and_has_zero_mean() {
    let source = sine_buffer(BUF_LEN, 512, 0.8);
    let mut engine = GrainEngine::new(SR, 21);
    // Non-overlapping grains (IOT 2400 samples, grains 960 samples) so
    // each spawn is observable in isolation via `active_rates`.
    let params = GrainParams {
        density_hz: 20.0,
        grain_seconds: 0.02,
        position_seconds: 0.3,
        texture: 1.0,
        mode: SchedulerMode::Sync,
        detune_spread_cents: 25.0,
        ..GrainParams::default()
    };

    let mut cents = Vec::new();
    let mut left = vec![0.0_f32; BLOCK];
    let mut right = vec![0.0_f32; BLOCK];
    let mut write_pos = 0.0_f64;
    let mut last_spawned = 0;
    for _ in 0..(20.0 * SR as f64 / BLOCK as f64) as usize {
        left.fill(0.0);
        right.fill(0.0);
        engine.process(&source, write_pos, &params, &mut left, &mut right);
        write_pos += BLOCK as f64;
        let spawned = engine.grains_spawned();
        if spawned > last_spawned {
            assert_eq!(spawned, last_spawned + 1, "more than one spawn per block");
            assert_eq!(engine.active_grains(), 1, "grains must not overlap here");
            let rate = engine.active_rates().next().unwrap();
            cents.push(1200.0 * rate.log2());
            last_spawned = spawned;
        }
    }

    assert!(cents.len() >= 300, "too few grains: {}", cents.len());
    let mean = cents.iter().sum::<f64>() / cents.len() as f64;
    assert!(
        mean.abs() < 0.5,
        "mean signed detune {mean} cents not ~0 over {} grains",
        cents.len()
    );
    let mean_abs = cents.iter().map(|c| c.abs()).sum::<f64>() / cents.len() as f64;
    assert!(
        (5.0..20.0).contains(&mean_abs),
        "detune magnitude off: mean |cents| = {mean_abs}, expected ~12.5"
    );
    // Strict ± alternation between consecutive grains.
    for pair in cents.windows(2) {
        if pair[0].abs() > 1e-6 && pair[1].abs() > 1e-6 {
            assert!(
                pair[0].signum() != pair[1].signum(),
                "consecutive detunes {} and {} share a sign",
                pair[0],
                pair[1]
            );
        }
    }
}

// --- Anti-alias lowpass. ----------------------------------------------

#[test]
fn anti_alias_filter_reduces_folded_energy() {
    // 20480 cycles over 65536 samples = exactly 15 kHz; at rate 2 the
    // resampled 30 kHz partial folds to 18 kHz.
    let source = sine_buffer(BUF_LEN, 20_480, 0.8);
    let render = |anti_alias: bool| {
        let mut engine = GrainEngine::new(SR, 11); // same seed → same schedule
        let params = GrainParams {
            density_hz: 40.0,
            grain_seconds: 0.1,
            position_seconds: 0.5,
            texture: 1.0,
            mode: SchedulerMode::Sync,
            pitch_semitones: 12.0,
            anti_alias,
            ..GrainParams::default()
        };
        render_seconds(&mut engine, &source, &params, 1.0).0
    };
    let with_aa = render(true);
    let without_aa = render(false);

    let seg_on = &with_aa[24_000..24_000 + 16_384];
    let seg_off = &without_aa[24_000..24_000 + 16_384];
    let aliased_on = dft_mag(seg_on, 18_000.0);
    let aliased_off = dft_mag(seg_off, 18_000.0);
    assert!(
        aliased_off > 1e-4,
        "no aliased energy without AA ({aliased_off}) — test signal invalid"
    );
    // A one-pole at 0.45·fs/rate = 10.8 kHz attenuates the 18 kHz fold
    // to ~40% of its energy; require a clearly measurable reduction.
    assert!(
        aliased_on < 0.7 * aliased_off,
        "AA filter ineffective: |18 kHz| on = {aliased_on}, off = {aliased_off}"
    );
}

// --- Reverse grains. --------------------------------------------------

#[test]
fn full_reverse_probability_is_click_free_with_negative_rates() {
    // 137 cycles ≈ 100.3 Hz, seamless wrap.
    let source = sine_buffer(BUF_LEN, 137, 0.9);
    let mut engine = GrainEngine::new(SR, 13);
    let params = GrainParams {
        density_hz: 30.0,
        grain_seconds: 0.08,
        position_seconds: 0.4,
        position_jitter_seconds: 0.1,
        texture: 1.0,
        mode: SchedulerMode::Async,
        reverse_probability: 1.0,
        ..GrainParams::default()
    };

    let total = (2.0 * SR) as usize;
    let mut left = vec![0.0_f32; total];
    let mut right = vec![0.0_f32; total];
    let mut write_pos = 0.0_f64;
    let mut rates_seen = 0_usize;
    let mut i = 0;
    while i < total {
        let n = BLOCK.min(total - i);
        engine.process(
            &source,
            write_pos,
            &params,
            &mut left[i..i + n],
            &mut right[i..i + n],
        );
        for rate in engine.active_rates() {
            assert!(rate < 0.0, "reverse-probability 1.0 spawned rate {rate}");
            rates_seen += 1;
        }
        write_pos += n as f64;
        i += n;
    }
    assert!(rates_seen > 100, "no reversed grains observed");

    let level = rms(&left[(SR as usize / 2)..]);
    assert!(level > 0.02, "reversed granulation near-silent: rms {level}");
    let d2 = max_second_difference(&left);
    assert!(d2 < 0.02, "reversed grains clicked: max d2 {d2}");
}

// --- Real-time safety with all pitch features enabled. ----------------

#[test]
fn render_path_never_allocates_with_pitch_features() {
    let source = sine_buffer(BUF_LEN, 512, 0.8);
    let mut engine = GrainEngine::new(SR, 17);
    let params = GrainParams {
        density_hz: 300.0,
        grain_seconds: 0.2,
        position_seconds: 0.4,
        position_jitter_seconds: 0.1,
        texture: 0.8,
        mode: SchedulerMode::Async,
        pitch_semitones: 7.0,
        detune_spread_cents: 30.0,
        reverse_probability: 0.5,
        anti_alias: true,
        ..GrainParams::default()
    };
    let mut left = vec![0.0_f32; BLOCK];
    let mut right = vec![0.0_f32; BLOCK];
    let mut write_pos = 0.0_f64;

    // Warm-up: saturate the pool (exercises steal + all pitch paths).
    for _ in 0..200 {
        engine.process(&source, write_pos, &params, &mut left, &mut right);
        write_pos += BLOCK as f64;
    }

    let before = thread_allocs();
    for _ in 0..200 {
        engine.process(&source, write_pos, &params, &mut left, &mut right);
        write_pos += BLOCK as f64;
    }
    let allocations = thread_allocs() - before;
    assert_eq!(
        allocations, 0,
        "pitch render path allocated {allocations} times"
    );
}

/// Grain cloud at `semitones` over `source`, with or without the
/// anti-alias read (HQ tier kernel either way).
fn render_transposed(source: &[f32], semitones: f32, anti_alias: bool) -> Vec<f32> {
    let mut engine = GrainEngine::new(SR, 23);
    let params = GrainParams {
        density_hz: 40.0,
        grain_seconds: 0.1,
        position_seconds: 0.5,
        texture: 1.0,
        mode: SchedulerMode::Sync,
        pitch_semitones: semitones,
        anti_alias,
        interp: resonance_dsp::InterpQuality::Bspline6,
        ..GrainParams::default()
    };
    render_seconds(&mut engine, source, &params, 1.0).0
}

/// DSP-09: +24 st (rate 4) reads a 10 kHz source partial at 40 kHz,
/// which folds to 8 kHz. The old anti-alias one-pole ran *after* the
/// resampling read and could only dull the fold by a few dB; the
/// band-limited read must keep it > 60 dB below the input.
#[test]
fn anti_alias_read_removes_folding_at_plus_24_semitones() {
    // 13653 cycles over 65536 samples ≈ 9999.76 Hz; ×4 folds to
    // 48000 − 39999.0 ≈ 8001 Hz.
    let cycles = 13_653;
    let f_src = cycles as f32 * SR / BUF_LEN as f32;
    let f_alias = SR - 4.0 * f_src;
    let source = sine_buffer(BUF_LEN, cycles, 0.8);
    let input_mag = dft_mag(&source[..16_384], f_src);

    let on = render_transposed(&source, 24.0, true);
    let off = render_transposed(&source, 24.0, false);
    let seg = 24_000..24_000 + 16_384;
    let alias_on = dft_mag(&on[seg.clone()], f_alias);
    let alias_off = dft_mag(&off[seg], f_alias);
    assert!(
        alias_off > 1e-2 * input_mag,
        "no fold without AA ({alias_off} vs input {input_mag}) — test signal invalid"
    );
    let rel_db = 20.0 * (alias_on / input_mag).log10();
    assert!(
        rel_db < -60.0,
        "8 kHz alias at {rel_db:.1} dB re input with anti-alias on (off: {:.1} dB)",
        20.0 * (alias_off / input_mag).log10()
    );
}

/// The band-limited read must still pass legitimate content: a 1 kHz
/// source at rate 4 comes out at 4 kHz at essentially the level the
/// plain kernel gives it.
#[test]
fn anti_alias_read_passes_in_band_transposition() {
    // 1365 cycles ≈ 999.8 Hz → ≈ 3999 Hz at rate 4.
    let cycles = 1_365;
    let f_out = 4.0 * cycles as f32 * SR / BUF_LEN as f32;
    let source = sine_buffer(BUF_LEN, cycles, 0.8);
    let seg = 24_000..24_000 + 16_384;
    let on = dft_mag(&render_transposed(&source, 24.0, true)[seg.clone()], f_out);
    let off = dft_mag(&render_transposed(&source, 24.0, false)[seg], f_out);
    assert!(off > 1e-3, "transposed output is silent ({off})");
    let rel_db = 20.0 * (on / off).log10();
    assert!(
        rel_db.abs() < 1.0,
        "in-band 4 kHz level changed by {rel_db:.2} dB with anti-alias on"
    );
}
