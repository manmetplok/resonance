//! Tests for WSOLA-style correlation-aligned grain onsets (ba todo
//! #1080, doc #252 §4-5): alignment measurably reduces the AM/comb
//! roughness of granulating a steady sine, chosen offsets stay inside
//! the configured search window, the off mode is bit-identical to the
//! pre-alignment engine on a fixed seed, and the render path stays
//! allocation-free with alignment active.

use resonance_dsp::{GrainEngine, GrainParams, SchedulerMode, SimpleRng};
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

// --- Signal helpers. --------------------------------------------------

/// Sine with an integer number of cycles over the buffer, so the
/// circular wrap is seamless.
fn sine_buffer(len: usize, cycles: usize, amp: f32) -> Vec<f32> {
    (0..len)
        .map(|i| (TAU * cycles as f64 * i as f64 / len as f64).sin() as f32 * amp)
        .collect()
}

/// Pink-ish noise (Paul Kellet economy filter over xorshift white).
fn pink_buffer(len: usize, seed: u64) -> Vec<f32> {
    let mut rng = SimpleRng::new(seed);
    let (mut b0, mut b1, mut b2) = (0.0_f32, 0.0_f32, 0.0_f32);
    (0..len)
        .map(|_| {
            let white = (rng.next_u32() >> 8) as f32 / (1 << 24) as f32 * 2.0 - 1.0;
            b0 = 0.997 * b0 + white * 0.029_591;
            b1 = 0.985 * b1 + white * 0.032_534;
            b2 = 0.950 * b2 + white * 0.048_056;
            (b0 + b1 + b2 + white * 0.05) * 2.0
        })
        .collect()
}

/// Run the engine for `seconds`, returning the accumulated left/right
/// output. The write head advances by `params.head_advance` per sample.
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

/// Envelope modulation depth `(max − min) / (max + min)` of the sliding
/// short-time RMS (512-sample windows, 128-sample hop) — the AM/comb
/// roughness metric of doc #252 §2/§4.
fn envelope_modulation_depth(x: &[f32]) -> f32 {
    let win = 512;
    let hop = 128;
    let mut min = f64::INFINITY;
    let mut max = f64::NEG_INFINITY;
    let mut start = 0;
    while start + win <= x.len() {
        let e = x[start..start + win]
            .iter()
            .map(|&v| v as f64 * v as f64)
            .sum::<f64>()
            / win as f64;
        let r = e.sqrt();
        min = min.min(r);
        max = max.max(r);
        start += hop;
    }
    ((max - min) / (max + min)) as f32
}

// --- Alignment quality: AM/comb roughness on a steady sine. -----------

#[test]
fn alignment_reduces_am_roughness_on_steady_sine() {
    // ≈ 293 Hz sine (period 163.84 samples). Grains: 80 ms at 25 /s ⇒
    // overlap 2, Hann at exactly 50% hop — the window sum is constant,
    // so any envelope ripple comes from carrier interference between
    // the two overlapping grains. ±10 ms position spray randomizes the
    // splice phase; the ±3 ms alignment window (> half a period) is
    // enough to re-snap every onset to a phase-coherent lag.
    let source = sine_buffer(BUF_LEN, 400, 0.8);
    let base = GrainParams {
        density_hz: 25.0,
        grain_seconds: 0.08,
        position_seconds: 0.5,
        position_jitter_seconds: 0.01,
        texture: 1.0,
        mode: SchedulerMode::Sync,
        align_window_seconds: 0.003,
        ..GrainParams::default()
    };

    let mut engine_off = GrainEngine::new(SR, 42);
    let params_off = GrainParams {
        align: false,
        ..base.clone()
    };
    let (left_off, _) = render_seconds(&mut engine_off, &source, &params_off, 4.0);

    let mut engine_on = GrainEngine::new(SR, 42);
    let params_on = GrainParams {
        align: true,
        ..base.clone()
    };
    let (left_on, _) = render_seconds(&mut engine_on, &source, &params_on, 4.0);

    // Skip the build-up; measure the steady-state envelope.
    let skip = SR as usize / 2;
    let depth_off = envelope_modulation_depth(&left_off[skip..]);
    let depth_on = envelope_modulation_depth(&left_on[skip..]);

    assert!(
        depth_off > 0.25,
        "unaligned splices should interfere audibly: depth {depth_off}"
    );
    assert!(
        depth_on < 0.5 * depth_off,
        "alignment must at least halve the AM depth: on {depth_on} vs off {depth_off}"
    );
    assert!(
        depth_on < 0.15,
        "aligned overlap should sum near-coherently: depth {depth_on}"
    );
    assert!(
        engine_on.aligned_spawns() > 20,
        "the aligner never engaged: {} aligned spawns",
        engine_on.aligned_spawns()
    );
    assert_eq!(
        engine_off.aligned_spawns(),
        0,
        "the off mode must never move an onset"
    );
}

// --- Alignment bound: lags never exceed the configured window. --------

#[test]
fn alignment_lags_stay_within_the_configured_window() {
    // Broadband material with heavy spray, pitch and reverse grains so
    // the correlator sees the full parameter space.
    let source = pink_buffer(BUF_LEN, 11);
    let stress = |window_seconds: f32, seed: u64| -> (f64, u64, f64) {
        let mut engine = GrainEngine::new(SR, seed);
        let params = GrainParams {
            density_hz: 60.0,
            grain_seconds: 0.05,
            position_seconds: 0.4,
            position_jitter_seconds: 0.05,
            texture: 0.8,
            mode: SchedulerMode::Async,
            pitch_semitones: 5.0,
            reverse_probability: 0.3,
            align: true,
            align_window_seconds: window_seconds,
            ..GrainParams::default()
        };
        render_seconds(&mut engine, &source, &params, 3.0);
        (
            engine.max_abs_align_lag_samples(),
            engine.aligned_spawns(),
            (window_seconds * SR) as f64,
        )
    };

    let (max_lag, aligned, bound) = stress(0.002, 21);
    assert!(aligned > 20, "aligner never engaged: {aligned} spawns");
    assert!(
        max_lag <= bound,
        "lag {max_lag} exceeded the ±{bound}-sample window"
    );
    assert!(max_lag > 0.0, "aligner engaged but applied no lag");

    // A window beyond the 10 ms cap must clamp to the cap.
    let (max_lag, aligned, _) = stress(0.05, 22);
    assert!(aligned > 20, "aligner never engaged: {aligned} spawns");
    assert!(
        max_lag <= (0.01 * SR) as f64,
        "lag {max_lag} exceeded the clamped 10 ms cap"
    );
}

// --- Off mode: bit-identical to the pre-alignment engine. -------------

fn fnv1a64(words: impl Iterator<Item = u32>) -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325_u64;
    for w in words {
        for b in w.to_le_bytes() {
            h ^= b as u64;
            h = h.wrapping_mul(0x1000_0000_01b3);
        }
    }
    h
}

#[test]
fn off_mode_is_bit_identical_to_pre_alignment_engine() {
    // FNV-1a over the raw f32 bit patterns of a 2 s stereo render that
    // exercises every engine feature, captured from the engine at
    // f9806af1 (before onset alignment existed) with the identical
    // scenario. Alignment off must reproduce it bit for bit.
    const REFERENCE_HASH: u64 = 0x768c_fc17_6897_c9d3;

    let mut rng = SimpleRng::new(77);
    let source: Vec<f32> = (0..BUF_LEN)
        .map(|i| {
            let white = (rng.next_u32() >> 8) as f32 / (1 << 24) as f32 * 2.0 - 1.0;
            let sine = (TAU * 200.0 * i as f64 / BUF_LEN as f64).sin() as f32;
            0.6 * sine + 0.2 * white
        })
        .collect();
    let mut engine = GrainEngine::new(SR, 12345);
    let params = GrainParams {
        density_hz: 60.0,
        grain_seconds: 0.05,
        position_seconds: 0.3,
        position_jitter_seconds: 0.08,
        size_jitter: 0.4,
        level_jitter: 0.3,
        pan_spread: 0.8,
        texture: 0.6,
        mode: SchedulerMode::Async,
        pitch_semitones: 3.0,
        detune_spread_cents: 15.0,
        reverse_probability: 0.25,
        anti_alias: true,
        ..GrainParams::default()
    };
    assert!(!params.align, "alignment must default to off");
    let (left, right) = render_seconds(&mut engine, &source, &params, 2.0);
    let hash = fnv1a64(left.iter().chain(right.iter()).map(|v| v.to_bits()));
    assert_eq!(
        hash, REFERENCE_HASH,
        "align-off output diverged from the pre-alignment engine"
    );
}

// --- Real-time safety with alignment active. --------------------------

#[test]
fn render_path_never_allocates_with_alignment_active() {
    let source = pink_buffer(BUF_LEN, 5);
    let mut engine = GrainEngine::new(SR, 5);
    // Overload parameters so spawning, aligning and voice stealing are
    // all exercised while measuring.
    let params = GrainParams {
        density_hz: 400.0,
        grain_seconds: 0.4,
        position_seconds: 0.25,
        position_jitter_seconds: 0.1,
        size_jitter: 0.5,
        level_jitter: 0.5,
        pan_spread: 1.0,
        texture: 0.7,
        mode: SchedulerMode::Async,
        pitch_semitones: 4.0,
        reverse_probability: 0.2,
        anti_alias: true,
        align: true,
        align_window_seconds: 0.005,
        ..GrainParams::default()
    };
    let mut left = vec![0.0_f32; BLOCK];
    let mut right = vec![0.0_f32; BLOCK];
    let mut write_pos = 0.0_f64;

    // Warm-up: saturate the pool before measuring.
    for _ in 0..100 {
        engine.process(&source, write_pos, &params, &mut left, &mut right);
        write_pos += BLOCK as f64;
    }
    assert!(engine.aligned_spawns() > 0, "aligner never engaged");

    let before = thread_allocs();
    for _ in 0..200 {
        engine.process(&source, write_pos, &params, &mut left, &mut right);
        write_pos += BLOCK as f64;
    }
    let allocations = thread_allocs() - before;
    assert_eq!(
        allocations, 0,
        "aligned grain engine allocated {allocations} times on the render path"
    );
}
