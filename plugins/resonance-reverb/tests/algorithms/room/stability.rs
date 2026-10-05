//! "For every algorithm" (§5.2) on Room, Chamber and Ambience: random
//! automation, a 60 s Freeze, reset equals fresh, a size sweep that does
//! not click, and the viz getters.

use std::f32::consts::TAU;

use resonance_reverb::dsp::{Algorithm, ReverbDsp};

use super::common::*;

/// 60 s of random automation of every setter (steps and ramps, Freeze
/// toggling), noise with impulses on top: finite, peak ≤ +24 dBFS.
fn survives_random_automation(algorithm: Algorithm, seed: u64) {
    // (min, max, log-scaled)
    const RANGES: [(f32, f32, bool); 11] = [
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
    ];
    let map = |k: usize, u: f32| {
        let (lo, hi, log) = RANGES[k];
        if log {
            lo * (hi / lo).powf(u)
        } else {
            lo + (hi - lo) * u
        }
    };
    let mut rng = Rng(seed);
    // Normalised position, ramp target and remaining ramp blocks per param.
    let mut pos: [f32; 11] = std::array::from_fn(|_| rng.next());
    let mut target = pos;
    let mut left = [0u32; 11];
    let mut freeze = false;

    let mut d = ReverbDsp::with_engines(SR, &[algorithm]);
    let blocks = (60.0 * SR) as usize / BLOCK;
    let mut peak = 0.0f32;
    let mut noise_on = true;
    for block in 0..blocks {
        for k in 0..11 {
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
        d.set_wet_filters(false, 600.0, false, 10_000.0, false);
        d.set_er_tail_balance(0.0);
        d.set_decay_shape(p(7), p(8), p(9));
        d.set_build(0.5);
        let diffusion = p(10);
        for i in 0..BLOCK {
            let mut x = if noise_on { 0.25 * rng.gauss() } else { 0.0 };
            if i == 0 && block % 97 == 0 {
                x += 1.0;
            }
            let (l, r) = d.process(x, -0.7 * x, diffusion, 1.0);
            assert!(
                l.is_finite() && r.is_finite(),
                "{algorithm:?}: non-finite at block {block}"
            );
            peak = peak.max(l.abs()).max(r.abs());
        }
    }
    let peak_db = 20.0 * peak.log10();
    println!("{algorithm:?} random automation: peak {peak_db:+.1} dBFS");
    assert!(peak_db <= 24.0, "{algorithm:?}: peak {peak_db:+.1} dBFS");
    assert!(peak > 1e-3, "{algorithm:?}: the fuzz rendered silence");
}

#[test]
fn room_survives_60_s_of_random_automation() {
    survives_random_automation(Algorithm::Room, 0x5eed_0000_524f_4f4d);
}

#[test]
fn chamber_survives_60_s_of_random_automation() {
    survives_random_automation(Algorithm::Chamber, 0x5eed_0043_4841_4d42);
}

#[test]
fn ambience_survives_60_s_of_random_automation() {
    survives_random_automation(Algorithm::Ambience, 0x5eed_414d_4249_454e);
}

/// Freeze held for 60 s: the tail's energy drifts by at most 0.1 dB.
fn freeze_holds(s: Setup) {
    let mut d = s.dsp();
    let mut rng = Rng(42);
    for _ in 0..(SR as usize) {
        let x = 0.3 * rng.gauss();
        d.process(x, 0.5 * x, s.diffusion, s.width);
    }
    d.set_freeze(true);
    let window = (4.0 * SR) as usize;
    let mut energies = Vec::new();
    let mut acc = 0.0f64;
    for i in 0..(61.0 * SR) as usize {
        let (l, r) = d.process(0.0, 0.0, s.diffusion, s.width);
        // Skip the first second: the reflections and diffusers drain.
        if i >= SR as usize {
            acc += (l as f64).powi(2) + (r as f64).powi(2);
            if (i + 1 - SR as usize).is_multiple_of(window) {
                energies.push(acc);
                acc = 0.0;
            }
        }
    }
    let first = energies[0];
    assert!(
        first > 1e-3,
        "{:?}: nothing frozen ({first:.2e})",
        s.algorithm
    );
    let worst = energies
        .iter()
        .map(|e| 10.0 * (e / first).log10())
        .fold(0.0f64, |m, db| if db.abs() > m.abs() { db } else { m });
    println!(
        "{:?} freeze: worst 4 s window over 60 s {worst:+.4} dB re the first",
        s.algorithm
    );
    assert!(
        worst.abs() <= 0.1,
        "{:?}: freeze drifted {worst:+.3} dB",
        s.algorithm
    );
}

#[test]
fn room_freeze_holds_the_tail_for_60_s() {
    freeze_holds(Setup::new(Algorithm::Room, 0.5, 1.2).with(|s| s.mod_depth = 0.6));
}

#[test]
fn chamber_freeze_holds_the_tail_for_60_s() {
    freeze_holds(Setup::vocal_chamber());
}

#[test]
fn ambience_freeze_holds_the_tail_for_60_s() {
    freeze_holds(Setup::new(Algorithm::Ambience, 0.5, 0.8));
}

/// A cleared engine renders bit-identically to a fresh one configured
/// with the same values, after a history with size moves (glides and a
/// geometry crossfade), decay and ER changes, Freeze, and the modulators
/// running.
fn reset_equals_fresh(s: Setup) {
    let mut used = s.dsp();
    let mut rng = Rng(7);
    for i in 0..(1.5 * SR) as usize {
        if i == 12_000 {
            used.set_size(0.9);
            used.set_decay(4.0);
            used.set_er_time(0.8);
        }
        if i == 12_500 {
            // Mid-crossfade: queued.
            used.set_size(0.7);
        }
        if i == 30_000 {
            used.set_freeze(true);
        }
        if i == 40_000 {
            used.set_freeze(false);
            used.set_size(0.3);
            used.set_decay(0.9);
            used.set_er_level(0.7);
        }
        let x = 0.3 * rng.gauss();
        used.process(x, -x, s.diffusion, s.width);
    }
    used.clear();

    let mut fresh = s.dsp();
    fresh.set_size(0.3);
    fresh.set_decay(0.9);
    fresh.set_er_time(0.8);
    fresh.set_er_level(0.7);

    let mut rng = Rng(9);
    let mut energy = 0.0f64;
    for i in 0..(0.6 * SR) as usize {
        let x = if i < 2_000 { 0.3 * rng.gauss() } else { 0.0 };
        let a = used.process(x, 0.4 * x, s.diffusion, s.width);
        let b = fresh.process(x, 0.4 * x, s.diffusion, s.width);
        assert_eq!(
            (a.0.to_bits(), a.1.to_bits()),
            (b.0.to_bits(), b.1.to_bits()),
            "{:?}: reset and fresh differ at sample {i}: {a:?} vs {b:?}",
            s.algorithm
        );
        energy += (a.0 as f64).powi(2);
    }
    assert!(
        energy > 1e-3,
        "{:?}: the comparison rendered silence",
        s.algorithm
    );
}

#[test]
fn room_reset_equals_fresh() {
    reset_equals_fresh(Setup::new(Algorithm::Room, 0.5, 1.2).with(|s| {
        s.mod_depth = 0.8;
        s.mod_rate = 3.0;
    }));
}

#[test]
fn chamber_reset_equals_fresh() {
    reset_equals_fresh(Setup::new(Algorithm::Chamber, 0.5, 1.2).with(|s| {
        s.mod_depth = 1.0;
        s.mod_rate = 4.0;
    }));
}

#[test]
fn ambience_reset_equals_fresh() {
    reset_equals_fresh(Setup::new(Algorithm::Ambience, 0.5, 0.8));
}

/// A size sweep under a sustained sine (0.2 → 0.9 over 2 s, stepped per
/// 128-sample block as the plugin's block-rate smoother delivers it) does
/// not click.
///
/// The `switching.rs` check (largest sample step ≤ the held render's +
/// 1e-3) assumes the level does not move, and here it does: the lines
/// glide, the modes near 220 Hz move under the sine, and the steady-state
/// level rises by up to 2× along the sweep, which raises a sine's largest
/// step by the same factor without any discontinuity. So the steps are
/// compared per unit of level (step / peak, within 1e-3 of the held
/// rooms'), and the click detector proper is the second difference: a
/// sine's is `A·(2πf/fs)²` (≈ 4e-4 here), a discontinuity's is the jump
/// itself, so it must stay within 1e-3 of the held rooms' worst.
fn size_sweep_does_not_click(s: Setup) {
    let sine = |n: usize| 0.5 * (TAU * 220.0 * n as f32 / SR).sin();
    let warm = SR as usize;
    let window = 2 * SR as usize;
    let run = |d: &mut ReverbDsp, from: usize, sweep: bool| {
        let (mut l, mut r) = (Vec::with_capacity(window), Vec::with_capacity(window));
        for n in from..from + window {
            if sweep && (n - from).is_multiple_of(BLOCK) {
                d.set_size(0.2 + 0.7 * (n - from) as f32 / window as f32);
            }
            let x = sine(n);
            let (a, b) = d.process(x, x, s.diffusion, s.width);
            l.push(a);
            r.push(b);
        }
        (l, r)
    };
    // (largest step / peak, largest second difference)
    let measure = |l: &[f32], r: &[f32]| {
        let peak = l.iter().chain(r).fold(0.0f32, |m, x| m.max(x.abs()));
        let mut d2 = 0.0f32;
        for side in [l, r] {
            for w in side.windows(3) {
                d2 = d2.max((w[2] - 2.0 * w[1] + w[0]).abs());
            }
        }
        (max_step(l, r) / peak, d2, peak)
    };
    let held = |size: f32| {
        let mut d = s.with(|s| s.size = size).dsp();
        run(&mut d, 0, false);
        let (l, r) = run(&mut d, warm, false);
        measure(&l, &r)
    };
    let (a, b) = (held(0.2), held(0.9));
    let (still_step, still_d2) = (a.0.max(b.0), a.1.max(b.1));
    let mut d = s.with(|s| s.size = 0.2).dsp();
    run(&mut d, 0, false);
    let (l, r) = run(&mut d, warm, true);
    let (step, d2, peak) = measure(&l, &r);
    println!(
        "{:?} size sweep: step/peak {step:.5} (held {still_step:.5}), 2nd diff {d2:.5} \
         (held {still_d2:.5}), peak {peak:.3}",
        s.algorithm
    );
    assert!(peak > 0.05, "{:?}: the sweep is near silent", s.algorithm);
    assert!(
        step <= still_step + 1e-3,
        "{:?}: the size sweep clicks: step/peak {step:.5} against {still_step:.5} held",
        s.algorithm
    );
    assert!(
        d2 <= still_d2 + 1e-3,
        "{:?}: the size sweep clicks: 2nd difference {d2:.5} against {still_d2:.5} held",
        s.algorithm
    );
}

#[test]
fn room_size_sweep_does_not_click() {
    size_sweep_does_not_click(Setup::new(Algorithm::Room, 0.2, 1.2));
}

#[test]
fn chamber_size_sweep_does_not_click() {
    size_sweep_does_not_click(Setup::vocal_chamber());
}

#[test]
fn ambience_size_sweep_does_not_click() {
    size_sweep_does_not_click(Setup::new(Algorithm::Ambience, 0.2, 0.8));
}

/// The tank view and ER view read the engine: eight line-pair lengths
/// that follow `size`, live energies after an impulse, and the first 12
/// shoebox taps (ascending, inside the box's reach, with gains).
#[test]
fn the_viz_getters_describe_the_room() {
    for algorithm in [Algorithm::Room, Algorithm::Chamber, Algorithm::Ambience] {
        let s = Setup::new(algorithm, 0.2, 1.0);
        let mut d = s.dsp();
        let small = d.fdn_delay_ms();
        let small_er = d.er_tap_times_ms();
        d.set_size(0.9);
        let big = d.fdn_delay_ms();
        let big_er = d.er_tap_times_ms();
        for k in 0..8 {
            assert!(
                big[k] > small[k],
                "{algorithm:?}: line pair {k} did not grow"
            );
            if k > 0 {
                assert!(
                    small[k] >= small[k - 1],
                    "{algorithm:?}: pairs not ascending"
                );
            }
        }
        assert!(
            small[0] > 1.0 && big[7] < 150.0,
            "{algorithm:?}: {small:?} / {big:?}"
        );
        for k in 0..12 {
            let (l, r) = small_er[k];
            assert!(
                l > 0.0 && r > 0.0 && l < 100.0,
                "{algorithm:?}: tap {k} at {l}/{r} ms"
            );
            if k > 0 {
                assert!(l >= small_er[k - 1].0, "{algorithm:?}: taps not ascending");
            }
        }
        assert!(
            big_er[11].0 > small_er[11].0,
            "{algorithm:?}: ER did not grow with size"
        );
        let gains = d.er_tap_gains();
        assert!(gains.iter().all(|g| g.0.abs() > 0.0 && g.1.abs() > 0.0));

        for i in 0..4_800 {
            let x = if i == 0 { 1.0 } else { 0.0 };
            d.process(x, x, s.diffusion, s.width);
        }
        let e = d.channel_energies();
        assert!(
            e.iter().all(|&v| v > 0.0),
            "{algorithm:?}: dead line pair {e:?}"
        );
        println!("{algorithm:?}: lines {small:.1?} -> {big:.1?} ms, energies {e:.4?}");
    }
}
