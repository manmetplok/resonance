//! R8 Shimmer acceptance (reverb-algorithms.md §4.4, §5.2 "Shimmer" and
//! "for every algorithm").
//!
//! Every render goes through `ReverbDsp::with_engines(SR, &[Shimmer])`
//! with every setter pinned ([`Setup`]) and `set_extras` called
//! explicitly ([`Sh`]), 100 % wet, no pre-delay, return EQ off, ER/tail
//! balance centred, width 1. Numbers print with `--nocapture`:
//!
//!     cargo test -p resonance-reverb --test algorithms shimmer -- --nocapture

use std::f32::consts::TAU;

use resonance_metering::decay::{modal_peakiness_db, ImpulseReport};
use resonance_reverb::dsp::algo::shimmer::shifter::{
    max_read_weight, ratio, PitchShifter, ReadWeights, SEMITONES,
};
use resonance_reverb::dsp::algo::shimmer::ShimmerEngine;
use resonance_reverb::dsp::{Algorithm, Extras, ReverbDsp};

use crate::common::*;

/// A Shimmer setup: every engine setter ([`Setup`], the plugin defaults
/// at `size`/`decay`) plus the two shimmer extras.
#[derive(Clone, Copy, Debug)]
struct Sh {
    s: Setup,
    semitones: f32,
    amount: f32,
}

impl Sh {
    fn new(size: f32, decay: f32, semitones: f32, amount: f32) -> Self {
        Self {
            s: Setup::new(Algorithm::Shimmer, size, decay),
            semitones,
            amount,
        }
    }

    fn with(self, f: impl FnOnce(&mut Setup)) -> Self {
        Self {
            s: self.s.with(f),
            ..self
        }
    }

    fn extras(&self) -> Extras {
        Extras {
            shimmer_semitones: self.semitones,
            shimmer_amount: self.amount,
            ..Extras::default()
        }
    }

    fn dsp(&self) -> ReverbDsp {
        let mut d = self.s.dsp();
        d.set_extras(self.extras());
        d
    }

    /// Unit impulse on both channels at sample 0.
    fn impulse(&self, seconds: f32) -> (Vec<f32>, Vec<f32>) {
        self.render(seconds, |i| if i == 0 { 1.0 } else { 0.0 })
    }

    /// The same mono `input` on both channels.
    fn render(&self, seconds: f32, mut input: impl FnMut(usize) -> f32) -> (Vec<f32>, Vec<f32>) {
        let mut d = self.dsp();
        let n = (seconds * SR) as usize;
        let (mut l, mut r) = (Vec::with_capacity(n), Vec::with_capacity(n));
        for i in 0..n {
            let x = input(i);
            let (a, b) = d.process(x, x, self.s.diffusion, self.s.width);
            l.push(a);
            r.push(b);
        }
        (l, r)
    }
}

// ---------------------------------------------------------------------------
// The shifter and its cap
// ---------------------------------------------------------------------------

/// The loop's cap per pitch is `0.97/√c_max`, and `c_max` bounds the
/// shifter's energy gain on every prefix of any input: checked at every
/// sample against impulses at many alignments of a sweep, noise, sparse
/// random bursts and sines.
#[test]
fn the_shifter_gain_is_inside_the_cap() {
    let weights = ReadWeights::new(SR);
    println!("\npitch  c_max  cap    worst measured gain² (re c_max)");
    for &st in &SEMITONES {
        let c = weights.c_max(st);
        let cap = (0.97 / c.sqrt()).min(1.0);
        assert!((1.0..=4.0).contains(&c), "{st:+}: c_max {c}");
        // A geometric fact of the rate: another rate gets its own.
        let c44 = max_read_weight(ratio(st), 44_100.0);
        assert!((1.0..=4.0).contains(&c44), "{st:+} at 44.1 kHz: c_max {c44}");

        let gain2 = |x: &[f32]| {
            let mut s = PitchShifter::new(SR, 0.0);
            s.set_ratio(ratio(st));
            let (mut ein, mut eout) = (0.0f64, 0.0f64);
            for i in 0..x.len() + 3_000 {
                let v = x.get(i).copied().unwrap_or(0.0);
                ein += (v as f64).powi(2);
                let y = s.process(v) as f64;
                eout += y * y;
                assert!(
                    eout <= c as f64 * ein * 1.000_01 + 1e-12,
                    "{st:+}: prefix energy {eout} > c_max·{ein} at {i}"
                );
            }
            eout / ein
        };
        let mut worst = 0.0f64;
        for start in (0..5_000).step_by(37) {
            let mut x = vec![0.0f32; start + 1];
            x[start] = 1.0;
            worst = worst.max(gain2(&x));
        }
        let mut rng = Rng(0xA11CE ^ st.to_bits() as u64);
        for _ in 0..3 {
            let x: Vec<f32> = (0..8_000).map(|_| rng.range(-1.0, 1.0)).collect();
            worst = worst.max(gain2(&x));
            let x: Vec<f32> = (0..8_000)
                .map(|_| if rng.next() < 0.01 { rng.range(-1.0, 1.0) } else { 0.0 })
                .collect();
            worst = worst.max(gain2(&x));
        }
        for f in [40.0, 220.0, 1_000.0, 6_000.0] {
            let x: Vec<f32> = (0..8_000).map(|n| (TAU * f * n as f32 / SR).sin()).collect();
            worst = worst.max(gain2(&x));
        }
        println!("{st:>+5} {c:>6.3} {cap:>5.3}  {worst:.3} ({:.2})", worst / c as f64);
        assert!(worst <= c as f64 * 1.000_01, "{st:+}: gain² {worst} over c_max {c}");
        assert!((cap as f64).powi(2) * worst < 1.0, "{st:+}: capped gain² {worst}");
    }
    // An off-label pitch falls back to the geometric worst case.
    assert!(weights.c_max(3.0) >= 5.0);
}

/// The engine runs at that cap.
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

/// Power of `x` at `f` (Hann-windowed DFT bin).
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

/// Power of `x` within ±4 % of `f`. A delay-line shifter is exact in
/// pitch within a grain, but the crossfade from one grain to the next
/// resets the phase, so a shifted sine is a cluster of lines spaced at
/// the sweep rate around the target, not one bin.
fn band_power(x: &[f32], f: f32) -> f64 {
    let bin = SR / x.len() as f32;
    let k = (0.04 * f / bin).ceil() as i32;
    (-k..=k).map(|j| tone_power(x, f + j as f32 * bin)).sum()
}

/// The left output for a sustained sine at `f`, 0.3 peak.
fn sine_render(sh: &Sh, f: f32, secs: f32) -> Vec<f32> {
    sh.render(secs, |n| 0.3 * (TAU * f * n as f32 / SR).sin()).0
}

/// A sustained 330 Hz sine for 4 s at an 8 s decay, amount 0.6: for
/// every pitch the output holds a tone cluster at `330·2^(s/12)` far
/// above the same room at amount 0, and it grows as the loop
/// recirculates (from the first 0.25 s to 3.5 s). For +12 the octave's
/// octave (4f) is there too.
#[test]
fn a_sustained_sine_grows_a_tone_at_the_shifted_pitch() {
    let f = 330.0;
    let window = |x: &[f32], from: f32| {
        let a = (from * SR) as usize;
        x[a..a + SR as usize / 4].to_vec()
    };
    println!("\npitch  target Hz  dB over amount 0 (3.5 s)  growth  re the sine");
    for &st in &SEMITONES {
        let target = f * ratio(st);
        let on = sine_render(&Sh::new(0.6, 8.0, st, 0.6), f, 3.8);
        let off = sine_render(&Sh::new(0.6, 8.0, st, 0.0), f, 3.8);
        let rel = |hz: f32| {
            let p = band_power(&window(&on, 3.5), hz);
            let q = band_power(&window(&off, 3.5), hz);
            10.0 * (p / q.max(1e-30)).log10()
        };
        let early = band_power(&window(&on, 0.1), target);
        let late = band_power(&window(&on, 3.5), target);
        let main = band_power(&window(&on, 3.5), f);
        let over = rel(target);
        let growth = 10.0 * (late / early).log10();
        let level = 10.0 * (late / main).log10();
        print!("{st:>+5} {target:>9.1}   {over:>+6.1}  {growth:>+6.1}  {level:>+6.1}");
        assert!(over >= 20.0, "{st:+}: {target:.0} Hz only {over:+.1} dB over amount 0");
        assert!(growth >= 3.0, "{st:+}: the shifted tone does not grow ({growth:+.1} dB)");
        assert!(level >= -20.0, "{st:+}: the shifted tone is {level:+.1} dB under the sine");
        if st == 12.0 {
            let r4 = rel(4.0 * f);
            print!("  4f {r4:+.1} dB");
            assert!(r4 >= 15.0, "+12: the second octave is only {r4:+.1} dB over amount 0");
        }
        println!();
    }
}

#[test]
fn at_amount_zero_it_is_a_plain_hall() {
    println!("\namount 0: decay  midT30  err%   {}", ImpulseReport::table_header());
    for (size, decay) in [(0.5, 1.5), (0.6, 3.0), (0.9, 6.0)] {
        let sh = Sh::new(size, decay, 12.0, 0.0);
        let (l, r) = sh.impulse(render_seconds(decay));
        assert_not_silent(&format!("amount 0 {size}/{decay}"), &l, &r);
        let rep = ImpulseReport::analyze(&l, &r, SR);
        let mid = rep.mid_t30().expect("no mid T30");
        let err = (mid - decay) / decay;
        println!("{decay:>15.1}s {mid:>7.3} {:>+6.1}  {rep}", 100.0 * err);
        assert!(err.abs() <= 0.07, "size {size} decay {decay}: mid T30 {mid:.3}");
    }
}

/// The decay compensation (see `shimmer/tank.rs`): with the shifter
/// routed in, the mid T30 stays within ±20 % of the knob for every pitch
/// at amounts 0.15–1 (decays 3 and 6 s; ±7 % is the plain hall's
/// tolerance, the rest is the compensation's single `τ` for six pitches).
#[test]
fn the_decay_holds_with_the_shimmer_routed_in() {
    println!("\npitch  decay  amount  mid T30");
    for &st in &SEMITONES {
        for (decay, amount) in [(3.0, 0.15), (3.0, 0.4), (6.0, 0.3), (6.0, 1.0)] {
            let sh = Sh::new(0.7, decay, st, amount);
            let (l, r) = sh.impulse(render_seconds(decay));
            assert_not_silent(&format!("{st:+} {decay}/{amount}"), &l, &r);
            let t = ImpulseReport::analyze(&l, &r, SR).mid_t30().unwrap_or(0.0);
            let err = t / decay - 1.0;
            println!("{st:>+5} {decay:>5.1} {amount:>7.2}  {t:.2} s ({:+.0} %)", 100.0 * err);
            assert!(err.abs() <= 0.2, "{st:+} decay {decay} amount {amount}: mid T30 {t:.2}");
        }
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
    let pk = |sh: &Sh| {
        let mut rng = Rng(0x5EED_0FAA);
        let drive = (10.0 * SR) as usize;
        let (l, r) = sh.render(11.3, |i| if i < drive { 0.3 * rng.gauss() } else { 0.0 });
        assert_not_silent("whistle", &l, &r);
        let p = |x: &[f32], at: f32| modal_peakiness_db(x, SR, at, 100.0, 8_000.0).unwrap();
        (0.5 * (p(&l, 9.0) + p(&r, 9.0)), 0.5 * (p(&l, 10.2) + p(&r, 10.2)))
    };
    let shimmer = pk(&Sh::new(0.7, 20.0, 12.0, 1.0));
    let plain = pk(&Sh::new(0.7, 20.0, 12.0, 0.0));
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
/// after the bloom the frozen halo holds its energy, second by second.
#[test]
fn freeze_at_plus_24_holds_for_two_minutes() {
    let sh = Sh::new(0.7, 10.0, 24.0, 1.0);
    let mut d = sh.dsp();
    let mut rng = Rng(0x0F2E_E2E0);
    // The bloom: 3 s of noise with the shifter at full share.
    for _ in 0..(3.0 * SR) as usize {
        d.process(0.3 * rng.gauss(), 0.3 * rng.gauss(), sh.s.diffusion, 1.0);
    }
    d.set_freeze(true);
    let mut peak = 0.0f32;
    let mut second = |d: &mut ReverbDsp, rng: &mut Rng| {
        let mut e = 0.0f64;
        for _ in 0..SR as usize {
            let (a, b) = d.process(0.3 * rng.gauss(), 0.3 * rng.gauss(), sh.s.diffusion, 1.0);
            assert!(a.is_finite() && b.is_finite(), "non-finite while frozen");
            peak = peak.max(a.abs()).max(b.abs());
            e += (a as f64).powi(2) + (b as f64).powi(2);
        }
        e
    };
    // The ramp lands and the reflections drain.
    second(&mut d, &mut rng);
    let first = second(&mut d, &mut rng);
    let mut worst = 0.0f64;
    for _ in 0..118 {
        let e = second(&mut d, &mut rng);
        let db = 10.0 * (e / first).log10();
        if db.abs() > worst.abs() {
            worst = db;
        }
    }
    println!(
        "\nfreeze +24/amount 1: 1 s energy {:.2} dB, worst second over 118 s {worst:+.4} dB, \
         peak {peak:.3}",
        10.0 * first.log10()
    );
    assert!(first > 1.0, "nothing was frozen ({first})");
    assert!(worst.abs() <= 1.0, "the frozen tail drifted {worst:+.3} dB");
}

/// 60 s of random automation of every setter and both extras (steps and
/// ramps, pitch switches, Freeze toggling), noise with impulses on top:
/// finite, peak ≤ +24 dBFS. (`common::survives_random_automation` does
/// not reach the extras.)
#[test]
fn sixty_seconds_of_random_automation_stay_finite_and_bounded() {
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
        (0.0, 1.0, false),       // build
        (0.0, 1.0, false),       // shimmer_amount
        (0.0, 250.0, false),     // predelay
    ];
    let map = |k: usize, u: f32| {
        let (lo, hi, log) = RANGES[k];
        if log {
            lo * (hi / lo).powf(u)
        } else {
            lo + (hi - lo) * u
        }
    };
    let mut rng = Rng(0x00C0_FFEE_5EED_0008);
    let mut pos: [f32; 14] = std::array::from_fn(|_| rng.next());
    let mut target = pos;
    let mut left = [0u32; 14];
    let (mut freeze, mut noise_on, mut pitch) = (false, true, 0usize);
    let mut d = ReverbDsp::with_engines(SR, &[Algorithm::Shimmer]);
    d.set_wet_filters(false, 600.0, false, 10_000.0, false);
    d.set_er_tail_balance(0.0);
    let mut peak = 0.0f32;
    for block in 0..(60.0 * SR) as usize / BLOCK {
        for k in 0..14 {
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
        if rng.next() < 0.01 {
            pitch = (rng.next() * SEMITONES.len() as f32) as usize % SEMITONES.len();
        }
        let p = |k: usize| map(k, pos[k]);
        d.set_size(p(0));
        d.set_decay(p(1));
        d.set_freeze(freeze);
        d.set_damping(p(2));
        d.set_predelay(p(13));
        d.set_er_level(p(3));
        d.set_er_time(p(4));
        d.set_mod_rate(p(5));
        d.set_mod_depth(p(6));
        d.set_decay_shape(p(7), p(8), p(9));
        d.set_build(p(11));
        d.set_extras(Extras {
            shimmer_semitones: SEMITONES[pitch],
            shimmer_amount: p(12),
            ..Extras::default()
        });
        let diffusion = p(10);
        for i in 0..BLOCK {
            let mut x = if noise_on { 0.25 * rng.gauss() } else { 0.0 };
            if i == 0 && block % 97 == 0 {
                x += 1.0;
            }
            let (l, r) = d.process(x, -0.7 * x, diffusion, 1.0);
            assert!(l.is_finite() && r.is_finite(), "non-finite at block {block}");
            peak = peak.max(l.abs()).max(r.abs());
        }
    }
    let peak_db = 20.0 * peak.log10();
    println!("\nShimmer random automation: peak {peak_db:+.1} dBFS");
    assert!(peak_db <= 24.0, "peak {peak_db:+.1} dBFS over 60 s of automation");
    assert!(peak > 1e-3, "the fuzz rendered silence");
}

// ---------------------------------------------------------------------------
// Reset, clicks
// ---------------------------------------------------------------------------

/// Noise for 0.3 s, an impulse at 0.5 s, then silence.
fn probe(n: usize) -> (f32, f32) {
    let v = noise(n);
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
    let first = Sh::new(0.3, 1.5, 7.0, 0.4);
    let then = Sh {
        semitones: -12.0,
        amount: 0.9,
        ..Sh::new(0.8, 6.0, 0.0, 0.0)
    }
    .with(|s| {
        s.damping = 5_000.0;
        s.er_level = 0.6;
        s.er_time = 0.7;
        s.mod_rate = 2.0;
        s.mod_depth = 0.8;
        (s.low_mult, s.low_xover, s.high_mult) = (1.4, 300.0, 0.45);
        s.build = 0.9;
        s.diffusion = 0.6;
    });
    for freeze_after in [false, true] {
        let mut reused = first.dsp();
        run_probe(&mut reused, 0, (0.7 * SR) as usize, first.s.diffusion);
        // A retune mid-signal (glides, the amount and cap slews, the
        // Freeze ramp in flight), then the host's reset.
        then.s.apply(&mut reused);
        reused.set_extras(then.extras());
        reused.set_freeze(freeze_after);
        run_probe(&mut reused, 0, 3_000, then.s.diffusion);
        reused.clear();

        let mut fresh = then.dsp();
        fresh.set_freeze(freeze_after);

        let mut a = run_probe(&mut reused, 0, 12_000, then.s.diffusion);
        let mut b = run_probe(&mut fresh, 0, 12_000, then.s.diffusion);
        reused.set_freeze(false);
        fresh.set_freeze(false);
        a.extend(run_probe(&mut reused, 12_000, (SR as usize) - 12_000, then.s.diffusion));
        b.extend(run_probe(&mut fresh, 12_000, (SR as usize) - 12_000, then.s.diffusion));

        let peak = b.iter().fold(0.0f32, |m, x| m.max(x.abs()));
        assert!(peak > 1e-3, "the reference render is silent (freeze {freeze_after})");
        let diff = a.iter().zip(&b).position(|(x, y)| x.to_bits() != y.to_bits());
        assert_eq!(diff, None, "reset differs from fresh (freeze {freeze_after}) at {diff:?}");
    }
}

/// One second of [`sine`] after one second of warm-up, with `event`
/// called at every block start (samples into the measured second).
fn sine_second(sh: &Sh, event: impl FnMut(&mut ReverbDsp, usize)) -> Clicks {
    let mut d = sh.dsp();
    let sec = SR as usize;
    run_sine(&mut d, &sh.s, 0, sec, |_, _| {});
    let (l, r) = run_sine(&mut d, &sh.s, sec, sec, event);
    Clicks::of(&l, &r)
}

/// The largest second difference of a run over the RMS of the second
/// differences within ±5 ms of it, on either channel: a click is a
/// spike in its own neighbourhood. Unlike [`Clicks`] (per unit of the
/// run's peak) it does not move with the mix of partials a shimmer tail
/// holds: a frozen +12 tail beats, and its step/peak wanders by a few
/// 1e-3 from one steady second to the next with nothing switching.
fn spikiness(l: &[f32], r: &[f32]) -> f32 {
    let half = (0.005 * SR) as usize;
    let mut worst = 0.0f32;
    for side in [l, r] {
        let d2: Vec<f64> = side
            .windows(3)
            .map(|w| (w[2] - 2.0 * w[1] + w[0]) as f64)
            .collect();
        let mut acc = vec![0.0f64; d2.len() + 1];
        for (i, v) in d2.iter().enumerate() {
            acc[i + 1] = acc[i] + v * v;
        }
        for (i, v) in d2.iter().enumerate() {
            let (a, b) = (i.saturating_sub(half), (i + half + 1).min(d2.len()));
            let rms = ((acc[b] - acc[a]) / (b - a) as f64).sqrt();
            if rms > 0.0 {
                worst = worst.max((v.abs() / rms) as f32);
            }
        }
    }
    worst
}

/// Freeze engaged and released under a sustained sine, at the defaults
/// and at amount 1 on +12, −12 and +24 (where the routed share fades
/// with the ramp too). Both transitions, against the steady renders
/// either side of them (running and frozen, two seconds of each):
///
/// - second difference per unit of peak within the click margin
///   ([`Clicks`], as `common::assert_freeze_is_click_free` holds the
///   other engines), the margin being [`FREEZE_CLICK_MARGIN`] or how far
///   two steady seconds of one state differ, whichever is larger;
/// - step per unit of peak within 10 % plus that margin (the size-sweep
///   allowance: the sine re-entering a held tail full of octaves steps
///   more for a while, with nothing discontinuous);
/// - no spikier ([`spikiness`]) than 1.5 × the steady renders (they read
///   3–4; a discontinuity reads well over 10).
#[test]
fn freeze_engage_and_release_do_not_click() {
    for sh in [
        Sh::new(0.6, 4.0, 12.0, 0.3),
        Sh::new(0.6, 4.0, 12.0, 1.0),
        Sh::new(0.6, 4.0, -12.0, 1.0),
        Sh::new(0.6, 4.0, 24.0, 1.0),
    ] {
        let mut d = sh.dsp();
        let sec = SR as usize;
        let mut n = 0;
        let mut window = |d: &mut ReverbDsp, freeze: Option<bool>| {
            let (l, r) = run_sine(d, &sh.s, n, sec, |d, k| {
                if let (0, Some(f)) = (k, freeze) {
                    d.set_freeze(f);
                }
            });
            n += sec;
            (Clicks::of(&l, &r), spikiness(&l, &r))
        };
        window(&mut d, None);
        let held = window(&mut d, None);
        let held2 = window(&mut d, None);
        let engage = window(&mut d, Some(true));
        let frozen = window(&mut d, None);
        let frozen2 = window(&mut d, None);
        let release = window(&mut d, Some(false));
        let wander = |a: &Clicks, b: &Clicks| (a.step - b.step).abs().max((a.d2 - b.d2).abs());
        let margin = FREEZE_CLICK_MARGIN
            .max(wander(&held.0, &held2.0))
            .max(wander(&frozen.0, &frozen2.0));
        let steady = held.0.max(held2.0).max(frozen.0).max(frozen2.0);
        let calm = held.1.max(held2.1).max(frozen.1).max(frozen2.1);
        println!(
            "\n{:+} / {}: step/peak held {:.5} engage {:.5} frozen {:.5} release {:.5}; \
             2nd diff/peak {:.5} {:.5} {:.5} {:.5}; margin {margin:.5}; spikiness held \
             {:.1} engage {:.1} frozen {:.1} release {:.1}",
            sh.semitones,
            sh.amount,
            held.0.step,
            engage.0.step,
            frozen.0.step,
            release.0.step,
            held.0.d2,
            engage.0.d2,
            frozen.0.d2,
            release.0.d2,
            held.1,
            engage.1,
            frozen.1,
            release.1
        );
        for (what, (c, s)) in [("engage", engage), ("release", release)] {
            assert!(c.peak > 0.02, "freeze {what}: near silent");
            assert!(
                c.d2 <= steady.d2 + margin,
                "freeze {what}: 2nd diff/peak {:.5} against {:.5} steady",
                c.d2,
                steady.d2
            );
            assert!(
                c.step <= 1.1 * steady.step + margin,
                "freeze {what}: step/peak {:.5} against {:.5} steady",
                c.step,
                steady.step
            );
            assert!(s <= 1.5 * calm, "freeze {what}: spikiness {s:.1} against {calm:.1} steady");
        }
    }
}

/// `shimmer_pitch` stepped through every label, and `shimmer_amount`
/// stepped 0 → 1 → 0, under a sustained sine: within the click margin of
/// the held renders at the settings passed through (a higher pitch
/// legitimately steps more).
#[test]
fn pitch_and_amount_switches_do_not_click() {
    let base = Sh::new(0.6, 4.0, 12.0, 0.8);
    let held_pitch = SEMITONES
        .iter()
        .map(|&st| sine_second(&Sh { semitones: st, ..base }, |_, _| {}))
        .reduce(Clicks::max)
        .unwrap();
    let pitch = sine_second(&base, |d, k| {
        let i = k * SEMITONES.len() / SR as usize;
        d.set_extras(Extras {
            shimmer_semitones: SEMITONES[(i + 1) % SEMITONES.len()],
            ..base.extras()
        });
    });
    let held_amount = [0.0, 1.0]
        .iter()
        .map(|&a| sine_second(&Sh { amount: a, ..base }, |_, _| {}))
        .reduce(Clicks::max)
        .unwrap();
    let amount = sine_second(&base, |d, k| {
        let a = if (SR as usize / 3..2 * SR as usize / 3).contains(&k) { 1.0 } else { 0.0 };
        d.set_extras(Extras {
            shimmer_amount: a,
            ..base.extras()
        });
    });
    println!(
        "\npitch switch: step/peak {:.5} (held {:.5}), 2nd diff {:.5} ({:.5}); \
         amount switch: {:.5} ({:.5}), {:.5} ({:.5})",
        pitch.step,
        held_pitch.step,
        pitch.d2,
        held_pitch.d2,
        amount.step,
        held_amount.step,
        amount.d2,
        held_amount.d2
    );
    pitch.assert_within(&held_pitch, FREEZE_CLICK_MARGIN, "pitch switch");
    amount.assert_within(&held_amount, FREEZE_CLICK_MARGIN, "amount switch");
}

/// Size and build swept across their range under a sustained sine, as
/// the Hall's (the same glides).
#[test]
fn size_and_build_sweeps_do_not_click() {
    let from = Sh::new(0.1, 2.0, 12.0, 0.5).with(|s| s.build = 0.2);
    let to = Sh::new(0.95, 2.0, 12.0, 0.5).with(|s| s.build = 0.9);
    let held = sine_second(&from, |_, _| {}).max(sine_second(&to, |_, _| {}));
    let sweep = sine_second(&from, |d, k| {
        let t = k as f32 / SR;
        d.set_size(from.s.size + (to.s.size - from.s.size) * t);
        d.set_build(from.s.build + (to.s.build - from.s.build) * t);
    });
    println!("\nsweep: step/peak {:.5} (held {:.5})", sweep.step, held.step);
    // The Hall's allowance: its 0.1 sample/sample glides add up to 10 %
    // Doppler on the step.
    assert!(sweep.peak > 0.05, "near silent sweep");
    assert!(sweep.step <= 1.1 * held.step + 2e-3, "the sweep clicks: {:.5}", sweep.step);
}

// ---------------------------------------------------------------------------
// Memory, viz, presets
// ---------------------------------------------------------------------------

#[test]
fn the_engine_allocates_under_three_megabytes_at_96k() {
    let bytes = ShimmerEngine::new(96_000.0).buffer_bytes();
    let at48 = ShimmerEngine::new(48_000.0).buffer_bytes();
    let mib = |b: usize| b as f64 / (1024.0 * 1024.0);
    println!("\nShimmer buffers: {:.2} MiB at 96 kHz, {:.2} MiB at 48 kHz", mib(bytes), mib(at48));
    assert!(bytes <= 3_000_000, "Shimmer holds {bytes} bytes of buffers at 96 kHz");
}

#[test]
fn the_viz_getters_describe_the_engine() {
    let sh = Sh::new(0.5, 2.0, 12.0, 0.5);
    let mut d = sh.dsp();
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
/// sounding, with a shifted tone at their own settings, and a tail near
/// their decay.
#[test]
fn the_shimmer_presets_are_voiced_as_shimmers() {
    let presets = [
        ("Shimmer Drone", include_str!("../../presets/shimmer_drone.json"), 0),
        ("Octave Halo", include_str!("../../presets/octave_halo.json"), 0),
        ("Fifth Bloom", include_str!("../../presets/fifth_bloom.json"), 1),
    ];
    println!();
    for (name, json, pitch) in presets {
        let v: serde_json::Value = serde_json::from_str(json).unwrap();
        assert_eq!(v["meta"]["name"], name);
        let p = &v["state"]["doc"]["params"];
        let f = |k: &str| p[k].as_f64().unwrap_or_else(|| panic!("{name}: no `{k}`")) as f32;
        assert_eq!(f("algorithm"), Algorithm::Shimmer as u8 as f32, "{name} is not a Shimmer");
        assert_eq!(f("shimmer_pitch"), pitch as f32, "{name}: shimmer_pitch");
        let sh = Sh {
            semitones: SEMITONES[pitch],
            amount: f("shimmer_amount"),
            ..Sh::new(f("size"), f("decay"), 0.0, 0.0)
        }
        .with(|s| {
            s.damping = f("damping");
            s.er_level = f("er_level");
            s.er_time = f("er_time");
            s.mod_rate = f("mod_rate");
            s.mod_depth = f("mod_depth");
            (s.low_mult, s.low_xover, s.high_mult) =
                (f("low_decay_mult"), f("low_xover"), f("high_decay_mult"));
            s.build = f("tail_build");
            s.diffusion = f("diffusion");
        });
        assert!(sh.amount >= 0.3, "{name}: shimmer_amount {}", sh.amount);
        let (l, r) = sh.impulse(render_seconds(sh.s.decay));
        assert_not_silent(name, &l, &r);
        let t30 = ImpulseReport::analyze(&l, &r, SR).mid_t30().unwrap_or(0.0);
        let f0 = 330.0;
        let on = sine_render(&sh, f0, 2.5);
        let off = sine_render(&Sh { amount: 0.0, ..sh }, f0, 2.5);
        let tail = |x: &[f32]| band_power(&x[2 * SR as usize..], f0 * ratio(sh.semitones));
        let rel = 10.0 * (tail(&on) / tail(&off).max(1e-30)).log10();
        println!(
            "{name}: shifted tone {rel:+.1} dB over the room unshifted; mid T30 {t30:.2} s \
             (decay {} s)",
            sh.s.decay
        );
        assert!(rel >= 15.0, "{name}: shifted tone only {rel:+.1} dB");
        let err = t30 / sh.s.decay - 1.0;
        assert!(err.abs() <= 0.3, "{name}: mid T30 {t30:.2} s at decay {}", sh.s.decay);
    }
}

// ---------------------------------------------------------------------------
// Goldens
// ---------------------------------------------------------------------------

fn extras_defaults(d: &mut ReverbDsp) {
    d.set_extras(Extras {
        shimmer_semitones: 12.0,
        shimmer_amount: 0.3,
        ..Extras::default()
    });
}

fn extras_down_full(d: &mut ReverbDsp) {
    d.set_extras(Extras {
        shimmer_semitones: -12.0,
        shimmer_amount: 1.0,
        ..Extras::default()
    });
}

fn extras_fifth_full(d: &mut ReverbDsp) {
    d.set_extras(Extras {
        shimmer_semitones: 7.0,
        shimmer_amount: 1.0,
        ..Extras::default()
    });
}

/// Impulse at the defaults; a small, dark octave-down at full share with
/// no modulation; a noise burst into a large, modulated fifth at full
/// share. The extras are set before the first sample (frame-0 edit).
fn scenarios() -> [Scenario; 3] {
    let s = |size, decay| Setup::new(Algorithm::Shimmer, size, decay);
    [
        Scenario {
            name: "impulse_defaults",
            setup: s(0.6, 4.0),
            predelay_ms: 0.0,
            frames: (0.4 * SR) as usize,
            input: impulse_lr,
            edit: Some((0, extras_defaults)),
        },
        Scenario {
            name: "impulse_down_dark",
            setup: s(0.05, 2.0).with(|s| {
                s.damping = 1_500.0;
                s.er_level = 0.9;
                s.er_time = 0.1;
                s.mod_depth = 0.0;
                (s.low_mult, s.low_xover, s.high_mult) = (0.7, 400.0, 0.2);
                s.build = 0.0;
                s.diffusion = 0.3;
            }),
            predelay_ms: 0.0,
            frames: (0.3 * SR) as usize,
            input: impulse_lr,
            edit: Some((0, extras_down_full)),
        },
        Scenario {
            name: "burst_fifth_modulated",
            setup: s(0.95, 12.0).with(|s| {
                s.damping = 6_000.0;
                s.er_level = 0.3;
                s.er_time = 0.8;
                s.mod_rate = 3.0;
                s.mod_depth = 1.0;
                (s.low_mult, s.low_xover, s.high_mult) = (1.3, 250.0, 0.6);
                s.build = 0.8;
                s.diffusion = 0.95;
            }),
            predelay_ms: 0.0,
            frames: (0.5 * SR) as usize,
            input: burst,
            edit: Some((0, extras_fifth_full)),
        },
    ]
}

#[test]
fn shimmer_output_is_bit_exact() {
    check_golden(
        "shimmer_golden.f32",
        &["RESONANCE_BLESS", "RESONANCE_BLESS_SHIMMER_GOLDEN"],
        &scenarios(),
    );
}

