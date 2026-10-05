//! Shared rendering for the room-family tests: a `ReverbDsp` whose bank
//! holds one engine (so this holds before the algorithm joins
//! `Algorithm::BUILT`), every engine setter called explicitly, 100 % wet
//! by construction, no pre-delay, return EQ off, ER/tail balance centred.

use resonance_metering::decay::ImpulseReport;
use resonance_reverb::dsp::{Algorithm, ReverbDsp};

pub const SR: f32 = 48_000.0;
pub const BLOCK: usize = 128;

/// Every engine setter's value, plus the per-sample diffusion and width.
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
            width: 1.0,
        }
    }

    /// The `Vocal Chamber` preset's engine voicing.
    pub fn vocal_chamber() -> Self {
        Self {
            size: 0.45,
            decay: 1.4,
            damping: 6_000.0,
            diffusion: 0.85,
            er_level: 0.35,
            er_time: 0.45,
            mod_rate: 0.5,
            mod_depth: 0.0,
            low_mult: 1.4,
            low_xover: 250.0,
            high_mult: 0.45,
            ..Self::new(Algorithm::Chamber, 0.45, 1.4)
        }
    }

    pub fn with(self, f: impl FnOnce(&mut Self)) -> Self {
        let mut s = self;
        f(&mut s);
        s
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
        d.set_build(0.5);
    }

    /// The response to a unit impulse on both channels at sample 0.
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

    /// Impulse response, analysed (`render_seconds` long).
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

/// Mean energy of the two channels, dB re a unit impulse.
pub fn energy_db(l: &[f32], r: &[f32]) -> f64 {
    let e: f64 = l.iter().chain(r).map(|&x| (x as f64) * (x as f64)).sum();
    10.0 * (e / 2.0).max(1e-30).log10()
}

/// The §5.1 silence guard: total response energy above −40 dB re a unit
/// impulse.
pub fn assert_not_silent(what: &str, l: &[f32], r: &[f32]) {
    let e = energy_db(l, r);
    assert!(
        e > -40.0,
        "{what}: response energy {e:.1} dB (silence guard)"
    );
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

    pub fn gauss(&mut self) -> f32 {
        let (a, b) = (self.next().max(1e-7), self.next());
        (-2.0 * a.ln()).sqrt() * (std::f32::consts::TAU * b).cos()
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
