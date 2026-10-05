//! R8: the Nonlinear engine (reverb-algorithms.md §4.4, §5.2).
//!
//! Every render goes through `ReverbDsp::with_engines(SR, &[Nonlinear])`,
//! a bank holding only Nonlinear (so this holds before it joins
//! `Algorithm::BUILT`), with every engine setter and `set_extras` called
//! explicitly: 100 % wet, no pre-delay, return EQ off, ER/tail centred.
//!
//! The measurements print with
//!
//!     cargo test -p resonance-reverb --test algorithms nonlinear -- --nocapture
//!
//! The golden is re-blessed with `RESONANCE_BLESS=1` (or
//! `RESONANCE_BLESS_NONLINEAR=1` for this file alone).

use resonance_dsp_test_support as golden;
use resonance_reverb::dsp::{Algorithm, Extras, ReverbDsp};

use crate::common::{
    assert_not_silent, energy_db, render_scenario, Rng, Scenario, Setup, BLOCK, SR,
};

const GATED: i32 = 0;
const REVERSE: i32 = 1;
const FLAT: i32 = 2;

/// Every setter's value, plus diffusion, width and the nonlinear extras.
#[derive(Clone, Copy, Debug)]
struct Voicing {
    size: f32,
    damping: f32,
    diffusion: f32,
    mod_rate: f32,
    mod_depth: f32,
    shape: i32,
    length_ms: f32,
    width: f32,
}

impl Voicing {
    fn shaped(shape: i32, length_ms: f32) -> Self {
        Self {
            size: 0.5,
            damping: 8_000.0,
            diffusion: 0.8,
            mod_rate: 1.0,
            mod_depth: 0.3,
            shape,
            length_ms,
            width: 1.0,
        }
    }
}

fn extras(v: Voicing) -> Extras {
    Extras {
        nl_shape: v.shape,
        nl_length_ms: v.length_ms,
        ..Extras::default()
    }
}

impl Voicing {
    fn setup(self) -> Setup {
        Setup::new(Algorithm::Nonlinear, self.size, 2.0).with(|s| {
            s.damping = self.damping;
            s.diffusion = self.diffusion;
            s.mod_rate = self.mod_rate;
            s.mod_depth = self.mod_depth;
            s.width = self.width;
        })
    }
}

/// Every setter (the shared [`Setup`]) and `set_extras`.
fn dsp(v: Voicing) -> ReverbDsp {
    let mut d = v.setup().dsp();
    d.set_extras(extras(v));
    d
}

fn render(v: Voicing, n: usize, input: impl Fn(usize) -> f32) -> (Vec<f32>, Vec<f32>) {
    let mut d = dsp(v);
    let (mut l, mut r) = (Vec::with_capacity(n), Vec::with_capacity(n));
    for i in 0..n {
        let x = input(i);
        let (a, b) = d.process(x, x, v.diffusion, v.width);
        l.push(a);
        r.push(b);
    }
    (l, r)
}

/// A snare-like burst at sample `at`: noise with a 1 ms rise and a 25 ms
/// decay.
fn snare_at(i: usize, at: usize) -> f32 {
    if i < at {
        return 0.0;
    }
    let k = i - at;
    let mut s = (k as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ 0xD1B5_4A32_D192_ED03;
    s ^= s >> 29;
    s = s.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    s ^= s >> 32;
    let n = (s >> 40) as f32 / (1u64 << 24) as f32 * 2.0 - 1.0;
    let t = k as f32 / SR;
    0.8 * n * (t / 0.001).min(1.0) * (-t / 0.025).exp()
}

fn snare(i: usize) -> f32 {
    snare_at(i, 0)
}

/// RMS of both channels per 1 ms frame, dB.
fn envelope_db(l: &[f32], r: &[f32]) -> Vec<f32> {
    let w = (0.001 * SR) as usize;
    l.chunks(w)
        .zip(r.chunks(w))
        .map(|(a, b)| {
            let e: f32 = a.iter().chain(b).map(|x| x * x).sum::<f32>() / (2 * a.len()) as f32;
            10.0 * e.max(1e-20).log10()
        })
        .collect()
}

/// Mean level (dB) of `env` over ms `range`.
fn mean_db(env: &[f32], range: std::ops::Range<usize>) -> f32 {
    let n = range.len() as f32;
    let p: f32 = env[range]
        .iter()
        .map(|&d| 10f32.powf(d / 10.0))
        .sum::<f32>()
        / n;
    10.0 * p.max(1e-20).log10()
}

/// Gated: the gate closes `nl_length` after the hit, ±5 ms. The edge is
/// where the 1 ms envelope first drops 6 dB under the plateau (the
/// release's half-amplitude point) after the plateau; the plateau is the
/// level from 20 ms to `L` − 20 ms.
#[test]
fn gate_length_follows_nl_length() {
    for length in [100.0f32, 300.0, 700.0] {
        let v = Voicing::shaped(GATED, length);
        let n = ((length + 200.0) * 0.001 * SR) as usize;
        let (l, r) = render(v, n, snare);
        let env = envelope_db(&l, &r);
        let lms = length as usize;
        let plateau = mean_db(&env, 20..lms - 20);
        // Flat: no 20 ms stretch of the plateau strays more than 2 dB
        // (single 1 ms frames of a noise burst scatter by ±3 dB).
        let spread = (20..lms - 40)
            .step_by(20)
            .map(|a| (mean_db(&env, a..a + 20) - plateau).abs())
            .fold(0.0f32, f32::max);
        let edge = (lms - 20..env.len())
            .find(|&i| env[i] < plateau - 6.0)
            .expect("the gate never closed");
        let after = mean_db(&env, lms + 15..lms + 100);
        println!(
            "nonlinear gated {length} ms: plateau {plateau:.1} dB (±{spread:.1}), edge at {edge} ms, \
             {:.0} dB after",
            after - plateau
        );
        assert!(
            (edge as f32 - length).abs() <= 5.0,
            "{length} ms gate closed at {edge} ms"
        );
        assert!(
            spread < 2.0,
            "{length} ms: plateau not flat (±{spread:.1} dB)"
        );
        assert!(
            after < plateau - 60.0,
            "{length} ms: {:.1} dB after the gate",
            after - plateau
        );
    }
}

/// Reverse: the energy rises quarter by quarter over `nl_length`, then
/// stops.
#[test]
fn reverse_rises_over_nl_length() {
    for length in [200.0f32, 500.0] {
        let v = Voicing::shaped(REVERSE, length);
        let n = ((length + 150.0) * 0.001 * SR) as usize;
        let (l, r) = render(v, n, snare);
        let env = envelope_db(&l, &r);
        let q = length as usize / 4;
        let quarters: Vec<f32> = (0..4).map(|k| mean_db(&env, k * q..(k + 1) * q)).collect();
        let after = mean_db(&env, length as usize + 15..length as usize + 100);
        println!("nonlinear reverse {length} ms: quarters {quarters:.1?} dB, {after:.1} dB after");
        for k in 1..4 {
            assert!(
                quarters[k] > quarters[k - 1] + 3.0,
                "{length} ms: {quarters:?}"
            );
        }
        assert!(
            quarters[3] - quarters[0] > 20.0,
            "{length} ms: {quarters:?}"
        );
        assert!(
            after < quarters[3] - 60.0,
            "{length} ms: {after:.1} dB after"
        );
    }
}

/// Flat: flat for `nl_length`, then a decay: falling, but still there
/// where Gated is silent.
#[test]
fn flat_decays_after_nl_length() {
    let length = 300.0f32;
    let n = (1.0 * SR) as usize;
    let (l, r) = render(Voicing::shaped(FLAT, length), n, snare);
    let env = envelope_db(&l, &r);
    let plateau = mean_db(&env, 20..280);
    let a = mean_db(&env, 330..380);
    let b = mean_db(&env, 430..480);
    let c = mean_db(&env, 530..580);
    println!(
        "nonlinear flat 300 ms: plateau {plateau:.1} dB, then {:.1} / {:.1} / {:.1} dB",
        a - plateau,
        b - plateau,
        c - plateau
    );
    assert!(plateau - a > 2.0 && a - b > 5.0 && b - c > 5.0);
    assert!(b > plateau - 40.0, "decayed like a gate");
    let (gl, gr) = render(Voicing::shaped(GATED, length), n, snare);
    let gated = mean_db(&envelope_db(&gl, &gr), 430..480);
    assert!(b > gated + 30.0, "flat {b:.1} vs gated {gated:.1}");
}

/// A second snare hit after the gate has closed opens it again for
/// another `nl_length`; a sustained tone opens it once and does not
/// machine-gun it.
#[test]
fn transients_retrigger_and_sustained_tones_do_not() {
    let length = 200.0f32;
    let v = Voicing::shaped(GATED, length);
    let hit2 = (0.5 * SR) as usize;
    let (l, r) = render(v, (0.9 * SR) as usize, |i| snare(i) + snare_at(i, hit2));
    let env = envelope_db(&l, &r);
    let first = mean_db(&env, 20..180);
    let gap = mean_db(&env, 230..490);
    let second = mean_db(&env, 520..680);
    let after = mean_db(&env, 730..890);
    println!(
        "nonlinear retrigger: hit 1 {first:.1} dB, gap {gap:.1}, hit 2 {second:.1}, after {after:.1}"
    );
    assert!((second - first).abs() < 2.0, "{first:.1} vs {second:.1}");
    assert!(gap < first - 60.0 && after < first - 60.0);

    // A 220 Hz tone at −10 dBFS for 2 s: one trigger at the onset, then
    // the gate stays shut.
    let tone = |i: usize| 0.3 * (std::f32::consts::TAU * 220.0 * i as f32 / SR).sin();
    let (l, r) = render(v, (2.0 * SR) as usize, tone);
    let env = envelope_db(&l, &r);
    let onset = mean_db(&env, 20..180);
    let held = mean_db(&env, 300..2000);
    let loudest = env[300..2000].iter().cloned().fold(f32::MIN, f32::max);
    println!("nonlinear sustained tone: onset {onset:.1} dB, after the gate {held:.1} (loudest ms {loudest:.1})");
    assert!(
        onset > -40.0,
        "the tone's onset did not open the gate ({onset:.1} dB)"
    );
    assert!(
        loudest < onset - 60.0,
        "the tone retriggered ({loudest:.1} dB vs {onset:.1})"
    );

    // Noise at a steady level: no machine-gunning either.
    let mut s = 0x1234_5678u32;
    let noise: Vec<f32> = (0..(2.0 * SR) as usize)
        .map(|_| {
            s ^= s << 13;
            s ^= s >> 17;
            s ^= s << 5;
            0.2 * (s as f32 / u32::MAX as f32 * 2.0 - 1.0)
        })
        .collect();
    let (l, r) = render(v, noise.len(), |i| noise[i]);
    let env = envelope_db(&l, &r);
    let onset = mean_db(&env, 20..180);
    let loudest = env[300..2000].iter().cloned().fold(f32::MIN, f32::max);
    println!("nonlinear steady noise: onset {onset:.1} dB, loudest ms after {loudest:.1}");
    assert!(
        loudest < onset - 60.0,
        "steady noise retriggered ({loudest:.1} dB)"
    );
}

/// The silence guard and level at the defaults (Gated 300 ms, a unit
/// impulse).
#[test]
fn nonlinear_at_the_defaults_is_not_silent() {
    let v = Voicing::shaped(GATED, 300.0);
    let (l, r) = render(v, (0.6 * SR) as usize, |i| if i == 0 { 1.0 } else { 0.0 });
    let e = energy_db(&l, &r);
    println!("nonlinear defaults: energy {e:.1} dB re a unit impulse");
    assert_not_silent("nonlinear defaults", &l, &r);
    assert!(e < 6.0, "energy {e:.1} dB");
    // Decorrelated: the two sides of a mono hit differ.
    let c: f64 = l.iter().zip(&r).map(|(&a, &b)| a as f64 * b as f64).sum();
    let (el, er): (f64, f64) = (
        l.iter().map(|&a| (a as f64).powi(2)).sum(),
        r.iter().map(|&a| (a as f64).powi(2)).sum(),
    );
    let corr = c / (el * er).sqrt();
    println!("nonlinear L/R correlation {corr:.3}");
    assert!(corr.abs() < 0.5, "L/R correlation {corr:.3}");
}

/// 60 s of random automation of every setter and both nonlinear extras,
/// as steps and ramps (the shape as steps), over noise with impulses:
/// finite, never above +24 dBFS.
#[test]
fn nonlinear_survives_60_s_of_random_automation() {
    // (min, max, log-scaled)
    const RANGES: [(f32, f32, bool); 14] = [
        (0.0, 1.0, false),       // size
        (0.1, 30.0, true),       // decay
        (200.0, 20_000.0, true), // damping
        (0.0, 1.0, false),       // er_level
        (0.0, 1.0, false),       // er_time
        (0.01, 5.0, true),       // mod_rate
        (0.0, 1.0, false),       // mod_depth
        (0.25, 4.0, true),       // low_decay_mult
        (50.0, 1_000.0, true),   // low_xover
        (0.05, 1.0, false),      // high_decay_mult
        (0.0, 1.0, false),       // diffusion
        (50.0, 1_000.0, false),  // nl_length
        (0.0, 2.99, false),      // nl_shape (floored)
        (0.0, 1.0, false),       // build
    ];
    const K: usize = RANGES.len();
    let map = |k: usize, u: f32| {
        let (lo, hi, log) = RANGES[k];
        if log {
            lo * (hi / lo).powf(u)
        } else {
            lo + (hi - lo) * u
        }
    };
    let mut rng = Rng(0x5eed_0e0e_1111_d00d);
    let mut pos: [f32; K] = std::array::from_fn(|_| rng.next());
    let mut target = pos;
    let mut left = [0u32; K];
    let mut freeze = false;

    let mut d = ReverbDsp::with_engines(SR, &[Algorithm::Nonlinear]);
    let blocks = (60.0 * SR) as usize / BLOCK;
    let mut peak = 0.0f32;
    let mut noise_on = true;
    for block in 0..blocks {
        for k in 0..K {
            if left[k] == 0 && rng.next() < 0.03 {
                target[k] = rng.next();
                if rng.next() < 0.5 {
                    pos[k] = target[k];
                } else {
                    left[k] = 1 + (rng.next() * 150.0) as u32;
                }
            }
            if left[k] > 0 {
                pos[k] += (target[k] - pos[k]) / left[k] as f32;
                left[k] -= 1;
            }
        }
        if rng.next() < 0.004 {
            freeze = !freeze;
        }
        if rng.next() < 0.01 {
            noise_on = !noise_on;
        }
        let p = |k: usize| map(k, pos[k]);
        d.set_size(p(0));
        d.set_decay(p(1));
        d.set_freeze(freeze);
        d.set_damping(p(2));
        d.set_predelay(0.0);
        d.set_er_level(p(3));
        d.set_er_time(p(4));
        d.set_mod_rate(p(5));
        d.set_mod_depth(p(6));
        d.set_decay_shape(p(7), p(8), p(9));
        d.set_build(p(13));
        d.set_extras(Extras {
            nl_length_ms: p(11),
            nl_shape: p(12).floor() as i32,
            ..Extras::default()
        });
        let diffusion = p(10);
        for i in 0..BLOCK {
            let mut x = if noise_on { 0.25 * rng.gauss() } else { 0.0 };
            if i == 0 && block % 97 == 0 {
                x += 1.0;
            }
            let (l, r) = d.process(x, -0.7 * x, diffusion, 1.0);
            assert!(
                l.is_finite() && r.is_finite(),
                "non-finite at block {block}"
            );
            peak = peak.max(l.abs()).max(r.abs());
        }
    }
    let peak_db = 20.0 * peak.log10();
    println!("nonlinear random automation: peak {peak_db:+.1} dBFS");
    assert!(peak_db <= 24.0, "peak {peak_db:+.1} dBFS");
    assert!(peak > 1e-3, "the fuzz rendered silence");
}

/// A cleared engine renders bit-identically to a fresh one configured
/// with the same values, after a history with triggers, a size glide,
/// shape and length changes and the modulators running.
#[test]
fn reset_equals_fresh() {
    let v = Voicing {
        mod_depth: 0.8,
        mod_rate: 3.0,
        ..Voicing::shaped(FLAT, 250.0)
    };
    let mut used = dsp(v);
    let mut rng = Rng(7);
    for i in 0..(1.5 * SR) as usize {
        if i == 12_000 {
            used.set_size(0.9);
            used.set_extras(extras(Voicing::shaped(REVERSE, 400.0)));
        }
        if i == 40_000 {
            used.set_size(0.3);
            used.set_damping(5_000.0);
            used.set_extras(extras(Voicing::shaped(GATED, 150.0)));
        }
        let x = 0.3 * rng.gauss() + snare_at(i % 20_000, 0);
        used.process(x, -x, v.diffusion, v.width);
    }
    used.clear();

    let mut fresh = dsp(v);
    fresh.set_size(0.3);
    fresh.set_damping(5_000.0);
    fresh.set_extras(extras(Voicing::shaped(GATED, 150.0)));

    for i in 0..(0.6 * SR) as usize {
        let x = snare(i) + snare_at(i, 15_000);
        let a = used.process(x, 0.4 * x, v.diffusion, v.width);
        let b = fresh.process(x, 0.4 * x, v.diffusion, v.width);
        assert_eq!(
            (a.0.to_bits(), a.1.to_bits()),
            (b.0.to_bits(), b.1.to_bits()),
            "reset and fresh differ at sample {i}: {a:?} vs {b:?}"
        );
    }
}

/// Changing shape and length while the envelope runs does not click. A
/// sine's onset opens a 600 ms gate; 200 ms in the shape flips to a
/// 300 ms Reverse (a 13 dB drop), 50 ms later to a 100 ms Flat already
/// past its hold (a cut). The largest second difference stays within
/// 1.5× that of the undisturbed gate, whose own 10 ms release is in the
/// window.
#[test]
fn envelope_edits_mid_burst_do_not_click() {
    let v = Voicing::shaped(GATED, 600.0);
    let n = (0.8 * SR) as usize;
    let tone = |i: usize| 0.3 * (std::f32::consts::TAU * 220.0 * i as f32 / SR).sin();
    let worst = |edit: bool| {
        let mut d = dsp(v);
        let (mut prev, mut prev_step, mut worst) = (0.0f32, 0.0f32, 0.0f32);
        for i in 0..n {
            if edit && i == (0.2 * SR) as usize {
                d.set_extras(extras(Voicing::shaped(REVERSE, 300.0)));
            }
            if edit && i == (0.25 * SR) as usize {
                d.set_extras(extras(Voicing::shaped(FLAT, 100.0)));
            }
            let (l, _) = d.process(tone(i), tone(i), v.diffusion, v.width);
            let step = l - prev;
            if i > (0.1 * SR) as usize {
                worst = worst.max((step - prev_step).abs());
            }
            prev_step = step;
            prev = l;
        }
        worst
    };
    let (steady, edited) = (worst(false), worst(true));
    println!("nonlinear second difference: undisturbed {steady:.5}, with edits {edited:.5}");
    assert!(steady > 0.0);
    assert!(edited <= 1.5 * steady, "edits {edited:.5} vs {steady:.5}");
}

/// The tank view: line pairs that follow `size`, energies that follow the
/// gate, no ER taps.
#[test]
fn the_viz_getters_describe_the_burst() {
    let mut d = dsp(Voicing::shaped(GATED, 100.0));
    let small = d.fdn_delay_ms();
    assert!(small.iter().all(|&ms| ms > 3.0 && ms < 40.0), "{small:?}");
    for i in 0..4_800 {
        let x = if i == 0 { 1.0 } else { 0.0 };
        d.process(x, x, 0.8, 1.0);
    }
    assert!(
        d.channel_energies().iter().all(|&e| e > 0.0),
        "{:?}",
        d.channel_energies()
    );
    assert!(d
        .er_tap_times_ms()
        .iter()
        .all(|&(a, b)| a == 0.0 && b == 0.0));
    let mut big = dsp(Voicing {
        size: 1.0,
        ..Voicing::shaped(GATED, 100.0)
    });
    big.process(0.0, 0.0, 0.8, 1.0);
    let ratio = big.fdn_delay_ms()[7] / small[7];
    assert!(
        (ratio - 1.4).abs() < 0.05,
        "size 1 / size 0.5 lines: {ratio:.3}"
    );
}

// ---------------------------------------------------------------------
// Golden

/// A unit impulse on both channels at sample 0.
fn impulse_mono(n: usize) -> (f32, f32) {
    let x = if n == 0 { 1.0 } else { 0.0 };
    (x, x)
}

/// The nonlinear extras of each scenario are pinned by an edit at frame
/// 0 (before the first sample, so the same as configuring them).
fn scenarios() -> Vec<Scenario> {
    let big = Voicing {
        size: 0.8,
        damping: 5_000.0,
        diffusion: 0.95,
        mod_rate: 2.0,
        mod_depth: 1.0,
        shape: REVERSE,
        length_ms: 120.0,
        width: 0.8,
    };
    vec![
        // A unit impulse through a 150 ms gate: the diffusers, the burst,
        // the decay compensation and the release edge.
        Scenario {
            name: "impulse_gated_150",
            setup: Voicing::shaped(GATED, 150.0).setup(),
            predelay_ms: 0.0,
            frames: 9_600,
            input: impulse_mono,
            edit: Some((0, |d| d.set_extras(extras(Voicing::shaped(GATED, 150.0))))),
        },
        // Two snares into a big, modulated reverse envelope with a
        // pre-delay and narrowed width: the ramp, the cut, the detector
        // and the retrigger.
        Scenario {
            name: "snares_reverse_retrigger",
            setup: big.setup(),
            predelay_ms: 8.0,
            frames: 14_400,
            input: |n| {
                let s = snare(n) + snare_at(n, 7_200);
                (s, 0.6 * s)
            },
            edit: Some((0, |d| d.set_extras(extras(Voicing::shaped(REVERSE, 120.0))))),
        },
        // A flat envelope with its natural decay, mono-in from the right.
        Scenario {
            name: "snare_flat_decay",
            setup: Voicing::shaped(FLAT, 80.0).setup(),
            predelay_ms: 0.0,
            frames: 9_600,
            input: |n| (0.0, snare(n)),
            edit: Some((0, |d| d.set_extras(extras(Voicing::shaped(FLAT, 80.0))))),
        },
    ]
}

/// The shared `check_golden`, less its last-quarter tail guard: a gated
/// envelope ends in silence by design. Every scenario keeps its silence
/// guard re its own input.
#[test]
fn nonlinear_golden_is_bit_exact() {
    let mut rendered = Vec::new();
    for s in scenarios() {
        let (l, r, e_in) = render_scenario(&s);
        assert!(
            l.iter().chain(&r).all(|x| x.is_finite()),
            "{}: non-finite",
            s.name
        );
        let e = energy_db(&l, &r) - 10.0 * e_in.log10();
        assert!(
            e > -40.0,
            "{}: {e:.1} dB re its input (silence guard)",
            s.name
        );
        rendered.extend(l);
        rendered.extend(r);
    }

    let path = golden::golden_path(env!("CARGO_MANIFEST_DIR"), "nonlinear_golden.f32");
    if golden::blessed(&["RESONANCE_BLESS", "RESONANCE_BLESS_NONLINEAR"]) {
        golden::bless_f32(&path, &rendered);
        return;
    }
    let want = golden::load_golden_f32(&path, rendered.len(), "RESONANCE_BLESS=1");
    let diff = golden::compare_f32(&rendered, &want);
    if let Some((i, got, want)) = diff.first_diff {
        panic!(
            "Nonlinear output changed: {}/{} samples differ, peak delta {:.3e}; first at \
             sample {i} (got {got:?}, want {want:?}). Re-bless with RESONANCE_BLESS=1 \
             only for an intended change.",
            diff.diff_count,
            rendered.len(),
            diff.max_abs,
        );
    }
}
