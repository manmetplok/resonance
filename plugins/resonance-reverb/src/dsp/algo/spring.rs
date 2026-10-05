//! **Spring** (R8, reverb-algorithms.md §4.4): two dispersive springs in
//! parallel, after the parametric spring model of Välimäki, Parker & Abel
//! (in "Fifty Years of Artificial Reverberation", IEEE TASLP 2012).
//!
//! ```text
//!              ┌──────────── drip: transient × input ─► DispersiveAllpass(0.95) ─┐
//!  (L+R)/2 ─ HPF ─┬──────────────────────────────────────────────────────────────┤
//!                 │   ┌───────────────────────────────────────────────┐           │
//!                 └─► + ─► DispersiveAllpass(a, 40) ─┬─► delay(D) ─► LPF ─ × g ─┘  (per spring)
//!                                                    └─► late (A → L, B → R)
//! ```
//!
//! - **Echoes.** Each spring is a loop: a cascade of [`STAGES`] first-order
//!   allpasses ([`DispersiveAllpass`], coefficient `a` > 0, so the bass
//!   trails the treble), a delay, a 12 dB/oct low-pass and a gain. Every
//!   pass round the loop is one echo, and every echo runs through the
//!   cascade again, so the chirp lengthens echo by echo, the "boing" of a
//!   real spring. The output is tapped after the cascade.
//! - **`size`** is the spring's round-trip time, 30–90 ms (log), at
//!   1 kHz: the delay is the round trip minus the cascade's and the
//!   low-pass's group delay at 1 kHz, so the echo spacing there is the
//!   round trip whatever the tension. A size move glides the read head at
//!   [`GLIDE`] samples per sample (a short pitch bend, no click).
//! - **`spring_tension`** sets the chirp rate: `a` = 0.55 (a nearly plain
//!   delay, short chirps) to 0.92 (the bass trails by ~11 ms per pass at
//!   250 Hz against 1 kHz). It glides over ~20 ms; the delay follows it.
//! - **Glides.** Gain, filter and coefficient moves glide with a 20 ms
//!   time constant, the read head at [`GLIDE`]: a decay, damping, size or
//!   tension throw on a sustained note does not click.
//! - **`decay`** is the T60 of the echo train at 1 kHz: the loop gain is
//!   `10^(−3·T/T60)` for the round trip `T`, divided by the low-pass's
//!   gain at 1 kHz. A frequency whose loop time differs (the cascade's
//!   dispersion) decays in proportion to its own loop time: at the
//!   default tension 500 Hz runs ~1 % long, at full tension ~11 %.
//! - **`damping`** is the loop low-pass: 0.5625 × `damping` (so the 8 kHz
//!   default gives the classic tank's ~4.5 kHz), clamped to 1–12 kHz.
//! - **`spring_drip`** feeds a transient-gated copy of the input
//!   (`|fast| ≫ |slow|` peak followers, the ducker's [`Ballistics`]) through
//!   a steeper, separate cascade (coefficient 0.95) into both springs: a
//!   pluck or a snare gets an extra falling "drip" chirp that then echoes
//!   with the rest, a sustained note gets none once it has settled.
//! - **Stereo.** Mono-in (the sum, high-passed at 90 Hz: a spring tank
//!   carries no low bass). Spring B is 13 % longer and 2 % less dispersive
//!   than A; A is the left output, B the right.
//! - **No early reflections.** Everything is returned in `late_*` and
//!   `er_*` is zero, so the shared ER/tail balance only scales the whole
//!   spring (toward −1 it fades out). `er_level`, `er_time`, `diffusion`,
//!   `mod_*`, the decay-shape multipliers and `tail_build` are ignored.
//! - **Freeze** is ignored (the spec greys it for Spring).
//!
//! Every setter dedupes (the plugin calls each one every block); the loop
//! gain and filter are redesigned once at the next sample however many
//! of them moved; nothing after [`SpringEngine::new`] allocates; `clear()`
//! lands every glide on its target, so a cleared engine is a fresh one.

use resonance_dsp::dynamics::Ballistics;
use resonance_dsp::reverb::{allpass_read, stage_group_delay, DispersiveAllpass};
use resonance_dsp::{Biquad, DelayLine};

use super::super::er::ER_TAPS;
use super::super::CHANNELS;
use super::{Extras, Wet};

/// Allpass stages per spring.
pub const STAGES: usize = 40;
/// Allpass stages in the drip cascade.
const DRIP_STAGES: usize = 24;
/// The drip cascade's coefficient: a long chirp into the bass.
const DRIP_COEFFICIENT: f32 = 0.95;
/// Drip level at `spring_drip` 1.
const DRIP_GAIN: f32 = 1.5;
/// Round trip at `size` 0 and 1, ms.
const ROUND_TRIP_MS: (f32, f32) = (30.0, 90.0);
/// Spring B's length and dispersion against A's.
const B_LENGTH: f32 = 1.13;
const B_DISPERSION: f32 = 0.98;
/// Coefficient at `spring_tension` 0 and 1.
const TENSION_COEFFICIENT: (f32, f32) = (0.55, 0.92);
/// The frequency the round trip and the decay are exact at.
const REFERENCE_HZ: f32 = 1_000.0;
/// Read-head slew on a size or tension change, samples per sample.
const GLIDE: f32 = 0.04;
/// Coefficient glide time constant, ms.
const COEFFICIENT_GLIDE_MS: f32 = 20.0;
/// Input high-pass, Hz.
const INPUT_HPF_HZ: f32 = 90.0;
/// Loop low-pass cutoff per Hz of `damping`, and its clamp.
const DAMPING_SCALE: f32 = 0.5625;
const LOOP_LPF_RANGE: (f32, f32) = (1_000.0, 12_000.0);
/// Butterworth Q of the loop low-pass.
const LOOP_LPF_Q: f32 = std::f32::consts::FRAC_1_SQRT_2;
/// Largest loop gain (a 30 s decay on the longest spring is 0.9979).
const MAX_LOOP_GAIN: f32 = 0.9995;
/// Output gain: a unit impulse into the default spring comes back at
/// −8 dB of energy (Room at the global defaults: −9.6 dB).
const OUT_GAIN: f32 = 0.3;
/// Transient detector: fast and slow peak followers (attack, release ms)
/// and how far the fast one must sit over the slow one to count.
const FAST_MS: (f32, f32) = (0.1, 10.0);
const SLOW_MS: (f32, f32) = (20.0, 150.0);
const TRANSIENT_RATIO: f32 = 2.0;

/// One dispersive loop.
struct Spring {
    cascade: DispersiveAllpass,
    line: DelayLine,
    interp: f32,
    lpf: Biquad,
    /// Current and target read position (`DelayLine::tap` units).
    read: f32,
    read_target: f32,
    /// Current and target cascade coefficient.
    a: f32,
    a_target: f32,
    /// Current and target loop gain.
    gain: f32,
    gain_target: f32,
    /// The loop low-pass's target coefficients `[b0, b1, b2, a1, a2]`; the
    /// live ones glide to them (the stability triangle is convex, so
    /// every point between two stable low-passes is stable).
    lpf_target: [f32; 5],
    /// Gain and filter are at their targets.
    settled: bool,
}

impl Spring {
    fn new(max_line: usize) -> Self {
        Self {
            cascade: DispersiveAllpass::new(STAGES, STAGES, 0.0),
            line: DelayLine::new(max_line),
            interp: 0.0,
            lpf: Biquad::identity(),
            read: 1.0,
            read_target: 1.0,
            a: 0.0,
            a_target: 0.0,
            gain: 0.0,
            gain_target: 0.0,
            lpf_target: [1.0, 0.0, 0.0, 0.0, 0.0],
            settled: true,
        }
    }

    /// Land every glide and empty the loop.
    fn clear(&mut self) {
        self.cascade.clear();
        self.line.clear();
        self.interp = 0.0;
        self.lpf.reset();
        self.read = self.read_target;
        self.a = self.a_target;
        self.cascade.set_coefficient(self.a);
        self.land();
    }

    /// Put the gain and the filter on their targets.
    fn land(&mut self) {
        self.gain = self.gain_target;
        let [b0, b1, b2, a1, a2] = self.lpf_target;
        self.lpf.assign_raw(b0, b1, b2, a1, a2);
        self.settled = true;
    }

    /// One step of the gain and filter glides.
    fn glide(&mut self, k: f32) {
        let f = &mut self.lpf;
        let live = [&mut f.b0, &mut f.b1, &mut f.b2, &mut f.a1, &mut f.a2];
        let mut done = (self.gain_target - self.gain).abs() < 1e-7;
        self.gain += k * (self.gain_target - self.gain);
        for (c, &t) in live.into_iter().zip(&self.lpf_target) {
            done &= (t - *c).abs() < 1e-7;
            *c += k * (t - *c);
        }
        if done {
            self.land();
        }
    }

    /// The glides' step and the loop's return for this sample (read
    /// before this sample's cascade output is pushed).
    #[inline]
    fn feedback(&mut self, a_step: f32) -> f32 {
        if !self.settled {
            self.glide(a_step);
        }
        if self.a != self.a_target {
            let d = self.a_target - self.a;
            self.a = if d.abs() < 1e-6 {
                self.a_target
            } else {
                self.a + d * a_step
            };
            self.cascade.set_coefficient(self.a);
        }
        if self.read != self.read_target {
            let d = self.read_target - self.read;
            self.read = if d.abs() <= GLIDE {
                self.read_target
            } else {
                self.read + GLIDE.copysign(d)
            };
        }
        let back = allpass_read(&self.line, self.read, &mut self.interp);
        self.lpf.process(back) * self.gain
    }
}

/// A transient detector: how much the input's fast peak envelope stands
/// over its slow one, `0..=1`.
struct Transient {
    fast: Ballistics,
    slow: Ballistics,
    fast_env: f32,
    slow_env: f32,
}

impl Transient {
    fn new(sample_rate: f32) -> Self {
        Self {
            fast: Ballistics::from_times(sample_rate, FAST_MS.0, FAST_MS.1),
            slow: Ballistics::from_times(sample_rate, SLOW_MS.0, SLOW_MS.1),
            fast_env: 0.0,
            slow_env: 0.0,
        }
    }

    fn clear(&mut self) {
        self.fast_env = 0.0;
        self.slow_env = 0.0;
    }

    #[inline]
    fn next(&mut self, x: f32) -> f32 {
        let m = x.abs();
        self.fast_env = self.fast.step_envelope(self.fast_env, m);
        self.slow_env = self.slow.step_envelope(self.slow_env, m);
        if self.fast_env <= 1e-9 {
            return 0.0;
        }
        (1.0 - TRANSIENT_RATIO * self.slow_env / self.fast_env).clamp(0.0, 1.0)
    }
}

pub struct SpringEngine {
    sample_rate: f32,
    springs: [Spring; 2],
    hpf: Biquad,
    drip: DispersiveAllpass,
    transient: Transient,
    /// Per-sample coefficient glide step.
    a_step: f32,
    // Last values set (dedupe).
    size: f32,
    decay: f32,
    damping: f32,
    tension: f32,
    drip_amount: f32,
    /// The loop gain and filter need a redesign (at the next sample).
    dirty: bool,
    primed: bool,
    energies: [f32; CHANNELS],
}

impl SpringEngine {
    pub fn new(sample_rate: f32) -> Self {
        let longest = ROUND_TRIP_MS.1 * B_LENGTH * 0.001 * sample_rate;
        let max_line = longest.ceil() as usize + 8;
        let mut hpf = Biquad::identity();
        hpf.set_first_order_high_pass(sample_rate, INPUT_HPF_HZ);
        let defaults = Extras::default();
        let mut e = Self {
            sample_rate,
            springs: [Spring::new(max_line), Spring::new(max_line)],
            hpf,
            drip: DispersiveAllpass::new(DRIP_STAGES, DRIP_STAGES, DRIP_COEFFICIENT),
            transient: Transient::new(sample_rate),
            a_step: 1.0 - (-1.0 / (COEFFICIENT_GLIDE_MS * 0.001 * sample_rate)).exp(),
            size: 0.5,
            decay: 2.0,
            damping: 8_000.0,
            tension: defaults.spring_tension,
            drip_amount: defaults.spring_drip,
            dirty: false,
            primed: false,
            energies: [0.0; CHANNELS],
        };
        e.redesign();
        e.clear();
        e
    }

    /// Round trip of spring `k`, seconds.
    fn round_trip(&self, k: usize) -> f32 {
        let (lo, hi) = ROUND_TRIP_MS;
        let t = lo * (hi / lo).powf(self.size.clamp(0.0, 1.0)) * 0.001;
        if k == 0 {
            t
        } else {
            t * B_LENGTH
        }
    }

    fn loop_cutoff(&self) -> f32 {
        let (lo, hi) = LOOP_LPF_RANGE;
        (self.damping * DAMPING_SCALE).clamp(lo, hi.min(0.45 * self.sample_rate))
    }

    /// Coefficient targets, delays, loop gains and filters for the current
    /// values. Glides start from where they are.
    fn redesign(&mut self) {
        self.dirty = false;
        let sr = self.sample_rate;
        let (a0, a1) = TENSION_COEFFICIENT;
        let a = a0 + (a1 - a0) * self.tension.clamp(0.0, 1.0);
        let fc = self.loop_cutoff();
        // A 2nd-order Butterworth's group delay well under its cutoff.
        let lpf_delay = std::f32::consts::SQRT_2 / (std::f32::consts::TAU * fc) * sr;
        let t60 = self.decay.clamp(0.05, 60.0);
        for k in 0..2 {
            let t = self.round_trip(k);
            let a_k = if k == 0 { a } else { a * B_DISPERSION };
            let cascade = STAGES as f32 * stage_group_delay(a_k, REFERENCE_HZ, sr);
            let line = (t * sr - cascade - lpf_delay).max(2.0);
            let s = &mut self.springs[k];
            s.a_target = a_k;
            // `allpass_read` at `pos` delays by `pos + 1` samples, read
            // before this sample is pushed: `line` in all.
            s.read_target = line - 1.0;
            let mut design = Biquad::identity();
            design.set_low_pass(sr, fc, LOOP_LPF_Q);
            s.lpf_target = [design.b0, design.b1, design.b2, design.a1, design.a2];
            let lpf_gain = design.magnitude(REFERENCE_HZ, sr).max(1e-3);
            s.gain_target = (10f32.powf(-3.0 * t / t60) / lpf_gain).min(MAX_LOOP_GAIN);
            s.settled = false;
        }
        if !self.primed {
            // Nothing is sounding: land the glides.
            for s in &mut self.springs {
                s.read = s.read_target;
                s.a = s.a_target;
                s.cascade.set_coefficient(s.a);
                s.land();
            }
        }
    }

    pub fn set_size(&mut self, v: f32) {
        if v != self.size {
            self.size = v;
            self.dirty = true;
        }
    }

    pub fn set_decay(&mut self, v: f32) {
        if v != self.decay {
            self.decay = v;
            self.dirty = true;
        }
    }

    /// Spring ignores Freeze (§4.2).
    pub fn set_freeze(&mut self, _v: bool) {}

    pub fn set_damping(&mut self, v: f32) {
        if v != self.damping {
            self.damping = v;
            self.dirty = true;
        }
    }

    pub fn set_er_level(&mut self, _v: f32) {}
    pub fn set_er_time(&mut self, _v: f32) {}
    pub fn set_mod_rate(&mut self, _v: f32) {}
    pub fn set_mod_depth(&mut self, _v: f32) {}
    pub fn set_decay_shape(&mut self, _low_mult: f32, _low_xover_hz: f32, _high_mult: f32) {}
    pub fn set_build(&mut self, _v: f32) {}

    pub fn set_extras(&mut self, extras: &Extras) {
        if extras.spring_tension != self.tension {
            self.tension = extras.spring_tension;
            self.dirty = true;
        }
        self.drip_amount = extras.spring_drip.clamp(0.0, 1.0);
    }

    #[inline]
    pub fn process(&mut self, l: f32, r: f32, _diffusion: f32) -> Wet {
        if self.dirty {
            self.redesign();
        }
        self.primed = true;
        let x = self.hpf.process(0.5 * (l + r));
        let t = self.transient.next(x);
        let drip = self.drip.process(x * t * self.drip_amount * DRIP_GAIN);
        let input = x + drip;
        let a_step = self.a_step;
        let fa = self.springs[0].feedback(a_step);
        let fb = self.springs[1].feedback(a_step);
        // Both cascades at once: two independent chains interleave.
        let [sa, sb] = &mut self.springs;
        let (ya, yb) = sa
            .cascade
            .process_pair(&mut sb.cascade, input + fa, input + fb);
        sa.line.push(ya);
        sb.line.push(yb);
        // Tank view: a ~4 ms follower per spring, A on the even slots.
        let (ea, eb) = (ya.abs(), yb.abs());
        for (c, e) in self.energies.iter_mut().enumerate() {
            let m = if c % 2 == 0 { ea } else { eb };
            *e += 0.005 * (m - *e);
        }
        Wet {
            er_l: 0.0,
            er_r: 0.0,
            late_l: ya * OUT_GAIN,
            late_r: yb * OUT_GAIN,
        }
    }

    pub fn clear(&mut self) {
        if self.dirty {
            self.redesign();
        }
        for s in &mut self.springs {
            s.clear();
        }
        self.hpf.reset();
        self.drip.clear();
        self.transient.clear();
        self.energies = [0.0; CHANNELS];
        self.primed = false;
    }

    pub fn channel_energies(&self) -> [f32; CHANNELS] {
        self.energies
    }

    /// The two springs' round trips, ms (A on the even slots).
    pub fn fdn_delay_ms(&self) -> [f32; CHANNELS] {
        std::array::from_fn(|c| self.round_trip(c % 2) * 1000.0)
    }

    pub fn er_tap_times_ms(&self) -> [(f32, f32); ER_TAPS] {
        [(0.0, 0.0); ER_TAPS]
    }

    pub fn er_tap_gains(&self) -> [(f32, f32); ER_TAPS] {
        [(0.0, 0.0); ER_TAPS]
    }
}
