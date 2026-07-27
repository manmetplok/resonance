//! Tests for the real-time grain engine core: pool bounds, voice
//! stealing, overlap compensation, envelopes, scheduler timing and the
//! no-allocation guarantee (ba todo #1071, doc #252 §2/§5/§8).

use resonance_dsp::{GrainEngine, GrainParams, SchedulerMode, SimpleRng, MAX_GRAINS};
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

fn rms(x: &[f32]) -> f32 {
    (x.iter().map(|&v| v as f64 * v as f64).sum::<f64>() / x.len() as f64).sqrt() as f32
}

/// Largest second difference — a click/discontinuity detector that is
/// insensitive to the (smooth) granulated signal itself.
fn max_second_difference(x: &[f32]) -> f32 {
    x.windows(3)
        .map(|w| (w[2] - 2.0 * w[1] + w[0]).abs())
        .fold(0.0_f32, f32::max)
}

/// Contiguous runs of nonzero samples, as `(start, len)`, excluding runs
/// touching either end of the slice.
fn nonzero_segments(x: &[f32]) -> Vec<(usize, usize)> {
    let mut segments = Vec::new();
    let mut start: Option<usize> = None;
    for (i, &v) in x.iter().enumerate() {
        match (start, v != 0.0) {
            (None, true) => start = Some(i),
            (Some(s), false) => {
                segments.push((s, i - s));
                start = None;
            }
            _ => {}
        }
    }
    // Drop segments cut off by the slice boundaries.
    segments.retain(|&(s, _)| s > 0);
    segments
}

// --- Pool bounds & voice stealing. ------------------------------------

#[test]
fn pool_saturates_without_exceeding_max_and_steals_are_click_free() {
    // 400 grains/s of 0.4 s grains wants ~160 concurrent grains — far
    // beyond the pool — so the engine must steal constantly.
    let source = sine_buffer(BUF_LEN, 68, 0.9); // ≈ 49.8 Hz, seamless wrap
    let mut engine = GrainEngine::new(SR, 7);
    let params = GrainParams {
        density_hz: 400.0,
        grain_seconds: 0.4,
        position_seconds: 0.25,
        position_jitter_seconds: 0.1,
        texture: 1.0,
        mode: SchedulerMode::Async,
        ..GrainParams::default()
    };

    let total = (1.5 * SR) as usize;
    let mut left = vec![0.0_f32; total];
    let mut right = vec![0.0_f32; total];
    let mut write_pos = 0.0_f64;
    let mut saturated = false;
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
        let active = engine.active_grains();
        assert!(
            active <= MAX_GRAINS,
            "active grain count {active} exceeded the pool"
        );
        if active == MAX_GRAINS {
            saturated = true;
        }
        write_pos += n as f64;
        i += n;
    }
    assert!(saturated, "overload stress never filled the pool");
    assert!(
        engine.grains_spawned() < 400 * 2,
        "steals must drop the incoming onset, not grow the pool"
    );

    // The engine must keep sounding under permanent voice stealing...
    let level = rms(&left[(SR as usize / 2)..]);
    assert!(level > 0.05, "stressed output is near-silent: rms {level}");
    // ...and stealing must ramp grains out, never hard-kill them. A
    // hard kill would show up as a second difference on the order of a
    // full grain amplitude (~0.05); legitimate output stays ~20x lower.
    let d2 = max_second_difference(&left);
    assert!(d2 < 0.02, "discontinuity under voice stealing: max d2 {d2}");
}

// --- Equal-power overlap compensation. --------------------------------

#[test]
fn rms_stays_flat_across_density_sweep() {
    let source = pink_buffer(BUF_LEN, 42);
    let densities = [10.0_f32, 25.0, 50.0, 100.0, 200.0];
    let mut levels_db = Vec::new();
    for (k, &density_hz) in densities.iter().enumerate() {
        let mut engine = GrainEngine::new(SR, 1000 + k as u64);
        let params = GrainParams {
            density_hz,
            grain_seconds: 0.08,
            position_seconds: 0.6,
            position_jitter_seconds: 0.5,
            texture: 1.0,
            mode: SchedulerMode::Async,
            ..GrainParams::default()
        };
        let (left, _) = render_seconds(&mut engine, &source, &params, 8.5);
        let level = rms(&left[(SR as usize / 2)..]);
        levels_db.push(20.0 * level.log10());
    }
    let max = levels_db.iter().cloned().fold(f32::NEG_INFINITY, f32::max);
    let min = levels_db.iter().cloned().fold(f32::INFINITY, f32::min);
    assert!(
        max - min <= 1.5,
        "overlap compensation failed: levels {levels_db:?} dB span {:.2} dB",
        max - min
    );
}

// --- Grain envelopes. -------------------------------------------------

#[test]
fn every_grain_envelope_starts_and_ends_near_zero() {
    // DC source so the output *is* the envelope; texture 0 exercises
    // the enforced minimum raised-cosine edge on a boxcar grain.
    let source = vec![0.8_f32; BUF_LEN];
    let mut engine = GrainEngine::new(SR, 3);
    let params = GrainParams {
        density_hz: 4.0,
        grain_seconds: 0.05,
        position_seconds: 0.3,
        texture: 0.0,
        mode: SchedulerMode::Sync,
        ..GrainParams::default()
    };
    let (left, _) = render_seconds(&mut engine, &source, &params, 2.0);

    let segments = nonzero_segments(&left);
    assert!(
        segments.len() >= 6,
        "expected >= 6 complete grains, got {}",
        segments.len()
    );
    // Expected plateau: DC 0.8 × overlap gain 1/sqrt(0.2) × centre pan.
    let expected_peak = 0.8 * (1.0_f32 / 0.2_f32.sqrt()) * std::f32::consts::FRAC_1_SQRT_2;
    for &(start, len) in &segments {
        let seg = &left[start..start + len];
        let peak = seg.iter().fold(0.0_f32, |m, &v| m.max(v.abs()));
        assert!(
            (peak - expected_peak).abs() < 0.02,
            "grain at {start}: plateau {peak} vs expected {expected_peak}"
        );
        assert!(
            seg[0].abs() < 0.02 * peak && seg[len - 1].abs() < 0.02 * peak,
            "grain at {start} does not start/end near zero: {} / {}",
            seg[0],
            seg[len - 1]
        );
        let dur = (0.05 * SR) as usize;
        assert!(
            len.abs_diff(dur) <= 4,
            "grain at {start}: length {len} vs nominal {dur}"
        );
    }
}

// --- Scheduler timing. ------------------------------------------------

/// Onset indices: first nonzero sample after a zero sample.
fn onsets(x: &[f32]) -> Vec<usize> {
    (1..x.len())
        .filter(|&i| x[i] != 0.0 && x[i - 1] == 0.0)
        .collect()
}

#[test]
fn sync_scheduler_inter_onset_time_is_sample_exact() {
    let source = vec![1.0_f32; BUF_LEN];
    let mut engine = GrainEngine::new(SR, 1);
    // IOT = 48000 / 100 = 480 samples; 192-sample grains never overlap.
    let params = GrainParams {
        density_hz: 100.0,
        grain_seconds: 0.004,
        position_seconds: 0.2,
        texture: 1.0,
        mode: SchedulerMode::Sync,
        ..GrainParams::default()
    };
    let (left, _) = render_seconds(&mut engine, &source, &params, 1.0);
    let starts = onsets(&left);
    assert!(starts.len() > 90, "too few onsets: {}", starts.len());
    for pair in starts.windows(2) {
        assert_eq!(
            pair[1] - pair[0],
            480,
            "sync IOT must be exactly 480 samples"
        );
    }
}

#[test]
fn async_scheduler_iot_mean_matches_density_and_is_randomized() {
    let source = vec![1.0_f32; BUF_LEN];
    let mut engine = GrainEngine::new(SR, 9);
    // Base IOT = 960 samples; the async draw spans [0.5, 1.5)·base, so
    // grains of 192 samples still never overlap.
    let params = GrainParams {
        density_hz: 50.0,
        grain_seconds: 0.004,
        position_seconds: 0.2,
        texture: 1.0,
        mode: SchedulerMode::Async,
        ..GrainParams::default()
    };
    let (left, _) = render_seconds(&mut engine, &source, &params, 30.0);
    let starts = onsets(&left);
    let gaps: Vec<f64> = starts.windows(2).map(|p| (p[1] - p[0]) as f64).collect();
    assert!(gaps.len() > 1000, "too few onsets: {}", gaps.len());

    let mean = gaps.iter().sum::<f64>() / gaps.len() as f64;
    assert!(
        (mean - 960.0).abs() < 0.02 * 960.0,
        "async IOT mean {mean} not ~960 samples"
    );
    let var = gaps.iter().map(|g| (g - mean) * (g - mean)).sum::<f64>() / gaps.len() as f64;
    let std = var.sqrt();
    // Uniform [0.5, 1.5)·base has std = base/sqrt(12) ≈ 277 samples.
    assert!(std > 150.0, "async IOT barely randomized: std {std}");
    let min = gaps.iter().cloned().fold(f64::INFINITY, f64::min);
    let max = gaps.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
    assert!(
        min < 700.0 && max > 1200.0,
        "async IOT range [{min}, {max}] too narrow"
    );
    // The spawn counter agrees with the observed onsets (last grain may
    // still be sounding).
    let spawned = engine.grains_spawned() as usize;
    assert!(
        spawned.abs_diff(starts.len()) <= 1,
        "spawn counter {spawned} vs {} observed onsets",
        starts.len()
    );
}

// --- Real-time safety. ------------------------------------------------

#[test]
fn render_path_never_allocates() {
    let source = pink_buffer(BUF_LEN, 5);
    let mut engine = GrainEngine::new(SR, 5);
    // Overload parameters so the voice-steal path is exercised too.
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
    // The pool sits at/near saturation (a steal-release may have just
    // freed a slot at the block boundary).
    assert!(engine.active_grains() >= MAX_GRAINS - 2);

    let before = thread_allocs();
    for _ in 0..200 {
        engine.process(&source, write_pos, &params, &mut left, &mut right);
        write_pos += BLOCK as f64;
    }
    let allocations = thread_allocs() - before;
    assert_eq!(
        allocations, 0,
        "grain engine allocated {allocations} times on the render path"
    );
}
