//! R5 Hall acceptance (reverb-algorithms.md §5.2, the Hall row and "for
//! every algorithm").
//!
//! Every render goes through `ReverbDsp::with_engines(SR, &[Hall])` (or
//! `[Classic]` for the comparisons) with every setter called explicitly,
//! 100 % wet, no pre-delay, return EQ off, ER/tail balance centred, width 1.
//! The measured numbers print with `--nocapture`:
//!
//!     cargo test -p resonance-reverb --test algorithms hall -- --nocapture
//!
//! Metrics are `resonance_metering::decay`'s (§5.1). Two the harness does
//! not have are defined here:
//!
//! - **Peakiness floor.** Peakiness reads several dB even on exponentially
//!   decaying white noise, and more the shorter the decay (a 1 s segment
//!   of a 2.5 s decay holds fewer independent spectral values at low
//!   frequencies). So "≤ Classic − 3 dB" is only reachable where Classic
//!   sits at least 3 dB above that floor. Each cell is compared with
//!   `max(Classic − 3, floor + 2)`, the floor being the mean peakiness of
//!   six seeded decaying noises at the same T60; the cells where Classic
//!   is peaky (small size, long decay) bind on `Classic − 3`.
//! - **Wobble** (pitch modulation of the tail, L4). A sustained 1 kHz sine
//!   at a 10 s decay; after 3 s the response is heterodyned to DC,
//!   averaged to 1 kHz, Hann-windowed over 2 s and its spectrum taken over
//!   ±50 Hz in 0.1 Hz steps. The figure is the power-weighted RMS spread of
//!   that spectrum around 1 kHz, in cents. A time-invariant reverb puts all
//!   of a sustained sine at exactly 1 kHz (the residue is the window and
//!   the onset transient still decaying, ~0.5 cent); delay modulation
//!   spreads it into sidebands. Unlike a zero-crossing pitch track it is
//!   not thrown by the amplitude fades a modulated reverb puts on a sine
//!   (up to 30 dB deep here), which make instantaneous frequency
//!   meaningless at the nulls.

use std::sync::OnceLock;

use resonance_metering::decay::{modal_peakiness_db, ImpulseReport, PEAKINESS_START_S};
use resonance_reverb::dsp::algo::hall::HallEngine;
use resonance_reverb::dsp::{Algorithm, ReverbDsp};

use crate::common::*;

/// The Hall at `size`/`decay`, the plugin's defaults (decay shape
/// included) otherwise.
fn hall(size: f32, decay: f32) -> Setup {
    Setup::new(Algorithm::Hall, size, decay)
}

/// Long enough for the T30 fit (−35 dB at 0.58·T60) with margin, and for
/// the 1.2 s the peakiness segment needs.
fn t30_seconds(decay: f32) -> f32 {
    (0.75 * decay + 0.6).max(1.3)
}

fn mean_peakiness(l: &[f32], r: &[f32]) -> f32 {
    let pk = |x: &[f32]| modal_peakiness_db(x, SR, PEAKINESS_START_S, 100.0, 8_000.0).unwrap();
    0.5 * (pk(l) + pk(r))
}

// ---------------------------------------------------------------------------
// The shared grid: Hall and Classic at sizes 0.5/0.9 × decays 1/2.5/5/10 s
// ---------------------------------------------------------------------------

const SIZES: [f32; 2] = [0.5, 0.9];
const DECAYS: [f32; 4] = [1.0, 2.5, 5.0, 10.0];

struct Cell {
    size: f32,
    decay: f32,
    hall: ImpulseReport,
    classic: ImpulseReport,
}

/// Rendered once, shared by the tests that read it.
fn grid() -> &'static [Cell] {
    static GRID: OnceLock<Vec<Cell>> = OnceLock::new();
    GRID.get_or_init(|| {
        let mut cells = Vec::new();
        for &size in &SIZES {
            for &decay in &DECAYS {
                let s = hall(size, decay);
                let (l, r) = s.impulse(t30_seconds(decay));
                assert_not_silent(&format!("hall {size}/{decay}"), &l, &r);
                let hall = ImpulseReport::analyze(&l, &r, SR);
                let (l, r) = s.on(Algorithm::Classic).impulse(t30_seconds(decay));
                let classic = ImpulseReport::analyze(&l, &r, SR);
                cells.push(Cell {
                    size,
                    decay,
                    hall,
                    classic,
                });
            }
        }
        cells
    })
}

#[test]
fn mid_decay_tracks_the_knob_within_7_percent() {
    println!(
        "\nHall vs Classic (48 kHz, impulse, defaults, build 0.5)\n\
         alg     size decay  midT30  err%   {}",
        ImpulseReport::table_header()
    );
    for c in grid() {
        for (name, rep) in [("Hall", &c.hall), ("Classic", &c.classic)] {
            let mid = rep.mid_t30();
            println!(
                "{name:<7} {:>4.1} {:>5.1}s {:>7} {:>6}  {rep}",
                c.size,
                c.decay,
                mid.map_or("-".into(), |t| format!("{t:.3}")),
                mid.map_or("-".into(), |t| format!("{:+.1}", 100.0 * (t - c.decay) / c.decay)),
            );
        }
        let mid = c.hall.mid_t30().expect("no mid T30");
        let err = (mid - c.decay) / c.decay;
        assert!(
            err.abs() <= 0.07,
            "size {} decay {} s: mid T30 {mid:.3} s ({:+.1} %)",
            c.size,
            c.decay,
            100.0 * err
        );
    }
}

#[test]
fn the_late_tail_is_decorrelated_and_survives_mono() {
    for c in grid() {
        let iacc = c.hall.late_iacc.unwrap();
        let mono = c.hall.mono_fold_db.unwrap();
        assert!(iacc <= 0.2, "size {} decay {}: late IACC {iacc:.3}", c.size, c.decay);
        assert!(mono >= -3.5, "size {} decay {}: mono fold {mono:.2} dB", c.size, c.decay);
    }
}

/// Mean peakiness of seeded, exponentially decaying white noise at `t60`
/// (the floor no colourless tail measures below; see the module docs).
fn noise_floor(t60: f32) -> f32 {
    let n = (1.3 * SR) as usize;
    let mut sum = 0.0;
    for seed in 1..=6u64 {
        let mut st = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
        let x: Vec<f32> = (0..n)
            .map(|i| {
                st ^= st << 13;
                st ^= st >> 7;
                st ^= st << 17;
                let u = (st >> 40) as f32 / (1u64 << 24) as f32 - 0.5;
                u * (-6.9 * i as f32 / SR / t60).exp()
            })
            .collect();
        sum += modal_peakiness_db(&x, SR, PEAKINESS_START_S, 100.0, 8_000.0).unwrap();
    }
    sum / 6.0
}

#[test]
fn the_tail_is_less_peaky_than_classic() {
    // The grid's sizes, plus size 0.2 (where Classic is peakiest).
    let mut rows = Vec::new();
    for c in grid() {
        rows.push((c.size, c.decay, c.hall.peakiness_db.unwrap(), c.classic.peakiness_db.unwrap()));
    }
    for &decay in &DECAYS {
        let s = hall(0.2, decay);
        let (l, r) = s.impulse(1.3);
        assert_not_silent(&format!("hall 0.2/{decay}"), &l, &r);
        let hall = mean_peakiness(&l, &r);
        let (l, r) = s.on(Algorithm::Classic).impulse(1.3);
        rows.push((0.2, decay, hall, mean_peakiness(&l, &r)));
    }
    println!("\nsize decay  Hall  Classic  floor  limit (dB peakiness)");
    let (mut hall_sum, mut classic_sum, mut bound_by_classic) = (0.0, 0.0, 0);
    for &decay in &DECAYS {
        let floor = noise_floor(decay);
        for &(size, d, hall, classic) in rows.iter().filter(|r| r.1 == decay) {
            let limit = (classic - 3.0).max(floor + 2.0);
            if classic - 3.0 >= floor + 2.0 {
                bound_by_classic += 1;
            }
            println!("{size:>4.1} {d:>5.1} {hall:>5.1} {classic:>7.1} {floor:>6.1} {limit:>6.1}");
            assert!(
                hall <= limit,
                "size {size} decay {d}: Hall peakiness {hall:.1} dB > {limit:.1} \
                 (Classic {classic:.1}, noise floor {floor:.1})"
            );
            hall_sum += hall;
            classic_sum += classic;
        }
    }
    let n = rows.len() as f32;
    println!("mean: Hall {:.2}, Classic {:.2}", hall_sum / n, classic_sum / n);
    assert!(bound_by_classic >= 2, "no cell actually tested Classic − 3 dB");
    assert!(
        hall_sum / n <= classic_sum / n - 1.0,
        "Hall's mean peakiness {:.2} is not 1 dB under Classic's {:.2}",
        hall_sum / n,
        classic_sum / n
    );
}

#[test]
fn band_decays_follow_the_multipliers_within_15_percent() {
    // Crossovers two octaves from the measured bands (125 Hz and 8 kHz),
    // where the first-order shelves have reached their asymptote.
    for shape in [(1.5, 500.0, 0.5), (0.6, 500.0, 0.3)] {
        let s = Setup {
            damping: 2_000.0,
            low_mult: shape.0,
            low_xover: shape.1,
            high_mult: shape.2,
            ..hall(0.5, 3.0)
        };
        let (l, r) = s.impulse(t30_seconds(3.0 * shape.0.max(1.0)));
        assert_not_silent(&format!("bands {shape:?}"), &l, &r);
        let rep = ImpulseReport::analyze(&l, &r, SR);
        let band = |hz: f32| {
            let b = rep.bands.iter().find(|b| b.center_hz == hz).unwrap();
            b.times.t30.unwrap()
        };
        let (lo, mid, hi) = (band(125.0), band(1_000.0), band(8_000.0));
        let (want_lo, want_hi) = (3.0 * shape.0, 3.0 * shape.2);
        println!(
            "shape {shape:?}: 125 Hz {lo:.2} s (want {want_lo:.2}), 1 kHz {mid:.2} s (want 3.00), \
             8 kHz {hi:.2} s (want {want_hi:.2})"
        );
        assert!((lo / want_lo - 1.0).abs() <= 0.15, "{shape:?}: 125 Hz T30 {lo:.2}");
        assert!((hi / want_hi - 1.0).abs() <= 0.15, "{shape:?}: 8 kHz T30 {hi:.2}");
        assert!((mid / 3.0 - 1.0).abs() <= 0.07, "{shape:?}: 1 kHz T30 {mid:.2}");
    }
}

/// Time (s) of the loudest 10 ms of a response.
fn energy_peak_s(l: &[f32], r: &[f32]) -> f32 {
    let w = (0.01 * SR) as usize;
    let mut best = (0.0f32, 0usize);
    for k in 0..l.len() / w {
        let e: f32 = (k * w..(k + 1) * w).map(|i| l[i] * l[i] + r[i] * r[i]).sum();
        if e > best.0 {
            best = (e, k);
        }
    }
    best.1 as f32 * 0.01
}

#[test]
fn the_build_delays_density_and_the_late_energy_peak() {
    println!("\nsize build  dens0.9  late peak (ER off)");
    for size in [0.5, 0.9] {
        let mut dens = Vec::new();
        let mut peaks = Vec::new();
        for build in [0.0, 0.5, 1.0] {
            let s = Setup {
                build,
                ..hall(size, 2.5)
            };
            let (l, r) = s.impulse(0.7);
            assert_not_silent(&format!("build {build}"), &l, &r);
            let rep = ImpulseReport::analyze(&l, &r, SR);
            let d = rep.echo_density.time_to_reach(0.9).unwrap();
            // The late field alone: its energy envelope is what builds.
            let (l, r) = s.with(|s| s.er_level = 0.0).impulse(0.7);
            let p = energy_peak_s(&l, &r);
            println!("{size:>4.1} {build:>5.1}  {:>5.0} ms  {:>5.0} ms", d * 1e3, p * 1e3);
            dens.push(d);
            peaks.push(p);
        }
        assert!(
            (0.060..=0.150).contains(&dens[1]),
            "size {size}: echo density reaches 0.9 at {:.0} ms at the default build",
            dens[1] * 1e3
        );
        // At the default size the whole range is monotonic. At a large
        // size, build 0 injects all 16 lines within 20 ms and their first
        // returns arrive as separate lumps 38-190 ms later, which holds
        // the density near 0.85 until the second pass: the no-build end
        // is the fast, slightly grainy one there, so only the upper half
        // of the range is held to the order.
        let rises = if size == 0.5 {
            dens[0] <= dens[1] && dens[1] <= dens[2] && dens[2] - dens[0] >= 0.025
        } else {
            dens[2] - dens[1] >= 0.015
        };
        assert!(rises, "size {size}: density 0.9 times {dens:?} do not rise with the build");
        assert!(
            peaks[0] < peaks[1] && peaks[1] < peaks[2] && peaks[2] - peaks[0] >= 0.1,
            "size {size}: late-energy peaks {peaks:?} do not move later with the build"
        );
    }
}

// ---------------------------------------------------------------------------
// Wobble
// ---------------------------------------------------------------------------

/// RMS spread (cents) of the steady-state response to a 1 kHz sine at a
/// 10 s decay; see the module docs.
fn sine_spread_cents(alg: Algorithm, mod_depth: f32, mod_rate: f32) -> f32 {
    let s = Setup {
        mod_depth,
        mod_rate,
        ..hall(0.5, 10.0)
    };
    let mut d = s.on(alg).dsp();
    let hop = (0.001 * SR) as usize;
    let f = 1_000.0f64;
    let start = (3.0 * SR) as usize;
    let mut base = Vec::new();
    let (mut re, mut im) = (0.0f64, 0.0f64);
    for n in 0..(5.0 * SR) as usize {
        let ph = std::f64::consts::TAU * f * n as f64 / SR as f64;
        let x = 0.5 * ph.sin() as f32;
        let (l, _) = d.process(x, x, s.diffusion, 1.0);
        if n >= start {
            re += l as f64 * ph.cos();
            im -= l as f64 * ph.sin();
            if (n - start + 1).is_multiple_of(hop) {
                base.push((re, im));
                (re, im) = (0.0, 0.0);
            }
        }
    }
    let m = base.len();
    let (mut total, mut moment) = (0.0f64, 0.0f64);
    for k in -500..=500 {
        let df = k as f64 * 0.1;
        let (mut a, mut b) = (0.0f64, 0.0f64);
        for (j, &(x, y)) in base.iter().enumerate() {
            let w = 0.5 - 0.5 * (std::f64::consts::TAU * j as f64 / m as f64).cos();
            let (s, c) = (-std::f64::consts::TAU * df * j as f64 * 0.001).sin_cos();
            a += w * (x * c - y * s);
            b += w * (x * s + y * c);
        }
        let p = a * a + b * b;
        total += p;
        moment += p * df * df;
    }
    let spread_hz = (moment / total).sqrt();
    (1200.0 * (1.0 + spread_hz / f).log2()) as f32
}

#[test]
fn the_modulation_does_not_wobble_a_long_tail() {
    let hall = sine_spread_cents(Algorithm::Hall, 0.3, 1.0);
    let hall_max = sine_spread_cents(Algorithm::Hall, 1.0, 5.0);
    let classic = sine_spread_cents(Algorithm::Classic, 0.3, 1.0);
    println!(
        "\nsustained-sine spread at 10 s decay: Hall {hall:.2} cents (defaults), \
         {hall_max:.2} (depth 1, 5 Hz); Classic {classic:.2} (defaults)"
    );
    assert!(hall <= 2.0, "Hall at the default modulation spreads a sine {hall:.2} cents");
    assert!(hall_max <= 10.0, "Hall at full modulation spreads a sine {hall_max:.2} cents");
    assert!(hall * 3.0 <= classic, "Hall ({hall:.2}) is not well under Classic ({classic:.2})");
}

// ---------------------------------------------------------------------------
// For every algorithm: stability, freeze, reset, glides
// ---------------------------------------------------------------------------

/// One automated parameter: stepped or ramped to random targets.
struct Lane {
    lo: f32,
    hi: f32,
    value: f32,
    step: f32,
    left: u32,
}

impl Lane {
    fn new(lo: f32, hi: f32, value: f32) -> Self {
        Self {
            lo,
            hi,
            value,
            step: 0.0,
            left: 0,
        }
    }

    fn advance(&mut self, rng: &mut Rng) -> f32 {
        if self.left > 0 {
            self.value += self.step;
            self.left -= 1;
        } else {
            let roll = rng.next();
            if roll < 0.02 {
                self.value = rng.range(self.lo, self.hi);
            } else if roll < 0.04 {
                let blocks = rng.range(20.0, 400.0) as u32;
                self.step = (rng.range(self.lo, self.hi) - self.value) / blocks as f32;
                self.left = blocks;
            }
        }
        self.value
    }
}

#[test]
fn sixty_seconds_of_random_automation_stay_finite_and_bounded() {
    let mut rng = Rng(0x00C0_FFEE_5EED_0005);
    let mut d = hall(0.5, 2.0).dsp();
    let mut lanes = [
        Lane::new(0.0, 1.0, 0.5),         // size
        Lane::new(0.1, 30.0, 2.0),        // decay
        Lane::new(200.0, 20_000.0, 8e3),  // damping
        Lane::new(0.0, 1.0, 0.4),         // er_level
        Lane::new(0.0, 1.0, 0.5),         // er_time
        Lane::new(0.0, 5.0, 1.0),         // mod_rate
        Lane::new(0.0, 1.0, 0.3),         // mod_depth
        Lane::new(0.25, 4.0, 1.0),        // low_decay_mult
        Lane::new(50.0, 1_000.0, 250.0),  // low_xover
        Lane::new(0.05, 1.0, 0.5),        // high_decay_mult
        Lane::new(0.0, 1.0, 0.5),         // build
        Lane::new(0.0, 1.0, 0.8),         // diffusion
        Lane::new(0.0, 250.0, 0.0),       // predelay
    ];
    let mut frozen = false;
    let (mut peak, mut energy) = (0.0f32, 0.0f64);
    let mut noise_amp = 0.3f32;
    let blocks = (60.0 * SR) as usize / BLOCK;
    for block in 0..blocks {
        let v: Vec<f32> = lanes.iter_mut().map(|l| l.advance(&mut rng)).collect();
        d.set_size(v[0]);
        d.set_decay(v[1]);
        d.set_damping(v[2]);
        d.set_er_level(v[3]);
        d.set_er_time(v[4]);
        d.set_mod_rate(v[5]);
        d.set_mod_depth(v[6]);
        d.set_decay_shape(v[7], v[8], v[9]);
        d.set_build(v[10]);
        d.set_predelay(v[12]);
        if rng.next() < 0.004 {
            frozen = !frozen;
        }
        d.set_freeze(frozen);
        if rng.next() < 0.01 {
            // Noise segments of random level, with silences.
            noise_amp = if rng.next() < 0.3 { 0.0 } else { rng.range(0.0, 0.5) };
        }
        for i in 0..BLOCK {
            let mut x = noise_amp * (2.0 * rng.next() - 1.0);
            let mut y = noise_amp * (2.0 * rng.next() - 1.0);
            if rng.next() < 2e-5 {
                (x, y) = (1.0, -1.0);
            }
            let (a, b) = d.process(x, y, v[11], 1.0);
            assert!(
                a.is_finite() && b.is_finite(),
                "non-finite output at block {block}, sample {i}: {v:?}"
            );
            peak = peak.max(a.abs()).max(b.abs());
            energy += (a as f64).powi(2) + (b as f64).powi(2);
        }
    }
    let peak_db = 20.0 * peak.log10();
    println!("\nfuzz: peak {peak_db:+.1} dBFS, energy {:.1} dB", 10.0 * energy.log10());
    assert!(peak_db <= 24.0, "peak {peak_db:.1} dBFS over 60 s of automation");
    assert!(energy > 1.0, "the fuzz render was silent");
}

#[test]
fn freeze_holds_the_tail_for_sixty_seconds() {
    let s = hall(0.7, 5.0);
    let mut d = s.dsp();
    let mut rng = Rng(0x0F2E_E2E0);
    for _ in 0..SR as usize {
        let (x, y) = (0.5 * (2.0 * rng.next() - 1.0), 0.5 * (2.0 * rng.next() - 1.0));
        d.process(x, y, s.diffusion, 1.0);
    }
    d.set_freeze(true);
    // The input keeps playing: Freeze must mute it.
    let window = |d: &mut ReverbDsp, secs: f32, rng: &mut Rng| {
        let mut e = 0.0f64;
        for _ in 0..(secs * SR) as usize {
            let (x, y) = (0.5 * (2.0 * rng.next() - 1.0), 0.5 * (2.0 * rng.next() - 1.0));
            let (a, b) = d.process(x, y, s.diffusion, 1.0);
            e += (a as f64).powi(2) + (b as f64).powi(2);
        }
        e
    };
    // Settle: the input ramp, the build line and the ER drain, the
    // modulation fades out.
    window(&mut d, 0.5, &mut rng);
    let first = window(&mut d, 1.0, &mut rng);
    window(&mut d, 58.0, &mut rng);
    let last = window(&mut d, 1.0, &mut rng);
    let drift = 10.0 * (last / first).log10();
    println!("\nfreeze: 1 s energy {:.2} dB, drift over 59 s {drift:+.4} dB", 10.0 * first.log10());
    assert!(first > 1.0, "nothing was frozen ({first})");
    assert!(drift.abs() <= 0.1, "frozen tail drifted {drift:+.3} dB in 59 s");
}

/// Noise for 0.3 s, an impulse at 0.5 s, then silence.
fn probe(n: usize) -> (f32, f32) {
    let mut s = (n as u64).wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1) as u32;
    s ^= s >> 16;
    s = s.wrapping_mul(2_246_822_519);
    s ^= s >> 13;
    let v = (s >> 8) as f32 * (2.0 / (1 << 23) as f32) - 1.0;
    if n < (0.3 * SR) as usize {
        (0.4 * v, -0.3 * v)
    } else if n == (0.5 * SR) as usize {
        (1.0, 1.0)
    } else {
        (0.0, 0.0)
    }
}

fn run_probe(d: &mut ReverbDsp, from: usize, len: usize, diffusion: f32) -> Vec<f32> {
    let mut out = Vec::with_capacity(2 * len);
    for n in from..from + len {
        let (x, y) = probe(n);
        let (a, b) = d.process(x, y, diffusion, 1.0);
        out.push(a);
        out.push(b);
    }
    out
}

#[test]
fn reset_renders_exactly_like_a_fresh_engine() {
    let first = hall(0.3, 1.5);
    let then = Setup {
        size: 0.8,
        decay: 4.0,
        damping: 5_000.0,
        er_level: 0.6,
        er_time: 0.7,
        mod_rate: 2.0,
        mod_depth: 0.8,
        low_mult: 1.4,
        low_xover: 300.0,
        high_mult: 0.45,
        build: 0.9,
        diffusion: 0.6,
        ..hall(0.5, 2.0)
    };
    for freeze_after in [false, true] {
        let mut reused = first.dsp();
        run_probe(&mut reused, 0, (0.7 * SR) as usize, first.diffusion);
        // A retune mid-signal (size, ER and build glides in flight), then
        // the host's reset.
        then.apply(&mut reused);
        reused.set_freeze(freeze_after);
        run_probe(&mut reused, 0, 3_000, then.diffusion);
        reused.clear();

        let mut fresh = then.dsp();
        fresh.set_freeze(freeze_after);

        let mut a = run_probe(&mut reused, 0, 12_000, then.diffusion);
        let mut b = run_probe(&mut fresh, 0, 12_000, then.diffusion);
        // Release a frozen pair and keep comparing: the state behind the
        // freeze has to match too.
        reused.set_freeze(false);
        fresh.set_freeze(false);
        a.extend(run_probe(&mut reused, 12_000, (SR as usize) - 12_000, then.diffusion));
        b.extend(run_probe(&mut fresh, 12_000, (SR as usize) - 12_000, then.diffusion));

        let peak = b.iter().fold(0.0f32, |m, x| m.max(x.abs()));
        assert!(peak > 1e-3, "the reference render is silent (freeze {freeze_after})");
        let diff = a.iter().zip(&b).position(|(x, y)| x.to_bits() != y.to_bits());
        assert_eq!(diff, None, "reset differs from fresh (freeze {freeze_after}) at {diff:?}");
    }
}

/// Size, build and ER spacing swept together across their range under a
/// sustained 220 Hz sine, at block rate as the plugin's smoothers deliver
/// them. `switching.rs`'s pattern (largest step against a held render
/// plus a margin) with two changes: the held reference is rendered at
/// *both* ends of the sweep, and steps are taken over each render's peak
/// ([`Clicks`]). The steady level of a sine through a room depends on the
/// room, and a sweep drags the room's modes across the sine (the swept
/// level overshoots both ends'), so an absolute step bound would flag
/// level, not clicks. The glides (0.1 samples per sample) may add their
/// 10 % Doppler.
#[test]
fn size_build_and_er_sweeps_under_a_sustained_sine_do_not_click() {
    let from = Setup {
        build: 0.2,
        er_time: 0.3,
        ..hall(0.1, 2.0)
    };
    let to = Setup {
        build: 0.9,
        er_time: 0.8,
        ..hall(0.95, 2.0)
    };
    let (warm, window) = (SR as usize, SR as usize / 2);
    let held = |s: &Setup| {
        let mut d = s.dsp();
        run_sine(&mut d, s, 0, warm, |_, _| {});
        let (l, r) = run_sine(&mut d, s, warm, window, |_, _| {});
        Clicks::of(&l, &r).step
    };
    let held = held(&from).max(held(&to));
    let mut swept = from.dsp();
    run_sine(&mut swept, &from, 0, warm, |_, _| {});
    let (l, r) = run_sine(&mut swept, &from, warm, window, |d, k| {
        let t = k as f32 / window as f32;
        d.set_size(from.size + (to.size - from.size) * t);
        d.set_build(from.build + (to.build - from.build) * t);
        d.set_er_time(from.er_time + (to.er_time - from.er_time) * t);
    });
    let Clicks { step: sweep, peak, .. } = Clicks::of(&l, &r);
    println!("\nsweep: step/peak {sweep:.5} against {held:.5} held (peak {peak:.3})");
    assert!(peak > 0.05, "the swept render is near silent (peak {peak})");
    assert!(
        sweep <= 1.1 * held + 2e-3,
        "the sweep clicks: step/peak {sweep:.5} against {held:.5} held"
    );
}

/// Freeze engaged and released under a sustained sine: no click either
/// way, though the build line and the diffusers are still feeding the
/// loop when Freeze lands.
#[test]
fn freeze_engage_and_release_do_not_click() {
    assert_freeze_is_click_free(&hall(0.5, 2.0), FREEZE_CLICK_MARGIN);
    assert_freeze_is_click_free(
        &hall(0.9, 5.0).with(|s| s.build = 1.0),
        FREEZE_CLICK_MARGIN,
    );
}

#[test]
fn the_engine_allocates_about_two_megabytes_at_96k() {
    let bytes = HallEngine::new(96_000.0).buffer_bytes();
    let at48 = HallEngine::new(48_000.0).buffer_bytes();
    println!("\nHall buffers: {:.2} MiB at 96 kHz, {:.2} MiB at 48 kHz", mib(bytes), mib(at48));
    assert!(bytes <= 2_750_000, "Hall holds {bytes} bytes of buffers at 96 kHz");
}

fn mib(b: usize) -> f64 {
    b as f64 / (1024.0 * 1024.0)
}

#[test]
fn the_viz_getters_describe_the_engine() {
    let mut d = hall(0.5, 2.0).dsp();
    for n in 0..24_000 {
        let x = if n == 0 { 1.0 } else { 0.0 };
        d.process(x, x, 0.8, 1.0);
    }
    let delays = d.fdn_delay_ms();
    assert!(delays.windows(2).all(|w| w[0] < w[1]), "folded line lengths {delays:?}");
    assert!(delays[0] >= 25.0 && delays[7] <= 160.0, "size 0.5 lines {delays:?}");
    assert!(d.channel_energies().iter().all(|&e| e > 0.0), "a line holds no energy");
    let times = d.er_tap_times_ms();
    assert!((times[0].0 - 30.0).abs() < 0.5, "first ER tap at {} ms", times[0].0);
    assert!((times[11].0 - 120.0).abs() < 0.5, "last ER tap at {} ms", times[11].0);
    assert!(d.er_tap_gains().iter().all(|g| g.0.abs() + g.1.abs() > 0.0));
}

// ---------------------------------------------------------------------------
// Presets
// ---------------------------------------------------------------------------

/// The Hall presets (§6): on the Hall, in its voicing (bass ×1.2–1.5,
/// treble ×0.4–0.6), and sounding when rendered at their own settings.
#[test]
fn the_hall_presets_are_voiced_as_halls() {
    let presets = [
        ("Warm Hall", include_str!("../../presets/warm_hall.json")),
        ("Cathedral", include_str!("../../presets/cathedral.json")),
        ("Ambient Bloom", include_str!("../../presets/ambient_bloom.json")),
        ("String Hall", include_str!("../../presets/string_hall.json")),
    ];
    for (name, json) in presets {
        let v: serde_json::Value = serde_json::from_str(json).unwrap();
        assert_eq!(v["meta"]["name"], name);
        let p = &v["state"]["doc"]["params"];
        let f = |k: &str| p[k].as_f64().unwrap_or_else(|| panic!("{name}: no `{k}`")) as f32;
        assert_eq!(f("algorithm"), Algorithm::Hall as u8 as f32, "{name} is not on the Hall");
        let (lo, hi) = (f("low_decay_mult"), f("high_decay_mult"));
        assert!((1.2..=1.5).contains(&lo), "{name}: bass decay ×{lo}");
        assert!((0.4..=0.6).contains(&hi), "{name}: treble decay ×{hi}");
        let s = Setup {
            size: f("size"),
            decay: f("decay"),
            damping: f("damping"),
            er_level: f("er_level"),
            er_time: f("er_time"),
            mod_rate: f("mod_rate"),
            mod_depth: f("mod_depth"),
            low_mult: lo,
            low_xover: f("low_xover"),
            high_mult: hi,
            build: f("tail_build"),
            diffusion: f("diffusion"),
            ..hall(0.5, 2.0)
        };
        let (l, r) = s.impulse(1.0);
        assert_not_silent(name, &l, &r);
    }
}

// ---------------------------------------------------------------------------
// Goldens
// ---------------------------------------------------------------------------

fn scenarios() -> [Scenario; 3] {
    [
        // The defaults at a medium hall: ER pattern, build, onset.
        Scenario {
            name: "impulse_defaults",
            setup: hall(0.6, 3.0),
            predelay_ms: 0.0,
            frames: 12_000,
            input: impulse_lr,
            edit: None,
        },
        // Small, dark, no build, little diffusion, no modulation: the
        // other corner of every stage.
        Scenario {
            name: "impulse_small_dark_fast",
            setup: Setup {
                damping: 1_500.0,
                er_level: 0.9,
                er_time: 0.1,
                mod_depth: 0.0,
                low_mult: 0.7,
                low_xover: 400.0,
                high_mult: 0.2,
                build: 0.0,
                diffusion: 0.3,
                ..hall(0.05, 0.8)
            },
            predelay_ms: 0.0,
            frames: 9_600,
            input: impulse_lr,
            edit: None,
        },
        // A noise burst into a large, modulated, slow-building hall: the
        // SmoothRandom modulation and the absorption at a Hall voicing.
        Scenario {
            name: "burst_modulated",
            setup: Setup {
                damping: 4_500.0,
                er_level: 0.5,
                er_time: 0.8,
                mod_rate: 3.0,
                mod_depth: 1.0,
                low_mult: 1.4,
                low_xover: 300.0,
                high_mult: 0.45,
                build: 0.8,
                diffusion: 0.95,
                ..hall(0.95, 6.0)
            },
            predelay_ms: 0.0,
            frames: 16_800,
            input: burst,
            edit: None,
        },
    ]
}

#[test]
fn hall_output_is_bit_exact() {
    check_golden(
        "hall_golden.f32",
        &["RESONANCE_BLESS", "RESONANCE_BLESS_HALL_GOLDEN"],
        &scenarios(),
    );
}
