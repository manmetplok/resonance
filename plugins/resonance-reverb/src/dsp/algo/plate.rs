//! **Plate** (R3): Dattorro's figure-of-eight tank (reverb-algorithms.md
//! §4.4; J. Dattorro, "Effect Design Part 1", JAES 45(9), 1997, Fig. 1 —
//! Griesinger's Lexicon topology).
//!
//! ```text
//!  L ─ onset cascade ─┬─ 4 input allpasses ─► + ─ AP1ᴮ(mod) ─ D1ᴮ ─ AP2ᴮ ─ D2ᴮ ─┐
//!                     └─► er_l                ▲                               │
//!                                             └──── D2ᴬ ◄── AP2ᴬ ◄── D1ᴬ ◄─┐  │
//!  R ─ onset cascade ─┬─ 4 input allpasses ─► + ─ AP1ᴬ(mod) ─────────────────┘  │
//!                     └─► er_r                ▲                               │
//!                                             └───────────── D2ᴮ's end ◄──────┘
//!  late = Dattorro's 7 output taps per side, read from inside both branches
//! ```
//!
//! **Lengths.** Every delay and allpass length is the paper's value at its
//! 29 761 Hz reference, scaled to the session rate and by `size`
//! (0.5×–1.5×). A size move glides the tank lengths (fractional reads,
//! ≤ [`TAP_SLEW_PER_SAMPLE`] on the longest line) instead of relocating
//! them, the rule Classic's FDN follows; the input diffusers and the
//! output taps step to the nearest sample.
//!
//! **Decay.** The paper's single `decay` multiplier cannot track a T60: a
//! sample's time round the loop depends on every element in it, and an
//! allpass inside a loop spreads the modes' decay rates (its group delay
//! ranges 0.18×–5.7× its length at g = 0.7, so the slow modes win and the
//! measured T30 runs ~10 % long). Instead **every** tank element is
//! absorbent (Dahl & Jot, DAFx 2000): each delay, and the inner delay of
//! each allpass, carries an [`Absorption`] designed for its own length.
//! Every path through the tank is then attenuated by exactly what its
//! duration owes the T60, so the response is the lossless tank's times the
//! target envelope, band by band: `decay` is the mid T60, `damping` the
//! treble crossover, and `high_decay_mult` / `low_decay_mult` /
//! `low_xover` the rest ([`DecayBands::from_mults`]). The paper's damping
//! low-pass and both `decay` gains are replaced by these.
//!
//! **Short decays shrink the plate.** The tank's loop is 0.725 s at 1×. A
//! decay shorter than the loop dies inside the first pass, where the tap
//! energy still follows the injected wavefront round the figure of eight
//! instead of a diffuse field, and the measured T30 misses by 15–25 %. So
//! the tank scale is capped to keep the loop ≤ T60 / [`MIN_PASSES`]; the
//! cap only bites below ~0.87 s, and a decay change that moves it glides
//! like a size change.
//!
//! **Modulation.** The first allpass of each branch (the paper's modulated
//! ones) moves by up to [`MOD_EXCURSION_PAPER`] reference samples at full
//! `mod_depth`, driven by a [`SmoothRandom`] at `mod_rate` new targets per
//! second (the second branch 13 % faster), not the paper's quadrature
//! sines: a periodic detune is audible as a regular wobble under long
//! tails (L4), where a random one only blurs the modes.
//!
//! **Stereo in.** The paper is mono-in. Here each side has its own onset
//! cascade and input diffusers (the right side's lengths are offset), and L
//! feeds branch B, whose delays Dattorro's *left* output taps read first,
//! so a left source's tail starts on the left. A mono source reaches the
//! two branches through different diffusion, which only decorrelates the
//! output further (late IACC ≈ 0.02–0.05).
//!
//! **ER/tail.** A plate has no discrete early reflections. The shared
//! ER/tail balance (§4.2) weights the onset against the rest instead:
//! `er_*` is the input through a short dense allpass cascade (eight stages
//! of 0.1–2.5 ms), which is noise-dense from the first millisecond, fills
//! the gap before the tank taps start (≈ 9 ms at 1×) and is gone within
//! ~50 ms; `late_*` is the tank. The tank is fed from the same cascade
//! followed by the paper's four input diffusers (allpass cascades commute,
//! so tapping the onset before the long stages changes nothing for the
//! tank). `er_level` scales the onset (0 = the bare tank, which builds
//! over ~30 ms); `er_time` stretches the cascade 0.5×–1.5×, a shorter or
//! longer attack.
//!
//! **Freeze**: every absorption lossless and the input muted. The
//! allpasses and the allpass-interpolated reads are lossless, so the
//! figure of eight holds its energy.
//!
//! Allocation happens only in [`PlateEngine::new`]; `clear` returns the
//! engine to its constructed state for the parameters last set
//! (modulators re-seeded, glide snapped).

use resonance_dsp::reverb::{allpass_read, Absorption, Allpass, DecayBands, SmoothRandom};
use resonance_dsp::DelayLine;

use super::super::er::ER_TAPS;
use super::super::{CHANNELS, TAP_SLEW_PER_SAMPLE};
use super::Wet;

/// The sample rate Dattorro's lengths are given at.
const PAPER_RATE: f32 = 29_761.0;

/// Input diffusers (the paper's four), left side.
const INPUT_AP_L: [f32; 4] = [142.0, 107.0, 379.0, 277.0];
/// Right side: nearby primes, so a mono source reaches the two branches
/// through different diffusion.
const INPUT_AP_R: [f32; 4] = [151.0, 101.0, 367.0, 293.0];
/// Input diffusion 1 and 2 at `diffusion` 1 (the paper's values).
const INPUT_G: [f32; 4] = [0.75, 0.75, 0.625, 0.625];

/// Tank branch lengths: modulated allpass, delay, allpass, delay.
const TANK_A: [f32; 4] = [672.0, 4_453.0, 1_800.0, 3_720.0];
const TANK_B: [f32; 4] = [908.0, 4_217.0, 2_656.0, 3_163.0];
/// Decay diffusion 1 (the modulated allpass) and 2, with the paper's signs.
const DECAY_DIFFUSION_1: f32 = -0.70;
const DECAY_DIFFUSION_2: f32 = 0.50;
/// Peak excursion of the modulated allpasses at `mod_depth` 1, reference
/// samples: twice the paper's 16, so the default depth 0.3 sits near half
/// the paper's.
const MOD_EXCURSION_PAPER: f32 = 32.0;
/// The second branch's modulator runs this much faster, so the two never
/// move in lockstep.
const MOD_RATE_SPREAD: f32 = 1.13;
/// Seeds of the two modulators.
const SEED_A: u64 = 0x504c_4154_455f_4131;
const SEED_B: u64 = 0x504c_4154_455f_4232;

/// The whole figure of eight at 1×, seconds at any rate.
const LOOP_1X_S: f32 =
    (672.0 + 4_453.0 + 1_800.0 + 3_720.0 + 908.0 + 4_217.0 + 2_656.0 + 3_163.0) / PAPER_RATE;
/// The decay must span at least this many loops; below that the tank
/// shrinks (see the module docs).
const MIN_PASSES: f32 = 1.2;

/// Which tank element an output tap reads.
#[derive(Clone, Copy)]
enum Seg {
    D1A,
    Ap2A,
    D2A,
    D1B,
    Ap2B,
    D2B,
}

/// Output taps (element, reference samples, sign), Dattorro Table 2.
const OUT_TAPS_L: [(Seg, f32, f32); 7] = [
    (Seg::D1B, 266.0, 1.0),
    (Seg::D1B, 2_974.0, 1.0),
    (Seg::Ap2B, 1_913.0, -1.0),
    (Seg::D2B, 1_996.0, 1.0),
    (Seg::D1A, 1_990.0, -1.0),
    (Seg::Ap2A, 187.0, -1.0),
    (Seg::D2A, 1_066.0, -1.0),
];
const OUT_TAPS_R: [(Seg, f32, f32); 7] = [
    (Seg::D1A, 353.0, 1.0),
    (Seg::D1A, 3_627.0, 1.0),
    (Seg::Ap2A, 1_228.0, -1.0),
    (Seg::D2A, 2_673.0, 1.0),
    (Seg::D1B, 2_111.0, -1.0),
    (Seg::Ap2B, 335.0, -1.0),
    (Seg::D2B, 121.0, -1.0),
];
/// The paper's per-tap gain (0.6) times the level match to Classic: the
/// impulse response's energy at the defaults is within 0.5 dB of
/// Classic's, so an algorithm switch does not jump in loudness.
const OUT_GAIN: f32 = 0.6 * 0.3;

/// Onset cascade stage lengths, ms at `er_time` 0.5, per side.
const ONSET_MS_L: [f32; 8] = [0.11, 0.17, 0.29, 0.43, 0.67, 1.03, 1.61, 2.39];
const ONSET_MS_R: [f32; 8] = [0.13, 0.19, 0.31, 0.47, 0.71, 1.09, 1.53, 2.53];
const ONSET_G: f32 = 0.7;
/// Onset level at `er_level` 1. At the default 0.4 the onset sits at the
/// tank's early level, so the envelope is flat across the hand-over.
const ONSET_GAIN: f32 = 0.2;

/// `size` 0..1 (and `er_time` 0..1) → scale 0.5×..1.5×.
const MIN_SCALE: f32 = 0.5;
const MAX_SCALE: f32 = 1.5;

/// One-pole coefficient of the tank-view energy follower (Classic's).
const ENERGY_SMOOTH: f32 = 0.995;

/// A plain tank delay, read before the write. Integer length when
/// settled, an allpass-interpolated fractional one while `size` glides.
struct TankDelay {
    line: DelayLine,
    len: usize,
    interp: f32,
}

impl TankDelay {
    fn new(max_len: usize, len: usize) -> Self {
        Self {
            line: DelayLine::new(max_len + 2),
            len,
            interp: 0.0,
        }
    }

    /// The sample `len` (or `glide` while gliding) samples back.
    #[inline]
    fn read(&mut self, glide: Option<f32>) -> f32 {
        match glide {
            None => {
                let v = self.line.tap(self.len - 1);
                self.interp = v;
                v
            }
            Some(len) => allpass_read(&self.line, (len - 1.0).max(0.5), &mut self.interp),
        }
    }

    fn clear(&mut self) {
        self.line.clear();
        self.interp = 0.0;
    }
}

/// One tank branch: modulated allpass → delay → allpass → delay, every
/// element absorbent.
struct Branch {
    ap1: Allpass,
    d1: TankDelay,
    ap2: Allpass,
    d2: TankDelay,
    /// Absorption of AP1's inner delay, D1, AP2's inner delay and D2.
    abs: [Absorption; 4],
    lfo: SmoothRandom,
    paper: [f32; 4],
}

impl Branch {
    fn new(paper: [f32; 4], seed: u64, base: f32, sample_rate: f32) -> Self {
        let max = |p: f32| (p * base * MAX_SCALE).ceil() as usize + 2;
        let exc = (MOD_EXCURSION_PAPER * base).ceil() as usize + 2;
        let len = |p: f32| (p * base).round().max(2.0) as usize;
        Self {
            ap1: Allpass::new(max(paper[0]) + exc, len(paper[0]), DECAY_DIFFUSION_1),
            d1: TankDelay::new(max(paper[1]), len(paper[1])),
            ap2: Allpass::new(max(paper[2]), len(paper[2]), DECAY_DIFFUSION_2),
            d2: TankDelay::new(max(paper[3]), len(paper[3])),
            abs: [Absorption::lossless(); 4],
            lfo: SmoothRandom::new(seed, 1.0, 0.0, sample_rate),
            paper,
        }
    }

    fn set_lengths(&mut self, scale: f32) {
        let len = |p: f32| (p * scale).round().max(2.0) as usize;
        self.ap1.set_delay(len(self.paper[0]));
        self.d1.len = len(self.paper[1]);
        self.ap2.set_delay(len(self.paper[2]));
        self.d2.len = len(self.paper[3]);
    }

    /// Redesign every absorption for its element's (target) length, or
    /// make it lossless (`None`: Freeze).
    fn design(&mut self, bands: Option<&DecayBands>, sample_rate: f32) {
        let lengths = [self.ap1.delay(), self.d1.len, self.ap2.delay(), self.d2.len];
        for (a, len) in self.abs.iter_mut().zip(lengths) {
            match bands {
                Some(b) => a.design(b, len as f32, sample_rate),
                None => a.set_lossless(),
            }
        }
    }

    /// D2's output, the branch's end: read at the start of the sample,
    /// before [`Branch::process`] writes D2's input.
    #[inline]
    fn end(&mut self, glide: Option<f32>) -> f32 {
        let v = self.d2.read(glide.map(|s| self.paper[3] * s));
        self.abs[3].process(v)
    }

    /// Run the branch on `x`. `glide` is the current scale while a size
    /// move glides. Returns AP1, D1 and AP2's outputs.
    #[inline]
    fn process(&mut self, x: f32, glide: Option<f32>) -> [f32; 3] {
        let p = self.paper;
        let off = |i: usize, d: usize| glide.map_or(0.0, |s| p[i] * s - d as f32);
        let ap1_off = off(0, self.ap1.delay()) + self.lfo.next_sample();
        let ap2_off = off(2, self.ap2.delay());
        let [abs1, abs_d1, abs2, _] = &mut self.abs;
        let a = self.ap1.process_nested(x, ap1_off, |s| abs1.process(s));
        let d1 = abs_d1.process(self.d1.read(glide.map(|s| p[1] * s)));
        self.d1.line.push(a);
        let b = self.ap2.process_nested(d1, ap2_off, |s| abs2.process(s));
        self.d2.line.push(b);
        [a, d1, b]
    }

    fn clear(&mut self) {
        self.ap1.clear();
        self.d1.clear();
        self.ap2.clear();
        self.d2.clear();
        for a in &mut self.abs {
            a.clear();
        }
        self.lfo.reset();
    }
}

/// Dattorro's plate. See the module docs.
pub struct PlateEngine {
    sample_rate: f32,
    /// `sample_rate / PAPER_RATE`.
    base: f32,

    onset_l: [Allpass; 8],
    onset_r: [Allpass; 8],
    input_l: [Allpass; 4],
    input_r: [Allpass; 4],
    a: Branch,
    b: Branch,

    // Parameters, as last set.
    size: f32,
    decay: f32,
    damping: f32,
    low_mult: f32,
    low_xover: f32,
    high_mult: f32,
    frozen: bool,
    er_level: f32,
    diffusion: f32,

    /// Tank scale (`base ×` the size scale): the target, and the current
    /// one while a size move glides.
    scale_target: f32,
    scale_cur: f32,
    /// Largest per-sample scale step (the longest line moves ≤ the slew).
    scale_slew: f32,
    /// Output tap positions at the current scale, samples.
    taps_l: [usize; 7],
    taps_r: [usize; 7],
    /// The absorptions need a redesign (a decay, size, damping, shape or
    /// freeze change). Done at the next sample, so a block's worth of
    /// setter calls costs one redesign.
    dirty: bool,
    /// False until the first sample after construction or `clear`: a size
    /// change before it snaps instead of gliding.
    primed: bool,

    energies: [f32; CHANNELS],
}

impl PlateEngine {
    pub fn new(sample_rate: f32) -> Self {
        let base = sample_rate / PAPER_RATE;
        let input = |p: f32, g: f32| {
            let max = (p * base * MAX_SCALE).ceil() as usize + 2;
            Allpass::new(max, (p * base).round().max(1.0) as usize, g)
        };
        let onset = |ms: f32| {
            let n = ms * 1e-3 * sample_rate;
            Allpass::new(
                (n * MAX_SCALE).ceil() as usize + 2,
                n.round().max(1.0) as usize,
                ONSET_G,
            )
        };
        let mut e = Self {
            sample_rate,
            base,
            onset_l: ONSET_MS_L.map(onset),
            onset_r: ONSET_MS_R.map(onset),
            input_l: std::array::from_fn(|i| input(INPUT_AP_L[i], INPUT_G[i])),
            input_r: std::array::from_fn(|i| input(INPUT_AP_R[i], INPUT_G[i])),
            a: Branch::new(TANK_A, SEED_A, base, sample_rate),
            b: Branch::new(TANK_B, SEED_B, base, sample_rate),
            size: 0.5,
            decay: 2.0,
            damping: 8_000.0,
            low_mult: 1.0,
            low_xover: 250.0,
            high_mult: 0.5,
            frozen: false,
            er_level: 0.4,
            diffusion: 1.0,
            // Not a valid scale, so the first retarget always places.
            scale_target: 0.0,
            scale_cur: 0.0,
            scale_slew: TAP_SLEW_PER_SAMPLE / TANK_A[1],
            taps_l: [0; 7],
            taps_r: [0; 7],
            dirty: true,
            primed: false,
            energies: [0.0; CHANNELS],
        };
        e.retarget_scale();
        e.set_er_time(0.5);
        e.set_mod_rate(1.0);
        e.set_mod_depth(0.3);
        e.update_absorption();
        e
    }

    pub fn set_size(&mut self, v: f32) {
        if v != self.size {
            self.size = v;
            self.retarget_scale();
        }
    }

    pub fn set_decay(&mut self, v: f32) {
        if v != self.decay {
            self.decay = v;
            self.dirty = true;
            self.retarget_scale();
        }
    }

    pub fn set_freeze(&mut self, v: bool) {
        if v != self.frozen {
            self.frozen = v;
            self.dirty = true;
        }
    }

    /// The treble crossover: above it the decay tends to
    /// `decay × high_decay_mult`.
    pub fn set_damping(&mut self, v: f32) {
        if v != self.damping {
            self.damping = v;
            self.dirty = true;
        }
    }

    /// Onset level (a plate has no discrete ERs; see the module docs).
    pub fn set_er_level(&mut self, v: f32) {
        self.er_level = v.clamp(0.0, 1.0);
    }

    /// Onset length: the onset cascade at 0.5×–1.5×.
    pub fn set_er_time(&mut self, v: f32) {
        let k = 1e-3 * self.sample_rate * (MIN_SCALE + (MAX_SCALE - MIN_SCALE) * v.clamp(0.0, 1.0));
        for (ap, ms) in self.onset_l.iter_mut().zip(ONSET_MS_L) {
            ap.set_delay((ms * k).round().max(1.0) as usize);
        }
        for (ap, ms) in self.onset_r.iter_mut().zip(ONSET_MS_R) {
            ap.set_delay((ms * k).round().max(1.0) as usize);
        }
    }

    pub fn set_mod_rate(&mut self, v: f32) {
        self.a.lfo.set_rate(v, self.sample_rate);
        self.b.lfo.set_rate(v * MOD_RATE_SPREAD, self.sample_rate);
    }

    pub fn set_mod_depth(&mut self, v: f32) {
        let exc = v.clamp(0.0, 1.0) * MOD_EXCURSION_PAPER * self.base;
        self.a.lfo.set_depth(exc);
        self.b.lfo.set_depth(exc);
    }

    pub fn set_decay_shape(&mut self, low_mult: f32, low_xover_hz: f32, high_mult: f32) {
        if (low_mult, low_xover_hz, high_mult) != (self.low_mult, self.low_xover, self.high_mult) {
            self.low_mult = low_mult;
            self.low_xover = low_xover_hz;
            self.high_mult = high_mult;
            self.dirty = true;
        }
    }

    /// Plate has no build envelope.
    pub fn set_build(&mut self, _v: f32) {}

    /// The tank scale from `size`, capped by the decay (module docs).
    fn retarget_scale(&mut self) {
        let by_size = MIN_SCALE + (MAX_SCALE - MIN_SCALE) * self.size.clamp(0.0, 1.0);
        let by_decay = (self.decay / (MIN_PASSES * LOOP_1X_S)).max(MIN_SCALE);
        let scale = self.base * by_size.min(by_decay);
        if scale == self.scale_target {
            return;
        }
        self.scale_target = scale;
        self.a.set_lengths(scale);
        self.b.set_lengths(scale);
        for (ap, p) in self.input_l.iter_mut().zip(INPUT_AP_L) {
            ap.set_delay((p * scale).round().max(1.0) as usize);
        }
        for (ap, p) in self.input_r.iter_mut().zip(INPUT_AP_R) {
            ap.set_delay((p * scale).round().max(1.0) as usize);
        }
        if !self.primed {
            self.scale_cur = scale;
            self.place_taps(scale);
        }
        self.dirty = true;
    }

    fn place_taps(&mut self, scale: f32) {
        let (a, b) = (&self.a, &self.b);
        let place = |taps: &[(Seg, f32, f32); 7]| {
            taps.map(|(seg, p, _)| {
                let len = match seg {
                    Seg::D1A => a.d1.len,
                    Seg::Ap2A => a.ap2.delay(),
                    Seg::D2A => a.d2.len,
                    Seg::D1B => b.d1.len,
                    Seg::Ap2B => b.ap2.delay(),
                    Seg::D2B => b.d2.len,
                };
                ((p * scale).round() as usize).min(len - 1)
            })
        };
        self.taps_l = place(&OUT_TAPS_L);
        self.taps_r = place(&OUT_TAPS_R);
    }

    fn update_absorption(&mut self) {
        self.dirty = false;
        if self.frozen {
            self.a.design(None, self.sample_rate);
            self.b.design(None, self.sample_rate);
            return;
        }
        let bands = DecayBands::from_mults(
            self.decay.max(0.05),
            self.low_mult.max(0.01),
            self.high_mult.max(0.01),
            self.low_xover,
            self.damping,
        );
        self.a.design(Some(&bands), self.sample_rate);
        self.b.design(Some(&bands), self.sample_rate);
    }

    #[inline]
    fn tap(&self, seg: Seg, k: usize) -> f32 {
        match seg {
            Seg::D1A => self.a.d1.line.tap(k),
            Seg::Ap2A => self.a.ap2.tap(k),
            Seg::D2A => self.a.d2.line.tap(k),
            Seg::D1B => self.b.d1.line.tap(k),
            Seg::Ap2B => self.b.ap2.tap(k),
            Seg::D2B => self.b.d2.line.tap(k),
        }
    }

    #[inline]
    pub fn process(&mut self, l: f32, r: f32, diffusion: f32) -> Wet {
        self.primed = true;
        if self.dirty {
            self.update_absorption();
        }
        if diffusion != self.diffusion {
            self.diffusion = diffusion;
            let d = diffusion.clamp(0.0, 1.0);
            for (i, (al, ar)) in self.input_l.iter_mut().zip(&mut self.input_r).enumerate() {
                al.set_gain(INPUT_G[i] * d);
                ar.set_gain(INPUT_G[i] * d);
            }
        }

        // Size glide: the tank lengths follow the current scale.
        let glide = if self.scale_cur != self.scale_target {
            let step = self.scale_target - self.scale_cur;
            if step.abs() <= self.scale_slew {
                self.scale_cur = self.scale_target;
            } else {
                self.scale_cur += self.scale_slew.copysign(step);
            }
            self.place_taps(self.scale_cur);
            (self.scale_cur != self.scale_target).then_some(self.scale_cur)
        } else {
            None
        };

        // Onset: the input through the short dense cascade, per side.
        let (mut ol, mut or) = (l, r);
        for ap in &mut self.onset_l {
            ol = ap.process(ol);
        }
        for ap in &mut self.onset_r {
            or = ap.process(or);
        }

        // Then the paper's input diffusers, into the tank.
        let (mut xl, mut xr) = (ol, or);
        for ap in &mut self.input_l {
            xl = ap.process(xl);
        }
        for ap in &mut self.input_r {
            xr = ap.process(xr);
        }

        // The figure of eight: each branch's end feeds the other's start.
        // L enters B (read first by the left taps), R enters A.
        let (in_a, in_b) = if self.frozen { (0.0, 0.0) } else { (xr, xl) };
        let end_a = self.a.end(glide);
        let end_b = self.b.end(glide);
        let sa = self.a.process(in_a + end_b, glide);
        let sb = self.b.process(in_b + end_a, glide);

        let mut late_l = 0.0;
        for (&(seg, _, sign), &k) in OUT_TAPS_L.iter().zip(&self.taps_l) {
            late_l += sign * self.tap(seg, k);
        }
        let mut late_r = 0.0;
        for (&(seg, _, sign), &k) in OUT_TAPS_R.iter().zip(&self.taps_r) {
            late_r += sign * self.tap(seg, k);
        }

        let segs = [sa[0], sa[1], sa[2], end_a, sb[0], sb[1], sb[2], end_b];
        for (e, s) in self.energies.iter_mut().zip(segs) {
            *e = *e * ENERGY_SMOOTH + s.abs() * (1.0 - ENERGY_SMOOTH);
        }

        let onset = self.er_level * ONSET_GAIN;
        Wet {
            er_l: ol * onset,
            er_r: or * onset,
            late_l: late_l * OUT_GAIN,
            late_r: late_r * OUT_GAIN,
        }
    }

    pub fn clear(&mut self) {
        for ap in self
            .onset_l
            .iter_mut()
            .chain(&mut self.onset_r)
            .chain(&mut self.input_l)
            .chain(&mut self.input_r)
        {
            ap.clear();
        }
        self.a.clear();
        self.b.clear();
        self.scale_cur = self.scale_target;
        self.place_taps(self.scale_cur);
        self.energies = [0.0; CHANNELS];
        self.primed = false;
    }

    /// Smoothed |signal| at the eight tank segments (A: AP1, D1, AP2, D2,
    /// then B), for the tank view.
    pub fn channel_energies(&self) -> [f32; CHANNELS] {
        self.energies
    }

    /// The eight tank segment lengths, ms, in [`Self::channel_energies`]
    /// order.
    pub fn fdn_delay_ms(&self) -> [f32; CHANNELS] {
        let mut out = [0.0; CHANNELS];
        for (o, &p) in out.iter_mut().zip(TANK_A.iter().chain(&TANK_B)) {
            *o = p * self.scale_cur / self.sample_rate * 1e3;
        }
        out
    }

    /// No discrete early reflections.
    pub fn er_tap_times_ms(&self) -> [(f32, f32); ER_TAPS] {
        [(0.0, 0.0); ER_TAPS]
    }

    /// No discrete early reflections.
    pub fn er_tap_gains(&self) -> [(f32, f32); ER_TAPS] {
        [(0.0, 0.0); ER_TAPS]
    }
}
