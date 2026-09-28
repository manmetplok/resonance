//! Tests for the de-harsh resonance suppressor
//! (`docs/design/deharsh-resonance-suppressor.md` §6, T1–T12).
//!
//! Own test binary with a counting `#[global_allocator]` for the
//! no-allocation guard (per thread, so parallel tests don't pollute it).

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::time::Instant;

use resonance_dsp::deharsh::{StftGeometry, DETECTOR_TAU_MS};
use resonance_dsp::{
    Biquad, OnePole, ResonanceSuppressor, SimpleRng, SuppressorConfig, SuppressorMode,
};
use rustfft::{num_complex::Complex, FftPlanner};

const SR: f32 = 48_000.0;
const BLOCK: usize = 128;

// --- Per-thread allocation counter. -----------------------------------

thread_local! {
    static ALLOC_COUNT: Cell<usize> = const { Cell::new(0) };
}

struct CountingAlloc;

// SAFETY: delegates entirely to `System`; the const-initialised
// thread-local counter never allocates itself.
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

// --- Signals. ---------------------------------------------------------

fn white(seed: u64, n: usize) -> Vec<f32> {
    let mut rng = SimpleRng::new(seed);
    (0..n)
        .map(|_| (rng.next_u32() as f64 / u32::MAX as f64 * 2.0 - 1.0) as f32)
        .collect()
}

/// Paul Kellet's refined pink filter over seeded white noise.
fn pink(seed: u64, n: usize) -> Vec<f32> {
    let w = white(seed, n);
    let (mut b0, mut b1, mut b2, mut b3, mut b4, mut b5, mut b6) =
        (0.0f64, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0);
    w.iter()
        .map(|&x| {
            let x = x as f64;
            b0 = 0.99886 * b0 + x * 0.0555179;
            b1 = 0.99332 * b1 + x * 0.0750759;
            b2 = 0.96900 * b2 + x * 0.1538520;
            b3 = 0.86650 * b3 + x * 0.3104856;
            b4 = 0.55000 * b4 + x * 0.5329522;
            b5 = -0.7616 * b5 - x * 0.0168980;
            let y = b0 + b1 + b2 + b3 + b4 + b5 + b6 + x * 0.5362;
            b6 = x * 0.115926;
            y as f32
        })
        .collect()
}

fn scale_to_rms_db(x: &mut [f32], rms_db: f32) {
    let rms = (x.iter().map(|&v| (v as f64).powi(2)).sum::<f64>() / x.len() as f64).sqrt();
    let g = 10f64.powf(rms_db as f64 / 20.0) / rms.max(1e-30);
    for v in x.iter_mut() {
        *v = (*v as f64 * g) as f32;
    }
}

fn bell(x: &[f32], freq: f32, q: f32, gain_db: f32) -> Vec<f32> {
    let mut bq = Biquad::default();
    bq.set_bell(SR, freq, q, gain_db);
    x.iter().map(|&v| bq.process(v)).collect()
}

fn sine(freq: f32, amp: f32, n: usize) -> Vec<f32> {
    (0..n)
        .map(|i| amp * (std::f64::consts::TAU * freq as f64 * i as f64 / SR as f64).sin() as f32)
        .collect()
}

/// A pinned, enabled config: the exit-criterion params (T1/T2 run the
/// resonance and the broadband signals through the same config).
///
/// Sharpness 24 / selectivity 5 rather than the defaults (8 / 6): the
/// detection kernel averages power over the cut width, so a Q 10
/// resonance reads ≈ 9 dB above its reference at Q 8 and ≈ 13 dB at
/// Q 24. Measured on this signal: Q 8/T 6 cuts 3.5 dB, Q 24/T 5 cuts
/// 7.8 dB with broadband bands within ±0.18 dB.
fn cfg_t1() -> SuppressorConfig {
    SuppressorConfig {
        enabled: true,
        depth_db: 12.0,
        selectivity_db: 5.0,
        sharpness_q: 24.0,
        attack_ms: 10.0,
        release_ms: 100.0,
        low_hz: 1000.0,
        high_hz: 8000.0,
        mode: SuppressorMode::Stereo,
        mix: 1.0,
        delta: false,
    }
}

fn run_with(
    sup: &mut ResonanceSuppressor,
    l: &[f32],
    r: &[f32],
    mut cfg_at: impl FnMut(usize) -> SuppressorConfig,
) -> (Vec<f32>, Vec<f32>) {
    let (mut ol, mut or) = (l.to_vec(), r.to_vec());
    let mut start = 0;
    let mut block = 0;
    while start < ol.len() {
        let end = (start + BLOCK).min(ol.len());
        let cfg = cfg_at(block);
        sup.process_stereo(&mut ol[start..end], &mut or[start..end], &cfg);
        start = end;
        block += 1;
    }
    (ol, or)
}

fn run(cfg: &SuppressorConfig, l: &[f32], r: &[f32]) -> (Vec<f32>, Vec<f32>) {
    let mut sup = ResonanceSuppressor::new(SR);
    run_with(&mut sup, l, r, |_| *cfg)
}

fn latency() -> usize {
    StftGeometry::for_sample_rate(SR).latency()
}

// --- Measurement. -----------------------------------------------------

/// Welch PSD (Hann 8192, hop 4096), bins `0..=4096`.
fn psd(x: &[f32]) -> Vec<f64> {
    let n = 8192;
    let fft = FftPlanner::new().plan_fft_forward(n);
    let w: Vec<f64> = (0..n)
        .map(|i| 0.5 - 0.5 * (std::f64::consts::TAU * i as f64 / n as f64).cos())
        .collect();
    let mut acc = vec![0.0f64; n / 2 + 1];
    let mut frames = 0;
    let mut buf = vec![Complex::new(0.0f64, 0.0); n];
    let mut start = 0;
    while start + n <= x.len() {
        for i in 0..n {
            buf[i] = Complex::new(x[start + i] as f64 * w[i], 0.0);
        }
        fft.process(&mut buf);
        for (k, a) in acc.iter_mut().enumerate() {
            *a += buf[k].norm_sqr();
        }
        frames += 1;
        start += n / 2;
    }
    assert!(frames > 0, "signal too short for the PSD");
    acc.iter().map(|a| a / frames as f64).collect()
}

fn band_db(p: &[f64], f1: f32, f2: f32) -> f64 {
    let bin_hz = SR as f64 / 8192.0;
    let lo = (f1 as f64 / bin_hz).ceil() as usize;
    let hi = ((f2 as f64 / bin_hz).floor() as usize).min(p.len() - 1);
    let e: f64 = p[lo..=hi].iter().sum();
    10.0 * e.max(1e-300).log10()
}

/// Input and output, aligned by the latency, with `skip` seconds of
/// settling dropped.
fn aligned<'a>(input: &'a [f32], output: &'a [f32], skip_s: f32) -> (&'a [f32], &'a [f32]) {
    let lat = latency();
    let skip = (skip_s * SR) as usize;
    let len = input.len() - lat;
    (&input[skip..len], &output[skip + lat..])
}

fn assert_not_silent(x: &[f32], what: &str) {
    let peak = x.iter().fold(0.0f32, |m, v| m.max(v.abs()));
    assert!(peak > 1e-3, "{what}: output is silent (peak {peak})");
}

/// Level change (out − in, dB) in the 1/3-oct bands centred 1..8 kHz
/// and over the whole 1–8 kHz span.
fn third_octave_changes(input: &[f32], output: &[f32]) -> (Vec<(f32, f64)>, f64) {
    let (pi, po) = (psd(input), psd(output));
    let mut out = Vec::new();
    for i in 0..=9 {
        let fc = 1000.0 * 2f32.powf(i as f32 / 3.0);
        let (f1, f2) = (fc * 2f32.powf(-1.0 / 6.0), fc * 2f32.powf(1.0 / 6.0));
        out.push((fc, band_db(&po, f1, f2) - band_db(&pi, f1, f2)));
    }
    let total = band_db(&po, 1000.0, 8000.0) - band_db(&pi, 1000.0, 8000.0);
    (out, total)
}

const DUR: usize = 6 * 48_000;

// --- T1: a synthetic resonance is cut. -------------------------------

/// T1 on `cfg`: returns the 1/12-oct cut at 3.2 kHz after asserting
/// that the neighbouring bands barely move.
fn resonance_cut(cfg: &SuppressorConfig) -> f64 {
    let mut x = bell(&pink(1, DUR), 3200.0, 10.0, 15.0);
    scale_to_rms_db(&mut x, -18.0);
    let (ol, _) = run(cfg, &x, &x);
    assert_not_silent(&ol, "t1");
    let (i, o) = aligned(&x, &ol, 0.5);
    let (pi, po) = (psd(i), psd(o));
    let (f1, f2) = (3200.0 * 2f32.powf(-1.0 / 24.0), 3200.0 * 2f32.powf(1.0 / 24.0));
    let cut = band_db(&pi, f1, f2) - band_db(&po, f1, f2);
    eprintln!("t1: 1/12-oct cut at 3.2 kHz = {cut:.2} dB");
    // Local, not a broad dip: 1/3-oct bands ±1 oct away barely move.
    for fc in [1600.0f32, 6400.0] {
        let (a, b) = (fc * 2f32.powf(-1.0 / 6.0), fc * 2f32.powf(1.0 / 6.0));
        let d = band_db(&po, a, b) - band_db(&pi, a, b);
        assert!(d.abs() < 1.0, "band at {fc} Hz moved {d:.2} dB");
    }
    cut
}

#[test]
fn t1_resonance_at_3k2_is_cut_at_least_6_db() {
    let cut = resonance_cut(&cfg_t1());
    assert!(cut >= 6.0, "resonance cut only {cut:.2} dB");
}

// --- T2: broadband signals are untouched. -----------------------------

fn assert_broadband_untouched_with(cfg: &SuppressorConfig, mut x: Vec<f32>, what: &str) {
    scale_to_rms_db(&mut x, -18.0);
    let (ol, _) = run(cfg, &x, &x);
    assert_not_silent(&ol, what);
    let (i, o) = aligned(&x, &ol, 0.5);
    let (bands, total) = third_octave_changes(i, o);
    for (fc, d) in &bands {
        assert!(d.abs() < 0.5, "{what}: 1/3-oct band {fc:.0} Hz moved {d:.3} dB");
    }
    eprintln!("{what}: total change {total:.3} dB, bands {bands:.2?}");
    assert!(total.abs() < 0.5, "{what}: 1–8 kHz total moved {total:.3} dB");
}

fn assert_broadband_untouched(x: Vec<f32>, what: &str) {
    assert_broadband_untouched_with(&cfg_t1(), x, what);
}

/// A curved spectrum: pink through a first-order lowpass at 2 kHz, so the
/// slope bends from −3 to −9 dB/oct across the band.
fn curved(seed: u64) -> Vec<f32> {
    let mut lp = OnePole::new();
    lp.set_cutoff(2000.0, SR);
    pink(seed, DUR).iter().map(|&v| lp.process(v)).collect()
}

#[test]
fn t2_pink_noise_changes_less_than_half_a_db() {
    assert_broadband_untouched(pink(2, DUR), "pink");
}

#[test]
fn t2_white_noise_changes_less_than_half_a_db() {
    assert_broadband_untouched(white(3, DUR), "white");
}

#[test]
fn t2_curved_spectrum_changes_less_than_half_a_db() {
    assert_broadband_untouched(curved(4), "curved");
}

// --- The shipped defaults meet the exit criterion. --------------------

/// The defaults are the exit-criterion configuration: sharpness and
/// selectivity are the pinned T1/T2 values, so they can never drift
/// from it. Default depth (6 dB) caps the cut near 6 dB, so T1 is
/// checked at depth 12, the only field the test changes. The spec's
/// criterion is about detection, not about the depth cap.
#[test]
fn defaults_meet_the_exit_criterion() {
    let d = SuppressorConfig::default();
    assert_eq!((d.sharpness_q, d.selectivity_db), (24.0, 5.0));
    let on = SuppressorConfig { enabled: true, ..d };
    let cut = resonance_cut(&SuppressorConfig { depth_db: 12.0, ..on });
    assert!(cut >= 6.0, "defaults cut the resonance only {cut:.2} dB");
    // T2 at the full defaults, depth included.
    assert_broadband_untouched_with(&on, pink(2, DUR), "defaults/pink");
    assert_broadband_untouched_with(&on, white(3, DUR), "defaults/white");
    assert_broadband_untouched_with(&on, curved(4), "defaults/curved");
    // At the default depth the resonance is still cut by (nearly) the
    // full cap.
    let capped = resonance_cut(&on);
    eprintln!("defaults: cut {cut:.2} dB at depth 12, {capped:.2} dB at depth 6");
    assert!(capped >= 5.0, "defaults at depth 6 cut only {capped:.2} dB");
}

// --- T3 (DSP level): latency is constant. -----------------------------

#[test]
fn t3_latency_is_one_frame_at_every_rate_and_config() {
    for (sr, frame) in [(44_100.0, 2048), (48_000.0, 2048), (96_000.0, 4096), (192_000.0, 8192)] {
        let mut sup = ResonanceSuppressor::new(sr);
        assert_eq!(sup.latency(), frame, "latency at {sr}");
        let configs = [
            SuppressorConfig::default(),
            cfg_t1(),
            SuppressorConfig { depth_db: 0.0, ..cfg_t1() },
            SuppressorConfig { mix: 0.0, ..cfg_t1() },
            SuppressorConfig { delta: true, ..cfg_t1() },
            SuppressorConfig { mode: SuppressorMode::MidSide, ..cfg_t1() },
        ];
        let mut buf = vec![0.1f32; 256];
        let mut buf_r = buf.clone();
        for c in &configs {
            sup.process_stereo(&mut buf, &mut buf_r, c);
            assert_eq!(sup.latency(), frame, "latency moved under {c:?}");
        }
    }
}

#[test]
fn t3_impulse_comes_out_after_exactly_the_latency_when_on() {
    let lat = latency();
    let n = lat * 4;
    let mut x = vec![0.0f32; n];
    x[1000] = 1.0;
    let cfg = SuppressorConfig { depth_db: 0.0, ..cfg_t1() };
    let (ol, or) = run(&cfg, &x, &x);
    let peak = ol
        .iter()
        .enumerate()
        .max_by(|a, b| a.1.abs().total_cmp(&b.1.abs()))
        .unwrap()
        .0;
    assert_eq!(peak, 1000 + lat);
    assert!((ol[1000 + lat] - 1.0).abs() < 1e-5 && (or[1000 + lat] - 1.0).abs() < 1e-5);
}

// --- T4: delta + output reconstructs the delayed dry signal. ----------

#[test]
fn t4_delta_plus_output_is_the_delayed_input() {
    let lat = latency();
    let mut x = bell(&pink(5, 3 * 48_000), 3200.0, 10.0, 15.0);
    scale_to_rms_db(&mut x, -18.0);
    let xr: Vec<f32> = x.iter().rev().copied().collect();
    for mix in [0.3f32, 1.0] {
        let cfg = SuppressorConfig { mix, ..cfg_t1() };
        let (wl, wr) = run(&cfg, &x, &xr);
        let (dl, dr) = run(&SuppressorConfig { delta: true, ..cfg }, &x, &xr);
        assert_not_silent(&dl, "delta");
        let mut err = 0.0f32;
        for i in lat..x.len() {
            err = err.max((wl[i] + dl[i] - x[i - lat]).abs());
            err = err.max((wr[i] + dr[i] - xr[i - lat]).abs());
        }
        assert!(err < 1e-6, "mix {mix}: delta + out off by {err}");
    }
}

// --- T5: off is bit-transparent. --------------------------------------

#[test]
fn t5_off_is_the_input_delayed_bit_exactly() {
    let lat = latency();
    let x = pink(6, 48_000);
    let xr = white(7, 48_000);
    // Every other field set to something that would cut hard if on.
    let cfg = SuppressorConfig { enabled: false, selectivity_db: 0.0, depth_db: 24.0, ..cfg_t1() };
    let (ol, or) = run(&cfg, &x, &xr);
    assert!(ol[..lat].iter().chain(&or[..lat]).all(|&v| v == 0.0));
    for i in lat..x.len() {
        assert_eq!(ol[i].to_bits(), x[i - lat].to_bits(), "L differs at {i}");
        assert_eq!(or[i].to_bits(), xr[i - lat].to_bits(), "R differs at {i}");
    }
}

#[test]
fn t5_back_to_off_after_on_is_bit_exact_again() {
    let lat = latency();
    let mut x = bell(&pink(8, 2 * 48_000), 3200.0, 10.0, 15.0);
    scale_to_rms_db(&mut x, -18.0);
    let mut sup = ResonanceSuppressor::new(SR);
    let switch_block = 48_000 / BLOCK;
    let (ol, _) = run_with(&mut sup, &x, &x, |b| SuppressorConfig {
        enabled: b < switch_block,
        ..cfg_t1()
    });
    // After the 10 ms fade the output is the delay tap again.
    let settled = switch_block * BLOCK + 480 + 1;
    for i in settled..x.len() {
        assert_eq!(ol[i].to_bits(), x[i - lat].to_bits(), "differs at {i}");
    }
}

// --- T6: WOLA identity. -----------------------------------------------

#[test]
fn t6_unity_gains_reconstruct_the_delayed_input() {
    let lat = latency();
    let x = white(9, 48_000);
    let xr = pink(10, 48_000);
    for mode in [SuppressorMode::Stereo, SuppressorMode::MidSide] {
        let cfg = SuppressorConfig { depth_db: 0.0, mode, ..cfg_t1() };
        let (ol, or) = run(&cfg, &x, &xr);
        let mut err = 0.0f32;
        for i in 2 * lat..x.len() {
            err = err.max((ol[i] - x[i - lat]).abs()).max((or[i] - xr[i - lat]).abs());
        }
        assert!(err < 1e-5, "{mode:?}: reconstruction error {err}");
    }
}

// --- T7: level invariance, silence, floor. ----------------------------

fn resonance_cut_db(level_db: f32) -> f64 {
    let mut x = bell(&pink(11, 4 * 48_000), 3200.0, 10.0, 15.0);
    scale_to_rms_db(&mut x, level_db);
    let (ol, _) = run(&cfg_t1(), &x, &x);
    let (i, o) = aligned(&x, &ol, 0.5);
    let (pi, po) = (psd(i), psd(o));
    let (f1, f2) = (3200.0 * 2f32.powf(-1.0 / 24.0), 3200.0 * 2f32.powf(1.0 / 24.0));
    band_db(&pi, f1, f2) - band_db(&po, f1, f2)
}

#[test]
fn t7_cut_does_not_depend_on_level() {
    let a = resonance_cut_db(-24.0);
    let b = resonance_cut_db(-48.0);
    assert!(a > 3.0, "no cut at −24 dBFS ({a:.2})");
    assert!((a - b).abs() < 0.2, "cut {a:.3} dB at −24 vs {b:.3} dB at −48");
}

#[test]
fn t7_silence_in_is_silence_out_and_the_floor_holds() {
    let zeros = vec![0.0f32; 20_000];
    let (ol, or) = run(&cfg_t1(), &zeros, &zeros);
    assert!(ol.iter().chain(&or).all(|&v| v == 0.0));

    let mut x = bell(&pink(12, 48_000), 3200.0, 10.0, 15.0);
    scale_to_rms_db(&mut x, -130.0);
    let mut sup = ResonanceSuppressor::new(SR);
    let mut worst = 0.0f32;
    let mut start = 0;
    let (mut l, mut r) = (x.clone(), x.clone());
    while start < x.len() {
        let end = (start + BLOCK).min(x.len());
        sup.process_stereo(&mut l[start..end], &mut r[start..end], &cfg_t1());
        worst = worst.max(sup.max_cut_db());
        start = end;
    }
    assert_eq!(worst, 0.0, "cut below the floor");
}

// --- T8: no zipper, out-of-band untouched, clean toggles. ------------

/// Seeded random walk over the cut-shaping params, one step per block.
struct Walk {
    rng: SimpleRng,
    depth: f32,
    sel: f32,
    q: f32,
    low: f32,
}

impl Walk {
    fn new(seed: u64) -> Self {
        Self { rng: SimpleRng::new(seed), depth: 12.0, sel: 6.0, q: 8.0, low: 1000.0 }
    }

    fn uni(&mut self) -> f32 {
        self.rng.next_u32() as f32 / u32::MAX as f32 * 2.0 - 1.0
    }

    fn next(&mut self) -> SuppressorConfig {
        self.depth = (self.depth + 0.5 * self.uni()).clamp(0.0, 24.0);
        self.sel = (self.sel + 0.3 * self.uni()).clamp(0.0, 18.0);
        self.q = (self.q + 0.3 * self.uni()).clamp(3.0, 24.0);
        self.low = (self.low + 20.0 * self.uni()).clamp(1000.0, 2000.0);
        SuppressorConfig {
            depth_db: self.depth,
            selectivity_db: self.sel,
            sharpness_q: self.q,
            low_hz: self.low,
            ..cfg_t1()
        }
    }
}

#[test]
fn t8_automated_params_leave_no_zipper_sidebands() {
    let x = sine(3200.0, 0.3, 4 * 48_000);
    let mut sup = ResonanceSuppressor::new(SR);
    let mut walk = Walk::new(13);
    let (ol, _) = run_with(&mut sup, &x, &x, |_| walk.next());
    assert_not_silent(&ol, "t8");
    let (_, o) = aligned(&x, &ol, 0.5);
    let p = psd(o);
    let tone = band_db(&p, 3100.0, 3300.0);
    let rest = 10.0
        * (10f64.powf(band_db(&p, 200.0, 3100.0) / 10.0)
            + 10f64.powf(band_db(&p, 3300.0, 20_000.0) / 10.0))
        .log10();
    eprintln!("t8: residue {:.1} dBc", rest - tone);
    assert!(rest - tone < -60.0, "zipper residue {:.1} dBc", rest - tone);
}

#[test]
fn t8_out_of_band_signal_is_untouched_under_automation() {
    let lat = latency();
    let x = sine(500.0, 0.5, 2 * 48_000);
    let mut sup = ResonanceSuppressor::new(SR);
    let mut walk = Walk::new(14);
    let (ol, _) = run_with(&mut sup, &x, &x, |_| walk.next());
    let mut err = 0.0f32;
    for i in 2 * lat..x.len() {
        err = err.max((ol[i] - x[i - lat]).abs());
    }
    assert!(err < 1e-5, "500 Hz sine changed by {err}");
    assert_eq!(sup.max_cut_db(), 0.0);
}

#[test]
fn t8_toggling_on_and_off_crossfades() {
    let mut x = bell(&pink(15, 2 * 48_000), 3200.0, 10.0, 15.0);
    scale_to_rms_db(&mut x, -12.0);
    let off = run(&SuppressorConfig { enabled: false, ..cfg_t1() }, &x, &x).0;
    let on = run(&cfg_t1(), &x, &x).0;
    let per_toggle = (0.050 * SR) as usize / BLOCK;
    let mut sup = ResonanceSuppressor::new(SR);
    let tog = run_with(&mut sup, &x, &x, |b| SuppressorConfig {
        enabled: (b / per_toggle).is_multiple_of(2),
        ..cfg_t1()
    })
    .0;
    // The toggled output is always dry + f·(wet − dry) with f moving by
    // at most one ramp step per sample: the STFT runs while off, so the
    // wet path is the always-on render.
    let step = 1.0 / (0.010 * SR);
    for i in 1..x.len() {
        let (d0, d1, w0, w1) = (off[i - 1], off[i], on[i - 1], on[i]);
        let bound = (d1 - d0).abs().max((w1 - w0).abs()) + step * (w0 - d0).abs() * 1.01 + 1e-6;
        let jump = (tog[i] - tog[i - 1]).abs();
        assert!(jump <= bound, "step {jump} > {bound} at {i}");
    }
}

// --- T9: stereo packing. ----------------------------------------------

#[test]
fn t9_a_left_only_signal_leaves_right_silent() {
    let mut x = bell(&pink(16, 48_000), 3200.0, 10.0, 15.0);
    scale_to_rms_db(&mut x, -12.0);
    let zeros = vec![0.0f32; x.len()];
    for mode in [SuppressorMode::Stereo, SuppressorMode::MidSide] {
        let (ol, or) = run(&SuppressorConfig { mode, ..cfg_t1() }, &x, &zeros);
        assert_not_silent(&ol, "t9");
        let leak = or.iter().fold(0.0f32, |m, v| m.max(v.abs()));
        // Mid+Side sees M = S = L/2, measured and cut identically, so
        // R = M − S cancels too.
        assert!(leak < 1e-6, "{mode:?}: right leaks {leak}");
    }
}

// --- T10: modes. ------------------------------------------------------

#[test]
fn t10_stereo_mode_is_linked() {
    let mut x = bell(&pink(17, 2 * 48_000), 3200.0, 10.0, 15.0);
    scale_to_rms_db(&mut x, -12.0);
    let xr: Vec<f32> = x.iter().map(|v| 0.5 * v).collect();
    let (ol, or) = run(&cfg_t1(), &x, &xr);
    let mut err = 0.0f32;
    for i in 0..ol.len() {
        err = err.max((or[i] - 0.5 * ol[i]).abs());
    }
    assert!(err < 1e-6, "linked gains differ: {err}");
}

#[test]
fn t10_mid_mode_leaves_the_side_alone() {
    let lat = latency();
    let mut a = bell(&pink(18, 2 * 48_000), 3200.0, 10.0, 15.0);
    let mut b = pink(19, 2 * 48_000);
    scale_to_rms_db(&mut a, -12.0);
    scale_to_rms_db(&mut b, -18.0);
    let (l, r): (Vec<f32>, Vec<f32>) = a.iter().zip(&b).map(|(&m, &s)| (m + s, m - s)).unzip();
    let (ol, or) = run(&SuppressorConfig { mode: SuppressorMode::Mid, ..cfg_t1() }, &l, &r);
    let mut err = 0.0f32;
    for i in 2 * lat..l.len() {
        let s_out = 0.5 * (ol[i] - or[i]);
        err = err.max((s_out - b[i - lat]).abs());
    }
    assert!(err < 1e-5, "side changed by {err}");
}

#[test]
fn t10_mid_side_mode_cuts_a_side_resonance_and_spares_the_mid() {
    let mut m = pink(20, DUR);
    let mut s = bell(&pink(21, DUR), 3200.0, 10.0, 15.0);
    scale_to_rms_db(&mut m, -18.0);
    scale_to_rms_db(&mut s, -18.0);
    let (l, r): (Vec<f32>, Vec<f32>) = m.iter().zip(&s).map(|(&m, &s)| (m + s, m - s)).unzip();
    let (ol, or) = run(&SuppressorConfig { mode: SuppressorMode::MidSide, ..cfg_t1() }, &l, &r);
    let (mo, so): (Vec<f32>, Vec<f32>) =
        ol.iter().zip(&or).map(|(&l, &r)| (0.5 * (l + r), 0.5 * (l - r))).unzip();
    let (si, so) = aligned(&s, &so, 0.5);
    let (pi, po) = (psd(si), psd(so));
    let (f1, f2) = (3200.0 * 2f32.powf(-1.0 / 24.0), 3200.0 * 2f32.powf(1.0 / 24.0));
    let cut = band_db(&pi, f1, f2) - band_db(&po, f1, f2);
    assert!(cut >= 6.0, "side resonance cut only {cut:.2} dB");
    let (mi, mo) = aligned(&m, &mo, 0.5);
    let (bands, _) = third_octave_changes(mi, mo);
    for (fc, d) in bands {
        assert!(d.abs() < 0.5, "mid band {fc:.0} Hz moved {d:.3} dB");
    }
}

// --- T11: attack/release timing, CPU. ---------------------------------

/// Samples from `from` until `max_cut_db` (read per 64-sample block)
/// first satisfies `pred`.
fn time_until(
    sup: &mut ResonanceSuppressor,
    x: &[f32],
    cfg: &SuppressorConfig,
    from: usize,
    pred: impl Fn(f32) -> bool,
) -> (usize, f32) {
    let mut buf_l = x.to_vec();
    let mut buf_r = x.to_vec();
    let mut hit = None;
    let mut last = 0.0;
    let mut start = 0;
    while start < x.len() {
        let end = (start + 64).min(x.len());
        sup.process_stereo(&mut buf_l[start..end], &mut buf_r[start..end], cfg);
        last = sup.max_cut_db();
        if hit.is_none() && end > from && pred(last) {
            hit = Some(end - from);
        }
        start = end;
    }
    (hit.unwrap_or(usize::MAX), last)
}

#[test]
fn t11_attack_and_release_follow_their_time_constants() {
    let lat = latency();
    let n = 2 * 48_000;
    let t0 = 24_000;
    let mut noise = pink(22, n);
    scale_to_rms_db(&mut noise, -30.0);
    let tone = sine(3200.0, 0.1, n);
    let mut x: Vec<f32> = noise.clone();
    for i in t0..n {
        x[i] += tone[i];
    }
    let cfg = SuppressorConfig { attack_ms: 50.0, release_ms: 200.0, ..cfg_t1() };
    let attack = (0.050 * SR) as usize;

    // Final cut, from a long run. The tone stands far above the
    // selectivity line, so the target saturates at `depth` almost as
    // soon as it enters the frame: what follows is the one-pole.
    let mut sup = ResonanceSuppressor::new(SR);
    let (_, final_cut) = time_until(&mut sup, &x, &cfg, t0, |_| false);
    assert!(final_cut > 6.0, "tone not cut ({final_cut:.2} dB)");
    let mut sup = ResonanceSuppressor::new(SR);
    let (t_att, _) = time_until(&mut sup, &x, &cfg, t0, |c| c >= 0.63 * final_cut);
    eprintln!("attack: {t_att} samples to 63 % of {final_cut:.2} dB");
    assert!(t_att <= attack + lat, "attack took {t_att}");
    assert!(t_att >= attack / 2, "attack too fast ({t_att})");

    // Release: tone on, then off at t1. The target only drops once the
    // tone has drained out of the frame, so time the exponential itself:
    // 90 % → 37 % of the peak takes τ·ln(0.9/0.37) ≈ 0.89·τ.
    let t1 = 48_000;
    let mut y: Vec<f32> = noise.clone();
    for i in 0..t1 {
        y[i] += tone[i];
    }
    let release = 0.200 * SR;
    let mut sup = ResonanceSuppressor::new(SR);
    let _ = run_with(&mut sup, &y[..t1], &y[..t1], |_| cfg);
    let peak = sup.max_cut_db();
    let mut sup2 = ResonanceSuppressor::new(SR);
    let _ = run_with(&mut sup2, &y[..t1], &y[..t1], |_| cfg);
    let (t90, _) = time_until(&mut sup, &y[t1..], &cfg, 0, |c| c <= 0.9 * peak);
    let (t37, _) = time_until(&mut sup2, &y[t1..], &cfg, 0, |c| c <= 0.37 * peak);
    let seg = (t37 - t90) as f32;
    let expect = release * (0.9f32 / 0.37).ln();
    eprintln!("release: 90→37 % of {peak:.2} dB in {seg} samples (expect {expect:.0})");
    // ± one hop of read-out quantisation plus the detector integrator.
    let slack = 2.0 * StftGeometry::for_sample_rate(SR).hop as f32 + DETECTOR_TAU_MS * 1e-3 * SR;
    assert!((seg - expect).abs() <= slack, "release segment {seg} vs {expect}");
}

/// CPU budget: run by hand in release (`cargo test --release -p
/// resonance-dsp --test deharsh -- --ignored --nocapture`); wall-clock
/// limits are flaky under the concurrent `run-tests.py`.
#[test]
#[ignore]
fn t11_cpu_budget_under_two_percent_of_realtime() {
    let secs = 60;
    let mut x = bell(&pink(23, secs * 48_000), 3200.0, 10.0, 15.0);
    scale_to_rms_db(&mut x, -18.0);
    let mut best = f64::INFINITY;
    for _ in 0..3 {
        let mut sup = ResonanceSuppressor::new(SR);
        let (mut l, mut r) = (x.clone(), x.clone());
        let t = Instant::now();
        for (bl, br) in l.chunks_mut(BLOCK).zip(r.chunks_mut(BLOCK)) {
            sup.process_stereo(bl, br, &cfg_t1());
        }
        best = best.min(t.elapsed().as_secs_f64());
        assert_not_silent(&l, "cpu");
    }
    let pct = 100.0 * best / secs as f64;
    eprintln!("deharsh CPU: {pct:.3} % of realtime (48 kHz stereo)");
    assert!(pct < 2.0, "CPU {pct:.3} % of realtime");
}

// --- T12: RT safety and robustness. -----------------------------------

#[test]
fn t12_process_never_allocates() {
    let mut sup = ResonanceSuppressor::new(SR);
    let mut x = bell(&pink(24, 48_000), 3200.0, 10.0, 15.0);
    scale_to_rms_db(&mut x, -12.0);
    let mut walk = Walk::new(25);
    let configs: Vec<SuppressorConfig> = (0..x.len() / BLOCK + 1)
        .map(|b| {
            let mut c = walk.next();
            c.mode = SuppressorMode::from_index((b / 40) as i32 % 4);
            c.delta = b % 70 < 5;
            c.enabled = b % 90 > 3;
            c.mix = if b % 50 < 25 { 1.0 } else { 0.6 };
            c
        })
        .collect();
    let (mut l, mut r) = (x.clone(), x.clone());
    let mut mono = x.clone();
    let before = thread_allocs();
    for (b, (bl, br)) in l.chunks_mut(BLOCK).zip(r.chunks_mut(BLOCK)).enumerate() {
        sup.process_stereo(bl, br, &configs[b]);
    }
    sup.process_mono(&mut mono, &cfg_t1());
    sup.reset();
    let allocs = thread_allocs() - before;
    assert_eq!(allocs, 0, "process allocated {allocs} times");
}

#[test]
fn t12_hostile_input_stays_finite() {
    let n = 3 * latency();
    let signals: Vec<Vec<f32>> = vec![
        white(26, n),
        vec![1.0; n],
        (0..n).map(|i| if i % 2 == 0 { 1.0 } else { -1.0 }).collect(),
        vec![1e-38; n],
    ];
    for (k, x) in signals.iter().enumerate() {
        for mode in [SuppressorMode::Stereo, SuppressorMode::MidSide] {
            let cfg = SuppressorConfig { mode, selectivity_db: 0.0, depth_db: 24.0, ..cfg_t1() };
            let (ol, or) = run(&cfg, x, x);
            assert!(ol.iter().chain(&or).all(|v| v.is_finite()), "signal {k} {mode:?}: non-finite");
        }
    }
    // Garbage config values are sanitised.
    let bad = SuppressorConfig {
        depth_db: f32::NAN,
        selectivity_db: f32::INFINITY,
        sharpness_q: -3.0,
        attack_ms: 0.0,
        release_ms: f32::NAN,
        low_hz: 30_000.0,
        high_hz: 10.0,
        mix: 7.0,
        ..cfg_t1()
    };
    let (ol, _) = run(&bad, &signals[0], &signals[0]);
    assert!(ol.iter().all(|v| v.is_finite()));
}

