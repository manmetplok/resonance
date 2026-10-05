//! Shared helpers for the `algorithms` test modules: one engine in a
//! `ReverbDsp` with every setter pinned ([`Setup`]), the §5.1 silence
//! guard, the decaying-noise reference, the click metrics, the random
//! automation fuzz and the golden-scenario check. Referenced from the
//! modules as `crate::common::…`.
#![allow(dead_code)]

use std::f32::consts::TAU;
use std::path::PathBuf;

use resonance_dsp_test_support as golden;
use resonance_metering::decay::ImpulseReport;
use resonance_reverb::dsp::{Algorithm, ReverbDsp};

pub const SR: f32 = 48_000.0;
pub const BLOCK: usize = 128;

// ---------------------------------------------------------------------------
// Rendering
// ---------------------------------------------------------------------------

/// Every engine setter's value, plus the per-sample diffusion and width.
/// Rendered through a `ReverbDsp` whose bank holds only `algorithm`, 100 %
/// wet by construction, no pre-delay, return EQ off, ER/tail balance
/// centred.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Setup {
    pub algorithm: Algorithm,
    pub size: f32,
    pub decay: f32,
    pub damping: f32,
    pub diffusion: f32,
    pub er_level: f32,
    pub er_time: f32,
    pub mod_rate: f32,
    pub mod_depth: f32,
    pub low_mult: f32,
    pub low_xover: f32,
    pub high_mult: f32,
    pub build: f32,
    pub width: f32,
}

impl Setup {
    /// `algorithm` at `size`/`decay` with the global parameter defaults
    /// for everything else (what `baseline.rs` measured Classic with).
    pub fn new(algorithm: Algorithm, size: f32, decay: f32) -> Self {
        Self {
            algorithm,
            size,
            decay,
            damping: 8_000.0,
            diffusion: 0.8,
            er_level: 0.4,
            er_time: 0.5,
            mod_rate: 1.0,
            mod_depth: 0.3,
            low_mult: 1.0,
            low_xover: 250.0,
            high_mult: 0.5,
            build: 0.5,
            width: 1.0,
        }
    }

    /// The `Vocal Chamber` preset's engine voicing.
    pub fn vocal_chamber() -> Self {
        Self {
            damping: 6_000.0,
            diffusion: 0.85,
            er_level: 0.35,
            er_time: 0.45,
            mod_rate: 0.5,
            mod_depth: 0.0,
            low_mult: 1.4,
            high_mult: 0.45,
            ..Self::new(Algorithm::Chamber, 0.45, 1.4)
        }
    }

    pub fn with(self, f: impl FnOnce(&mut Self)) -> Self {
        let mut s = self;
        f(&mut s);
        s
    }

    /// The same settings on another engine.
    pub fn on(self, algorithm: Algorithm) -> Self {
        Self { algorithm, ..self }
    }

    pub fn dsp(&self) -> ReverbDsp {
        let mut d = ReverbDsp::with_engines(SR, &[self.algorithm]);
        self.apply(&mut d);
        d
    }

    /// Every setter, in the plugin's block-loop order.
    pub fn apply(&self, d: &mut ReverbDsp) {
        d.set_size(self.size);
        d.set_decay(self.decay);
        d.set_freeze(false);
        d.set_damping(self.damping);
        d.set_predelay(0.0);
        d.set_er_level(self.er_level);
        d.set_er_time(self.er_time);
        d.set_mod_rate(self.mod_rate);
        d.set_mod_depth(self.mod_depth);
        d.set_wet_filters(false, 600.0, false, 10_000.0, false);
        d.set_er_tail_balance(0.0);
        d.set_decay_shape(self.low_mult, self.low_xover, self.high_mult);
        d.set_build(self.build);
    }

    /// The response to a unit impulse on both channels at sample 0 (a
    /// mono send).
    pub fn impulse(&self, seconds: f32) -> (Vec<f32>, Vec<f32>) {
        let mut d = self.dsp();
        let n = (seconds * SR) as usize;
        let (mut l, mut r) = (Vec::with_capacity(n), Vec::with_capacity(n));
        for i in 0..n {
            let x = if i == 0 { 1.0 } else { 0.0 };
            let (a, b) = d.process(x, x, self.diffusion, self.width);
            l.push(a);
            r.push(b);
        }
        (l, r)
    }

    /// Impulse response, silence-guarded and analysed (`render_seconds`
    /// long), with its energy re a unit impulse.
    pub fn report(&self) -> (ImpulseReport, f64) {
        let (l, r) = self.impulse(render_seconds(self.decay));
        let e = energy_db(&l, &r);
        assert_not_silent(&format!("{self:?}"), &l, &r);
        (ImpulseReport::analyze(&l, &r, SR), e)
    }
}

/// Long enough for a T30 fit (−35 dB at 0.58 × T60) with room for the
/// noise-floor truncation, and for the late metrics on short decays.
pub fn render_seconds(decay: f32) -> f32 {
    (1.4 * decay + 0.6).max(1.3)
}

// ---------------------------------------------------------------------------
// Metrics and guards
// ---------------------------------------------------------------------------

/// Mean energy of the two channels, dB re a unit impulse.
pub fn energy_db(l: &[f32], r: &[f32]) -> f64 {
    let e: f64 = l.iter().chain(r).map(|&x| (x as f64) * (x as f64)).sum();
    10.0 * (e / 2.0).max(1e-30).log10()
}

/// The §5.1 silence guard: total response energy above −40 dB re a unit
/// impulse. Also refuses a non-finite response.
pub fn assert_not_silent(what: &str, l: &[f32], r: &[f32]) {
    assert!(
        l.iter().chain(r).all(|x| x.is_finite()),
        "{what}: non-finite output"
    );
    let e = energy_db(l, r);
    assert!(e > -40.0, "{what}: response energy {e:.1} dB (silence guard)");
}

/// One octave band's T30.
pub fn t30_at(rep: &ImpulseReport, hz: f32) -> f32 {
    rep.bands
        .iter()
        .find(|b| b.center_hz == hz)
        .and_then(|b| b.times.t30)
        .unwrap_or_else(|| panic!("no T30 at {hz} Hz"))
}

/// Deterministic xorshift, uniform in `[0, 1)`.
pub struct Rng(pub u64);

impl Rng {
    pub fn next(&mut self) -> f32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 >> 40) as f32 / (1u64 << 24) as f32
    }

    pub fn range(&mut self, lo: f32, hi: f32) -> f32 {
        lo + (hi - lo) * self.next()
    }

    pub fn gauss(&mut self) -> f32 {
        let (a, b) = (self.next().max(1e-7), self.next());
        (-2.0 * a.ln()).sqrt() * (TAU * b).cos()
    }
}

/// Mean peakiness of exponentially decaying Gaussian noise at `t60` over
/// four seeds: the colourless reference, scored by the same metric (one
/// seed scatters by ±0.5 dB).
pub fn noise_peakiness(t60: f32) -> f32 {
    let seeds = [
        0x9e37_79b9_7f4a_7c15,
        0x2545_f491_4f6c_dd1d,
        0x1234_5678_9abc_def1,
        77,
    ];
    let n = (1.3 * SR) as usize;
    let sum: f32 = seeds
        .iter()
        .map(|&s| {
            let mut rng = Rng(s);
            let (mut l, mut r) = (Vec::with_capacity(n), Vec::with_capacity(n));
            for i in 0..n {
                let env = 10f32.powf(-3.0 * i as f32 / (t60 * SR));
                l.push(env * rng.gauss());
                r.push(env * rng.gauss());
            }
            ImpulseReport::analyze(&l, &r, SR).peakiness_db.unwrap()
        })
        .sum();
    sum / seeds.len() as f32
}

/// Deterministic pseudo-noise from the sample index, `[-1, 1)`.
pub fn noise(n: usize) -> f32 {
    let mut s = (n as u64)
        .wrapping_mul(6_364_136_223_846_793_005)
        .wrapping_add(1) as u32;
    s ^= s >> 16;
    s = s.wrapping_mul(2_246_822_519);
    s ^= s >> 13;
    (s >> 8) as f32 * (2.0 / (1 << 23) as f32) - 1.0
}

/// Largest sample-to-sample step on either channel.
pub fn max_step(l: &[f32], r: &[f32]) -> f32 {
    let mut worst = 0.0f32;
    for side in [l, r] {
        for w in side.windows(2) {
            worst = worst.max((w[1] - w[0]).abs());
        }
    }
    worst
}

/// Click metrics of a stereo run, both per unit of the run's peak (so a
/// level change, e.g. a sweep dragging room modes across a sine, does
/// not read as a click): the largest sample step (a sine reads
/// `2·sin(ω/2)`, 0.029 at 220 Hz) and the largest second difference
/// (a sine reads `ω²`, 8.4e-4 at 220 Hz; a discontinuity reads the jump
/// itself, a gain step its size times the signal).
#[derive(Clone, Copy, Debug)]
pub struct Clicks {
    pub step: f32,
    pub d2: f32,
    pub peak: f32,
}

impl Clicks {
    pub fn of(l: &[f32], r: &[f32]) -> Self {
        let peak = l.iter().chain(r).fold(0.0f32, |m, x| m.max(x.abs()));
        let mut d2 = 0.0f32;
        for side in [l, r] {
            for w in side.windows(3) {
                d2 = d2.max((w[2] - 2.0 * w[1] + w[0]).abs());
            }
        }
        let p = peak.max(1e-9);
        Self {
            step: max_step(l, r) / p,
            d2: d2 / p,
            peak,
        }
    }

    /// The worse of two runs, metric by metric.
    pub fn max(self, o: Self) -> Self {
        Self {
            step: self.step.max(o.step),
            d2: self.d2.max(o.d2),
            peak: self.peak.max(o.peak),
        }
    }

    /// Within `margin` (per unit of peak) of `held` on both metrics.
    pub fn assert_within(&self, held: &Clicks, margin: f32, what: &str) {
        assert!(self.peak > 0.02, "{what}: near silent (peak {:.4})", self.peak);
        assert!(
            self.step <= held.step + margin,
            "{what} clicks: step/peak {:.5} against {:.5} held",
            self.step,
            held.step
        );
        assert!(
            self.d2 <= held.d2 + margin,
            "{what} clicks: 2nd difference/peak {:.5} against {:.5} held",
            self.d2,
            held.d2
        );
    }
}

/// A sustained 220 Hz sine at −6 dBFS, on both channels.
pub fn sine(n: usize) -> f32 {
    0.5 * (TAU * 220.0 * n as f32 / SR).sin()
}

/// `len` samples of [`sine`] from sample `from` through `d`, calling
/// `at_block(d, k)` at every block boundary `k` (samples since `from`).
pub fn run_sine(
    d: &mut ReverbDsp,
    s: &Setup,
    from: usize,
    len: usize,
    mut at_block: impl FnMut(&mut ReverbDsp, usize),
) -> (Vec<f32>, Vec<f32>) {
    let (mut l, mut r) = (Vec::with_capacity(len), Vec::with_capacity(len));
    for (k, n) in (from..from + len).enumerate() {
        if k.is_multiple_of(BLOCK) {
            at_block(d, k);
        }
        let x = sine(n);
        let (a, b) = d.process(x, x, s.diffusion, s.width);
        l.push(a);
        r.push(b);
    }
    (l, r)
}

/// What Freeze does to a sustained sine: the click metrics of the
/// running render (`held`), of the second after Freeze engages
/// (`engage`), of a settled frozen second (`frozen`) and of the second
/// after it is released (`release`). The input keeps playing throughout:
/// Freeze has to mute it.
pub struct FreezeClicks {
    pub held: Clicks,
    pub engage: Clicks,
    pub frozen: Clicks,
    pub release: Clicks,
}

pub fn freeze_clicks(s: &Setup) -> FreezeClicks {
    let mut d = s.dsp();
    let sec = SR as usize;
    let mut n = 0;
    let mut window = |d: &mut ReverbDsp, freeze: Option<bool>| {
        let (l, r) = run_sine(d, s, n, sec, |d, k| {
            if let (0, Some(f)) = (k, freeze) {
                d.set_freeze(f);
            }
        });
        n += sec;
        Clicks::of(&l, &r)
    };
    window(&mut d, None);
    let held = window(&mut d, None);
    let engage = window(&mut d, Some(true));
    let frozen = window(&mut d, None);
    let release = window(&mut d, Some(false));
    let c = FreezeClicks {
        held,
        engage,
        frozen,
        release,
    };
    println!(
        "{:?} freeze under a sine: step/peak held {:.5}, engage {:.5}, frozen {:.5}, \
         release {:.5}; 2nd diff/peak held {:.5}, engage {:.5}, frozen {:.5}, release {:.5}; \
         peaks {:.3} / {:.3} / {:.3} / {:.3}",
        s.algorithm,
        c.held.step,
        c.engage.step,
        c.frozen.step,
        c.release.step,
        c.held.d2,
        c.engage.d2,
        c.frozen.d2,
        c.release.d2,
        c.held.peak,
        c.engage.peak,
        c.frozen.peak,
        c.release.peak,
    );
    c
}

/// The margin [`assert_freeze_is_click_free`] allows the engines, per unit
/// of peak on both metrics.
pub const FREEZE_CLICK_MARGIN: f32 = 1e-3;

/// Freeze engaged and released under a sustained sine does not click:
/// both transitions within `margin` of the steady renders either side of
/// them (running and frozen), step and second difference per unit of
/// peak. The settled frozen second is held to the running one too, at
/// twice the margin: a click on engaging lands in the lossless loop and
/// is held there for as long as Freeze is (before the ramp, its second
/// difference read 0.02–0.04 of peak), while a clean frozen tail reads
/// only a little above the running one (the partials the modulation
/// spreads the sine into are no longer damped).
pub fn assert_freeze_is_click_free(s: &Setup, margin: f32) {
    let c = freeze_clicks(s);
    let what = |w: &str| format!("{:?} freeze {w}", s.algorithm);
    let steady = c.held.max(c.frozen);
    c.engage.assert_within(&steady, margin, &what("engage"));
    c.release.assert_within(&steady, margin, &what("release"));
    c.frozen.assert_within(&c.held, 2.0 * margin, &what("(held)"));
}

// ---------------------------------------------------------------------------
// Stability
// ---------------------------------------------------------------------------

/// 60 s of random automation of every setter (steps and ramps, Freeze
/// toggling), noise with impulses on top: finite, peak ≤ +24 dBFS.
pub fn survives_random_automation(algorithm: Algorithm, seed: u64) {
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

/// Freeze held for 60 s after a second of noise: the tail's energy, in
/// 4 s windows after the first second (the input ramp, the reflections
/// and the diffusers drain), drifts by at most 0.1 dB.
pub fn assert_freeze_holds_60_s(s: Setup) {
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
        if i >= SR as usize {
            acc += (l as f64).powi(2) + (r as f64).powi(2);
            if (i + 1 - SR as usize).is_multiple_of(window) {
                energies.push(acc);
                acc = 0.0;
            }
        }
    }
    let first = energies[0];
    assert!(first > 1e-3, "{:?}: nothing frozen ({first:.2e})", s.algorithm);
    let worst = energies
        .iter()
        .map(|e| 10.0 * (e / first).log10())
        .fold(0.0f64, |m, db| if db.abs() > m.abs() { db } else { m });
    println!(
        "{:?} freeze: worst 4 s window over 60 s {worst:+.4} dB re the first",
        s.algorithm
    );
    assert!(worst.abs() <= 0.1, "{:?}: freeze drifted {worst:+.3} dB", s.algorithm);
}

// ---------------------------------------------------------------------------
// Goldens
// ---------------------------------------------------------------------------

/// A parameter edit applied at a frame.
pub type Edit = (usize, fn(&mut ReverbDsp));

/// One golden scenario: every setter pinned, an input, a length.
pub struct Scenario {
    pub name: &'static str,
    pub setup: Setup,
    pub predelay_ms: f32,
    pub frames: usize,
    pub input: fn(usize) -> (f32, f32),
    /// A parameter edit at a frame (a size glide, Freeze).
    pub edit: Option<Edit>,
}

/// A unit impulse, L at 0 and R at 37.
pub fn impulse_lr(n: usize) -> (f32, f32) {
    (
        if n == 0 { 1.0 } else { 0.0 },
        if n == 37 { 1.0 } else { 0.0 },
    )
}

/// A 15 ms Hann-windowed noise burst, decorrelated L/R.
pub fn burst(n: usize) -> (f32, f32) {
    if n < 720 {
        let w = 0.5 - 0.5 * (TAU * n as f32 / 720.0).cos();
        (0.8 * w * noise(n), 0.8 * w * noise(n + 9_973))
    } else {
        (0.0, 0.0)
    }
}

/// The scenario's output and its input energy.
pub fn render_scenario(s: &Scenario) -> (Vec<f32>, Vec<f32>, f64) {
    let v = s.setup;
    let mut d = v.dsp();
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

/// Render every scenario (each one silence-guarded re its own input
/// energy, and holding a tail in its last quarter) and compare the lot,
/// scenario by scenario as L then R, with the golden `file`; `bless`
/// names the environment variables that re-bless it.
pub fn check_golden(file: &str, bless: &[&str], scenarios: &[Scenario]) {
    let mut rendered = Vec::new();
    for s in scenarios {
        let (l, r, e_in) = render_scenario(s);
        assert!(
            l.iter().chain(&r).all(|x| x.is_finite()),
            "{}: non-finite",
            s.name
        );
        let e = energy_db(&l, &r) - 10.0 * e_in.log10();
        assert!(e > -40.0, "{}: {e:.1} dB re its input (silence guard)", s.name);
        let tail = s.frames * 3 / 4;
        let tail_db = energy_db(&l[tail..], &r[tail..]) - 10.0 * e_in.log10();
        assert!(
            tail_db > -60.0,
            "{}: no tail in the last quarter ({tail_db:.1} dB)",
            s.name
        );
        println!("{}: {e:.1} dB, last quarter {tail_db:.1} dB re input", s.name);
        rendered.extend(l);
        rendered.extend(r);
    }

    let path: PathBuf = golden::golden_path(env!("CARGO_MANIFEST_DIR"), file);
    if golden::blessed(bless) {
        golden::bless_f32(&path, &rendered);
        return;
    }
    let want = golden::load_golden_f32(&path, rendered.len(), "RESONANCE_BLESS=1");
    let diff = golden::compare_f32(&rendered, &want);
    if let Some((i, got, want)) = diff.first_diff {
        panic!(
            "{file}: output changed: {}/{} samples differ, peak delta {:.3e}; first at \
             sample {i} (got {got:?}, want {want:?}). Re-bless with RESONANCE_BLESS=1 \
             only for an intended change.",
            diff.diff_count,
            rendered.len(),
            diff.max_abs,
        );
    }
}
