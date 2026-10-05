//! [`RoomCore`]: the engine Room, Chamber and Ambience share. A
//! [`Voicing`] fixes what differs between them; the parameters mean the
//! same thing on all three.
//!
//! ```text
//!   in ─┬──────────────────────────────► ShoeboxEarly (two ears, scatter) ──► er
//!       │                          (or ┐ from the diffused input: Ambience)
//!       └─► input diffuser (6 allpasses per side) ─┬─► Fdn<N> ──fold──┬─► late
//!                                                  └─ × direct_diffuse ┘
//! ```
//!
//! - **Late feed.** The FDN is fed from the *input* (through the
//!   diffuser), not from the early reflections: the ERs' fixed tap
//!   pattern is a comb, and fed into the loop it would print its ripple
//!   on the whole tail. The handover is in time instead: the diffused
//!   input itself (× `Voicing::direct_diffuse`) is the tail's onset from
//!   the first millisecond, the shortest line (6.4–10 ms on Room) comes
//!   round inside the ER cluster, and the tail's density grows while the
//!   reflections thin out (their envelope follows `decay`, so the sum
//!   reads as one slope), so the ER energy hands over without a gap.
//! - **`diffusion`** is the input diffuser's allpass coefficient,
//!   `diffusion × Voicing::diffuser_gain`. At 0 the allpasses are plain
//!   delays (7–9 ms in all, so the late onset does not move with the
//!   knob) and each line is fed a clean copy of the input; toward 1 every
//!   line is fed a dense smear and the echo density builds faster. On
//!   Ambience, whose reflections read the diffused input, it also sets
//!   how much each reflection is smeared.
//! - **Injection and fold.** The diffused left feeds the even lines and
//!   the right the odd ones, with fixed signs. The stereo output is two
//!   orthogonal ±1/√N combinations of all N line outputs (random signs
//!   for L, the same signs times a balanced Thue–Morse pattern for R), so
//!   the late L and R are uncorrelated for uncorrelated equal-energy
//!   lines: a mono send comes back wide.
//! - **`decay`** is the mid T60 exactly: `Fdn` designs each line's
//!   absorption from its own length ([`DecayBands::from_mults`] with
//!   `low_decay_mult`, `high_decay_mult`, `low_xover` and `damping` as the
//!   treble crossover). Clamped to [`Voicing::decay_range`].
//! - **`size`** scales the FDN lines by `size_scale.0 … size_scale.1`
//!   (log) and the shoebox geometry. The lines glide at [`GLIDE`] (a
//!   gentle Doppler bend, never a jump), the ERs crossfade.
//! - **Modulation**: each line's read moves by up to
//!   `mod_depth × Voicing::mod_depth_max` samples (at 48 kHz, scaled with
//!   the rate) along `SmoothRandom` curves at `mod_rate` targets per
//!   second.
//! - **Freeze** ramps in and out over 100 ms ([`FreezeRamp`]): the input
//!   (so the reflections, the diffusers and the direct diffuse path) and
//!   the loop's injection fade while the loop's loss falls to none, and
//!   only then does the `Fdn` switch to lossless. The reflections and the
//!   diffusers drain.
//!
//! Every setter dedupes (the plugin calls each one every block), the loop
//! redesign a decay, damping or shape change needs runs once at the next
//! sample however many of them moved, and nothing after [`RoomCore::new`]
//! allocates.

use resonance_dsp::reverb::{Allpass, DecayBands, Fdn, FdnConfig};

use super::super::super::er::ER_TAPS;
use super::super::super::CHANNELS;
use super::super::Wet;
use super::early::{EarlyVoicing, ShoeboxEarly};
use super::freeze::{stretch, FreezeRamp, FreezeTick};

/// FDN read-head slew on a size change, samples per sample: at most a
/// 4 % (0.7 semitone) Doppler bend while the lines move, and a full-range
/// size throw settles in 0.7 s on Room (≈ 1 300 samples on its longest
/// line), 1.6 s on Chamber. Faster (the 0.25 Classic uses) turns a block-rate size
/// automation into a staircase: each block's small length change is
/// covered in a few samples at +25 % pitch and then stops, and those
/// velocity jumps read as grit on a sustained note.
const GLIDE: f32 = 0.04;

/// Allpass stages in each side's input diffuser.
pub(in crate::dsp::algo) const DIFFUSER_STAGES: usize = 6;

/// What makes one member of the room family sound like itself.
#[derive(Clone, Copy, Debug)]
pub(in crate::dsp::algo) struct Voicing {
    pub early: EarlyVoicing,
    /// FDN line range at size scale 1, ms.
    pub line_ms: (f32, f32),
    /// FDN size scale at `size` 0 and 1 (log in between).
    pub size_scale: (f32, f32),
    /// Input diffuser lengths per side, ms.
    pub diffuser_ms: [[f32; DIFFUSER_STAGES]; 2],
    /// Diffuser coefficient at `diffusion` 1.
    pub diffuser_gain: f32,
    /// Feed the reflections from the diffused input instead of the dry
    /// one: every image becomes a short dense burst (Ambience's cluster).
    pub diffused_early: bool,
    /// Modulation depth at `mod_depth` 1, samples at 48 kHz (0 = none).
    pub mod_depth_max: f32,
    /// The `decay` clamp, seconds.
    pub decay_range: (f32, f32),
    /// Share of the diffused input added straight to the late output
    /// (the tail's onset, before the shortest line comes round).
    pub direct_diffuse: f32,
    /// Late output gain.
    pub late_level: f32,
    /// Late gain scales as `(1 s / T60)^late_decay_norm`: 0 keeps the
    /// physical behaviour (a longer decay is a louder tail), 0.5 holds the
    /// tail's energy constant as `decay` moves (Ambience: the tail must
    /// stay under the cluster at every decay).
    pub late_decay_norm: f32,
    pub seed: u64,
}

/// Balanced ±1 pattern (Thue–Morse): any 2ᵏ-long prefix sums to zero.
fn thue_morse(i: usize) -> f32 {
    if i.count_ones().is_multiple_of(2) {
        1.0
    } else {
        -1.0
    }
}

pub(in crate::dsp::algo) struct RoomCore<const N: usize> {
    sample_rate: f32,
    v: Voicing,
    early: ShoeboxEarly,
    fdn: Fdn<N>,
    diffuser: [[Allpass; DIFFUSER_STAGES]; 2],
    /// Per-line injection gains from the diffused left and right.
    in_l: [f32; N],
    in_r: [f32; N],
    /// Per-line fold weights into the late left and right.
    out_l: [f32; N],
    out_r: [f32; N],
    // Last values set, for dedupe (the loop is designed for the initial
    // ones in `new`; NaN: not set yet).
    size: f32,
    er_time: f32,
    decay: f32,
    damping: f32,
    shape: (f32, f32, f32),
    mod_rate: f32,
    mod_depth: f32,
    freeze: FreezeRamp,
    primed: bool,
    /// The loop needs a redesign (a decay, damping or shape change). Done
    /// at the next sample, so a block's worth of setter calls costs one.
    dirty: bool,
    /// The diffusion the diffuser coefficients were last set for (NaN:
    /// not yet).
    diffusion: f32,
    /// Tank-view envelope, one per line pair.
    energies: [f32; CHANNELS],
    /// `late_level` with the decay normalisation applied.
    late_gain: f32,
}

impl<const N: usize> RoomCore<N> {
    pub(in crate::dsp::algo) fn new(sample_rate: f32, v: Voicing) -> Self {
        let rate_scale = sample_rate / 48_000.0;
        let config = FdnConfig {
            max_size: v.size_scale.1 * 1.05,
            max_mod_depth: v.mod_depth_max * rate_scale,
            seed: v.seed,
            ..FdnConfig::new(v.line_ms.0, v.line_ms.1)
        };
        let mut fdn = Fdn::new(sample_rate, config);
        fdn.set_glide(GLIDE);
        let diffuser = std::array::from_fn(|side| {
            std::array::from_fn(|k| {
                let d = ((v.diffuser_ms[side][k] * 0.001 * sample_rate).round() as usize).max(1);
                Allpass::new(d, d, 0.0)
            })
        });
        // Fixed pseudo-random signs from the seed.
        let mut bits = v.seed ^ 0xA5A5_5A5A_C3C3_3C3C;
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
        // Each side feeds N/2 lines: unit energy per side into the tank.
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
            out_r[i] = o * thue_morse(i) * fold;
        }
        let mut core = Self {
            sample_rate,
            v,
            early: ShoeboxEarly::new(sample_rate, v.early),
            fdn,
            diffuser,
            in_l,
            in_r,
            out_l,
            out_r,
            size: f32::NAN,
            er_time: 0.5,
            decay: 2.0,
            damping: 8_000.0,
            shape: (1.0, 250.0, 0.5),
            mod_rate: f32::NAN,
            mod_depth: 0.0,
            freeze: FreezeRamp::new(sample_rate),
            primed: false,
            dirty: false,
            diffusion: f32::NAN,
            energies: [0.0; CHANNELS],
            late_gain: v.late_level,
        };
        // Design the loop for the defaults above, so a setter that
        // repeats one of them (and is deduped) leaves a consistent loop.
        core.redesign();
        core
    }

    fn size_scale(&self, size: f32) -> f32 {
        let (a, b) = self.v.size_scale;
        a * (b / a).powf(size.clamp(0.0, 1.0))
    }

    pub(in crate::dsp::algo) fn set_size(&mut self, v: f32) {
        if v == self.size {
            return;
        }
        self.size = v;
        let scale = self.size_scale(v);
        if self.primed {
            self.fdn.set_size(scale);
        } else {
            // Nothing is sounding: land the read heads on the new lengths
            // (a glide from the construction size would be audible later).
            self.fdn.set_glide(0.0);
            self.fdn.set_size(scale);
            self.fdn.set_glide(GLIDE);
        }
        self.early.set_geometry(v, self.er_time);
    }

    pub(in crate::dsp::algo) fn set_er_time(&mut self, v: f32) {
        self.er_time = v;
        let size = if self.size.is_nan() { 0.5 } else { self.size };
        self.early.set_geometry(size, v);
    }

    pub(in crate::dsp::algo) fn set_er_level(&mut self, v: f32) {
        self.early.set_level(v);
    }

    fn redesign(&mut self) {
        self.dirty = false;
        let (lo, hi) = self.v.decay_range;
        let (low_mult, low_xover, high_mult) = self.shape;
        let t60 = self.decay.clamp(lo, hi);
        self.early.set_decay(t60);
        self.late_gain = self.v.late_level * t60.recip().powf(self.v.late_decay_norm);
        let bands = DecayBands::from_mults(t60, low_mult, high_mult, low_xover, self.damping);
        self.fdn.set_decay(stretch(bands, self.freeze.loss()));
    }

    pub(in crate::dsp::algo) fn set_decay(&mut self, v: f32) {
        if v != self.decay {
            self.decay = v;
            self.dirty = true;
        }
    }

    pub(in crate::dsp::algo) fn set_damping(&mut self, v: f32) {
        if v != self.damping {
            self.damping = v;
            self.dirty = true;
        }
    }

    pub(in crate::dsp::algo) fn set_decay_shape(&mut self, low: f32, xover: f32, high: f32) {
        if (low, xover, high) != self.shape {
            self.shape = (low, xover, high);
            self.dirty = true;
        }
    }

    pub(in crate::dsp::algo) fn set_freeze(&mut self, on: bool) {
        if on == self.freeze.on() {
            return;
        }
        self.freeze.set(on);
        if !self.primed {
            // Nothing is sounding: land in the settled state, the one
            // `clear` leaves.
            self.settle_freeze();
        } else if !on && self.fdn.is_frozen() {
            // Out of the lossless loop at zero loss (the design it holds
            // since the ramp landed); the ramp brings the loss back.
            self.fdn.set_freeze(false);
        } else if on && self.freeze.held() {
            // Re-engaged before a sample moved the ramp off frozen: the
            // ramp is already there and will not `Engage` again.
            self.fdn.set_freeze(true);
        }
    }

    /// The Freeze ramp at its target and the loop in the matching state
    /// (cleared: its modulation fade complete). Only while nothing is
    /// sounding.
    fn settle_freeze(&mut self) {
        self.freeze.snap();
        self.fdn.set_freeze(self.freeze.on());
        self.fdn.clear();
        self.dirty = true;
    }

    pub(in crate::dsp::algo) fn set_mod_rate(&mut self, v: f32) {
        if v != self.mod_rate {
            self.mod_rate = v;
            self.apply_modulation();
        }
    }

    pub(in crate::dsp::algo) fn set_mod_depth(&mut self, v: f32) {
        if v != self.mod_depth {
            self.mod_depth = v;
            self.apply_modulation();
        }
    }

    fn apply_modulation(&mut self) {
        let rate = if self.mod_rate.is_nan() {
            0.5
        } else {
            self.mod_rate
        };
        let depth =
            self.mod_depth.clamp(0.0, 1.0) * self.v.mod_depth_max * (self.sample_rate / 48_000.0);
        self.fdn.set_modulation(rate.max(0.0), depth);
    }

    #[inline]
    pub(in crate::dsp::algo) fn process(&mut self, l: f32, r: f32, diffusion: f32) -> Wet {
        self.primed = true;
        match self.freeze.tick() {
            FreezeTick::Still | FreezeTick::Moving => {}
            FreezeTick::Redesign => self.dirty = true,
            FreezeTick::Engage => {
                // Zero loss (a unity design, within one step of the last),
                // then the exact lossless loop.
                self.redesign();
                self.fdn.set_freeze(true);
            }
        }
        if self.dirty {
            self.redesign();
        }
        if diffusion != self.diffusion {
            self.diffusion = diffusion;
            let g = diffusion.clamp(0.0, 1.0) * self.v.diffuser_gain;
            for ap in self.diffuser.iter_mut().flatten() {
                ap.set_gain(g);
            }
        }
        // Freeze fades the input out, and the loop's injection again, so
        // it is silent when the loop closes (see `freeze.rs`).
        let g = self.freeze.gain();
        let (l, r) = (l * g, r * g);
        let (mut dl, mut dr) = (l, r);
        for ap in &mut self.diffuser[0] {
            dl = ap.process(dl);
        }
        for ap in &mut self.diffuser[1] {
            dr = ap.process(dr);
        }
        let (er_l, er_r) = if self.v.diffused_early {
            self.early.process(dl, dr)
        } else {
            self.early.process(l, r)
        };
        let input: [f32; N] =
            std::array::from_fn(|i| g * (self.in_l[i] * dl + self.in_r[i] * dr));
        let y = self.fdn.tick(&input);
        let (mut late_l, mut late_r) = (self.v.direct_diffuse * dl, self.v.direct_diffuse * dr);
        for ((yi, ol), or) in y.iter().zip(&self.out_l).zip(&self.out_r) {
            late_l += ol * yi;
            late_r += or * yi;
        }
        // Tank view: a ~4 ms follower of each line pair's magnitude.
        let per = N / CHANNELS;
        for (c, e) in self.energies.iter_mut().enumerate() {
            let mut m = 0.0f32;
            for k in 0..per {
                m += y[c * per + k].abs();
            }
            *e += 0.005 * (m / per as f32 - *e);
        }
        let k = self.late_gain;
        Wet {
            er_l,
            er_r,
            late_l: late_l * k,
            late_r: late_r * k,
        }
    }

    pub(in crate::dsp::algo) fn clear(&mut self) {
        self.early.clear();
        self.settle_freeze();
        for ap in self.diffuser.iter_mut().flatten() {
            ap.clear();
        }
        self.energies = [0.0; CHANNELS];
        self.primed = false;
    }

    /// Per line-pair envelopes (`N / 8` lines folded into each).
    pub(in crate::dsp::algo) fn channel_energies(&self) -> [f32; CHANNELS] {
        self.energies
    }

    /// Mean length of each line pair, ms.
    pub(in crate::dsp::algo) fn fdn_delay_ms(&self) -> [f32; CHANNELS] {
        let len = self.fdn.line_lengths();
        let per = N / CHANNELS;
        std::array::from_fn(|c| {
            let sum: usize = len[c * per..(c + 1) * per].iter().sum();
            sum as f32 / per as f32 * 1000.0 / self.sample_rate
        })
    }

    pub(in crate::dsp::algo) fn er_tap_times_ms(&self) -> [(f32, f32); ER_TAPS] {
        self.early.tap_times_ms()
    }

    pub(in crate::dsp::algo) fn er_tap_gains(&self) -> [(f32, f32); ER_TAPS] {
        self.early.tap_gains()
    }
}
