//! Tests for `resonance_dsp::reverb` (reverb-algorithms.md R2): the FDN,
//! its absorption and matrices, the allpass, the random modulator and the
//! shoebox early reflections.

use resonance_dsp::reverb::{
    hadamard_in_place, householder_in_place, loop_gain, Absorption, Allpass, DecayBands, Fdn, FdnConfig, MatrixKind,
    ShoeboxEr, SmoothRandom, FIRST_ORDER_TAPS, MAX_TAPS,
};
use resonance_dsp::{Biquad, SimpleRng};

const FS: f32 = 48_000.0;

// ---------------------------------------------------------------- helpers

fn matrix_of<const N: usize>(apply: fn(&mut [f32; N])) -> [[f32; N]; N] {
    // Column j is M·e_j.
    let mut m = [[0.0; N]; N];
    for j in 0..N {
        let mut e = [0.0f32; N];
        e[j] = 1.0;
        apply(&mut e);
        for i in 0..N {
            m[i][j] = e[i];
        }
    }
    m
}

fn assert_orthogonal<const N: usize>(apply: fn(&mut [f32; N]), name: &str) {
    let m = matrix_of(apply);
    for a in 0..N {
        for b in 0..N {
            let dot: f64 = (0..N).map(|i| m[i][a] as f64 * m[i][b] as f64).sum();
            let want = if a == b { 1.0 } else { 0.0 };
            assert!((dot - want).abs() < 1e-6, "{name} N={N}: (MᵀM)[{a}][{b}] = {dot}");
        }
    }
}

fn gcd(mut a: usize, mut b: usize) -> usize {
    while b != 0 {
        (a, b) = (b, a % b);
    }
    a
}

fn alternating<const N: usize>(x: f32) -> [f32; N] {
    std::array::from_fn(|i| if i % 2 == 0 { x } else { -x })
}

/// Mono impulse response of an FDN: an alternating-sign impulse into every
/// line, the alternating-sign sum of the outputs.
fn fdn_ir<const N: usize>(fdn: &mut Fdn<N>, samples: usize) -> Vec<f32> {
    let scale = 1.0 / (N as f32).sqrt();
    let imp = alternating::<N>(scale);
    let zero = [0.0f32; N];
    (0..samples)
        .map(|n| {
            let y = fdn.tick(if n == 0 { &imp } else { &zero });
            y.iter()
                .enumerate()
                .map(|(i, v)| if i % 2 == 0 { *v } else { -*v })
                .sum::<f32>()
                * scale
        })
        .collect()
}

/// T60 from a Schroeder backward-integrated energy decay curve, by a
/// least-squares line through the −5…−35 dB span, extrapolated to −60.
fn t60_of(ir: &[f32]) -> f64 {
    let mut edc = vec![0.0f64; ir.len()];
    let mut acc = 0.0f64;
    for i in (0..ir.len()).rev() {
        acc += ir[i] as f64 * ir[i] as f64;
        edc[i] = acc;
    }
    let total = edc[0];
    let (mut sx, mut sy, mut sxx, mut sxy, mut n) = (0.0, 0.0, 0.0, 0.0, 0.0);
    for (i, e) in edc.iter().enumerate() {
        let db = 10.0 * (e / total).max(1e-30).log10();
        if (-35.0..=-5.0).contains(&db) {
            let t = i as f64 / FS as f64;
            sx += t;
            sy += db;
            sxx += t * t;
            sxy += t * db;
            n += 1.0;
        }
    }
    assert!(n > 100.0, "decay never spanned −5…−35 dB");
    let slope = (n * sxy - sx * sy) / (n * sxx - sx * sx);
    -60.0 / slope
}

fn filtered(ir: &[f32], mut f: Biquad) -> Vec<f32> {
    ir.iter().map(|&x| f.process(x)).collect()
}

fn noise(seed: u64, n: usize) -> Vec<f32> {
    let mut rng = SimpleRng::new(seed);
    (0..n)
        .map(|_| rng.next_u32() as f32 / u32::MAX as f32 * 2.0 - 1.0)
        .collect()
}

fn hall<const N: usize>(seed: u64) -> Fdn<N> {
    let mut cfg = FdnConfig::new(40.0, 200.0);
    cfg.seed = seed;
    Fdn::new(FS, cfg)
}

fn run_noise<const N: usize>(fdn: &mut Fdn<N>, input: &[f32]) -> Vec<f32> {
    input
        .iter()
        .map(|&x| fdn.tick(&alternating::<N>(x)).iter().sum::<f32>())
        .collect()
}

// --------------------------------------------------------------- matrices

#[test]
fn feedback_matrices_are_orthogonal() {
    assert_orthogonal::<8>(householder_in_place::<8>, "Householder");
    assert_orthogonal::<16>(householder_in_place::<16>, "Householder");
    assert_orthogonal::<8>(hadamard_in_place::<8>, "Hadamard");
    assert_orthogonal::<16>(hadamard_in_place::<16>, "Hadamard");
    assert_eq!(MatrixKind::default_for(8), MatrixKind::Householder);
    assert_eq!(MatrixKind::default_for(16), MatrixKind::Hadamard);
}

fn lossless_energy_holds<const N: usize>(modulated: bool) {
    let mut fdn = hall::<N>(7);
    fdn.set_decay(DecayBands::flat(f32::INFINITY));
    for x in noise(3, 4800) {
        fdn.tick(&alternating::<N>(x));
    }
    if modulated {
        // Freeze fades the modulation out, then holds energy.
        fdn.set_decay(DecayBands::flat(2.0));
        fdn.set_modulation(1.0, 8.0);
        for _ in 0..4800 {
            fdn.tick(&[0.0; N]);
        }
        fdn.set_freeze(true);
        for _ in 0..4800 {
            fdn.tick(&[1.0; N]); // muted by freeze
        }
    }
    let e0 = fdn.stored_energy();
    assert!(e0 > 1.0, "excitation left no energy ({e0})");
    let mut worst = 0.0f64;
    for _ in 0..10 {
        for _ in 0..FS as usize {
            fdn.tick(&[0.0; N]);
        }
        let drift = 10.0 * (fdn.stored_energy() / e0).log10();
        worst = worst.max(drift.abs());
    }
    assert!(
        worst <= 0.1,
        "N={N} modulated={modulated}: energy drifted {worst:.4} dB over 10 s"
    );
}

#[test]
fn lossless_fdn_preserves_energy_for_10_s() {
    lossless_energy_holds::<8>(false);
    lossless_energy_holds::<16>(false);
}

#[test]
fn freeze_mutes_input_and_preserves_energy() {
    lossless_energy_holds::<8>(true);
    lossless_energy_holds::<16>(true);
}

// ------------------------------------------------------------- absorption

/// Per-pass loop magnitude at 50 Hz, 1 kHz and 16 kHz against the design
/// target. A first-order shelf's skirt costs a fixed share of the *log*
/// gain, so the absolute dB error grows with the per-pass loss: the 0.5 dB
/// bound holds wherever a line recirculates at least five times per band
/// T60 (≤ 12 dB per pass, every realistic FDN line); beyond that the
/// implied T60 is checked instead (within 10 %). The mid band is solved
/// for, so it is exact everywhere.
#[test]
fn absorption_matches_band_targets() {
    let probes = [(50.0f32, 0usize), (1000.0, 1), (16_000.0, 2)];
    let mut tight = 0;
    for &len in &[331.0f32, 1499.0, 4801.0, 9601.0] {
        for &(t60, lo, hi) in &[
            (2.0f32, 1.0f32, 1.0f32),
            (2.0, 2.0, 0.5),
            (1.0, 1.5, 0.3),
            (4.0, 0.5, 1.0),
            (0.5, 1.3, 0.6),
        ] {
            let bands = DecayBands::from_mults(t60, lo, hi, 250.0, 4000.0);
            let a = Absorption::new(&bands, len, FS);
            for &(f, band) in &probes {
                let t = [bands.t60_low, bands.t60_mid, bands.t60_high][band];
                let want = loop_gain(len, t, FS);
                let got = a.magnitude(f, FS);
                let err_db = 20.0 * (got / want).log10();
                let implied = -3.0 * len / (FS * got.log10());
                let rel = implied / t - 1.0;
                let case = format!("len {len} T60 {t60} ×{lo}/×{hi} @ {f} Hz");
                if band == 1 {
                    assert!(rel.abs() < 0.01, "{case}: mid T60 {implied:.3} vs {t:.3}");
                }
                if 20.0 * want.log10() >= -12.0 {
                    tight += 1;
                    assert!(err_db.abs() < 0.5, "{case}: {err_db:.3} dB off");
                    assert!(rel.abs() < 0.05, "{case}: implied T60 {implied:.3} vs {t:.3}");
                } else {
                    assert!(rel.abs() < 0.10, "{case}: implied T60 {implied:.3} vs {t:.3}");
                }
            }
        }
    }
    assert!(tight >= 40, "only {tight} probes in the ≤ 12 dB/pass regime");
}

#[test]
fn absorption_never_gains() {
    for &(lo, hi) in &[
        (4.0f32, 0.05f32),
        (0.25, 1.0),
        (4.0, 1.0),
        (0.25, 0.05),
        (1.0, 0.05),
        (3.0, 0.2),
    ] {
        for &(xl, xh) in &[
            (250.0f32, 4000.0f32),
            (50.0, 1000.0),
            (1000.0, 12_000.0),
            (400.0, 1000.0),
        ] {
            for &len in &[200.0f32, 2000.0, 9600.0] {
                let bands = DecayBands::from_mults(2.0, lo, hi, xl, xh);
                let a = Absorption::new(&bands, len, FS);
                for k in 0..=240 {
                    let f = 10.0 * (2400.0f32).powf(k as f32 / 240.0);
                    let m = a.magnitude(f.min(FS / 2.0), FS);
                    assert!(m <= 1.0 + 1e-6, "×{lo}/×{hi} {xl}/{xh} len {len}: |H({f:.0})| = {m}");
                }
            }
        }
    }
}

#[test]
fn absorption_is_exact_at_dc_and_nyquist() {
    let bands = DecayBands::from_mults(2.0, 2.0, 0.4, 300.0, 5000.0);
    let a = Absorption::new(&bands, 2400.0, FS);
    let dc = a.magnitude(0.0, FS);
    let ny = a.magnitude(FS / 2.0, FS);
    assert!((dc / loop_gain(2400.0, 4.0, FS) - 1.0).abs() < 1e-5, "DC {dc}");
    assert!((ny / loop_gain(2400.0, 0.8, FS) - 1.0).abs() < 1e-5, "Nyquist {ny}");
}

// -------------------------------------------------------------- FDN decay

/// Broadband T60 of the impulse response against the design. Short decays
/// run on room-length lines (10–60 ms), long ones on hall lines (40–200 ms),
/// as the engines will: a 0.5 s tail through 200 ms lines is only a couple
/// of recirculations and has no smooth exponential to fit.
fn broadband_t60_matches<const N: usize>(t60: f32, depth: f32) {
    let (lo, hi) = if t60 < 1.0 { (10.0, 60.0) } else { (40.0, 200.0) };
    let mut cfg = FdnConfig::new(lo, hi);
    cfg.seed = 11;
    let mut fdn = Fdn::<N>::new(FS, cfg);
    fdn.set_decay(DecayBands::flat(t60));
    fdn.set_modulation(0.7, depth);
    let ir = fdn_ir(&mut fdn, ((1.6 * t60 + 0.3) * FS) as usize);
    let got = t60_of(&ir);
    let rel = got / t60 as f64 - 1.0;
    assert!(
        rel.abs() <= 0.05,
        "N={N} depth {depth}: T60 {got:.3} s for a {t60} s design ({:+.1} %)",
        rel * 100.0
    );
}

#[test]
fn fdn_broadband_decay_matches_t60() {
    for t60 in [0.5, 2.0, 8.0] {
        for depth in [0.0, 4.0] {
            broadband_t60_matches::<8>(t60, depth);
            broadband_t60_matches::<16>(t60, depth);
        }
    }
}

fn band_t60s<const N: usize>(bands: DecayBands) -> [f64; 3] {
    let mut fdn = hall::<N>(5);
    fdn.set_decay(bands);
    let longest = bands.t60_low.max(bands.t60_mid).max(bands.t60_high);
    let ir = fdn_ir(&mut fdn, ((1.6 * longest + 0.3) * FS) as usize);
    let mut lp = Biquad::identity();
    lp.set_low_pass(FS, 100.0, 0.707);
    let mut bp = Biquad::identity();
    bp.set_band_pass(FS, 1000.0, 2.0);
    let mut hp = Biquad::identity();
    hp.set_high_pass(FS, 12_000.0, 0.707);
    [
        t60_of(&filtered(&ir, lp)),
        t60_of(&filtered(&ir, bp)),
        t60_of(&filtered(&ir, hp)),
    ]
}

#[test]
fn fdn_band_decays_follow_the_multipliers() {
    let bands = DecayBands::from_mults(2.0, 2.0, 0.5, 250.0, 4000.0);
    for (n, [low, mid, high]) in [(8, band_t60s::<8>(bands)), (16, band_t60s::<16>(bands))] {
        assert!(
            low > mid && mid > high,
            "N={n}: band T60s not ordered: {low:.2} / {mid:.2} / {high:.2}"
        );
        for (got, want, name) in [(low, 4.0, "low"), (mid, 2.0, "mid"), (high, 1.0, "high")] {
            let rel = got / want - 1.0;
            assert!(rel.abs() <= 0.15, "N={n} {name}: {got:.3} s vs {want} s");
        }
    }
    // Bass shorter than mid when the multiplier is below 1.
    let [low, mid, _] = band_t60s::<16>(DecayBands::from_mults(2.0, 0.5, 1.0, 250.0, 4000.0));
    assert!(low < mid, "low mult 0.5: {low:.2} vs {mid:.2}");
}

// ------------------------------------------------- lengths, seeds, reset

#[test]
fn line_lengths_are_pairwise_coprime_at_every_size() {
    fn check<const N: usize>() {
        for seed in [1u64, 42, 0xDEAD_BEEF] {
            let mut fdn = hall::<N>(seed);
            for size in [0.05f32, 0.3, 0.5, 1.0, 1.37, 2.0] {
                fdn.set_size(size);
                let l = fdn.line_lengths();
                for i in 0..N {
                    for j in i + 1..N {
                        assert_eq!(gcd(l[i], l[j]), 1, "N={N} seed {seed} size {size}: {l:?}");
                    }
                }
                // Spread over the configured range (log-uniform strata).
                if size == 1.0 {
                    let ms = |s: usize| s as f32 * 1000.0 / FS;
                    assert!(ms(l[0]) < 40.0 * 1.15 && ms(l[N - 1]) > 200.0 * 0.85, "N={N}: {l:?}");
                }
            }
        }
    }
    check::<8>();
    check::<16>();
}

#[test]
fn fdn_is_deterministic_per_seed() {
    let input = noise(9, 9600);
    let (mut a, mut b, mut c) = (hall::<16>(3), hall::<16>(3), hall::<16>(4));
    for f in [&mut a, &mut b, &mut c] {
        f.set_modulation(0.9, 6.0);
    }
    assert_eq!(a.line_lengths(), b.line_lengths());
    assert_ne!(a.line_lengths(), c.line_lengths());
    assert_eq!(run_noise(&mut a, &input), run_noise(&mut b, &input));
}

#[test]
fn fdn_clear_equals_fresh() {
    let setup = |f: &mut Fdn<16>| {
        f.set_decay(DecayBands::from_mults(3.0, 1.4, 0.5, 250.0, 4000.0));
        f.set_modulation(0.8, 5.0);
        f.set_size(1.2);
    };
    let input = noise(2, 12_000);
    let mut used = hall::<16>(8);
    setup(&mut used);
    run_noise(&mut used, &input);
    used.clear();
    let mut fresh = hall::<16>(8);
    setup(&mut fresh);
    let a = run_noise(&mut used, &input);
    let b = run_noise(&mut fresh, &input);
    assert!(
        a.iter().zip(&b).all(|(x, y)| x.to_bits() == y.to_bits()),
        "clear() != fresh"
    );
}

#[test]
fn fdn_glide_moves_reads_without_a_jump() {
    let mut fdn = hall::<8>(1);
    fdn.set_glide(0.05);
    let input = noise(4, 24_000);
    let out1 = run_noise(&mut fdn, &input[..12_000]);
    fdn.set_size(1.5);
    let out2 = run_noise(&mut fdn, &input[12_000..]);
    let max_step = |v: &[f32]| v.windows(2).map(|w| (w[1] - w[0]).abs()).fold(0.0f32, f32::max);
    assert!(max_step(&out2) < 1.5 * max_step(&out1) + 0.5, "glide stepped");
    assert!(out2.iter().all(|v| v.is_finite()));
}

// ---------------------------------------------------------------- allpass

fn energy(v: &[f32]) -> f64 {
    v.iter().map(|&x| x as f64 * x as f64).sum()
}

#[test]
fn allpass_impulse_response_has_unit_energy() {
    for &(m, g) in &[(37usize, 0.7f32), (142, -0.6), (1, 0.5), (900, 0.75)] {
        let mut ap = Allpass::new(1000, m, g);
        let ir: Vec<f32> = (0..200_000)
            .map(|n| ap.process(if n == 0 { 1.0 } else { 0.0 }))
            .collect();
        let e = energy(&ir);
        assert!((e - 1.0).abs() < 1e-4, "M {m} g {g}: energy {e}");
    }
    // Nested: an allpass inside an allpass's delay path stays allpass.
    let mut outer = Allpass::new(200, 120, 0.5);
    let mut inner = Allpass::new(100, 41, -0.6);
    let ir: Vec<f32> = (0..200_000)
        .map(|n| outer.process_nested(if n == 0 { 1.0 } else { 0.0 }, 0.0, |s| inner.process(s)))
        .collect();
    let e = energy(&ir);
    assert!((e - 1.0).abs() < 1e-4, "nested: energy {e}");
}

#[test]
fn allpass_keeps_a_sines_rms() {
    for f in [100.0f32, 1000.0, 7000.0] {
        let mut ap = Allpass::new(200, 113, 0.7);
        let x: Vec<f32> = (0..96_000)
            .map(|n| (std::f32::consts::TAU * f * n as f32 / FS).sin())
            .collect();
        let y: Vec<f32> = x.iter().map(|&s| ap.process(s)).collect();
        // Settled half, a whole number of periods (all probes divide 48 k).
        let (ex, ey) = (energy(&x[48_000..]), energy(&y[48_000..]));
        let db = 10.0 * (ey / ex).log10();
        assert!(db.abs() < 0.01, "{f} Hz: {db:.4} dB");
    }
}

#[test]
fn allpass_clear_equals_fresh_and_taps_read_inside() {
    let input = noise(5, 5000);
    let mut used = Allpass::new(300, 200, 0.6);
    for &x in &input {
        used.process_modulated(x, 1.3);
    }
    used.clear();
    let mut fresh = Allpass::new(300, 200, 0.6);
    for &x in &input {
        assert_eq!(
            used.process_modulated(x, 0.4).to_bits(),
            fresh.process_modulated(x, 0.4).to_bits()
        );
    }
    // tap(0) is the last value written into the line: w = x + g·s.
    let mut ap = Allpass::new(10, 3, 0.5);
    ap.process(1.0);
    assert_eq!(ap.tap(0), 1.0);
    ap.process(0.0);
    ap.process(0.0);
    ap.process(0.0); // s = 1 → w = 0.5
    assert_eq!(ap.tap(0), 0.5);
}

// ---------------------------------------------------------- smooth random

#[test]
fn smooth_random_is_bounded_continuous_and_deterministic() {
    for &(rate, depth) in &[(0.5f32, 3.0f32), (2.0, 10.0), (8.0, 1.0)] {
        let mut a = SmoothRandom::new(77, rate, depth, FS);
        let mut b = SmoothRandom::new(77, rate, depth, FS);
        let mut c = SmoothRandom::new(78, rate, depth, FS);
        let va: Vec<f32> = (0..480_000).map(|_| a.next_sample()).collect();
        let vb: Vec<f32> = (0..480_000).map(|_| b.next_sample()).collect();
        let vc: Vec<f32> = (0..480_000).map(|_| c.next_sample()).collect();
        assert_eq!(va, vb);
        assert_ne!(va, vc);
        let peak = va.iter().fold(0.0f32, |m, v| m.max(v.abs()));
        assert!(peak <= depth, "rate {rate}: peak {peak} > depth {depth}");
        assert!(peak > 0.5 * depth, "rate {rate}: barely moves ({peak})");
        let bound = 3.0 * depth * rate / FS * 1.01 + 1e-6;
        let step = va.windows(2).map(|w| (w[1] - w[0]).abs()).fold(0.0f32, f32::max);
        assert!(step <= bound, "rate {rate}: step {step} > {bound}");
        a.reset();
        let again: Vec<f32> = (0..1000).map(|_| a.next_sample()).collect();
        assert_eq!(&again[..], &va[..1000], "reset() != fresh");
    }
}

// ---------------------------------------------------------------- shoebox

fn dist(a: [f32; 3], b: [f32; 3]) -> f32 {
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt()
}

#[test]
fn shoebox_first_order_delays_match_geometry() {
    let (room, src, lst) = ([6.0f32, 8.0, 3.0], [2.0f32, 5.0, 1.2], [3.5f32, 2.0, 1.6]);
    let er = ShoeboxEr::new(room, src, lst, 0.3);
    let taps = er.taps(FS);
    assert_eq!(taps.len(), MAX_TAPS);
    assert_eq!(MAX_TAPS, 6 + 18);
    let first: Vec<_> = taps.iter().filter(|t| t.order == 1).collect();
    assert_eq!(first.len(), FIRST_ORDER_TAPS);
    // The six wall images.
    let mut want = Vec::new();
    for axis in 0..3 {
        for img in [-src[axis], 2.0 * room[axis] - src[axis]] {
            let mut p = src;
            p[axis] = img;
            want.push(dist(p, lst) / 343.0 * FS);
        }
    }
    for w in want {
        assert!(
            first.iter().any(|t| (t.delay_samples - w).abs() <= 1.0),
            "no first-order tap at {w:.1} samples: {first:?}"
        );
    }
    let direct = dist(src, lst) / 343.0 * FS;
    assert!((er.direct_delay_samples(FS) - direct).abs() <= 1.0);
    assert!(
        taps.iter().all(|t| t.delay_samples > direct),
        "a reflection beat the direct sound"
    );
    // Sorted by delay.
    assert!(taps.windows(2).all(|w| w[0].delay_samples <= w[1].delay_samples));
}

#[test]
fn shoebox_gains_fall_with_distance_and_pan_by_side() {
    let er = ShoeboxEr::new([10.0, 12.0, 4.0], [4.0, 9.0, 1.5], [5.0, 3.0, 1.5], 0.2);
    let taps = er.taps(FS);
    let mag = |t: &resonance_dsp::reverb::ErTap| (t.gain_l * t.gain_l + t.gain_r * t.gain_r).sqrt();
    for order in [1u8, 2] {
        let same: Vec<_> = taps.iter().filter(|t| t.order == order).collect();
        for w in same.windows(2) {
            assert!(mag(w[1]) <= mag(w[0]) + 1e-6, "order {order}: gain rose with delay");
        }
    }
    assert!(taps.iter().all(|t| mag(t) < 1.0));
    // More absorption, quieter reflections.
    let damp = ShoeboxEr { absorption: 0.6, ..er }.taps(FS);
    assert!(taps.iter().zip(&damp).all(|(a, b)| mag(b) < mag(a)));
    // The right wall (x = 10, listener at x = 5) is heard on the right.
    let right_wall = 2.0 * 10.0 - 4.0;
    let want = dist([right_wall, 9.0, 1.5], [5.0, 3.0, 1.5]) / 343.0 * FS;
    let t = taps
        .iter()
        .find(|t| t.order == 1 && (t.delay_samples - want).abs() < 0.5)
        .unwrap();
    assert!(t.gain_r > t.gain_l, "{t:?}");
}

#[test]
fn shoebox_preallocated_matches_and_first_order_only_has_six() {
    let er = ShoeboxEr::new([5.0, 7.0, 3.0], [1.0, 4.0, 1.0], [2.5, 1.5, 1.7], 0.4);
    let mut buf = [Default::default(); MAX_TAPS];
    let n = er.write_taps(FS, &mut buf);
    assert_eq!(&buf[..n], &er.taps(FS)[..]);
    let first = ShoeboxEr { max_order: 1, ..er };
    assert_eq!(first.tap_count(), FIRST_ORDER_TAPS);
    assert_eq!(first.taps(FS).len(), FIRST_ORDER_TAPS);
    // Scaling the room scales every delay.
    let big = er.scaled(2.0).taps(FS);
    for (a, b) in er.taps(FS).iter().zip(&big) {
        assert!((b.delay_samples - 2.0 * a.delay_samples).abs() < 0.05);
    }
}
