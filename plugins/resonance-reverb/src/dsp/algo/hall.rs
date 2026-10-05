//! **Hall** (reverb-algorithms.md §4.4, phase R5): sparse early
//! reflections, a 16-line FDN with per-line absorption and slow random
//! modulation, and a build that brings the late field in over 20–300 ms.
//!
//! ```text
//!          ┌─> spread ER (12 taps, 30–120 ms, tapering) ───────────────────> er
//! input ───┤
//!          └─> input diffusion (8 allpasses/side) ─> build line ─┐
//!                                  16 taps over T_build, rising weights
//!                                                                ├──────────┐
//!                    Fdn<16> (Hadamard, Absorption, SmoothRandom) <┘  direct │
//!                                └──> two orthogonal ±1 output mixes <──────┘ ─> late
//! ```
//!
//! **Late field.** A 16-line [`Fdn`] with Hadamard feedback and lines of
//! 40–200 ms at `size` 1 (20–100 ms at `size` 0; distinct primes from the
//! Fdn's seeded log-uniform draw). Per-line [`Absorption`] makes `decay`
//! the mid-band T60 exactly, `damping` the treble crossover, and
//! `low_decay_mult` / `low_xover` / `high_decay_mult` the band multipliers
//! ([`DecayBands::from_mults`]). Modulation is the Fdn's per-line
//! [`SmoothRandom`] (Lexicon random-hall style): `mod_rate / 2` new targets
//! per second and, at full `mod_depth`, up to 24 samples (0.5 ms) of read
//! offset. Its pitch excursion is bounded by `3·depth·rate/fs`: about a
//! cent at the defaults, so it breaks the modes up without a wobble.
//!
//! The Fdn seed (49) was picked from a scan of 80 seeds for the lowest
//! modal peakiness over sizes 0.2–0.9, decays 2.5–10 s and every build.
//! A 16-line Hadamard network's late spectrum varies by a couple of dB of
//! peakiness from one draw of line lengths to the next.
//!
//! **Build.** The diffused input is written into a stereo *build line* and
//! each FDN line is fed from its own tap on it: line `i` reads at
//! `u_i · T_build` (the `u_i` are a fixed permutation spread over `[0, 1)`,
//! even lines from the left side, odd from the right) with a weight that
//! rises with `u_i` (`0.1 + u_i`, normalised to unit energy per side).
//! `T_build = 20 ms · 15^build`: 20 ms at 0, 77 ms at the 0.5 default,
//! 300 ms at 1. The late field is therefore not a gain ramp on a tail that
//! is already there: its *injection* is spread over `T_build` with the
//! later injections louder, and each injection starts its own
//! recirculation. The late energy peaks later and the response turns
//! noise-like later as `build` rises, while the energy put into the loop
//! (so the decay and the level) does not depend on it. A quarter of each
//! injection is also heard directly (the diffused onset of the late field,
//! as Classic sums its diffuser output), so the late field fades in under
//! the reflections instead of starting at the shortest line. The spread
//! ER carries the first 30–120 ms and tapers off across it: a crossfade
//! from discrete reflections into the wash. Moving `build` glides the
//! taps.
//!
//! **Stereo.** The late outputs are two ±1 mixes of the line outputs, the
//! bent function `b0·b1 ⊕ b2·b3` of the line index for L and the same
//! `⊕ b1` for R. The two are orthogonal (uncorrelated L and R: low IACC,
//! no mono cancellation), and each is "flat" through the Hadamard: the
//! signal an output reads is spread evenly over the lines it feeds next.
//! A plain Hadamard row would make each output the next input of one
//! line, a comb at that line's length.
//!
//! **Freeze.** The input (ER and late path) ramps to zero over 10 ms; once
//! it is silent the Fdn goes lossless (its own freeze also mutes its input
//! and fades the modulation out). Release unfreezes the Fdn at once and
//! ramps the input back.
//!
//! **Parameter changes** are applied lazily, at most once per sample and
//! only when something moved: the plugin calls every setter every block,
//! and a redesign of 16 absorption filters is not free. Before the first
//! sample (fresh or cleared) everything snaps; after it, the FDN lines,
//! the ER taps and the build taps glide at [`GLIDE`] samples per sample
//! (a ±1.6-semitone bend at most while they move, no click).
//!
//! Every buffer is allocated in [`HallEngine::new`]: 2.6 MiB at 96 kHz,
//! 1.3 MiB at 48 kHz (see [`HallEngine::buffer_bytes`]).

mod er;

use resonance_dsp::reverb::{Allpass, DecayBands, Fdn, FdnConfig};
use resonance_dsp::DelayLine;

use super::super::er::ER_TAPS;
use super::super::CHANNELS;
use super::Wet;
use er::HallEr;

const LINES: usize = 16;
/// FDN line range at `size` 1.
const FDN_MIN_MS: f32 = 40.0;
const FDN_MAX_MS: f32 = 200.0;
/// Fdn size at `size` 0 (it reaches 1 at `size` 1).
const FDN_SIZE_MIN: f32 = 0.5;
const FDN_SEED: u64 = 49;
/// Modulation depth at `mod_depth` 1, samples at 48 kHz.
const MOD_DEPTH_48K: f32 = 24.0;
/// SmoothRandom targets per second per Hz of `mod_rate`.
const MOD_RATE_SCALE: f32 = 0.5;

/// Slew of every moving read head (FDN lines, ER taps, build taps), in
/// samples per sample.
const GLIDE: f32 = 0.1;

/// Input diffusers per side, ms.
const DIFFUSERS: usize = 8;
const DIFFUSER_MS: [[f32; DIFFUSERS]; 2] = [
    [1.9, 3.1, 4.7, 7.9, 11.3, 17.9, 23.3, 29.9],
    [2.1, 3.3, 5.1, 7.3, 10.7, 19.3, 24.7, 28.1],
];
/// Allpass coefficient at `diffusion` 1.
const DIFFUSION_MAX_G: f32 = 0.75;

/// Build spread at `build` 0, and its ratio to the spread at `build` 1.
const BUILD_MIN_MS: f32 = 20.0;
const BUILD_RATIO: f32 = 15.0;
/// Each line's position in the build window, `u_i`: even lines (left)
/// take the quarter points, odd lines (right) the three-quarter points,
/// both in a scrambled order so long and short lines are fed early and
/// late alike.
const BUILD_ORDER: [u8; 8] = [3, 6, 0, 5, 2, 7, 1, 4];
/// Tap weight at the very start of the window (`BUILD_FLOOR + u`).
const BUILD_FLOOR: f32 = 0.1;

/// Share of each line's injection heard directly (the late field's
/// diffused onset, see the module docs).
const DIRECT: f32 = 0.25;
/// Output gain per line (the ±1 mixes over 16 lines, normalised).
const OUT_GAIN: f32 = 0.25;

/// Freeze input ramp, ms.
const FREEZE_RAMP_MS: f32 = 10.0;

/// Output sign of line `i`: the bent function `b0·b1 ⊕ b2·b3` of its
/// index bits for L, the same `⊕ b1` for R (see the module docs).
fn output_sign(i: usize, right: bool) -> f32 {
    let b = |k: usize| (i >> k) & 1;
    let f = (b(0) & b(1)) ^ (b(2) & b(3)) ^ if right { b(1) } else { 0 };
    if f == 0 {
        1.0
    } else {
        -1.0
    }
}

#[inline]
fn glide(pos: &mut f32, target: f32) {
    if *pos != target {
        *pos += (target - *pos).clamp(-GLIDE, GLIDE);
    }
}

pub struct HallEngine {
    sample_rate: f32,
    er: HallEr,
    diffusers: [[Allpass; DIFFUSERS]; 2],
    diffusion: f32,
    build_l: DelayLine,
    build_r: DelayLine,
    /// `u_i` and the normalised weight of each line's build tap.
    build_u: [f32; LINES],
    build_w: [f32; LINES],
    build_pos: [f32; LINES],
    build_target: [f32; LINES],
    build_max: f32,
    fdn: Fdn<LINES>,
    out_l: [f32; LINES],
    out_r: [f32; LINES],

    // Parameters as last set (applied lazily, see the module docs).
    size: f32,
    t60: f32,
    damping: f32,
    shape: (f32, f32, f32),
    mod_rate: f32,
    mod_depth: f32,
    build: f32,
    frozen: bool,
    dirty: bool,
    /// False until the first sample since `new`/`clear`: changes snap.
    primed: bool,

    /// Input gain, ramped by Freeze.
    in_gain: f32,
    ramp_step: f32,

    energy: [f32; LINES],
}

impl HallEngine {
    pub fn new(sample_rate: f32) -> Self {
        let sr = sample_rate;
        let fdn = Fdn::new(
            sr,
            FdnConfig {
                min_ms: FDN_MIN_MS,
                max_ms: FDN_MAX_MS,
                max_size: 1.0,
                max_mod_depth: MOD_DEPTH_48K * sr / 48_000.0,
                seed: FDN_SEED,
                matrix: None,
            },
        );
        let diffusers = DIFFUSER_MS.map(|side| {
            side.map(|ms| {
                let d = ((ms * 0.001 * sr).round() as usize).max(1);
                Allpass::new(d, d, 0.0)
            })
        });
        let build_max = (BUILD_MIN_MS * BUILD_RATIO * 0.001 * sr).ceil() + 2.0;
        let mut build_u = [0.0; LINES];
        for (k, &o) in BUILD_ORDER.iter().enumerate() {
            build_u[2 * k] = (o as f32 + 0.25) / 8.0;
            build_u[2 * k + 1] = (BUILD_ORDER[(k + 3) % 8] as f32 + 0.75) / 8.0;
        }
        let mut build_w = build_u.map(|u| BUILD_FLOOR + u);
        for side in 0..2 {
            let e: f32 = build_w.iter().skip(side).step_by(2).map(|w| w * w).sum();
            let n = 1.0 / e.sqrt();
            for w in build_w.iter_mut().skip(side).step_by(2) {
                *w *= n;
            }
        }
        let mut hall = Self {
            sample_rate: sr,
            er: HallEr::new(sr),
            diffusers,
            diffusion: 0.0,
            build_l: DelayLine::new(build_max as usize + 4),
            build_r: DelayLine::new(build_max as usize + 4),
            build_u,
            build_w,
            build_pos: [0.0; LINES],
            build_target: [0.0; LINES],
            build_max,
            fdn,
            out_l: std::array::from_fn(|i| OUT_GAIN * output_sign(i, false)),
            out_r: std::array::from_fn(|i| OUT_GAIN * output_sign(i, true)),
            size: 0.5,
            t60: 2.0,
            damping: 8_000.0,
            shape: (1.0, 250.0, 0.5),
            mod_rate: 1.0,
            mod_depth: 0.3,
            build: 0.5,
            frozen: false,
            dirty: true,
            primed: false,
            in_gain: 1.0,
            ramp_step: 1.0 / (FREEZE_RAMP_MS * 0.001 * sr).max(1.0),
            energy: [0.0; LINES],
        };
        hall.apply();
        hall
    }

    pub fn set_size(&mut self, v: f32) {
        let v = v.clamp(0.0, 1.0);
        if v != self.size {
            self.size = v;
            self.dirty = true;
        }
    }

    /// Mid-band T60, seconds.
    pub fn set_decay(&mut self, v: f32) {
        if v != self.t60 {
            self.t60 = v;
            self.dirty = true;
        }
    }

    pub fn set_freeze(&mut self, v: bool) {
        if v == self.frozen {
            return;
        }
        self.frozen = v;
        if !self.primed {
            // Nothing in flight: land in the settled state directly, the
            // state `clear` leaves a frozen (or released) engine in.
            self.in_gain = if v { 0.0 } else { 1.0 };
            self.fdn.set_freeze(v);
            self.fdn.clear();
        } else if !v {
            self.fdn.set_freeze(false);
        }
        // Freezing while running: `process` ramps the input out first.
    }

    /// The treble crossover, Hz (decay above it tends to `decay ×
    /// high_decay_mult`). Held at least an octave above `low_xover`.
    pub fn set_damping(&mut self, v: f32) {
        if v != self.damping {
            self.damping = v;
            self.dirty = true;
        }
    }

    pub fn set_er_level(&mut self, v: f32) {
        self.er.set_level(v);
    }

    pub fn set_er_time(&mut self, v: f32) {
        if self.er.set_time(v) {
            self.dirty = true;
        }
    }

    pub fn set_mod_rate(&mut self, v: f32) {
        if v != self.mod_rate {
            self.mod_rate = v;
            self.dirty = true;
        }
    }

    pub fn set_mod_depth(&mut self, v: f32) {
        if v != self.mod_depth {
            self.mod_depth = v;
            self.dirty = true;
        }
    }

    pub fn set_decay_shape(&mut self, low_mult: f32, low_xover_hz: f32, high_mult: f32) {
        let s = (low_mult, low_xover_hz, high_mult);
        if s != self.shape {
            self.shape = s;
            self.dirty = true;
        }
    }

    /// Hall has no creative parameters.
    pub fn set_extras(&mut self, _extras: &super::Extras) {}

    /// The build, `0..=1`: the late field's injection spread
    /// `T_build = 20 ms · 15^build`.
    pub fn set_build(&mut self, v: f32) {
        let v = v.clamp(0.0, 1.0);
        if v != self.build {
            self.build = v;
            self.dirty = true;
        }
    }

    /// The decay target the Fdn is designed for.
    fn bands(&self) -> DecayBands {
        let (lo, xover, hi) = self.shape;
        let damping = self.damping.max(2.0 * xover);
        DecayBands::from_mults(self.t60.max(0.05), lo.max(0.01), hi.max(0.01), xover, damping)
    }

    /// Push the parameters into the DSP (see the module docs).
    fn apply(&mut self) {
        self.dirty = false;
        let snap = !self.primed;
        if snap {
            self.fdn.set_glide(0.0);
        }
        let fdn_size = FDN_SIZE_MIN + (1.0 - FDN_SIZE_MIN) * self.size;
        if fdn_size != self.fdn.size() {
            self.fdn.set_size(fdn_size);
        }
        if snap {
            self.fdn.set_glide(GLIDE);
        }
        let bands = self.bands();
        if bands != self.fdn.decay() {
            self.fdn.set_decay(bands);
        }
        let depth = self.mod_depth.clamp(0.0, 1.0) * MOD_DEPTH_48K * self.sample_rate / 48_000.0;
        self.fdn
            .set_modulation(self.mod_rate.max(0.0) * MOD_RATE_SCALE, depth);

        self.er.set_size(self.size);
        self.er.retarget();
        let spread = BUILD_MIN_MS * BUILD_RATIO.powf(self.build) * 0.001 * self.sample_rate;
        for (t, u) in self.build_target.iter_mut().zip(&self.build_u) {
            *t = (u * spread).min(self.build_max);
        }
        if snap {
            self.er.snap();
            self.build_pos = self.build_target;
        }
    }

    #[inline]
    pub fn process(&mut self, l: f32, r: f32, diffusion: f32) -> Wet {
        if self.dirty {
            self.apply();
        }
        self.primed = true;

        // Freeze: ramp the input out, then make the loop lossless.
        let target = if self.frozen { 0.0 } else { 1.0 };
        if self.in_gain != target {
            self.in_gain = if self.in_gain < target {
                (self.in_gain + self.ramp_step).min(1.0)
            } else {
                (self.in_gain - self.ramp_step).max(0.0)
            };
        }
        if self.frozen && self.in_gain == 0.0 && !self.fdn.is_frozen() {
            self.fdn.set_freeze(true);
        }
        let (l, r) = (l * self.in_gain, r * self.in_gain);

        let (er_l, er_r) = self.er.process(l, r);

        // Input diffusion.
        if diffusion != self.diffusion {
            self.diffusion = diffusion;
            let g = DIFFUSION_MAX_G * diffusion.clamp(0.0, 1.0);
            for ap in self.diffusers.iter_mut().flatten() {
                ap.set_gain(g);
            }
        }
        let mut dl = l;
        for ap in &mut self.diffusers[0] {
            dl = ap.process(dl);
        }
        let mut dr = r;
        for ap in &mut self.diffusers[1] {
            dr = ap.process(dr);
        }
        self.build_l.push(dl);
        self.build_r.push(dr);

        // Build: each line is fed from its own point of the window.
        let mut input = [0.0f32; LINES];
        for (i, x) in input.iter_mut().enumerate() {
            glide(&mut self.build_pos[i], self.build_target[i]);
            let line = if i % 2 == 0 { &self.build_l } else { &self.build_r };
            *x = self.build_w[i] * line.tap_linear(self.build_pos[i]);
        }

        let y = self.fdn.tick(&input);
        let (mut late_l, mut late_r) = (0.0f32, 0.0f32);
        for i in 0..LINES {
            let v = y[i] + DIRECT * input[i];
            late_l += self.out_l[i] * v;
            late_r += self.out_r[i] * v;
            self.energy[i] = self.energy[i] * 0.995 + y[i].abs() * 0.005;
        }

        Wet {
            er_l,
            er_r,
            late_l,
            late_r,
        }
    }

    /// Back to the freshly constructed state for the current settings.
    pub fn clear(&mut self) {
        self.er.clear();
        for ap in self.diffusers.iter_mut().flatten() {
            ap.clear();
        }
        self.build_l.clear();
        self.build_r.clear();
        self.build_pos = self.build_target;
        self.in_gain = if self.frozen { 0.0 } else { 1.0 };
        self.fdn.set_freeze(self.frozen);
        self.fdn.clear();
        self.energy = [0.0; LINES];
        self.primed = false;
    }

    /// The 16 line energies folded pairwise (adjacent lengths) into the
    /// tank view's 8.
    pub fn channel_energies(&self) -> [f32; CHANNELS] {
        std::array::from_fn(|c| 0.5 * (self.energy[2 * c] + self.energy[2 * c + 1]))
    }

    /// The 16 line lengths (ascending) folded pairwise into 8, ms.
    pub fn fdn_delay_ms(&self) -> [f32; CHANNELS] {
        let len = self.fdn.line_lengths();
        let k = 1000.0 / self.sample_rate;
        std::array::from_fn(|c| 0.5 * (len[2 * c] + len[2 * c + 1]) as f32 * k)
    }

    pub fn er_tap_times_ms(&self) -> [(f32, f32); ER_TAPS] {
        self.er.tap_times_ms()
    }

    pub fn er_tap_gains(&self) -> [(f32, f32); ER_TAPS] {
        self.er.tap_gains()
    }

    /// Bytes held in delay buffers: the engine's audio memory, all of it
    /// allocated in [`HallEngine::new`].
    pub fn buffer_bytes(&self) -> usize {
        let f = std::mem::size_of::<f32>();
        // `Fdn::new`'s line allocation: the longest line plus prime-search
        // and modulation headroom, rounded up by `DelayLine`.
        let fdn_len = (FDN_MAX_MS * 0.001 * self.sample_rate).ceil() as usize
            + 80 * LINES
            + (MOD_DEPTH_48K * self.sample_rate / 48_000.0).ceil() as usize
            + 6;
        let fdn = LINES * fdn_len.next_power_of_two() * f;
        let build = 2 * (self.build_max as usize + 4).next_power_of_two() * f;
        let diff: usize = self
            .diffusers
            .iter()
            .flatten()
            .map(|ap| (ap.delay() + 2).next_power_of_two() * f)
            .sum();
        fdn + build + diff + self.er.buffer_bytes()
    }
}
