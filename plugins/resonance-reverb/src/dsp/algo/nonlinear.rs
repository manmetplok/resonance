//! **Nonlinear** (R8, reverb-algorithms.md §4.4): a dense, colourless
//! burst shaped by an envelope that is not exponential (the AMS RMX16
//! "Nonlin" family).
//!
//! ```text
//!   in ─► diffuser (6 allpasses per side) ─┬─► Fdn<16>, T60 6 s ─fold─┬─► × envelope ─► late
//!                                          └──── × direct share ──────┘        ▲
//!   max(|l|, |r|) ─► transient detector ─► trigger ──────────────────────────┘
//! ```
//!
//! - **The burst** is a room-family network built from the
//!   `resonance_dsp::reverb` primitives: six input allpasses per side
//!   (coefficient 0.55 + 0.2 × `diffusion`, so it is always dense) into a
//!   16-line [`Fdn`] with short lines (5–35 ms at `size` 0.5, scaled
//!   0.7×–1.4× by `size`, gliding), light random modulation (`mod_rate`,
//!   up to 6 samples at `mod_depth` 1), and a fixed mid T60 of
//!   [`BURST_T60`]. `damping` is the treble crossover above which the
//!   burst decays 0.7× as fast (it darkens a little as it holds). `decay` is
//!   ignored: `nl_length` rules.
//! - **Flat, not falling.** While the envelope runs, the output is also
//!   multiplied by `10^(3t/T60)` (`t` since the trigger), which cancels
//!   the burst's own decay: an impulse's burst holds a constant level for
//!   as long as the envelope lets it through.
//! - **The envelope** (`nl_shape`), `L` = `nl_length` (50–1000 ms):
//!   - `Gated`: flat, then a 10 ms raised-cosine release **centred on
//!     `L`**, so the gate's half-amplitude point is exactly `L` after the
//!     hit;
//!   - `Reverse`: rising from −40 dB to 0 dB, linear in dB over `L`
//!     (a decay played backwards), then cut;
//!   - `Flat`: flat for `L`, then a natural exponential decay whose T60
//!     is `L` again (the classic "non-linear" decay: hold, then fall).
//!
//!   Before the first trigger, and after the envelope ends, the output is
//!   silent. Every edge (the gate opening, a retrigger, Reverse's cut, a
//!   shape or length change mid-envelope) goes through a gain smoother of
//!   two 1.5 ms one-poles in series (continuous in slope, so none of them
//!   clicks); the envelope is read 3 ms ahead to cancel the smoother's
//!   delay, so the edges still land on `L`.
//! - **Retrigger.** The detector is the ducker's: [`Ballistics`] peak
//!   followers on `max(|l|, |r|)`, a fast one (0.3 ms attack, 8 ms
//!   release) and a slow one (25 ms attack, 250 ms release). A hit is the
//!   fast envelope standing [`TRIGGER_RATIO`] (+9.5 dB) over the slow one
//!   and over [`TRIGGER_THRESHOLD`] (−40 dBFS). After a trigger the
//!   detector disarms until the fast envelope falls back to
//!   [`REARM_RATIO`] of the slow one, and never retriggers within
//!   [`HOLD_MS`] (50 ms: sixteenths up to 300 BPM). So a snare restarts the
//!   envelope on every hit, while a sustained pad or tone (fast ≈ slow)
//!   triggers once at its onset and is then gated away.
//! - **No early reflections**: everything is in `late_*`, `er_*` is zero
//!   (`er_level`, `er_time` ignored), so the shared ER/tail balance only
//!   scales the whole effect. **Freeze** is ignored (§4.2); so are the
//!   decay-shape multipliers and `tail_build`.
//!
//! Every setter dedupes; nothing after [`NonlinearEngine::new`] allocates;
//! `clear()` returns the engine to its freshly configured state.

use resonance_dsp::dynamics::Ballistics;
use resonance_dsp::reverb::{Allpass, DecayBands, Fdn, FdnConfig};

use super::super::er::ER_TAPS;
use super::super::CHANNELS;
use super::{Extras, Wet};

/// FDN lines.
const N: usize = 16;
/// The burst's mid T60, seconds: long enough that its own decay is
/// cancelled by a gain of at most +10 dB over the longest envelope.
pub const BURST_T60: f32 = 6.0;
/// Treble decay multiplier above `damping`.
const BURST_HIGH_MULT: f32 = 0.7;
/// Lines at scale 1, ms, and the scale at `size` 0 and 1 (log).
const LINE_MS: (f32, f32) = (5.0, 35.0);
const SIZE_SCALE: (f32, f32) = (0.7, 1.4);
/// FDN read-head slew on a size change, samples per sample.
const GLIDE: f32 = 0.04;
/// Modulation depth at `mod_depth` 1, samples at 48 kHz.
const MOD_DEPTH_MAX: f32 = 6.0;
/// Input diffuser: lengths per side (ms), and coefficient at
/// `diffusion` 0 and the extra at 1.
const DIFFUSER_MS: [[f32; 6]; 2] = [
    [0.37, 0.61, 0.97, 1.43, 2.11, 3.07],
    [0.41, 0.67, 1.03, 1.51, 2.23, 2.93],
];
const DIFFUSER_GAIN: (f32, f32) = (0.55, 0.2);
/// Share of the diffused input added straight to the output (the burst's
/// first milliseconds, before the shortest line comes round).
const DIRECT_DIFFUSE: f32 = 0.35;
/// Output gain.
const LATE_LEVEL: f32 = 0.35;
/// `nl_length` range, ms.
const LENGTH_MS: (f32, f32) = (50.0, 1000.0);
/// Gated release, ms (centred on the gate length).
const RELEASE_MS: f32 = 10.0;
/// Reverse starts this far down, dB.
const REVERSE_FLOOR_DB: f32 = -40.0;
/// The envelope counts as ended below this.
const END_LEVEL: f32 = 1e-4;
/// Gain smoother time constant (each of two one-poles in series), ms.
const SMOOTH_MS: f32 = 1.5;
/// Detector: fast and slow followers (attack, release ms).
const FAST_MS: (f32, f32) = (0.3, 8.0);
const SLOW_MS: (f32, f32) = (25.0, 250.0);
/// A hit: fast over slow by this ratio (+9.5 dB) …
pub const TRIGGER_RATIO: f32 = 3.0;
/// … and over this level (−40 dBFS).
pub const TRIGGER_THRESHOLD: f32 = 0.01;
/// Re-armed once fast falls to this ratio of slow.
pub const REARM_RATIO: f32 = 1.5;
/// Shortest time between triggers, ms.
pub const HOLD_MS: f32 = 50.0;

/// `nl_shape`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Shape {
    Gated,
    Reverse,
    Flat,
}

impl Shape {
    fn from_index(i: i32) -> Self {
        match i {
            1 => Shape::Reverse,
            2 => Shape::Flat,
            _ => Shape::Gated,
        }
    }
}

/// The retrigger detector (see the module docs).
struct Detector {
    fast: Ballistics,
    slow: Ballistics,
    fast_env: f32,
    slow_env: f32,
    armed: bool,
    hold: u32,
    hold_left: u32,
}

impl Detector {
    fn new(sample_rate: f32) -> Self {
        Self {
            fast: Ballistics::from_times(sample_rate, FAST_MS.0, FAST_MS.1),
            slow: Ballistics::from_times(sample_rate, SLOW_MS.0, SLOW_MS.1),
            fast_env: 0.0,
            slow_env: 0.0,
            armed: true,
            hold: (HOLD_MS * 0.001 * sample_rate) as u32,
            hold_left: 0,
        }
    }

    fn clear(&mut self) {
        self.fast_env = 0.0;
        self.slow_env = 0.0;
        self.armed = true;
        self.hold_left = 0;
    }

    /// One sample; true on a hit.
    #[inline]
    fn next(&mut self, l: f32, r: f32) -> bool {
        let m = l.abs().max(r.abs());
        let m = if m.is_finite() { m } else { 0.0 };
        self.fast_env = self.fast.step_envelope(self.fast_env, m);
        self.slow_env = self.slow.step_envelope(self.slow_env, m);
        if self.hold_left > 0 {
            self.hold_left -= 1;
        }
        if !self.armed {
            if self.fast_env <= REARM_RATIO * self.slow_env {
                self.armed = true;
            }
            return false;
        }
        if self.hold_left == 0
            && self.fast_env > TRIGGER_THRESHOLD
            && self.fast_env > TRIGGER_RATIO * self.slow_env
        {
            self.armed = false;
            self.hold_left = self.hold;
            return true;
        }
        false
    }
}

pub struct NonlinearEngine {
    sample_rate: f32,
    fdn: Fdn<N>,
    diffuser: [[Allpass; 6]; 2],
    in_l: [f32; N],
    in_r: [f32; N],
    out_l: [f32; N],
    out_r: [f32; N],
    detector: Detector,
    // The envelope.
    shape: Shape,
    length: f32,
    release: f32,
    /// Samples since the last trigger; `None` while closed.
    t: Option<u32>,
    /// `10^(3t/T60)`: the burst-decay compensation, and its step.
    comp: f32,
    comp_step: f32,
    /// Smoothed output gain (two one-poles in series) and the coefficient.
    gain_pre: f32,
    gain: f32,
    smooth: f32,
    /// The smoother's delay, samples: the envelope is read this far
    /// ahead, so its edges land on time at the output.
    lead: f32,
    // Last values set (dedupe; NaN: not yet).
    size: f32,
    damping: f32,
    mod_rate: f32,
    mod_depth: f32,
    diffusion: f32,
    length_ms: f32,
    primed: bool,
    energies: [f32; CHANNELS],
}

impl NonlinearEngine {
    pub fn new(sample_rate: f32) -> Self {
        let rate_scale = sample_rate / 48_000.0;
        let config = FdnConfig {
            max_size: SIZE_SCALE.1 * 1.05,
            max_mod_depth: MOD_DEPTH_MAX * rate_scale,
            seed: 0x4E4F_4E4C_494E,
            ..FdnConfig::new(LINE_MS.0, LINE_MS.1)
        };
        let mut fdn = Fdn::new(sample_rate, config);
        fdn.set_glide(GLIDE);
        let diffuser = std::array::from_fn(|side| {
            std::array::from_fn(|k| {
                let d = ((DIFFUSER_MS[side][k] * 0.001 * sample_rate).round() as usize).max(1);
                Allpass::new(d, d, DIFFUSER_GAIN.0)
            })
        });
        // Fixed pseudo-random signs, as in the room family: L feeds the
        // even lines, R the odd ones; L and R fold with orthogonal signs.
        let mut bits: u64 = 0x9E37_79B9_7F4A_7C15;
        let mut sign = || {
            bits ^= bits << 13;
            bits ^= bits >> 7;
            bits ^= bits << 17;
            if bits & 1 == 0 {
                1.0f32
            } else {
                -1.0
            }
        };
        let fold = 1.0 / (N as f32).sqrt();
        let feed = 1.0 / ((N / 2) as f32).sqrt();
        let mut in_l = [0.0; N];
        let mut in_r = [0.0; N];
        let mut out_l = [0.0; N];
        let mut out_r = [0.0; N];
        for i in 0..N {
            let s = sign();
            if i % 2 == 0 {
                in_l[i] = s * feed;
            } else {
                in_r[i] = s * feed;
            }
            let o = sign();
            out_l[i] = o * fold;
            out_r[i] = if (i as u32).count_ones().is_multiple_of(2) {
                o * fold
            } else {
                -o * fold
            };
        }
        let defaults = Extras::default();
        let mut e = Self {
            sample_rate,
            fdn,
            diffuser,
            in_l,
            in_r,
            out_l,
            out_r,
            detector: Detector::new(sample_rate),
            shape: Shape::from_index(defaults.nl_shape),
            length: 0.0,
            release: RELEASE_MS * 0.001 * sample_rate,
            t: None,
            comp: 1.0,
            comp_step: 10f32.powf(3.0 / (BURST_T60 * sample_rate)),
            gain_pre: 0.0,
            gain: 0.0,
            smooth: 1.0 - (-1.0 / (SMOOTH_MS * 0.001 * sample_rate)).exp(),
            lead: 2.0 * SMOOTH_MS * 0.001 * sample_rate,
            size: f32::NAN,
            damping: 8_000.0,
            mod_rate: 0.5,
            mod_depth: 0.0,
            diffusion: f32::NAN,
            length_ms: f32::NAN,
            primed: false,
            energies: [0.0; CHANNELS],
        };
        e.set_length_ms(defaults.nl_length_ms);
        e.design_decay();
        e.apply_modulation();
        e
    }

    fn set_length_ms(&mut self, ms: f32) {
        if ms != self.length_ms {
            self.length_ms = ms;
            let ms = if ms.is_finite() { ms } else { 300.0 };
            self.length = ms.clamp(LENGTH_MS.0, LENGTH_MS.1) * 0.001 * self.sample_rate;
        }
    }

    fn design_decay(&mut self) {
        let xover = self.damping.clamp(500.0, 0.45 * self.sample_rate);
        self.fdn.set_decay(DecayBands::from_mults(
            BURST_T60,
            1.0,
            BURST_HIGH_MULT,
            250.0,
            xover,
        ));
    }

    fn apply_modulation(&mut self) {
        let depth = self.mod_depth.clamp(0.0, 1.0) * MOD_DEPTH_MAX * (self.sample_rate / 48_000.0);
        self.fdn.set_modulation(self.mod_rate.max(0.0), depth);
    }

    pub fn set_size(&mut self, v: f32) {
        if v == self.size {
            return;
        }
        self.size = v;
        let (a, b) = SIZE_SCALE;
        let scale = a * (b / a).powf(v.clamp(0.0, 1.0));
        if self.primed {
            self.fdn.set_size(scale);
        } else {
            self.fdn.set_glide(0.0);
            self.fdn.set_size(scale);
            self.fdn.set_glide(GLIDE);
        }
    }

    /// Ignored: `nl_length` sets the length.
    pub fn set_decay(&mut self, _v: f32) {}

    /// Nonlinear ignores Freeze (§4.2).
    pub fn set_freeze(&mut self, _v: bool) {}

    pub fn set_damping(&mut self, v: f32) {
        if v != self.damping {
            self.damping = v;
            self.design_decay();
        }
    }

    pub fn set_er_level(&mut self, _v: f32) {}
    pub fn set_er_time(&mut self, _v: f32) {}

    pub fn set_mod_rate(&mut self, v: f32) {
        if v != self.mod_rate {
            self.mod_rate = v;
            self.apply_modulation();
        }
    }

    pub fn set_mod_depth(&mut self, v: f32) {
        if v != self.mod_depth {
            self.mod_depth = v;
            self.apply_modulation();
        }
    }

    pub fn set_decay_shape(&mut self, _low_mult: f32, _low_xover_hz: f32, _high_mult: f32) {}
    pub fn set_build(&mut self, _v: f32) {}

    pub fn set_extras(&mut self, extras: &Extras) {
        self.shape = Shape::from_index(extras.nl_shape);
        self.set_length_ms(extras.nl_length_ms);
    }

    /// The envelope at `t` samples after the trigger (without the decay
    /// compensation); `None` once it has ended.
    #[inline]
    fn envelope(&self, t: f32) -> Option<f32> {
        let l = self.length;
        match self.shape {
            Shape::Gated => {
                let half = 0.5 * self.release;
                if t < l - half {
                    Some(1.0)
                } else if t < l + half {
                    let x = (t - (l - half)) / self.release;
                    Some(0.5 + 0.5 * (std::f32::consts::PI * x).cos())
                } else {
                    None
                }
            }
            Shape::Reverse => {
                if t < l {
                    Some(10f32.powf(REVERSE_FLOOR_DB / 20.0 * (1.0 - t / l)))
                } else {
                    None
                }
            }
            Shape::Flat => {
                if t < l {
                    Some(1.0)
                } else {
                    let g = 10f32.powf(-3.0 * (t - l) / l);
                    (g > END_LEVEL).then_some(g)
                }
            }
        }
    }

    #[inline]
    pub fn process(&mut self, l: f32, r: f32, diffusion: f32) -> Wet {
        self.primed = true;
        if diffusion != self.diffusion {
            self.diffusion = diffusion;
            let g = DIFFUSER_GAIN.0 + DIFFUSER_GAIN.1 * diffusion.clamp(0.0, 1.0);
            for ap in self.diffuser.iter_mut().flatten() {
                ap.set_gain(g);
            }
        }
        if self.detector.next(l, r) {
            self.t = Some(0);
            self.comp = 1.0;
        }
        let target = match self.t {
            Some(t) => match self.envelope(t as f32 + self.lead) {
                Some(env) => {
                    let g = env * self.comp;
                    self.t = Some(t + 1);
                    self.comp *= self.comp_step;
                    g
                }
                None => {
                    self.t = None;
                    0.0
                }
            },
            None => 0.0,
        };
        self.gain_pre += self.smooth * (target - self.gain_pre);
        self.gain += self.smooth * (self.gain_pre - self.gain);
        if target == 0.0 && self.gain < 1e-6 && self.gain_pre < 1e-6 {
            self.gain_pre = 0.0;
            self.gain = 0.0;
        }

        let (mut dl, mut dr) = (l, r);
        for ap in &mut self.diffuser[0] {
            dl = ap.process(dl);
        }
        for ap in &mut self.diffuser[1] {
            dr = ap.process(dr);
        }
        let input: [f32; N] = std::array::from_fn(|i| self.in_l[i] * dl + self.in_r[i] * dr);
        let y = self.fdn.tick(&input);
        let (mut late_l, mut late_r) = (DIRECT_DIFFUSE * dl, DIRECT_DIFFUSE * dr);
        for ((&ol, &or), &yi) in self.out_l.iter().zip(&self.out_r).zip(y.iter()) {
            late_l += ol * yi;
            late_r += or * yi;
        }
        let per = N / CHANNELS;
        for (c, e) in self.energies.iter_mut().enumerate() {
            let mut m = 0.0f32;
            for k in 0..per {
                m += y[c * per + k].abs();
            }
            *e += 0.005 * (self.gain * m / per as f32 - *e);
        }
        let k = LATE_LEVEL * self.gain;
        Wet {
            er_l: 0.0,
            er_r: 0.0,
            late_l: late_l * k,
            late_r: late_r * k,
        }
    }

    pub fn clear(&mut self) {
        self.fdn.clear();
        for ap in self.diffuser.iter_mut().flatten() {
            ap.clear();
        }
        self.detector.clear();
        self.t = None;
        self.comp = 1.0;
        self.gain_pre = 0.0;
        self.gain = 0.0;
        self.energies = [0.0; CHANNELS];
        self.primed = false;
    }

    /// Per line-pair envelopes, after the gate.
    pub fn channel_energies(&self) -> [f32; CHANNELS] {
        self.energies
    }

    /// Mean length of each line pair, ms.
    pub fn fdn_delay_ms(&self) -> [f32; CHANNELS] {
        let len = self.fdn.line_lengths();
        let per = N / CHANNELS;
        std::array::from_fn(|c| {
            let sum: usize = len[c * per..(c + 1) * per].iter().sum();
            sum as f32 / per as f32 * 1000.0 / self.sample_rate
        })
    }

    pub fn er_tap_times_ms(&self) -> [(f32, f32); ER_TAPS] {
        [(0.0, 0.0); ER_TAPS]
    }

    pub fn er_tap_gains(&self) -> [(f32, f32); ER_TAPS] {
        [(0.0, 0.0); ER_TAPS]
    }
}
