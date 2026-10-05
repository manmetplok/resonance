//! R3: the Plate engine (reverb-algorithms.md §4.4, §5.2).
//!
//! Every render goes through `ReverbDsp::with_engines(SR, &[Plate])`, a
//! bank holding only Plate (so this holds before Plate joins
//! `Algorithm::BUILT`), with every engine setter called explicitly: 100 %
//! wet by construction (`ReverbDsp` returns the wet signal), no pre-delay,
//! return EQ off, ER/tail balance centred.
//!
//! The acceptance tables print with
//!
//!     cargo test -p resonance-reverb --test algorithms plate -- --nocapture
//!
//! The golden is re-blessed with `RESONANCE_BLESS=1` (or
//! `RESONANCE_BLESS_PLATE=1` for this file alone).

use std::path::PathBuf;

use resonance_dsp_test_support as golden;
use resonance_metering::decay::ImpulseReport;
use resonance_reverb::dsp::{Algorithm, ReverbDsp};

const SR: f32 = 48_000.0;
const BLOCK: usize = 128;

/// Every engine setter's value, plus the per-sample diffusion and width.
#[derive(Clone, Copy, Debug)]
struct Voicing {
    size: f32,
    decay: f32,
    damping: f32,
    diffusion: f32,
    er_level: f32,
    er_time: f32,
    mod_rate: f32,
    mod_depth: f32,
    low_mult: f32,
    low_xover: f32,
    high_mult: f32,
    width: f32,
}

impl Voicing {
    /// The parameter defaults: global, not plate-specific.
    fn defaults() -> Self {
        Self {
            size: 0.5,
            decay: 2.0,
            damping: 8_000.0,
            diffusion: 0.8,
            er_level: 0.4,
            er_time: 0.5,
            mod_rate: 1.0,
            mod_depth: 0.3,
            low_mult: 1.0,
            low_xover: 250.0,
            high_mult: 0.5,
            width: 1.0,
        }
    }

    /// The Plate voicing of the factory plates (`Vocal Plate`): treble
    /// held to 0.8× above 8 kHz.
    fn plate() -> Self {
        Self {
            high_mult: 0.8,
            ..Self::defaults()
        }
    }
}

/// Call every setter, in the plugin's block order.
fn configure(d: &mut ReverbDsp, v: Voicing) {
    d.set_size(v.size);
    d.set_decay(v.decay);
    d.set_freeze(false);
    d.set_damping(v.damping);
    d.set_predelay(0.0);
    d.set_er_level(v.er_level);
    d.set_er_time(v.er_time);
    d.set_mod_rate(v.mod_rate);
    d.set_mod_depth(v.mod_depth);
    d.set_wet_filters(false, 600.0, false, 10_000.0, false);
    d.set_er_tail_balance(0.0);
    d.set_decay_shape(v.low_mult, v.low_xover, v.high_mult);
    d.set_build(0.5);
}

fn dsp(algorithm: Algorithm, v: Voicing) -> ReverbDsp {
    let mut d = ReverbDsp::with_engines(SR, &[algorithm]);
    configure(&mut d, v);
    d
}

/// Response to a unit impulse on both channels at sample 0.
fn impulse(algorithm: Algorithm, v: Voicing, seconds: f32) -> (Vec<f32>, Vec<f32>) {
    let mut d = dsp(algorithm, v);
    let n = (seconds * SR) as usize;
    let (mut l, mut r) = (Vec::with_capacity(n), Vec::with_capacity(n));
    for i in 0..n {
        let x = if i == 0 { 1.0 } else { 0.0 };
        let (a, b) = d.process(x, x, v.diffusion, v.width);
        l.push(a);
        r.push(b);
    }
    (l, r)
}

/// Long enough for a T30 fit (−35 dB at 0.58 × T60) with room for the
/// noise-floor truncation, and for the late metrics on short decays.
fn render_seconds(decay: f32) -> f32 {
    (1.4 * decay + 0.6).max(1.3)
}

/// Mean energy of the two channels, dB re a unit impulse.
fn energy_db(l: &[f32], r: &[f32]) -> f64 {
    let e: f64 = l.iter().chain(r).map(|&x| (x as f64) * (x as f64)).sum();
    10.0 * (e / 2.0).max(1e-30).log10()
}

/// The §5.1 silence guard: total response energy above −40 dB re a unit
/// impulse.
fn assert_not_silent(what: &str, l: &[f32], r: &[f32]) {
    let e = energy_db(l, r);
    assert!(
        e > -40.0,
        "{what}: response energy {e:.1} dB (silence guard)"
    );
}

/// Deterministic xorshift, uniform in `[0, 1)`.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> f32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 >> 40) as f32 / (1u64 << 24) as f32
    }

    fn gauss(&mut self) -> f32 {
        let (a, b) = (self.next().max(1e-7), self.next());
        (-2.0 * a.ln()).sqrt() * (std::f32::consts::TAU * b).cos()
    }
}

/// Exponentially decaying Gaussian noise at `t60`: the colourless
/// reference, scored by the same peakiness metric.
fn noise_ir(t60: f32, seconds: f32, seed: u64) -> (Vec<f32>, Vec<f32>) {
    let mut rng = Rng(seed);
    let n = (seconds * SR) as usize;
    let (mut l, mut r) = (Vec::with_capacity(n), Vec::with_capacity(n));
    for i in 0..n {
        let env = 10f32.powf(-3.0 * i as f32 / (t60 * SR));
        l.push(env * rng.gauss());
        r.push(env * rng.gauss());
    }
    (l, r)
}

/// Mean peakiness of decaying noise over four seeds (one seed scatters by
/// ±0.5 dB).
fn noise_peakiness(t60: f32) -> f32 {
    let seeds = [
        0x9e37_79b9_7f4a_7c15,
        0x2545_f491_4f6c_dd1d,
        0x1234_5678_9abc_def1,
        77,
    ];
    let sum: f32 = seeds
        .iter()
        .map(|&s| {
            let (l, r) = noise_ir(t60, 1.3, s);
            ImpulseReport::analyze(&l, &r, SR).peakiness_db.unwrap()
        })
        .sum();
    sum / seeds.len() as f32
}

fn t30_at(rep: &ImpulseReport, hz: f32) -> f32 {
    rep.bands
        .iter()
        .find(|b| b.center_hz == hz)
        .and_then(|b| b.times.t30)
        .unwrap_or_else(|| panic!("no T30 at {hz} Hz"))
}

/// §5.2's Plate row at one decay, sizes 0.5 and 1.0, the Plate voicing:
/// mid T30 within ±10 % of the knob, treble (8 kHz) ≥ 0.7 × mid, echo
/// density 0.9 by 10 ms, late IACC ≤ 0.3, mono fold no worse than −3.5 dB.
fn plate_row(decay: f32) {
    for size in [0.5, 1.0] {
        let v = Voicing {
            size,
            decay,
            ..Voicing::plate()
        };
        let (l, r) = impulse(Algorithm::Plate, v, render_seconds(decay));
        let rep = ImpulseReport::analyze(&l, &r, SR);
        let mid = rep.mid_t30().expect("mid T30");
        let treble = t30_at(&rep, 8_000.0) / mid;
        let dens = rep.echo_density.time_to_reach(0.9);
        println!(
            "Plate size {size:.1} decay {decay:>3.1}s: mid {mid:.3}s ({:+.1} %), \
             8k/mid {treble:.2}\n  {}\n  {rep}",
            100.0 * (mid / decay - 1.0),
            ImpulseReport::table_header()
        );

        let what = format!("size {size} decay {decay}");
        assert!(rep.finite, "{what}: non-finite output");
        assert_not_silent(&what, &l, &r);
        assert!(
            (mid / decay - 1.0).abs() <= 0.10,
            "{what}: mid T30 {mid:.3} s is not within 10 % of the knob"
        );
        assert!(treble >= 0.7, "{what}: treble/mid {treble:.2} < 0.7");
        let dens = dens.expect("echo density never reaches 0.9");
        assert!(
            dens <= 0.010,
            "{what}: echo density reaches 0.9 at {:.1} ms",
            dens * 1e3
        );
        let iacc = rep.late_iacc.unwrap();
        assert!(iacc <= 0.3, "{what}: late IACC {iacc:.3}");
        let mono = rep.mono_fold_db.unwrap();
        assert!(mono >= -3.5, "{what}: mono fold {mono:.2} dB");
    }
}

#[test]
fn plate_row_decay_0_5s() {
    plate_row(0.5);
}

#[test]
fn plate_row_decay_1_5s() {
    plate_row(1.5);
}

#[test]
fn plate_row_decay_3s() {
    plate_row(3.0);
}

#[test]
fn plate_row_decay_6s() {
    plate_row(6.0);
}

/// The numbers at the *global* parameter defaults (treble 0.5×, the
/// voicing's job is the presets'). Only the knob is asserted here; the
/// rest is the report.
#[test]
fn plate_at_the_global_defaults() {
    let v = Voicing::defaults();
    let (l, r) = impulse(Algorithm::Plate, v, render_seconds(v.decay));
    let rep = ImpulseReport::analyze(&l, &r, SR);
    let mid = rep.mid_t30().unwrap();
    println!(
        "Plate at the global defaults: mid {mid:.3}s, 8k/mid {:.2}, 125/mid {:.2}\n  {}\n  {rep}",
        t30_at(&rep, 8_000.0) / mid,
        t30_at(&rep, 125.0) / mid,
        ImpulseReport::table_header()
    );
    assert!((mid / v.decay - 1.0).abs() <= 0.10, "mid T30 {mid:.3}");
    assert_not_silent("defaults", &l, &r);
}

/// Peakiness against Classic at the baseline rows' size/decay/damping
/// (both engines at the global defaults otherwise, Classic computed
/// here). The spec asks for Classic − 2 dB; where that is below the
/// metric's own floor — decaying Gaussian noise at the same T60, which no
/// tail can beat — the bound is that floor + 1 dB instead (see the
/// report: at size 0.9 / 2 s Classic reads 5.6 dB and noise 5.4 dB).
#[test]
fn plate_is_less_coloured_than_classic() {
    let rows = [
        (0.2, 0.5, 8_000.0),
        (0.5, 2.0, 8_000.0),
        (0.9, 2.0, 8_000.0),
        (0.2, 8.0, 8_000.0),
        (0.5, 8.0, 8_000.0),
        (0.5, 8.0, 20_000.0),
    ];
    println!("size decay damp   Plate  Classic  noise  IACC(P)");
    for (size, decay, damping) in rows {
        let v = Voicing {
            size,
            decay,
            damping,
            ..Voicing::defaults()
        };
        let pk = |a| {
            let (l, r) = impulse(a, v, 1.3);
            let rep = ImpulseReport::analyze(&l, &r, SR);
            (rep.peakiness_db.unwrap(), rep.late_iacc.unwrap())
        };
        let (p, iacc) = pk(Algorithm::Plate);
        let (c, _) = pk(Algorithm::Classic);
        let n = noise_peakiness(decay);
        println!("{size:>4.1} {decay:>4.1}s {damping:>6.0} {p:>6.1} {c:>8.1} {n:>6.1} {iacc:>7.3}");
        let bound = (c - 2.0).max(n + 1.0);
        assert!(
            p <= bound,
            "size {size} decay {decay}: peakiness {p:.1} dB > {bound:.1} \
             (Classic {c:.1}, noise {n:.1})"
        );
        assert!(iacc <= 0.3, "size {size} decay {decay}: IACC {iacc:.3}");
    }
}

/// Bass and treble decay multipliers do what they say: the 125 Hz and
/// 8 kHz band T30s over the 1 kHz one are within ±15 % of the requested
/// multipliers. The crossovers sit two octaves from the measured bands
/// (500 Hz, 2 kHz) so the first-order shelves are near their asymptotes,
/// and their geometric mean is 1 kHz, where the absorption is exact.
/// (Not both shelves deep at once, e.g. ×0.5 and ×0.25 4× apart: there
/// `Absorption`'s mid compensation would need a broadband gain above 1,
/// which it caps to keep the loop passive, so the mid then decays fast —
/// a documented limit of the R2 primitive, not of the plate.)
#[test]
fn bass_and_treble_multipliers_do_what_they_say() {
    for (low, high) in [(2.0, 0.5), (0.5, 0.5), (1.5, 1.0), (1.0, 0.3)] {
        let v = Voicing {
            decay: 2.0,
            low_mult: low,
            low_xover: 500.0,
            high_mult: high,
            damping: 2_000.0,
            ..Voicing::plate()
        };
        let (l, r) = impulse(Algorithm::Plate, v, render_seconds(2.0 * low.max(1.0)));
        let rep = ImpulseReport::analyze(&l, &r, SR);
        let mid = t30_at(&rep, 1_000.0);
        let bass = t30_at(&rep, 125.0) / mid;
        let treble = t30_at(&rep, 8_000.0) / mid;
        println!(
            "low ×{low} high ×{high}: 1k {mid:.3}s, 125/1k {bass:.2}, 8k/1k {treble:.2}\n  {rep}"
        );
        assert!((mid / 2.0 - 1.0).abs() <= 0.10, "1 kHz T30 {mid:.3}");
        assert!(
            (bass / low - 1.0).abs() <= 0.15,
            "bass ×{low} measured ×{bass:.2}"
        );
        assert!(
            (treble / high - 1.0).abs() <= 0.15,
            "treble ×{high} measured ×{treble:.2}"
        );
    }
}

/// The onset (`er_*`) is the first ~50 ms: with the tail balanced out,
/// what is left after 50 ms is 40 dB below the onset's total.
#[test]
fn the_onset_is_the_first_50_ms() {
    let v = Voicing::plate();
    let mut d = dsp(Algorithm::Plate, v);
    d.set_er_tail_balance(-1.0);
    let n = (0.3 * SR) as usize;
    let (mut l, mut r) = (Vec::new(), Vec::new());
    for i in 0..n {
        let x = if i == 0 { 1.0 } else { 0.0 };
        let (a, b) = d.process(x, x, v.diffusion, v.width);
        l.push(a);
        r.push(b);
    }
    assert_not_silent("onset", &l, &r);
    let cut = (0.05 * SR) as usize;
    let total = energy_db(&l, &r);
    let late = energy_db(&l[cut..], &r[cut..]);
    assert!(
        late < total - 40.0,
        "onset after 50 ms at {:.1} dB re total",
        late - total
    );
}

/// 60 s of random automation of every engine setter, as steps and ramps,
/// over noise with impulses: finite throughout, never above +24 dBFS.
#[test]
fn plate_survives_60_s_of_random_automation() {
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
    let mut rng = Rng(0x5eed_f00d_cafe_d00d);
    // Normalised position, ramp target and remaining ramp blocks per param.
    let mut pos: [f32; 11] = std::array::from_fn(|_| rng.next());
    let mut target = pos;
    let mut left = [0u32; 11];
    let mut freeze = false;

    let mut d = ReverbDsp::with_engines(SR, &[Algorithm::Plate]);
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
        d.set_decay_shape(p(7), p(8), p(9));
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
    println!("random automation: peak {peak_db:+.1} dBFS");
    assert!(peak_db <= 24.0, "peak {peak_db:+.1} dBFS");
    assert!(peak > 1e-3, "the fuzz rendered silence");
}

/// Freeze held for 60 s: the tail's energy drifts by at most 0.1 dB.
#[test]
fn freeze_holds_the_tail_for_60_s() {
    let v = Voicing::plate();
    let mut d = dsp(Algorithm::Plate, v);
    let mut rng = Rng(42);
    for _ in 0..(SR as usize) {
        let x = 0.3 * rng.gauss();
        d.process(x, 0.5 * x, v.diffusion, v.width);
    }
    d.set_freeze(true);
    let window = (4.0 * SR) as usize;
    let mut energies = Vec::new();
    let mut acc = 0.0f64;
    for i in 0..(61.0 * SR) as usize {
        let (l, r) = d.process(0.0, 0.0, v.diffusion, v.width);
        // Skip the first second: the onset and input diffusers ring out.
        if i >= SR as usize {
            acc += (l as f64).powi(2) + (r as f64).powi(2);
            if (i + 1 - SR as usize).is_multiple_of(window) {
                energies.push(acc);
                acc = 0.0;
            }
        }
    }
    let first = energies[0];
    assert!(first > 1e-3, "nothing frozen ({first:.2e})");
    let worst = energies
        .iter()
        .map(|e| 10.0 * (e / first).log10())
        .fold(0.0f64, |m, db| if db.abs() > m.abs() { db } else { m });
    println!("freeze: worst 4 s window over 60 s {worst:+.3} dB re the first");
    assert!(worst.abs() <= 0.1, "freeze drifted {worst:+.3} dB");
}

/// A cleared engine renders bit-identically to a fresh one configured
/// with the same values, after a history with a size glide, a decay
/// change, Freeze, and the modulators running.
#[test]
fn reset_equals_fresh() {
    let v = Voicing {
        mod_depth: 0.8,
        mod_rate: 3.0,
        ..Voicing::plate()
    };
    let mut used = dsp(Algorithm::Plate, v);
    let mut rng = Rng(7);
    for i in 0..(1.5 * SR) as usize {
        if i == 12_000 {
            used.set_size(0.9);
            used.set_decay(4.0);
        }
        if i == 30_000 {
            used.set_freeze(true);
        }
        if i == 40_000 {
            used.set_freeze(false);
            used.set_size(0.3);
            used.set_decay(2.5);
        }
        let x = 0.3 * rng.gauss();
        used.process(x, -x, v.diffusion, v.width);
    }
    used.clear();

    let mut fresh = dsp(Algorithm::Plate, v);
    fresh.set_size(0.3);
    fresh.set_decay(2.5);

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

/// The tank view reads the plate: eight segment lengths that follow
/// `size`, live segment energies, and no ER taps.
#[test]
fn the_viz_getters_describe_the_tank() {
    let mut d = dsp(Algorithm::Plate, Voicing::plate());
    let small = d.fdn_delay_ms();
    // The paper's D1 of branch A, 4453 samples at 29.761 kHz, at 1×.
    assert!((small[1] - 149.6).abs() < 0.5, "D1A {:.2} ms", small[1]);
    for i in 0..48_000 {
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
    let mut big = dsp(
        Algorithm::Plate,
        Voicing {
            size: 1.0,
            ..Voicing::plate()
        },
    );
    big.process(0.0, 0.0, 0.8, 1.0);
    assert!((big.fdn_delay_ms()[1] / small[1] - 1.5).abs() < 0.01);
}

// ---------------------------------------------------------------------
// Golden

/// A parameter edit applied at a frame.
type Edit = (usize, fn(&mut ReverbDsp));

/// One golden scenario: every setter pinned, an input, a length.
struct Scenario {
    name: &'static str,
    voicing: Voicing,
    predelay_ms: f32,
    frames: usize,
    input: fn(usize) -> (f32, f32),
    /// A parameter edit at a frame (the burst pins a size glide).
    edit: Option<Edit>,
}

fn noise(n: usize) -> f32 {
    let mut s = (n as u64)
        .wrapping_mul(6_364_136_223_846_793_005)
        .wrapping_add(1) as u32;
    s ^= s >> 16;
    s = s.wrapping_mul(2_246_822_519);
    s ^= s >> 13;
    (s >> 8) as f32 * (2.0 / (1 << 23) as f32) - 1.0
}

fn scenarios() -> Vec<Scenario> {
    vec![
        // The Vocal Plate voicing on an impulse, L at 0 and R at 37: the
        // onset cascade, the input diffusers and the first 250 ms of the
        // tank, tap by tap.
        Scenario {
            name: "impulse_vocal_voicing",
            voicing: Voicing {
                size: 0.35,
                decay: 1.8,
                damping: 8_000.0,
                diffusion: 0.85,
                er_level: 0.45,
                er_time: 0.3,
                mod_rate: 1.2,
                mod_depth: 0.3,
                low_mult: 1.0,
                low_xover: 250.0,
                high_mult: 0.8,
                width: 1.0,
            },
            predelay_ms: 0.0,
            frames: 12_000,
            input: |n| {
                (
                    if n == 0 { 1.0 } else { 0.0 },
                    if n == 37 { 1.0 } else { 0.0 },
                )
            },
            edit: None,
        },
        // A 15 ms noise burst into a big, heavily modulated plate with a
        // shaped decay, a pre-delay and a narrowed width; at 150 ms the
        // size drops, so the glide's fractional reads are pinned too.
        Scenario {
            name: "burst_modulated_glide",
            voicing: Voicing {
                size: 0.8,
                decay: 4.0,
                damping: 6_000.0,
                diffusion: 0.95,
                er_level: 0.3,
                er_time: 0.7,
                mod_rate: 3.0,
                mod_depth: 1.0,
                low_mult: 1.4,
                low_xover: 400.0,
                high_mult: 0.6,
                width: 0.8,
            },
            predelay_ms: 10.0,
            frames: 14_400,
            input: |n| {
                if n < 720 {
                    let w = 0.5 - 0.5 * (std::f32::consts::TAU * n as f32 / 720.0).cos();
                    (0.8 * w * noise(n), 0.8 * w * noise(n + 9_973))
                } else {
                    (0.0, 0.0)
                }
            },
            edit: Some((7_200, |d| d.set_size(0.5))),
        },
    ]
}

/// The scenario's output (L block then R block) and its input energy.
fn render_scenario(s: &Scenario) -> (Vec<f32>, Vec<f32>, f64) {
    let v = s.voicing;
    let mut d = dsp(Algorithm::Plate, v);
    d.set_predelay(s.predelay_ms);
    let (mut l, mut r) = (Vec::with_capacity(s.frames), Vec::with_capacity(s.frames));
    let mut e_in = 0.0f64;
    for n in 0..s.frames {
        if let Some((at, edit)) = s.edit {
            if n == at {
                edit(&mut d);
            }
        }
        let (x, y) = (s.input)(n);
        e_in += 0.5 * ((x as f64).powi(2) + (y as f64).powi(2));
        let (a, b) = d.process(x, y, v.diffusion, v.width);
        l.push(a);
        r.push(b);
    }
    (l, r, e_in)
}

fn golden_path() -> PathBuf {
    golden::golden_path(env!("CARGO_MANIFEST_DIR"), "plate_golden.f32")
}

#[test]
fn plate_golden_is_bit_exact() {
    let mut rendered = Vec::new();
    for s in scenarios() {
        let (l, r, e_in) = render_scenario(&s);
        assert!(
            l.iter().chain(&r).all(|x| x.is_finite()),
            "{}: non-finite",
            s.name
        );
        // Silence guard, re the scenario's own input energy.
        let e = energy_db(&l, &r) - 10.0 * e_in.log10();
        assert!(
            e > -40.0,
            "{}: {e:.1} dB re its input (silence guard)",
            s.name
        );
        let tail = s.frames * 3 / 4;
        let tail_db = energy_db(&l[tail..], &r[tail..]) - 10.0 * e_in.log10();
        assert!(
            tail_db > -60.0,
            "{}: no tail in the last quarter ({tail_db:.1} dB)",
            s.name
        );
        rendered.extend(l);
        rendered.extend(r);
    }

    let path = golden_path();
    if golden::blessed(&["RESONANCE_BLESS", "RESONANCE_BLESS_PLATE"]) {
        golden::bless_f32(&path, &rendered);
        return;
    }
    let want = golden::load_golden_f32(&path, rendered.len(), "RESONANCE_BLESS=1");
    let diff = golden::compare_f32(&rendered, &want);
    if let Some((i, got, want)) = diff.first_diff {
        panic!(
            "Plate output changed: {}/{} samples differ, peak delta {:.3e}; first at \
             sample {i} (got {got:?}, want {want:?}). Re-bless with RESONANCE_BLESS=1 \
             only for an intended change.",
            diff.diff_count,
            rendered.len(),
            diff.max_abs,
        );
    }
}
