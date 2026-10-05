//! R8 Shimmer acceptance (reverb-algorithms.md §4.4, §5.2 "Shimmer" and
//! "for every algorithm").
//!
//! Every render goes through `ReverbDsp::with_engines(SR, &[Shimmer])`
//! with every setter and `set_extras` called explicitly, 100 % wet, no
//! pre-delay, return EQ off, ER/tail balance centred, width 1. Numbers
//! print with `--nocapture`:
//!
//!     cargo test -p resonance-reverb --test algorithms shimmer -- --nocapture

use std::path::PathBuf;

use resonance_dsp_test_support as golden;
use resonance_metering::decay::{modal_peakiness_db, ImpulseReport};
use resonance_reverb::dsp::algo::shimmer::shifter::{
    max_read_weight, ratio, PitchShifter, ReadWeights, SEMITONES,
};
use resonance_reverb::dsp::algo::shimmer::ShimmerEngine;
use resonance_reverb::dsp::{Algorithm, Extras, ReverbDsp};

const SR: f32 = 48_000.0;
const BLOCK: usize = 128;
const TAU: f32 = std::f32::consts::TAU;

/// Every engine setter's value for one render.
#[derive(Clone, Copy, Debug)]
struct Set {
    size: f32,
    decay: f32,
    damping: f32,
    er_level: f32,
    er_time: f32,
    mod_rate: f32,
    mod_depth: f32,
    /// `(low_decay_mult, low_xover, high_decay_mult)`.
    shape: (f32, f32, f32),
    build: f32,
    diffusion: f32,
    semitones: f32,
    amount: f32,
}

impl Set {
    /// The plugin's defaults at `size`/`decay`, shimmering at `semitones`
    /// with `amount`.
    fn at(size: f32, decay: f32, semitones: f32, amount: f32) -> Self {
        Self {
            size,
            decay,
            damping: 8_000.0,
            er_level: 0.4,
            er_time: 0.5,
            mod_rate: 1.0,
            mod_depth: 0.3,
            shape: (1.0, 250.0, 0.5),
            build: 0.5,
            diffusion: 0.8,
            semitones,
            amount,
        }
    }

    fn extras(&self) -> Extras {
        Extras {
            shimmer_semitones: self.semitones,
            shimmer_amount: self.amount,
            ..Extras::default()
        }
    }
}

fn configure(d: &mut ReverbDsp, s: &Set) {
    d.set_size(s.size);
    d.set_decay(s.decay);
    d.set_freeze(false);
    d.set_damping(s.damping);
    d.set_predelay(0.0);
    d.set_er_level(s.er_level);
    d.set_er_time(s.er_time);
    d.set_mod_rate(s.mod_rate);
    d.set_mod_depth(s.mod_depth);
    d.set_decay_shape(s.shape.0, s.shape.1, s.shape.2);
    d.set_build(s.build);
    d.set_extras(s.extras());
    d.set_wet_filters(false, 600.0, false, 10_000.0, false);
    d.set_er_tail_balance(0.0);
}

fn dsp(s: &Set) -> ReverbDsp {
    let mut d = ReverbDsp::with_engines(SR, &[Algorithm::Shimmer]);
    configure(&mut d, s);
    d
}

/// Unit impulse on both channels at sample 0.
fn ir(s: &Set, secs: f32) -> (Vec<f32>, Vec<f32>) {
    let mut d = dsp(s);
    let n = (secs * SR) as usize;
    let (mut l, mut r) = (Vec::with_capacity(n), Vec::with_capacity(n));
    for i in 0..n {
        let x = if i == 0 { 1.0 } else { 0.0 };
        let (a, b) = d.process(x, x, s.diffusion, 1.0);
        l.push(a);
        r.push(b);
    }
    (l, r)
}

fn energy_db(l: &[f32], r: &[f32]) -> f64 {
    let e: f64 = l.iter().chain(r).map(|&x| (x as f64) * (x as f64)).sum();
    10.0 * (e / 2.0).max(1e-30).log10()
}

/// The silence guard of §5.1: total IR energy above −40 dB re a unit
/// impulse, and finite.
fn guard(what: &str, l: &[f32], r: &[f32]) {
    let e = energy_db(l, r);
    assert!(e > -40.0, "{what}: IR energy {e:.1} dB (silence guard)");
    assert!(l.iter().chain(r).all(|x| x.is_finite()), "{what}: non-finite output");
}

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> f32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 >> 40) as f32 / (1u64 << 24) as f32
    }

    fn bipolar(&mut self) -> f32 {
        2.0 * self.next() - 1.0
    }

    fn range(&mut self, lo: f32, hi: f32) -> f32 {
        lo + (hi - lo) * self.next()
    }
}

// ---------------------------------------------------------------------------
// The shifter and its cap
// ---------------------------------------------------------------------------

/// The loop's cap per pitch is `0.97/√c_max`, and `c_max` bounds the
/// shifter's energy gain on any input: checked against impulses at every
/// alignment of a sweep, noise, sines and sparse random bursts.
#[test]
fn the_shifter_gain_is_inside_the_cap() {
    let weights = ReadWeights::new(SR);
    println!("\npitch  c_max  cap   worst measured gain² (rel. c_max)");
    for &st in &SEMITONES {
        let c = weights.c_max(st);
        let cap = (0.97 / c.sqrt()).min(1.0);
        assert!(c >= 1.0 && c <= 4.0, "{st:+}: c_max {c}");
        // The bound is a geometric fact of this rate; a second rate gets
        // its own (computed, not tabulated).
        let c44 = max_read_weight(ratio(st), 44_100.0);
        assert!((1.0..=4.0).contains(&c44), "{st:+} at 44.1 kHz: c_max {c44}");

        let gain2 = |x: &[f32]| {
            let mut s = PitchShifter::new(SR, 0.0);
            s.set_ratio(ratio(st));
            let (mut ein, mut eout) = (0.0f64, 0.0f64);
            for i in 0..x.len() + 6_000 {
                let v = x.get(i).copied().unwrap_or(0.0);
                ein += (v as f64).powi(2);
                let y = s.process(v) as f64;
                eout += y * y;
                // The prefix form of the bound, at every sample.
                assert!(
                    eout <= c as f64 * ein * 1.000_01 + 1e-12,
                    "{st:+}: prefix energy {eout} > c_max·{ein} at {i}"
                );
            }
            eout / ein
        };
        let mut worst = 0.0f64;
        // Impulses at many alignments over a sweep (and fractional
        // positions between them, via adjacent pairs).
        for start in (0..6_000).step_by(37) {
            let mut x = vec![0.0f32; start + 1];
            x[start] = 1.0;
            worst = worst.max(gain2(&x));
        }
        let mut rng = Rng(0xA11CE ^ st.to_bits() as u64);
        for _ in 0..4 {
            let x: Vec<f32> = (0..9_000).map(|_| rng.bipolar()).collect();
            worst = worst.max(gain2(&x));
            let x: Vec<f32> =
                (0..9_000).map(|_| if rng.next() < 0.01 { rng.bipolar() } else { 0.0 }).collect();
            worst = worst.max(gain2(&x));
        }
        for f in [40.0, 220.0, 1_000.0, 6_000.0] {
            let x: Vec<f32> = (0..9_000).map(|n| (TAU * f * n as f32 / SR).sin()).collect();
            worst = worst.max(gain2(&x));
        }
        println!("{st:>+5} {c:>6.3} {cap:>5.3}  {worst:.3} ({:.2})", worst / c as f64);
        assert!(worst <= c as f64 * 1.000_01, "{st:+}: gain² {worst} over c_max {c}");
        assert!(cap as f64 * cap as f64 * worst < 1.0, "{st:+}: capped gain {worst}");
    }
    // An off-label pitch falls back to the geometric worst case.
    assert!(weights.c_max(3.0) >= 5.0);
}

/// The engine's cap is that bound.
#[test]
fn the_engine_caps_each_pitch() {
    let weights = ReadWeights::new(SR);
    let mut e = ShimmerEngine::new(SR);
    for &st in &SEMITONES {
        e.set_extras(&Extras {
            shimmer_semitones: st,
            shimmer_amount: 1.0,
            ..Extras::default()
        });
        e.process(0.0, 0.0, 0.8);
        let want = (0.97 / weights.c_max(st).sqrt()).min(1.0);
        assert_eq!(e.shift_cap(), want, "{st:+}");
    }
}

// ---------------------------------------------------------------------------
// Pitch: the tail grows energy at f·2^(s/12)
// ---------------------------------------------------------------------------

/// Power of `x` at `f` (Hann-windowed DFT bin, no FFT).
fn tone_power(x: &[f32], f: f32) -> f64 {
    let n = x.len();
    let (mut re, mut im) = (0.0f64, 0.0f64);
    for (i, &v) in x.iter().enumerate() {
        let w = 0.5 - 0.5 * (std::f64::consts::TAU * i as f64 / n as f64).cos();
        let ph = std::f64::consts::TAU * f as f64 * i as f64 / SR as f64;
        re += w * v as f64 * ph.cos();
        im -= w * v as f64 * ph.sin();
    }
    (re * re + im * im) / (n as f64 * n as f64)
}

/// Power of `x` within ±4 % of `f` (a 2 % grid of [`tone_power`] bins).
/// A delay-line shifter is exact in pitch within a grain, but the
/// crossfade from one grain to the next resets the phase, so a shifted
/// sine is a cluster of lines spaced at the sweep rate around the target,
/// not one bin.
fn band_power(x: &[f32], f: f32) -> f64 {
    let bin = SR / x.len() as f32;
    let k = (0.04 * f / bin).ceil() as i32;
    (-k..=k).map(|j| tone_power(x, f + j as f32 * bin)).sum()
}

/// A sustained sine at `f` for `secs`; returns the left output.
fn sine_render(s: &Set, f: f32, secs: f32) -> Vec<f32> {
    let mut d = dsp(s);
    (0..(secs * SR) as usize)
        .map(|n| {
            let x = 0.3 * (TAU * f * n as f32 / SR).sin();
            d.process(x, x, s.diffusion, 1.0).0
        })
        .collect()
}

/// A sustained 330 Hz sine for 4 s at an 8 s decay, amount 0.6: for
/// every pitch the output holds a tone cluster at `330·2^(s/12)` (band
/// power within ±4 %, see [`band_power`]) far above the same room at
/// amount 0, and it grows as the loop recirculates. For +12 the octave's
/// octave (4f) is there too.
#[test]
fn a_sustained_sine_grows_a_tone_at_the_shifted_pitch() {
    let f = 330.0;
    let window = |x: &[f32], from: f32| {
        let a = (from * SR) as usize;
        x[a..a + SR as usize / 2].to_vec()
    };
    println!("\npitch  target Hz  dB over amount 0 (1 s, 3.5 s)  growth  re sine");
    for &st in &SEMITONES {
        let target = f * ratio(st);
        let on = sine_render(&Set::at(0.6, 8.0, st, 0.6), f, 4.0);
        let off = sine_render(&Set::at(0.6, 8.0, st, 0.0), f, 4.0);
        let rel = |from: f32, hz: f32| {
            let p = band_power(&window(&on, from), hz);
            let q = band_power(&window(&off, from), hz);
            10.0 * (p / q.max(1e-30)).log10()
        };
        let early = band_power(&window(&on, 1.0), target);
        let late = band_power(&window(&on, 3.5), target);
        let main = band_power(&window(&on, 3.5), f);
        let (r1, r35) = (rel(1.0, target), rel(3.5, target));
        let growth = 10.0 * (late / early).log10();
        let level = 10.0 * (late / main).log10();
        print!("{st:>+5} {target:>9.1}   {r1:>+6.1} {r35:>+6.1}  {growth:>+6.1}  {level:>+6.1}");
        assert!(r35 >= 20.0, "{st:+}: {target:.0} Hz only {r35:+.1} dB over amount 0");
        assert!(growth >= 1.0, "{st:+}: the shifted tone does not grow ({growth:+.1} dB)");
        assert!(level >= -20.0, "{st:+}: the shifted tone is {level:+.1} dB under the sine");
        if st == 12.0 {
            // The repeat: an octave on the octave.
            let r4 = rel(3.5, 4.0 * f);
            print!("  4f {r4:+.1} dB");
            assert!(r4 >= 15.0, "+12: the second octave is only {r4:+.1} dB over amount 0");
        }
        println!();
    }
}

/// How much the shifted share shortens the tail. Shifted energy climbs
/// out through the low-pass, so the tail shortens as the amount rises;
/// it must do so monotonically, and stay a reverb tail at the moderate
/// amounts presets use.
#[test]
fn the_tail_shortens_with_the_amount_but_stays_a_tail() {
    println!("\namount  mid T30 at a 6 s decay, +12");
    let mut last = f32::INFINITY;
    for amount in [0.0, 0.15, 0.3, 0.5, 1.0] {
        let s = Set::at(0.7, 6.0, 12.0, amount);
        let (l, r) = ir(&s, 5.2);
        guard(&format!("amount {amount}"), &l, &r);
        let t = ImpulseReport::analyze(&l, &r, SR).mid_t30().unwrap_or(0.0);
        println!("{amount:>6.2}  {t:.2} s");
        assert!(t <= last * 1.02, "amount {amount}: T30 {t:.2} s rose");
        last = t;
        if amount <= 0.3 {
            assert!(t >= 0.6 * 6.0, "amount {amount}: T30 {t:.2} s");
        }
    }
}

#[test]
fn at_amount_zero_it_is_a_plain_hall() {
    println!("\namount 0: decay  midT30  err%   {}", ImpulseReport::table_header());
    for (size, decay) in [(0.5, 1.5), (0.6, 3.0), (0.9, 6.0)] {
        let s = Set::at(size, decay, 12.0, 0.0);
        let (l, r) = ir(&s, (0.75 * decay + 0.6).max(1.3));
        guard(&format!("amount 0 {size}/{decay}"), &l, &r);
        let rep = ImpulseReport::analyze(&l, &r, SR);
        let mid = rep.mid_t30().expect("no mid T30");
        let err = (mid - decay) / decay;
        println!("{decay:>15.1}s {mid:>7.3} {:>+6.1}  {rep}", 100.0 * err);
        assert!(err.abs() <= 0.07, "size {size} decay {decay}: mid T30 {mid:.3}");
    }
}

// ---------------------------------------------------------------------------
// No whistle
// ---------------------------------------------------------------------------

/// Octave-on-octave build-up must stay a wash, not a whistle (the classic
/// shimmer failure: the loop piles energy into one tone near the top of
/// its passband). A 20 s room at amount 1 / +12 is fed white noise for
/// 10 s; the modal peakiness (100 Hz–8 kHz, §5.1's metric) of the last
/// driven second and of the first second of the tail after the input
/// stops stays within 3 dB of the same room at amount 0, and under 12 dB.
#[test]
fn octaves_do_not_build_into_a_whistle() {
    let pk = |s: &Set| {
        let mut d = dsp(s);
        let mut rng = Rng(0x5EED_0F_AA);
        let n = (11.3 * SR) as usize;
        let (mut l, mut r) = (Vec::with_capacity(n), Vec::with_capacity(n));
        for i in 0..n {
            let x = if i < (10.0 * SR) as usize { 0.3 * rng.bipolar() } else { 0.0 };
            let (a, b) = d.process(x, x, s.diffusion, 1.0);
            l.push(a);
            r.push(b);
        }
        guard("whistle", &l, &r);
        let p = |x: &[f32], at: f32| modal_peakiness_db(x, SR, at, 100.0, 8_000.0).unwrap();
        (0.5 * (p(&l, 9.0) + p(&r, 9.0)), 0.5 * (p(&l, 10.2) + p(&r, 10.2)))
    };
    let shimmer = pk(&Set::at(0.7, 20.0, 12.0, 1.0));
    let plain = pk(&Set::at(0.7, 20.0, 12.0, 0.0));
    println!(
        "\npeakiness, 20 s decay, noise for 10 s (driven / tail): +12 amount 1 \
         {:.1} / {:.1} dB, amount 0 {:.1} / {:.1} dB",
        shimmer.0, shimmer.1, plain.0, plain.1
    );
    for (what, s, p) in [("driven", shimmer.0, plain.0), ("tail", shimmer.1, plain.1)] {
        assert!(s <= p + 3.0, "{what}: shimmer {s:.1} dB vs plain {p:.1} dB");
        assert!(s <= 12.0, "{what}: shimmer peakiness {s:.1} dB");
    }
}

// ---------------------------------------------------------------------------
// Stability and freeze
// ---------------------------------------------------------------------------

/// Freeze at +24 / amount 1, held for 120 s with the input still playing:
/// the frozen halo holds its energy (and the energy cannot grow at all).
#[test]
fn freeze_at_plus_24_holds_for_two_minutes() {
    let s = Set::at(0.7, 10.0, 24.0, 1.0);
    let mut d = dsp(&s);
    let mut rng = Rng(0x0F2E_E2E0);
    // The bloom: 3 s of noise with the shifter at full share.
    for _ in 0..(3.0 * SR) as usize {
        let (x, y) = (0.5 * rng.bipolar(), 0.5 * rng.bipolar());
        d.process(x, y, s.diffusion, 1.0);
    }
    d.set_freeze(true);
    let window = |d: &mut ReverbDsp, secs: f32, rng: &mut Rng, peak: &mut f32| {
        let mut e = 0.0f64;
        for _ in 0..(secs * SR) as usize {
            let (x, y) = (0.5 * rng.bipolar(), 0.5 * rng.bipolar());
            let (a, b) = d.process(x, y, s.diffusion, 1.0);
            assert!(a.is_finite() && b.is_finite());
            *peak = peak.max(a.abs()).max(b.abs());
            e += (a as f64).powi(2) + (b as f64).powi(2);
        }
        e
    };
    let mut peak = 0.0f32;
    window(&mut d, 0.5, &mut rng, &mut peak);
    let first = window(&mut d, 1.0, &mut rng, &mut peak);
    let mut worst = 0.0f64;
    for _ in 0..117 {
        let e = window(&mut d, 1.0, &mut rng, &mut peak);
        worst = worst.max((10.0 * (e / first).log10()).abs());
    }
    let last = window(&mut d, 1.0, &mut rng, &mut peak);
    let drift = 10.0 * (last / first).log10();
    println!(
        "\nfreeze +24/amount 1: 1 s energy {:.2} dB, drift over 119 s {drift:+.4} dB \
         (worst second {worst:.4} dB), peak {:.3}",
        10.0 * first.log10(),
        peak
    );
    assert!(first > 1.0, "nothing was frozen ({first})");
    assert!(drift.abs() <= 1.0, "frozen tail drifted {drift:+.3} dB");
    assert!(worst <= 1.0, "a second strayed {worst:.3} dB from the first");
}

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
    let mut rng = Rng(0x00C0_FFEE_5EED_0008);
    let mut d = dsp(&Set::at(0.5, 4.0, 12.0, 0.3));
    let mut lanes = [
        Lane::new(0.0, 1.0, 0.5),         // size
        Lane::new(0.1, 30.0, 4.0),        // decay
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
        Lane::new(0.0, 1.0, 0.3),         // shimmer_amount
    ];
    let mut frozen = false;
    let mut pitch = 0usize;
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
        if rng.next() < 0.01 {
            pitch = (rng.next() * SEMITONES.len() as f32) as usize % SEMITONES.len();
        }
        d.set_extras(Extras {
            shimmer_semitones: SEMITONES[pitch],
            shimmer_amount: v[13],
            ..Extras::default()
        });
        if rng.next() < 0.004 {
            frozen = !frozen;
        }
        d.set_freeze(frozen);
        if rng.next() < 0.01 {
            noise_amp = if rng.next() < 0.3 { 0.0 } else { rng.range(0.0, 0.5) };
        }
        for i in 0..BLOCK {
            let mut x = noise_amp * rng.bipolar();
            let mut y = noise_amp * rng.bipolar();
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

// ---------------------------------------------------------------------------
// Reset, clicks
// ---------------------------------------------------------------------------

/// Noise for 0.3 s, an impulse at 0.5 s, then silence.
fn probe(n: usize) -> (f32, f32) {
    let v = noise(n as u64);
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
    let first = Set::at(0.3, 1.5, 7.0, 0.4);
    let then = Set {
        size: 0.8,
        decay: 6.0,
        damping: 5_000.0,
        er_level: 0.6,
        er_time: 0.7,
        mod_rate: 2.0,
        mod_depth: 0.8,
        shape: (1.4, 300.0, 0.45),
        build: 0.9,
        diffusion: 0.6,
        semitones: -12.0,
        amount: 0.9,
    };
    for freeze_after in [false, true] {
        let mut reused = dsp(&first);
        run_probe(&mut reused, 0, (0.7 * SR) as usize, first.diffusion);
        // A retune mid-signal (glides, amount and cap slews in flight),
        // then the host's reset.
        configure(&mut reused, &then);
        reused.set_freeze(freeze_after);
        run_probe(&mut reused, 0, 3_000, then.diffusion);
        reused.clear();

        let mut fresh = dsp(&then);
        fresh.set_freeze(freeze_after);

        let mut a = run_probe(&mut reused, 0, 12_000, then.diffusion);
        let mut b = run_probe(&mut fresh, 0, 12_000, then.diffusion);
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

/// Largest sample-to-sample step and largest second difference on either
/// channel of an interleaved run, each over the run's peak.
fn steps_over_peak(interleaved: &[f32]) -> (f32, f32) {
    let peak = interleaved.iter().fold(0.0f32, |m, x| m.max(x.abs())).max(1e-9);
    let (mut d1, mut d2) = (0.0f32, 0.0f32);
    for ch in 0..2 {
        let side: Vec<f32> = interleaved.iter().skip(ch).step_by(2).copied().collect();
        for w in side.windows(3) {
            d1 = d1.max((w[1] - w[0]).abs());
            d2 = d2.max((w[2] - 2.0 * w[1] + w[0]).abs());
        }
    }
    (d1 / peak, d2 / peak)
}

/// A sustained 220 Hz sine through the engine, with `event(d, k)` called
/// at every block start `k` (samples into the window).
fn sine_window(
    s: &Set,
    warm: usize,
    len: usize,
    mut event: impl FnMut(&mut ReverbDsp, usize),
) -> Vec<f32> {
    let sine = |n: usize| 0.5 * (TAU * 220.0 * n as f32 / SR).sin();
    let mut d = dsp(s);
    for n in 0..warm {
        let x = sine(n);
        d.process(x, x, s.diffusion, 1.0);
    }
    let mut out = Vec::with_capacity(2 * len);
    for k in 0..len {
        if k % BLOCK == 0 {
            event(&mut d, k);
        }
        let x = sine(warm + k);
        let (a, b) = d.process(x, x, s.diffusion, 1.0);
        out.push(a);
        out.push(b);
    }
    out
}

/// Freeze engaged and released, `shimmer_pitch` stepped through every
/// label and `shimmer_amount` stepped 0 → 1, under a sustained sine:
/// neither the steps nor the second differences rise above those of the
/// held renders at the settings passed through (switching.rs's pattern,
/// over the peak as in the size-sweep tests: the level legitimately
/// moves, and a higher pitch legitimately steps more).
#[test]
fn freeze_and_pitch_switches_under_a_sustained_sine_do_not_click() {
    let s = Set::at(0.6, 4.0, 12.0, 0.8);
    let (warm, len) = (SR as usize, SR as usize);
    let max2 = |a: (f32, f32), b: (f32, f32)| (a.0.max(b.0), a.1.max(b.1));
    let held_at = |s: &Set| steps_over_peak(&sine_window(s, warm, len, |_, _| {}));
    let held = held_at(&s);
    let held_pitch = SEMITONES
        .iter()
        .map(|&st| held_at(&Set { semitones: st, ..s }))
        .fold((0.0, 0.0), max2);
    let held_amount = max2(held_at(&Set { amount: 0.0, ..s }), held_at(&Set { amount: 1.0, ..s }));
    let quarter = len / 4;
    let freeze = steps_over_peak(&sine_window(&s, warm, len, |d, k| {
        if k == quarter {
            d.set_freeze(true);
        } else if k == 3 * quarter {
            d.set_freeze(false);
        }
    }));
    let pitch = steps_over_peak(&sine_window(&s, warm, len, |d, k| {
        let i = (k * SEMITONES.len() / len) % SEMITONES.len();
        d.set_extras(Extras {
            shimmer_semitones: SEMITONES[(i + 1) % SEMITONES.len()],
            ..s.extras()
        });
    }));
    let amount = steps_over_peak(&sine_window(&s, warm, len, |d, k| {
        let a = if k < len / 2 { 0.0 } else { 1.0 };
        d.set_extras(Extras {
            shimmer_amount: a,
            ..s.extras()
        });
    }));
    println!("\nstep/peak, 2nd diff/peak (switched against held):");
    for (what, got, held) in [
        ("freeze", freeze, held),
        ("pitch", pitch, held_pitch),
        ("amount", amount, held_amount),
    ] {
        println!("  {what:<6} {:.5} {:.5}  held {:.5} {:.5}", got.0, got.1, held.0, held.1);
        assert!(got.0 <= 1.1 * held.0 + 2e-3, "{what} steps {:.5} vs {:.5} held", got.0, held.0);
        assert!(got.1 <= 1.1 * held.1 + 1e-3, "{what} 2nd diff {:.5} vs {:.5}", got.1, held.1);
    }
}

// ---------------------------------------------------------------------------
// Memory, viz, presets
// ---------------------------------------------------------------------------

#[test]
fn the_engine_allocates_about_three_megabytes_at_96k() {
    let bytes = ShimmerEngine::new(96_000.0).buffer_bytes();
    let at48 = ShimmerEngine::new(48_000.0).buffer_bytes();
    let mib = |b: usize| b as f64 / (1024.0 * 1024.0);
    println!("\nShimmer buffers: {:.2} MiB at 96 kHz, {:.2} MiB at 48 kHz", mib(bytes), mib(at48));
    assert!(bytes <= 3_000_000, "Shimmer holds {bytes} bytes of buffers at 96 kHz");
}

#[test]
fn the_viz_getters_describe_the_engine() {
    let mut d = dsp(&Set::at(0.5, 2.0, 12.0, 0.5));
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
    assert!(d.er_tap_gains().iter().all(|g| g.0.abs() + g.1.abs() > 0.0));
}

/// The Shimmer presets: on the Shimmer, at the pitch their name promises,
/// and sounding (with a shifted tone) at their own settings.
#[test]
fn the_shimmer_presets_are_voiced_as_shimmers() {
    let presets = [
        ("Shimmer Drone", include_str!("../../presets/shimmer_drone.json"), 0),
        ("Octave Halo", include_str!("../../presets/octave_halo.json"), 0),
        ("Fifth Bloom", include_str!("../../presets/fifth_bloom.json"), 1),
    ];
    for (name, json, pitch) in presets {
        let v: serde_json::Value = serde_json::from_str(json).unwrap();
        assert_eq!(v["meta"]["name"], name);
        let p = &v["state"]["doc"]["params"];
        let f = |k: &str| p[k].as_f64().unwrap_or_else(|| panic!("{name}: no `{k}`")) as f32;
        assert_eq!(f("algorithm"), Algorithm::Shimmer as u8 as f32, "{name} is not a Shimmer");
        assert_eq!(f("shimmer_pitch"), pitch as f32, "{name}: shimmer_pitch");
        let s = Set {
            size: f("size"),
            decay: f("decay"),
            damping: f("damping"),
            er_level: f("er_level"),
            er_time: f("er_time"),
            mod_rate: f("mod_rate"),
            mod_depth: f("mod_depth"),
            shape: (f("low_decay_mult"), f("low_xover"), f("high_decay_mult")),
            build: f("tail_build"),
            diffusion: f("diffusion"),
            semitones: SEMITONES[pitch],
            amount: f("shimmer_amount"),
        };
        assert!(s.amount >= 0.3, "{name}: shimmer_amount {}", s.amount);
        let (l, r) = ir(&s, 1.0);
        guard(name, &l, &r);
        let f0 = 330.0;
        let on = sine_render(&s, f0, 2.5);
        let off = sine_render(&Set { amount: 0.0, ..s }, f0, 2.5);
        let tail = |x: &[f32]| band_power(&x[2 * SR as usize..], f0 * ratio(s.semitones));
        let rel = 10.0 * (tail(&on) / tail(&off).max(1e-30)).log10();
        println!("{name}: shifted tone {rel:+.1} dB over the same room unshifted");
        assert!(rel >= 15.0, "{name}: shifted tone only {rel:+.1} dB");
    }
}

// ---------------------------------------------------------------------------
// Goldens
// ---------------------------------------------------------------------------

/// Deterministic pseudo-noise from the absolute sample index.
fn noise(n: u64) -> f32 {
    let mut s = n.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1) as u32;
    s ^= s >> 16;
    s = s.wrapping_mul(2_246_822_519);
    s ^= s >> 13;
    (s >> 8) as f32 * (2.0 / (1 << 23) as f32) - 1.0
}

struct Scenario {
    name: &'static str,
    set: Set,
    burst: bool,
    seconds: f32,
}

fn scenarios() -> [Scenario; 3] {
    [
        // The defaults (+12, 0.3) at a medium size: ER, build, the first
        // shifted passes.
        Scenario {
            name: "impulse_defaults",
            set: Set::at(0.6, 4.0, 12.0, 0.3),
            burst: false,
            seconds: 0.4,
        },
        // Small, dark, octave down at full share, no modulation.
        Scenario {
            name: "impulse_down_dark",
            set: Set {
                damping: 1_500.0,
                er_level: 0.9,
                er_time: 0.1,
                mod_depth: 0.0,
                shape: (0.7, 400.0, 0.2),
                build: 0.0,
                diffusion: 0.3,
                ..Set::at(0.05, 2.0, -12.0, 1.0)
            },
            burst: false,
            seconds: 0.3,
        },
        // A noise burst into a large, modulated fifth at full share.
        Scenario {
            name: "burst_fifth_modulated",
            set: Set {
                damping: 6_000.0,
                er_level: 0.3,
                er_time: 0.8,
                mod_rate: 3.0,
                mod_depth: 1.0,
                shape: (1.3, 250.0, 0.6),
                build: 0.8,
                diffusion: 0.95,
                ..Set::at(0.95, 12.0, 7.0, 1.0)
            },
            burst: true,
            seconds: 0.5,
        },
    ]
}

/// Impulse: L at 0, R at 37. Burst: 15 ms of Hann-windowed noise.
fn render_scenario(s: &Scenario) -> (Vec<f32>, Vec<f32>) {
    let mut d = dsp(&s.set);
    let n = (s.seconds * SR) as usize;
    let (mut l, mut r) = (Vec::with_capacity(n), Vec::with_capacity(n));
    for i in 0..n {
        let (x, y) = if s.burst {
            if i < 720 {
                let w = 0.5 - 0.5 * (TAU * i as f32 / 720.0).cos();
                (0.8 * w * noise(i as u64), 0.8 * w * noise(i as u64 + 9_973))
            } else {
                (0.0, 0.0)
            }
        } else {
            (if i == 0 { 1.0 } else { 0.0 }, if i == 37 { 1.0 } else { 0.0 })
        };
        let (a, b) = d.process(x, y, s.set.diffusion, 1.0);
        l.push(a);
        r.push(b);
    }
    (l, r)
}

fn golden_path() -> PathBuf {
    golden::golden_path(env!("CARGO_MANIFEST_DIR"), "shimmer_golden.f32")
}

#[test]
fn shimmer_output_is_bit_exact() {
    let mut rendered = Vec::new();
    for s in scenarios() {
        let (l, r) = render_scenario(&s);
        guard(s.name, &l, &r);
        let q = 3 * l.len() / 4;
        let tail = l[q..].iter().chain(&r[q..]).fold(0.0f32, |m, x| m.max(x.abs()));
        assert!(tail > 1e-4, "scenario `{}` has no tail ({tail:.2e})", s.name);
        rendered.extend(l);
        rendered.extend(r);
    }

    let path = golden_path();
    if golden::blessed(&["RESONANCE_BLESS", "RESONANCE_BLESS_SHIMMER_GOLDEN"]) {
        golden::bless_f32(&path, &rendered);
        return;
    }
    let want = golden::load_golden_f32(&path, rendered.len(), "RESONANCE_BLESS=1");
    let diff = golden::compare_f32(&rendered, &want);
    if let Some((i, got, want)) = diff.first_diff {
        panic!(
            "Shimmer output changed: {}/{} samples differ, peak delta {:.3e}; first at \
             sample {i} (got {got:?}, want {want:?}). Re-bless with RESONANCE_BLESS=1 \
             only if the change was intended.",
            diff.diff_count,
            rendered.len(),
            diff.max_abs,
        );
    }
}

#[test]
fn zz_debug_dump() {
    let dir = "/tmp/claude-1000/-home-jorrit-Src-resonance/03597a48-f93d-43de-b168-370af8707cb2/scratchpad";
    for (name, s) in [
        ("sh_a1", Set::at(0.7, 20.0, 12.0, 1.0)),
        ("sh_a0", Set::at(0.7, 20.0, 12.0, 0.0)),
    ] {
        let mut d = dsp(&s);
        let mut rng = Rng(0x5EED_0F_AA);
        let mut out = Vec::new();
        for i in 0..(11.2 * SR) as usize {
            let x = if i < 2_400 { 0.5 * rng.bipolar() } else { 0.0 };
            out.push(d.process(x, x, s.diffusion, 1.0).0);
        }
        let bytes: Vec<u8> = out.iter().flat_map(|v| v.to_le_bytes()).collect();
        std::fs::write(format!("{dir}/{name}.f32"), bytes).unwrap();
    }
}
