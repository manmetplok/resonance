//! R8: the Spring engine (reverb-algorithms.md §4.4, §5.2).
//!
//! Every render goes through `ReverbDsp::with_engines(SR, &[Spring])`, a
//! bank holding only Spring (so this holds before Spring joins
//! `Algorithm::BUILT`), with every engine setter and `set_extras` called
//! explicitly: 100 % wet, no pre-delay, return EQ off, ER/tail centred.
//!
//! The measurements print with
//!
//!     cargo test -p resonance-reverb --test algorithms spring -- --nocapture
//!
//! The golden is re-blessed with `RESONANCE_BLESS=1` (or
//! `RESONANCE_BLESS_SPRING=1` for this file alone).

use resonance_dsp::Biquad;
use resonance_metering::decay::ImpulseReport;
use resonance_reverb::dsp::{Algorithm, Extras, ReverbDsp};

use crate::common::{assert_not_silent, check_golden, energy_db, Rng, Scenario, Setup, BLOCK, SR};

/// The setters Spring reads, plus diffusion, width and the spring extras
/// (everything else at the global defaults, as [`Setup::new`] has it).
#[derive(Clone, Copy, Debug)]
struct Voicing {
    size: f32,
    decay: f32,
    damping: f32,
    diffusion: f32,
    tension: f32,
    drip: f32,
    width: f32,
}

impl Voicing {
    /// The global defaults with no drip (so the echo train is the loop's
    /// alone).
    fn dry() -> Self {
        Self {
            size: 0.5,
            decay: 2.0,
            damping: 8_000.0,
            diffusion: 0.8,
            tension: 0.5,
            drip: 0.0,
            width: 1.0,
        }
    }

    fn setup(self) -> Setup {
        Setup::new(Algorithm::Spring, self.size, self.decay).with(|s| {
            s.damping = self.damping;
            s.diffusion = self.diffusion;
            s.width = self.width;
        })
    }
}

fn extras(v: Voicing) -> Extras {
    Extras {
        spring_tension: v.tension,
        spring_drip: v.drip,
        ..Extras::default()
    }
}

/// Every setter (the shared [`Setup`]) and `set_extras`.
fn dsp(v: Voicing) -> ReverbDsp {
    let mut d = v.setup().dsp();
    d.set_extras(extras(v));
    d
}

/// Render `input` (one sample per frame, both channels).
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

fn impulse(v: Voicing, seconds: f32) -> (Vec<f32>, Vec<f32>) {
    render(
        v,
        (seconds * SR) as usize,
        |i| if i == 0 { 1.0 } else { 0.0 },
    )
}

/// `x` through a band-pass at `hz` (Q 3).
fn band(x: &[f32], hz: f32) -> Vec<f32> {
    let mut bp = Biquad::identity();
    bp.set_band_pass(SR, hz, 3.0);
    x.iter().map(|&s| bp.process(s)).collect()
}

/// Energy centroid of `x` over `range`, samples.
fn centroid(x: &[f32], range: std::ops::Range<usize>) -> f64 {
    let (mut m, mut w) = (0.0f64, 0.0f64);
    for i in range {
        let e = (x[i] as f64).powi(2);
        m += i as f64 * e;
        w += e;
    }
    m / w.max(1e-30)
}

/// When the first pass of an impulse leaves the spring at `hz`, ms: the
/// energy centroid of the band-passed left output over the first
/// `window_ms`, minus the band-pass's own delay (the same centroid of a
/// band-passed impulse).
fn first_arrival_ms(l: &[f32], hz: f32, window_ms: f32) -> f64 {
    let w = (window_ms * 0.001 * SR) as usize;
    let mut unit = vec![0.0f32; w];
    unit[0] = 1.0;
    let own = centroid(&band(&unit, hz), 0..w);
    (centroid(&band(l, hz), 0..w) - own) / SR as f64 * 1000.0
}

/// The round trip `size` asks for, ms (the engine's 30–90 ms log map).
fn round_trip_ms(size: f32) -> f32 {
    30.0 * 3f32.powf(size)
}

/// The chirp: in every echo the bass leaves later than the treble, and
/// the gap grows from the first echo to the second (dispersion piles up
/// pass by pass).
#[test]
fn echoes_chirp_with_the_bass_trailing() {
    let v = Voicing {
        tension: 0.8,
        ..Voicing::dry()
    };
    let (l, _) = impulse(v, 0.4);
    let t = round_trip_ms(v.size);
    let lo = first_arrival_ms(&l, 250.0, 0.6 * t);
    let hi = first_arrival_ms(&l, 2_000.0, 0.6 * t);
    // The second echo: a window one round trip later, against the same
    // band-pass delay.
    let second = |hz: f32| {
        let (a, b) = (
            ((0.6 * t) * 0.001 * SR) as usize,
            ((1.6 * t) * 0.001 * SR) as usize,
        );
        let mut unit = vec![0.0f32; b];
        unit[a] = 1.0;
        let own = centroid(&band(&unit, hz), a..b) - a as f64;
        (centroid(&band(&l, hz), a..b) - own) / SR as f64 * 1000.0
    };
    let (lo2, hi2) = (second(250.0), second(2_000.0));
    println!(
        "spring chirp, tension 0.8: echo 1 at 250 Hz {lo:.2} ms, 2 kHz {hi:.2} ms; \
         echo 2 at 250 Hz {lo2:.2} ms, 2 kHz {hi2:.2} ms"
    );
    assert!(
        lo - hi > 3.0,
        "echo 1: 250 Hz {lo:.2} ms vs 2 kHz {hi:.2} ms"
    );
    assert!(
        lo2 - hi2 > 1.5 * (lo - hi),
        "the chirp did not lengthen: {:.2} ms then {:.2} ms",
        lo - hi,
        lo2 - hi2
    );
}

/// Echo spacing: the autocorrelation of the 1 kHz band's envelope peaks
/// at the round trip `size` asks for (30–90 ms), within 3 %.
#[test]
fn echo_spacing_is_the_round_trip() {
    for size in [0.0f32, 0.5, 1.0] {
        let v = Voicing {
            size,
            tension: 0.2,
            ..Voicing::dry()
        };
        let (l, _) = impulse(v, 1.0);
        let b = band(&l, 1_000.0);
        // 1 ms envelope.
        let mut env = Vec::with_capacity(b.len());
        let mut e = 0.0f32;
        for &s in &b {
            e += 0.02 * (s * s - e);
            env.push(e);
        }
        let lag_range = (0.02 * SR) as usize..(0.12 * SR) as usize;
        let n = env.len() - lag_range.end;
        let mut best = (0usize, f64::MIN);
        for lag in lag_range {
            let c: f64 = (0..n).map(|i| env[i] as f64 * env[i + lag] as f64).sum();
            if c > best.1 {
                best = (lag, c);
            }
        }
        let got = best.0 as f32 / SR * 1000.0;
        let want = round_trip_ms(size);
        println!("spring size {size}: echo spacing {got:.2} ms (round trip {want:.2} ms)");
        assert!(
            (got / want - 1.0).abs() < 0.03,
            "size {size}: spacing {got:.2} ms, want {want:.2}"
        );
    }
}

/// `decay` is the T60 of the echo train: mid T30 within ±15 %.
#[test]
fn decay_is_the_echo_trains_t60() {
    for (decay, size) in [(1.0f32, 0.5f32), (2.0, 0.3), (2.0, 0.8), (4.0, 0.5)] {
        let v = Voicing {
            decay,
            size,
            ..Voicing::dry()
        };
        let (l, r) = impulse(v, 1.4 * decay + 0.6);
        let rep = ImpulseReport::analyze(&l, &r, SR);
        let t30 = rep.mid_t30().expect("mid T30");
        let err = t30 / decay - 1.0;
        println!(
            "spring decay {decay} s size {size}: mid T30 {t30:.3} s ({:+.1} %)",
            err * 100.0
        );
        assert!(err.abs() <= 0.15, "decay {decay}: T30 {t30:.3} s");
    }
}

/// `spring_tension` measurably changes the dispersion: the 250 Hz vs
/// 2 kHz gap of the first echo grows at least threefold from 0 to 1, and
/// monotonically.
#[test]
fn tension_sets_the_chirp_rate() {
    let mut gaps = Vec::new();
    for tension in [0.0f32, 0.5, 1.0] {
        let v = Voicing {
            tension,
            ..Voicing::dry()
        };
        let (l, _) = impulse(v, 0.2);
        let t = round_trip_ms(v.size);
        let gap = first_arrival_ms(&l, 250.0, 0.6 * t) - first_arrival_ms(&l, 2_000.0, 0.6 * t);
        println!("spring tension {tension}: 250 Hz trails 2 kHz by {gap:.2} ms");
        gaps.push(gap);
    }
    assert!(gaps[0] < gaps[1] && gaps[1] < gaps[2], "{gaps:?}");
    assert!(gaps[2] > 3.0 * gaps[0].max(0.1), "{gaps:?}");
}

/// A snare-like burst: 3 ms of noise with a 25 ms decay.
fn snare(i: usize) -> f32 {
    let mut s = (i as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ 0xD1B5_4A32_D192_ED03;
    s ^= s >> 29;
    s = s.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    s ^= s >> 32;
    let n = (s >> 40) as f32 / (1u64 << 24) as f32 * 2.0 - 1.0;
    let t = i as f32 / SR;
    0.8 * n * (-t / 0.025).exp()
}

/// `spring_drip` adds energy to a transient's response and next to
/// nothing to a sustained tone's.
#[test]
fn drip_adds_transient_energy() {
    let n = (0.3 * SR) as usize;
    let hit = |drip: f32| {
        let v = Voicing {
            drip,
            ..Voicing::dry()
        };
        let (l, r) = render(v, n, snare);
        energy_db(&l, &r)
    };
    let (none, full) = (hit(0.0), hit(1.0));
    println!("spring drip on a snare: {none:.1} dB → {full:.1} dB (first 300 ms)");
    assert!(full - none > 1.5, "drip added {:.2} dB", full - none);

    // A 220 Hz tone, measured from 1 s on (the onset's drip has gone by
    // two decays' worth of echoes... and the steady state carries none).
    let tone = |drip: f32| {
        let v = Voicing {
            drip,
            decay: 0.5,
            ..Voicing::dry()
        };
        let (l, r) = render(v, (2.0 * SR) as usize, |i| {
            0.3 * (std::f32::consts::TAU * 220.0 * i as f32 / SR).sin()
        });
        let k = SR as usize;
        energy_db(&l[k..], &r[k..])
    };
    let (none, full) = (tone(0.0), tone(1.0));
    println!("spring drip on a sustained tone: {none:.2} dB → {full:.2} dB");
    assert!(
        (full - none).abs() < 0.2,
        "drip moved a sustained tone {:.2} dB",
        full - none
    );
}

/// The silence guard and level at the defaults (a unit impulse).
#[test]
fn spring_at_the_defaults_is_not_silent() {
    let v = Voicing {
        drip: 0.3,
        ..Voicing::dry()
    };
    let (l, r) = impulse(v, 3.0);
    let e = energy_db(&l, &r);
    let rep = ImpulseReport::analyze(&l, &r, SR);
    println!(
        "spring defaults: energy {e:.1} dB re a unit impulse\n{}\n{rep}",
        ImpulseReport::table_header()
    );
    assert_not_silent("spring defaults", &l, &r);
    assert!(e < 6.0, "energy {e:.1} dB");
}

/// 60 s of random automation of every setter and both spring extras, as
/// steps and ramps, over noise with impulses: finite, never above
/// +24 dBFS.
#[test]
fn spring_survives_60_s_of_random_automation() {
    // (min, max, log-scaled)
    const RANGES: [(f32, f32, bool); 13] = [
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
        (0.0, 1.0, false),       // spring_tension
        (0.0, 1.0, false),       // spring_drip
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
    let mut rng = Rng(0x5eed_5921_6a11_d00d);
    let mut pos: [f32; K] = std::array::from_fn(|_| rng.next());
    let mut target = pos;
    let mut left = [0u32; K];
    let mut freeze = false;

    let mut d = ReverbDsp::with_engines(SR, &[Algorithm::Spring]);
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
        d.set_build(0.5);
        d.set_extras(Extras {
            spring_tension: p(11),
            spring_drip: p(12),
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
    println!("spring random automation: peak {peak_db:+.1} dBFS");
    assert!(peak_db <= 24.0, "peak {peak_db:+.1} dBFS");
    assert!(peak > 1e-3, "the fuzz rendered silence");
}

/// A cleared engine renders bit-identically to a fresh one configured
/// with the same values, after a history with a size glide, a tension
/// glide, a decay and damping change and drip.
#[test]
fn reset_equals_fresh() {
    let v = Voicing {
        drip: 0.7,
        ..Voicing::dry()
    };
    let mut used = dsp(v);
    let mut rng = Rng(7);
    for i in 0..(1.5 * SR) as usize {
        if i == 12_000 {
            used.set_size(0.9);
            used.set_decay(4.0);
            used.set_extras(Extras {
                spring_tension: 0.9,
                ..extras(v)
            });
        }
        if i == 40_000 {
            used.set_size(0.3);
            used.set_decay(2.5);
            used.set_damping(5_000.0);
            used.set_extras(Extras {
                spring_tension: 0.3,
                ..extras(v)
            });
        }
        let x = 0.3 * rng.gauss();
        used.process(x, -x, v.diffusion, v.width);
    }
    used.clear();

    let mut fresh = dsp(v);
    fresh.set_size(0.3);
    fresh.set_decay(2.5);
    fresh.set_damping(5_000.0);
    fresh.set_extras(Extras {
        spring_tension: 0.3,
        ..extras(v)
    });

    let mut rng = Rng(9);
    for i in 0..(0.6 * SR) as usize {
        let x = if i < 2_000 { 0.3 * rng.gauss() } else { 0.0 };
        let a = used.process(x, 0.4 * x, v.diffusion, v.width);
        let b = fresh.process(x, 0.4 * x, v.diffusion, v.width);
        assert_eq!(
            (a.0.to_bits(), a.1.to_bits()),
            (b.0.to_bits(), b.1.to_bits()),
            "reset and fresh differ at sample {i}: {a:?} vs {b:?}"
        );
    }
}

/// A throw of each parameter the loop reads (size, tension, damping,
/// decay) on a sustained tone does not click: the largest second
/// difference after the throw stays within 4× that of the steady tone
/// (a size throw bends the pitch for a moment; the gain, filter and
/// coefficient glide).
#[test]
fn parameter_moves_on_running_audio_do_not_click() {
    let v = Voicing::dry();
    let n = (1.5 * SR) as usize;
    let tone = |i: usize| 0.3 * (std::f32::consts::TAU * 330.0 * i as f32 / SR).sin();
    let throws: [(&str, fn(&mut ReverbDsp)); 5] = [
        ("none", |_| {}),
        ("size 0.5 -> 1", |d| d.set_size(1.0)),
        ("tension 0.5 -> 1", |d| {
            d.set_extras(Extras {
                spring_tension: 1.0,
                ..extras(Voicing::dry())
            })
        }),
        ("damping 8 -> 3 kHz", |d| d.set_damping(3_000.0)),
        ("decay 2 -> 0.5 s", |d| d.set_decay(0.5)),
    ];
    let mut steady = 0.0f32;
    for (name, throw) in throws {
        let mut d = dsp(v);
        let (mut prev, mut prev_step, mut worst) = (0.0f32, 0.0f32, 0.0f32);
        for i in 0..n {
            if i == SR as usize / 2 {
                throw(&mut d);
            }
            let (l, _) = d.process(tone(i), tone(i), v.diffusion, v.width);
            let step = l - prev;
            if i > SR as usize / 4 {
                worst = worst.max((step - prev_step).abs());
            }
            prev_step = step;
            prev = l;
        }
        println!("spring throw {name}: largest second difference {worst:.5}");
        if name == "none" {
            steady = worst;
        } else {
            assert!(
                worst <= 4.0 * steady,
                "{name}: {worst:.5} vs steady {steady:.5}"
            );
        }
    }
}

/// The tank view: two round trips that follow `size`, live energies, no
/// ER taps.
#[test]
fn the_viz_getters_describe_the_springs() {
    let mut d = dsp(Voicing::dry());
    let ms = d.fdn_delay_ms();
    let want = round_trip_ms(0.5);
    assert!((ms[0] - want).abs() < 0.01, "{ms:?}");
    assert!((ms[1] / ms[0] - 1.13).abs() < 1e-3, "{ms:?}");
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
        ..Voicing::dry()
    });
    big.process(0.0, 0.0, 0.8, 1.0);
    assert!((big.fdn_delay_ms()[0] - 90.0).abs() < 0.01);
}

/// A 5 ms snare click into the default spring (default drip) at ER/tail
/// `balance`.
fn snare_at_balance(balance: f32, n: usize) -> (Vec<f32>, Vec<f32>) {
    let v = Voicing {
        drip: 0.3,
        ..Voicing::dry()
    };
    let mut d = dsp(v);
    d.set_er_tail_balance(balance);
    let (mut l, mut r) = (Vec::with_capacity(n), Vec::with_capacity(n));
    for i in 0..n {
        let x = if i < (0.005 * SR) as usize { snare(i) } else { 0.0 };
        let (a, b) = d.process(x, x, v.diffusion, v.width);
        l.push(a);
        r.push(b);
    }
    (l, r)
}

/// §4.2: Spring has no discrete ERs, so the ER/tail balance weights its
/// onset (the first pass through the springs, before the first echo
/// comes round) against the echoes. Balance −1 keeps an audible onset and
/// no echoes; +1 keeps the echoes and removes the onset.
#[test]
fn er_tail_balance_weights_the_onset_against_the_echoes() {
    let n = (0.6 * SR) as usize;
    // The first echo leaves the loop one round trip in (minus the
    // cascade's own ~5 ms): everything before 30 ms is onset at the
    // default size, everything after 90 ms is echoes.
    let onset = ..(0.030 * SR) as usize;
    let echoes = (0.090 * SR) as usize..;
    let (l0, r0) = snare_at_balance(0.0, n);
    let (le, re) = snare_at_balance(-1.0, n);
    let (lt, rt) = snare_at_balance(1.0, n);
    let db = |l: &[f32], r: &[f32]| energy_db(l, r);
    let centre_onset = db(&l0[onset], &r0[onset]);
    let centre_echoes = db(&l0[echoes.clone()], &r0[echoes.clone()]);
    let er_onset = db(&le[onset], &re[onset]);
    let er_echoes = db(&le[echoes.clone()], &re[echoes.clone()]);
    let tail_onset = db(&lt[onset], &rt[onset]);
    let tail_echoes = db(&lt[echoes.clone()], &rt[echoes.clone()]);
    println!(
        "spring balance: onset {centre_onset:.1} / {er_onset:.1} / {tail_onset:.1} dB, \
         echoes {centre_echoes:.1} / {er_echoes:.1} / {tail_echoes:.1} dB (0 / -1 / +1)"
    );
    // −1: the onset is all there, the echoes are gone.
    assert!((er_onset - centre_onset).abs() < 1.0, "-1 lost the onset");
    assert!(er_onset > -40.0, "-1 onset is inaudible: {er_onset:.1} dB");
    assert!(er_echoes < centre_echoes - 40.0, "-1 kept the echoes");
    // +1: the echoes are all there, the onset is gone.
    assert!((tail_echoes - centre_echoes).abs() < 1.0, "+1 lost the echoes");
    assert!(tail_onset < centre_onset - 40.0, "+1 kept the onset");
}

/// A 200 Hz tone burst every 150 ms (an abrupt start each time, so every
/// burst trips the drip's transient detector).
fn tone_bursts(i: usize) -> f32 {
    let period = (0.15 * SR) as usize;
    let k = i % period;
    if k < (0.08 * SR) as usize {
        0.5 * (std::f32::consts::TAU * 200.0 * k as f32 / SR).sin()
    } else {
        0.0
    }
}

/// `spring_drip` is set once per block; automating it during hits must
/// not zipper. Drip stepping between 0 and 1 every block, over tone
/// bursts, has no sharper edges (largest second difference) than drip
/// held at 1.
#[test]
fn drip_automation_during_hits_does_not_zipper() {
    let v = Voicing::dry();
    let n = (1.2 * SR) as usize;
    let block = 256;
    let worst = |automate: bool| {
        let mut d = dsp(Voicing { drip: 1.0, ..v });
        let (mut prev, mut prev_step, mut worst) = (0.0f32, 0.0f32, 0.0f32);
        for i in 0..n {
            if automate && i % block == 0 {
                let drip = if (i / block) % 2 == 0 { 1.0 } else { 0.0 };
                d.set_extras(Extras {
                    spring_drip: drip,
                    ..extras(v)
                });
            }
            let x = tone_bursts(i);
            let (l, _) = d.process(x, x, v.diffusion, v.width);
            let step = l - prev;
            worst = worst.max((step - prev_step).abs());
            prev_step = step;
            prev = l;
        }
        worst
    };
    let (held, automated) = (worst(false), worst(true));
    println!("spring drip: largest second difference held {held:.5}, automated {automated:.5}");
    assert!(
        automated <= 1.25 * held,
        "drip automation zippers: {automated:.5} vs held {held:.5}"
    );
}

// ---------------------------------------------------------------------
// Golden

/// A unit impulse on both channels at sample 0.
fn impulse_mono(n: usize) -> (f32, f32) {
    let x = if n == 0 { 1.0 } else { 0.0 };
    (x, x)
}

/// A snare, decorrelated L/R.
fn snare_lr(n: usize) -> (f32, f32) {
    (snare(n), 0.7 * snare(n + 101))
}

/// The spring extras are pinned by an edit at frame 0 (before the first
/// sample, so the same as configuring them), or by the scenario's own
/// mid-render edit.
fn scenarios() -> Vec<Scenario> {
    let tight = Voicing {
        size: 0.2,
        decay: 3.0,
        damping: 5_000.0,
        diffusion: 0.5,
        tension: 0.9,
        drip: 0.8,
        width: 0.8,
    };
    vec![
        // An impulse into the default spring with its default drip (0.3,
        // the engine's own default extras): the cascade, the loop, the
        // drip chirp.
        Scenario {
            name: "impulse_defaults",
            setup: Voicing::dry().setup(),
            predelay_ms: 0.0,
            frames: 9_600,
            input: impulse_mono,
            edit: None,
        },
        // A snare into a tight, dark, high-tension spring with lots of
        // drip, a pre-delay and a narrowed width.
        Scenario {
            name: "snare_tight_tense",
            setup: tight.setup(),
            predelay_ms: 5.0,
            frames: 9_600,
            input: snare_lr,
            edit: Some((0, |d| {
                d.set_extras(Extras {
                    spring_tension: 0.9,
                    spring_drip: 0.8,
                    ..Extras::default()
                })
            })),
        },
        // The same snare into the default spring; 100 ms in, size and
        // tension move, so the read-head and coefficient glides are
        // pinned too.
        Scenario {
            name: "snare_size_tension_glide",
            setup: Voicing::dry().setup(),
            predelay_ms: 0.0,
            frames: 12_000,
            input: snare_lr,
            edit: Some((4_800, |d| {
                d.set_size(0.8);
                d.set_extras(Extras {
                    spring_tension: 0.9,
                    ..Extras::default()
                });
            })),
        },
    ]
}

#[test]
fn spring_golden_is_bit_exact() {
    check_golden(
        "spring_golden.f32",
        &["RESONANCE_BLESS", "RESONANCE_BLESS_SPRING"],
        &scenarios(),
    );
}
