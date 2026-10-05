//! **Shimmer** (reverb-algorithms.md §4.4, phase R8): a hall whose
//! feedback loop runs partly through pitch shifters, so every pass adds a
//! copy of the tail shifted by `shimmer_pitch`: the rising, organ-like
//! halo of ambient and post-rock.
//!
//! ```text
//!          ┌─> spread ER (12 taps, 30–120 ms) ──────────────────────────────> er
//! input ───┤
//!          └─> input diffusion ─> build line ─ 16 taps over T_build ─┐
//!                                                                    ├─────┐
//!   16-line loop: absorption ─┬──────────────────────> Hadamard ─> lines   │ direct
//!                             └─ 4 components ─> shift ─> LP ─> HP ─> cap ─┘
//!                                (share `shimmer_amount`)                  └──> late
//! ```
//!
//! **The hall.** Everything around the loop is the Hall's design (see
//! `hall.rs`): its spread ER (`hall/er.rs`, shared), its 8-stage input
//! diffusion, its build line (`tail_build` spreads *when* energy enters
//! the loop: line `i` is fed from `u_i · T_build`, `T_build = 20 ms ·
//! 15^build`), its 40–200 ms line range (20–100 ms at `size` 0), seed 49
//! (so the same line lengths), per-line absorption from `decay`,
//! `damping` and the decay multipliers ([`DecayBands::from_mults`]), and
//! the same `SmoothRandom` modulation (`mod_rate / 2` targets per second,
//! up to 0.5 ms at full `mod_depth`). With `shimmer_amount` 0 the loop is
//! exactly that hall.
//!
//! **The shimmer** is in `shimmer/tank.rs` (where the shifted share enters
//! the loop, and the proof that no pitch or amount makes it run away) and
//! `shimmer/shifter.rs` (the two-tap crossfading shifter and the bound on
//! its gain that sets the loop's cap). The cap per pitch at 48 kHz:
//! +12 0.77, +7 0.86, +5 0.88, −12 0.69, +19 0.79, +24 0.60
//! (`the_shifter_gain_is_inside_the_cap` prints them). The tank also
//! compensates the decay for what the routed share loses per pass, so
//! `decay` stays the mid T30 within about ±15 % with the shimmer in.
//!
//! **Freeze** is the shared ramp (`room/freeze.rs`, 100 ms, as Plate and
//! Hall): the input, the build line's feed into the loop, the loop's loss
//! and the routed shimmer share all fade together, and only when the ramp
//! lands does the loop switch to exactly lossless (and its modulation
//! fade out). The tail holds what it had: the halo stops climbing while
//! frozen (see the tank's docs for why a frozen loop cannot keep
//! shifting at a constant level). Release ramps all four back.
//!
//! **Parameter changes** are applied lazily, once per sample at most and
//! only when something moved (the plugin calls every setter every
//! block). Before the first sample (fresh or cleared) everything snaps;
//! after it the lines, ER taps and build taps glide at [`GLIDE`] samples
//! per sample, and the routed share and the cap slew over 50 ms (the
//! decay compensation redesigned every 16 samples along the way). A
//! pitch change leaves the shifter taps where they are and changes only
//! their speed, so switching `shimmer_pitch` on running audio does not
//! step.
//!
//! Every buffer is allocated in [`ShimmerEngine::new`] (see
//! [`ShimmerEngine::buffer_bytes`]).

pub mod shifter;
mod tank;

use resonance_dsp::reverb::{Allpass, DecayBands};
use resonance_dsp::DelayLine;

use super::super::er::ER_TAPS;
use super::super::CHANNELS;
use super::room::{stretch, FreezeRamp, FreezeTick};
use super::{Extras, Wet};
use super::hall::er::HallEr;
use tank::{direction_sign, Tank, LINES};

/// Loop line range at `size` 1.
const FDN_MIN_MS: f32 = 40.0;
const FDN_MAX_MS: f32 = 200.0;
/// Loop size at `size` 0 (it reaches 1 at `size` 1).
const FDN_SIZE_MIN: f32 = 0.5;
const FDN_SEED: u64 = 49;
/// Modulation depth at `mod_depth` 1, samples at 48 kHz.
const MOD_DEPTH_48K: f32 = 24.0;
/// SmoothRandom targets per second per Hz of `mod_rate`.
const MOD_RATE_SCALE: f32 = 0.5;

/// Slew of every moving read head (lines, ER taps, build taps), in
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
/// Each line's position in the build window (the Hall's order).
const BUILD_ORDER: [u8; 8] = [3, 6, 0, 5, 2, 7, 1, 4];
/// Tap weight at the very start of the window (`BUILD_FLOOR + u`).
const BUILD_FLOOR: f32 = 0.1;

/// Share of each line's injection heard directly.
const DIRECT: f32 = 0.25;
/// Output gain per line (the ±1 mixes over 16 lines, normalised).
const OUT_GAIN: f32 = 0.25;

/// Samples between decay-compensation redesigns while the amount slews.
const REDESIGN_EVERY: u32 = 16;

#[inline]
fn glide(pos: &mut f32, target: f32) {
    if *pos != target {
        *pos += (target - *pos).clamp(-GLIDE, GLIDE);
    }
}

pub struct ShimmerEngine {
    sample_rate: f32,
    er: HallEr,
    diffusers: [[Allpass; DIFFUSERS]; 2],
    diffusion: f32,
    build_l: DelayLine,
    build_r: DelayLine,
    build_u: [f32; LINES],
    build_w: [f32; LINES],
    build_pos: [f32; LINES],
    build_target: [f32; LINES],
    build_max: f32,
    tank: Tank,
    out_l: [f32; LINES],
    out_r: [f32; LINES],

    // Parameters as last set (applied lazily).
    size: f32,
    t60: f32,
    damping: f32,
    shape: (f32, f32, f32),
    mod_rate: f32,
    mod_depth: f32,
    build: f32,
    semitones: f32,
    amount: f32,
    dirty: bool,
    /// False until the first sample since `new`/`clear`: changes snap.
    primed: bool,

    freeze: FreezeRamp,
    /// Samples to the next compensation redesign while the amount
    /// slews, and whether a final one is owed when it lands.
    amount_countdown: u32,
    amount_landing: bool,

    energy: [f32; LINES],
}

impl ShimmerEngine {
    pub fn new(sample_rate: f32) -> Self {
        let sr = sample_rate;
        let tank = Tank::new(sr, FDN_MIN_MS, FDN_MAX_MS, MOD_DEPTH_48K * sr / 48_000.0, FDN_SEED);
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
        let defaults = Extras::default();
        let mut engine = Self {
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
            tank,
            out_l: std::array::from_fn(|i| OUT_GAIN * direction_sign(i, 0)),
            out_r: std::array::from_fn(|i| OUT_GAIN * direction_sign(i, 0b0010)),
            size: 0.5,
            t60: 2.0,
            damping: 8_000.0,
            shape: (1.0, 250.0, 0.5),
            mod_rate: 1.0,
            mod_depth: 0.3,
            build: 0.5,
            semitones: defaults.shimmer_semitones,
            amount: defaults.shimmer_amount,
            dirty: true,
            primed: false,
            freeze: FreezeRamp::new(sr),
            amount_countdown: REDESIGN_EVERY,
            amount_landing: false,
            energy: [0.0; LINES],
        };
        engine.apply();
        engine
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
        if v == self.freeze.on() {
            return;
        }
        self.freeze.set(v);
        if !self.primed {
            // Nothing in flight: land in the settled state directly, the
            // state `clear` leaves a frozen (or released) engine in.
            self.settle_freeze();
        } else if !v && self.tank.is_frozen() {
            // Out of the lossless loop at zero loss (the design it holds
            // since the ramp landed); `process` ramps the loss back.
            self.tank.set_freeze(false);
        } else if v && self.freeze.held() {
            // Re-engaged before a sample moved the ramp off frozen: the
            // ramp is already there and will not `Engage` again.
            self.tank.set_freeze(true);
        }
        // Freezing while running: `process` ramps into it.
    }

    /// The Freeze ramp at its target and the tank in the matching state,
    /// cleared. Only while nothing sounds.
    fn settle_freeze(&mut self) {
        self.freeze.snap();
        self.tank.set_freeze(self.freeze.on());
        self.tank.clear();
        self.apply_loss();
    }

    /// The treble crossover, Hz (held at least an octave above
    /// `low_xover`).
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

    /// `shimmer_semitones` and `shimmer_amount`; the rest is not ours.
    pub fn set_extras(&mut self, extras: &Extras) {
        let amount = extras.shimmer_amount.clamp(0.0, 1.0);
        if extras.shimmer_semitones != self.semitones || amount != self.amount {
            self.semitones = extras.shimmer_semitones;
            self.amount = amount;
            self.dirty = true;
        }
    }

    /// The build, `0..=1`: the late field's injection spread
    /// `T_build = 20 ms · 15^build`.
    pub fn set_build(&mut self, v: f32) {
        let v = v.clamp(0.0, 1.0);
        if v != self.build {
            self.build = v;
            self.dirty = true;
        }
    }

    /// The decay target the loop is designed for, at the loss the Freeze
    /// ramp leaves.
    fn bands(&self) -> DecayBands {
        let (lo, xover, hi) = self.shape;
        let damping = self.damping.max(2.0 * xover);
        let bands =
            DecayBands::from_mults(self.t60.max(0.05), lo.max(0.01), hi.max(0.01), xover, damping);
        stretch(bands, self.freeze.loss())
    }

    /// Redesign the loop's absorption if its target moved (the decay, or
    /// the routed share it is compensated for).
    fn apply_loss(&mut self) {
        let share = self.tank.amount_now() * self.freeze.gain();
        self.tank.set_decay(self.bands(), share);
    }

    /// Push the parameters into the DSP.
    fn apply(&mut self) {
        self.dirty = false;
        let snap = !self.primed;
        if snap {
            self.tank.set_glide(0.0);
        }
        let fdn_size = FDN_SIZE_MIN + (1.0 - FDN_SIZE_MIN) * self.size;
        if fdn_size != self.tank.size() {
            self.tank.set_size(fdn_size);
        }
        if snap {
            self.tank.set_glide(GLIDE);
        }
        self.tank.set_semitones(self.semitones);
        self.tank.set_amount(self.amount);
        if snap {
            self.tank.snap_shift();
        }
        self.apply_loss();
        let depth = self.mod_depth.clamp(0.0, 1.0) * MOD_DEPTH_48K * self.sample_rate / 48_000.0;
        self.tank
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

        // Freeze: the input, the loop's feed, its loss and the routed
        // share fade together, then the loop goes lossless.
        match self.freeze.tick() {
            FreezeTick::Still | FreezeTick::Moving => {}
            FreezeTick::Redesign => self.apply_loss(),
            FreezeTick::Engage => {
                self.apply_loss();
                self.tank.set_freeze(true);
            }
        }
        // The decay compensation follows the slewing amount (see the
        // tank's docs), and lands with it.
        if self.tank.amount_moving() {
            self.amount_countdown -= 1;
            if self.amount_countdown == 0 {
                self.amount_countdown = REDESIGN_EVERY;
                self.apply_loss();
            }
            self.amount_landing = true;
        } else if self.amount_landing {
            self.amount_landing = false;
            self.amount_countdown = REDESIGN_EVERY;
            self.apply_loss();
        }
        let g = self.freeze.gain();
        let (l, r) = (l * g, r * g);

        let (er_l, er_r) = self.er.process(l, r);

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

        // Build: each line is fed from its own point of the window (and
        // the feed faded by Freeze, after the build line).
        let mut taps = [0.0f32; LINES];
        let mut feed = [0.0f32; LINES];
        for i in 0..LINES {
            glide(&mut self.build_pos[i], self.build_target[i]);
            let line = if i % 2 == 0 { &self.build_l } else { &self.build_r };
            taps[i] = self.build_w[i] * line.tap_linear(self.build_pos[i]);
            feed[i] = g * taps[i];
        }

        let y = self.tank.tick(&feed, g);
        let (mut late_l, mut late_r) = (0.0f32, 0.0f32);
        for i in 0..LINES {
            let v = y[i] + DIRECT * taps[i];
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
        self.settle_freeze();
        self.amount_countdown = REDESIGN_EVERY;
        self.amount_landing = false;
        self.energy = [0.0; LINES];
        self.primed = false;
    }

    /// The 16 line energies folded pairwise into the tank view's 8.
    pub fn channel_energies(&self) -> [f32; CHANNELS] {
        std::array::from_fn(|c| 0.5 * (self.energy[2 * c] + self.energy[2 * c + 1]))
    }

    /// The 16 line lengths (ascending) folded pairwise into 8, ms.
    pub fn fdn_delay_ms(&self) -> [f32; CHANNELS] {
        let len = self.tank.line_lengths();
        let k = 1000.0 / self.sample_rate;
        std::array::from_fn(|c| 0.5 * (len[2 * c] + len[2 * c + 1]) as f32 * k)
    }

    pub fn er_tap_times_ms(&self) -> [(f32, f32); ER_TAPS] {
        self.er.tap_times_ms()
    }

    pub fn er_tap_gains(&self) -> [(f32, f32); ER_TAPS] {
        self.er.tap_gains()
    }

    /// The gain cap on the shifted path at the current pitch (see
    /// `shimmer/tank.rs`).
    pub fn shift_cap(&self) -> f32 {
        self.tank.cap_target()
    }

    /// Bytes held in delay buffers: the engine's audio memory, all of it
    /// allocated in [`ShimmerEngine::new`].
    pub fn buffer_bytes(&self) -> usize {
        let f = std::mem::size_of::<f32>();
        let build = 2 * (self.build_max as usize + 4).next_power_of_two() * f;
        let diff: usize = self
            .diffusers
            .iter()
            .flatten()
            .map(|ap| (ap.delay() + 2).next_power_of_two() * f)
            .sum();
        self.tank.buffer_bytes() + build + diff + self.er.buffer_bytes()
    }
}
