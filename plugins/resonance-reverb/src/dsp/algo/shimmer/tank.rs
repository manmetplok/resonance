//! The Shimmer's late field: a 16-line feedback delay network whose
//! feedback runs partly through four pitch shifters.
//!
//! The lines, their lengths, absorption and modulation are
//! [`resonance_dsp::reverb::Fdn`]'s, rebuilt here from the same
//! primitives because the shifters sit *inside* the loop, between the
//! absorption and the feedback matrix, which `Fdn` does not open up. With
//! the same seed the line lengths equal the Hall's. One sample:
//!
//! ```text
//!   y   = absorption(line reads)                       (returned)
//!   m_k = ⟨d_k, y⟩                                      k = 0..3
//!   z_k = cap · HP(LP(shift_k(m_k)))
//!   y'  = y + Σ_k u_k·(z_k − m_k)·d_k
//!   line ← H·y' + input                                 (H: Hadamard)
//! ```
//!
//! The `d_k` are four orthonormal sign vectors over the 16 lines, each a
//! bent function of the line index plus a different linear term
//! (`(−1)^(b0·b1 ⊕ b2·b3 ⊕ ⟨α_k, i⟩)/4`): orthogonal because their linear
//! terms differ, and each spread evenly over every line by the Hadamard,
//! so no shifted component returns into a single line (a comb at its
//! length).
//!
//! **The amount routes, it does not blend.** `shimmer_amount` `a` sets
//! `u_k = clamp(4a − k, 0, 1)`: direction 0 fills first, then 1, … — at
//! most one is part-shifted, the others are fully replaced by their
//! shifted copy or untouched. A blend `(1 − u)m + uz` of two unrelated
//! signals keeps only `(1 − u)² + u²` of their energy, so blending every
//! direction at `u` = 0.3 would lose half of each pass's routed energy
//! while shifting a tenth of it; routing whole directions loses only what
//! the shifted path itself does. `a` = 1 replaces a quarter of the loop
//! on every pass.
//!
//! **Why the energy stays bounded.** Write `y = Σ m_k·d_k + y_⊥`. Then
//! `‖y'‖² = ‖y_⊥‖² + Σ_k ((1 − u_k)·m_k + u_k·z_k)²`, and per sample
//! `((1 − u)m + uz)² ≤ (1 − u)m² + uz²` (Jensen). The shifter path is
//! non-expansive on every prefix of time: the shifter's gain is at most
//! `√c_max` (see `shifter.rs`), the one-pole low-pass and the
//! `x − LP(x)` high-pass have `|H| ≤ 1`, and `cap = 0.97/√c_max`. So
//! `Σ_{t≤T} z_k² ≤ Σ_{t≤T} m_k²`, hence `Σ_{t≤T} ‖y'‖² ≤ Σ_{t≤T} ‖y‖²`:
//! for fixed settings the map from the absorbed line outputs to the
//! next line inputs is non-expansive, whatever the pitch. The absorption
//! is a strict contraction while running (`|A_i(f)| ≤ ρ < 1`: every
//! design T60 is finite, the compensation's at most [`MAX_T60_S`]), so
//! by the small-gain theorem the loop's total output energy is at most
//! `‖input‖² / (1 − ρ)²`: no pitch or amount turns the loop into an
//! oscillator. Frozen, `ρ = 1` and nothing is routed (below): the loop
//! is orthogonal and lossless, so its energy is constant. A parameter
//! move is a finite transient (the amount and the cap slew over 50 ms,
//! the Freeze ramp takes 100 ms) after which the same bound holds from
//! the state it left. The tests check the pieces (the shifter's gain
//! against `c_max` on every prefix, for impulses at every alignment,
//! noise, sines) and the whole (120 s frozen at +24, 60 s of random
//! automation).
//!
//! **Decay compensation.** The shifted path keeps only part of what it
//! is given (the shifter's crossfade averages to about two thirds of the
//! power of what it reads, the cap and the low-pass take more), so
//! routing costs the loop a fraction `L = Σ_k (1 − (1 − u_k)² − u_k²τ)/16`
//! of its energy on every pass (`τ` the typical retained share, [`TAU`]).
//! Each line's absorption is designed for the T60 that gives the knob's
//! decay *with* that loss: `1/T′ = 1/T + fs·log10(1 − L)/(6·dᵢ)`, bounded
//! at [`MAX_T60_S`]. Where the knob asks for more than routing allows the
//! lines go (nearly) lossless and the tail is as long as the shimmer lets
//! it be: the more of the loop is shifted, the sooner the energy climbs
//! out through the low-pass. The compensation only ever lengthens a
//! design T60, so the bound above is untouched. It follows the *slewed*
//! amount (the engine redesigns every 16 samples while the amount
//! moves): a jump in every line's gain at once would step the output.
//!
//! **Freeze.** The engine's Freeze ramp (`room/freeze.rs`) scales the
//! routed share by `1 − hold` along with the input, so at `hold` 1 nothing
//! is shifted and the frozen tank is the plain lossless network
//! (orthogonal `H`, unity absorption, integer reads once the modulation
//! has faded), which holds its energy exactly. The halo that has built up
//! is held; it stops climbing (a still-shifting frozen loop could only
//! keep its level with a gain above 1 on the shifted path, which is how
//! shimmer freezes run away).

use resonance_dsp::reverb::{
    allpass_read, hadamard_in_place, next_prime, Absorption, DecayBands, SmoothRandom,
};
use resonance_dsp::{DelayLine, SimpleRng};

use super::shifter::{self, PitchShifter, ReadWeights};

pub(super) const LINES: usize = 16;
/// Pitch-shifted feedback directions.
pub(super) const SHIFTERS: usize = 4;
/// The linear terms `α_k` of the shifted directions (see the module docs).
const ALPHA: [usize; SHIFTERS] = [0b0000, 0b0010, 0b0100, 0b0110];
/// Low-pass after each shifter (stops octave-on-octave build-up), Hz.
const SHIFT_LP_HZ: f32 = 5_000.0;
/// High-pass after each shifter (stops −12 piling up sub-bass), Hz.
const SHIFT_HP_HZ: f32 = 80.0;
/// Headroom under the proven bound.
const CAP_MARGIN: f32 = 0.97;
/// Slew time of the amount and the cap, ms.
const AMOUNT_SLEW_MS: f32 = 50.0;
/// Modulation fade when frozen, s (as `Fdn`).
const MOD_FADE_S: f32 = 0.05;
/// Longest T60 a running line is designed for (the compensation's
/// bound; keeps the absorption a strict contraction).
pub(super) const MAX_T60_S: f32 = 100.0;
/// Typical share of a routed direction's energy the shifted path hands
/// back per pass. One value fits every pitch: fitted on impulse
/// responses, it puts the mid T30 within −14…+12 % of the knob at
/// amounts 0.15–1 for all six (`the_decay_holds_with_the_shimmer_routed_in`).
const TAU: f32 = 0.35;

/// Sign of line `i` in direction `alpha` (the Hall's output signs are
/// `alpha` 0 and `0b0010`).
pub(super) fn direction_sign(i: usize, alpha: usize) -> f32 {
    let b = |k: usize| (i >> k) & 1;
    let f = (b(0) & b(1)) ^ (b(2) & b(3)) ^ ((i & alpha).count_ones() as usize & 1);
    if f == 0 {
        1.0
    } else {
        -1.0
    }
}

fn one_pole_coeff(hz: f32, sample_rate: f32) -> f32 {
    1.0 - (-std::f32::consts::TAU * hz / sample_rate).exp()
}

/// Routed share of direction `k` at amount `a` (see the module docs).
#[inline]
fn route(a: f32, k: usize) -> f32 {
    (a * SHIFTERS as f32 - k as f32).clamp(0.0, 1.0)
}

pub(super) struct Tank {
    sample_rate: f32,
    lines: [DelayLine; LINES],
    base_ms: [f32; LINES],
    len: [usize; LINES],
    read: [f32; LINES],
    interp: [f32; LINES],
    glide: f32,
    max_len: usize,
    size: f32,
    /// The decay target, and the routed share the absorption is
    /// compensated for.
    bands: DecayBands,
    design_share: f32,
    absorb: [Absorption; LINES],
    mods: [SmoothRandom; LINES],
    mod_depth: f32,
    max_mod_depth: f32,
    mod_scale: f32,
    mod_step: f32,
    frozen: bool,

    shifters: [PitchShifter; SHIFTERS],
    dirs: [[f32; LINES]; SHIFTERS],
    lp: [f32; SHIFTERS],
    hp: [f32; SHIFTERS],
    lp_a: f32,
    hp_a: f32,
    weights: ReadWeights,
    semitones: f32,
    amount: f32,
    amount_target: f32,
    cap: f32,
    cap_target: f32,
    slew: f32,

    out: [f32; LINES],
}

impl Tank {
    /// Lines of `min_ms..max_ms` at size 1 (buffers for size 1), seeded
    /// as `Fdn` seeds them.
    pub(super) fn new(sample_rate: f32, min_ms: f32, max_ms: f32, max_mod: f32, seed: u64) -> Self {
        let mut rng = SimpleRng::new(seed);
        let base_ms = std::array::from_fn(|i| {
            let u = rng.next_u32() as f32 / u32::MAX as f32;
            let pos = (i as f32 + 0.6 * (u - 0.5)) / (LINES - 1) as f32;
            min_ms * (max_ms / min_ms).powf(pos.clamp(0.0, 1.0))
        });
        let longest = (max_ms * sample_rate / 1000.0).ceil() as usize;
        let max_len = longest + 80 * LINES + max_mod.ceil() as usize + 4;
        let mut tank = Self {
            sample_rate,
            lines: std::array::from_fn(|_| DelayLine::new(max_len + 2)),
            base_ms,
            len: [0; LINES],
            read: [0.0; LINES],
            interp: [0.0; LINES],
            glide: 0.0,
            max_len,
            size: 1.0,
            bands: DecayBands::flat(2.0),
            design_share: 0.0,
            absorb: [Absorption::lossless(); LINES],
            mods: std::array::from_fn(|i| {
                SmoothRandom::new(
                    seed.wrapping_add(0x9E37_79B9 * (i as u64 + 1)),
                    0.5,
                    0.0,
                    sample_rate,
                )
            }),
            mod_depth: 0.0,
            max_mod_depth: max_mod,
            mod_scale: 1.0,
            mod_step: 1.0 / (MOD_FADE_S * sample_rate).max(1.0),
            frozen: false,
            // Staggered phases: the four shifters' grain boundaries never
            // coincide.
            shifters: std::array::from_fn(|k| PitchShifter::new(sample_rate, k as f32 / 8.0)),
            dirs: std::array::from_fn(|k| {
                std::array::from_fn(|i| 0.25 * direction_sign(i, ALPHA[k]))
            }),
            lp: [0.0; SHIFTERS],
            hp: [0.0; SHIFTERS],
            lp_a: one_pole_coeff(SHIFT_LP_HZ, sample_rate),
            hp_a: one_pole_coeff(SHIFT_HP_HZ, sample_rate),
            weights: ReadWeights::new(sample_rate),
            semitones: f32::NAN,
            amount: 0.0,
            amount_target: 0.0,
            cap: 0.0,
            cap_target: 0.0,
            slew: 1.0 / (AMOUNT_SLEW_MS * 0.001 * sample_rate).max(1.0),
            out: [0.0; LINES],
        };
        tank.set_semitones(12.0);
        tank.snap_shift();
        tank.set_size(1.0);
        tank
    }

    /// Scale the line lengths (as `Fdn::set_size`).
    pub(super) fn set_size(&mut self, size: f32) {
        self.size = size.clamp(0.05, 1.0);
        let floor = self.max_mod_depth.ceil() as usize + 2;
        let ceiling = self.max_len - self.max_mod_depth.ceil() as usize - 2;
        let mut prev = 0usize;
        for i in 0..LINES {
            let target = (self.base_ms[i] * self.size * self.sample_rate / 1000.0).round() as usize;
            let p = next_prime(target.max(prev + 1).max(floor)).min(ceiling);
            self.len[i] = p;
            prev = p;
        }
        if self.glide <= 0.0 {
            self.read = self.len.map(|l| l as f32);
        }
        self.redesign();
    }

    pub(super) fn size(&self) -> f32 {
        self.size
    }

    /// Read-head slew for size changes, samples per sample (0 = jump).
    pub(super) fn set_glide(&mut self, samples_per_sample: f32) {
        self.glide = samples_per_sample.max(0.0);
        if self.glide <= 0.0 {
            self.read = self.len.map(|l| l as f32);
        }
    }

    /// The decay target, compensated for routing `share` of the loop
    /// (the amount, scaled by the Freeze ramp). Redesigns only on a
    /// change.
    pub(super) fn set_decay(&mut self, bands: DecayBands, share: f32) {
        if bands != self.bands || share != self.design_share {
            self.bands = bands;
            self.design_share = share;
            self.redesign();
        }
    }

    pub(super) fn set_modulation(&mut self, rate_hz: f32, depth_samples: f32) {
        self.mod_depth = depth_samples.clamp(0.0, self.max_mod_depth);
        for (i, m) in self.mods.iter_mut().enumerate() {
            m.set_rate(rate_hz * (1.0 + 0.07 * i as f32 / LINES as f32), self.sample_rate);
            m.set_depth(self.mod_depth);
        }
    }

    /// Lossless loop, input muted, modulation faded out. Only once the
    /// routed share is 0 (the Freeze ramp sees to it).
    pub(super) fn set_freeze(&mut self, on: bool) {
        self.frozen = on;
        self.redesign();
    }

    pub(super) fn is_frozen(&self) -> bool {
        self.frozen
    }

    /// Pitch of the shifted path, semitones. The cap follows over 50 ms.
    pub(super) fn set_semitones(&mut self, semitones: f32) {
        if semitones == self.semitones {
            return;
        }
        self.semitones = semitones;
        let r = shifter::ratio(semitones);
        for s in &mut self.shifters {
            s.set_ratio(r);
        }
        self.cap_target = (CAP_MARGIN / self.weights.c_max(semitones).sqrt()).min(1.0);
    }

    /// Routed amount, `0..=1`; slews over 50 ms.
    pub(super) fn set_amount(&mut self, amount: f32) {
        self.amount_target = amount.clamp(0.0, 1.0);
    }

    /// The routed amount now (it slews towards the target).
    pub(super) fn amount_now(&self) -> f32 {
        self.amount
    }

    /// True while the amount is still slewing.
    pub(super) fn amount_moving(&self) -> bool {
        self.amount != self.amount_target
    }

    /// Put the amount and the cap on their targets (nothing in flight).
    pub(super) fn snap_shift(&mut self) {
        self.amount = self.amount_target;
        self.cap = self.cap_target;
    }

    /// The loop gain on the shifted path at the current pitch (where the
    /// slewing cap is headed).
    pub(super) fn cap_target(&self) -> f32 {
        self.cap_target
    }

    pub(super) fn line_lengths(&self) -> [usize; LINES] {
        self.len
    }

    /// One sample. `input` is added to the line writes (ignored while
    /// frozen); `share_gain` scales the routed amount (the Freeze ramp's
    /// `1 − hold`).
    #[inline]
    #[allow(clippy::needless_range_loop)] // eight per-line arrays in step, as `Fdn::tick`
    pub(super) fn tick(&mut self, input: &[f32; LINES], share_gain: f32) -> &[f32; LINES] {
        let target_scale = if self.frozen { 0.0 } else { 1.0 };
        if self.mod_scale != target_scale {
            self.mod_scale = if self.mod_scale < target_scale {
                (self.mod_scale + self.mod_step).min(1.0)
            } else {
                (self.mod_scale - self.mod_step).max(0.0)
            };
        }
        if self.amount != self.amount_target {
            let d = self.amount_target - self.amount;
            self.amount += d.clamp(-self.slew, self.slew);
        }
        if self.cap != self.cap_target {
            let d = self.cap_target - self.cap;
            self.cap += d.clamp(-self.slew, self.slew);
        }
        let modulated = self.mod_depth > 0.0 && self.mod_scale > 0.0;
        let mut y = [0.0f32; LINES];
        for i in 0..LINES {
            let target = self.len[i] as f32;
            if self.read[i] != target {
                let d = target - self.read[i];
                self.read[i] += d.clamp(-self.glide, self.glide);
            }
            let raw = if modulated {
                let m = self.mods[i].next_sample() * self.mod_scale;
                allpass_read(&self.lines[i], self.read[i] - 1.0 + m, &mut self.interp[i])
            } else if self.read[i] == target {
                let s = self.lines[i].tap(self.len[i] - 1);
                self.interp[i] = s;
                s
            } else {
                allpass_read(&self.lines[i], self.read[i] - 1.0, &mut self.interp[i])
            };
            y[i] = self.absorb[i].process(raw);
        }
        self.out = y;

        // The shifted path. Every shifter runs whatever the amount, so a
        // direction routed in later fades in current audio, not a stale
        // buffer.
        let share = self.amount * share_gain;
        for k in 0..SHIFTERS {
            let dir = &self.dirs[k];
            let m: f32 = dir.iter().zip(&self.out).map(|(u, v)| u * v).sum();
            let s = self.shifters[k].process(m);
            self.lp[k] += self.lp_a * (s - self.lp[k]);
            let lp = self.lp[k];
            self.hp[k] += self.hp_a * (lp - self.hp[k]);
            let u = route(share, k);
            if u > 0.0 {
                let z = self.cap * (lp - self.hp[k]);
                let delta = u * (z - m);
                for (v, d) in y.iter_mut().zip(dir) {
                    *v += delta * d;
                }
            }
        }

        hadamard_in_place(&mut y);
        let gate = if self.frozen { 0.0 } else { 1.0 };
        for i in 0..LINES {
            self.lines[i].push(y[i] + gate * input[i]);
        }
        &self.out
    }

    /// Back to the freshly constructed state for the current settings.
    pub(super) fn clear(&mut self) {
        for l in &mut self.lines {
            l.clear();
        }
        for a in &mut self.absorb {
            a.clear();
        }
        for m in &mut self.mods {
            m.reset();
        }
        for s in &mut self.shifters {
            s.clear();
        }
        self.lp = [0.0; SHIFTERS];
        self.hp = [0.0; SHIFTERS];
        self.snap_shift();
        self.read = self.len.map(|l| l as f32);
        self.interp = [0.0; LINES];
        self.mod_scale = if self.frozen { 0.0 } else { 1.0 };
        self.out = [0.0; LINES];
    }

    /// Energy lost per pass to routing `share` (see the module docs).
    fn routing_loss(&self, share: f32) -> f32 {
        let lost: f32 = (0..SHIFTERS)
            .map(|k| {
                let u = route(share, k);
                1.0 - (1.0 - u) * (1.0 - u) - u * u * TAU
            })
            .sum();
        (lost / LINES as f32).clamp(0.0, 0.99)
    }

    fn redesign(&mut self) {
        if self.frozen {
            for a in &mut self.absorb {
                a.set_lossless();
            }
            return;
        }
        // log10 of the per-pass energy kept by the routing (≤ 0).
        let kept = (1.0 - self.routing_loss(self.design_share)).log10();
        let fs = self.sample_rate;
        for i in 0..LINES {
            let d = self.len[i] as f32;
            // The compensation lengthens a T60 up to `MAX_T60_S` (or
            // leaves it alone above that: the Freeze ramp's own stretch
            // runs out to infinity, the lossless design it lands on).
            let stretch = |t60: f32| {
                let bound = t60.max(MAX_T60_S);
                let inv = 1.0 / t60 + fs * kept / (6.0 * d);
                if inv <= 1.0 / bound {
                    bound
                } else {
                    1.0 / inv
                }
            };
            let b = &self.bands;
            let bands = DecayBands {
                t60_low: stretch(b.t60_low),
                t60_mid: stretch(b.t60_mid),
                t60_high: stretch(b.t60_high),
                ..*b
            };
            self.absorb[i].design(&bands, d, fs);
        }
    }

    /// Bytes held in delay buffers.
    pub(super) fn buffer_bytes(&self) -> usize {
        let f = std::mem::size_of::<f32>();
        LINES * (self.max_len + 2).next_power_of_two() * f
            + self.shifters.iter().map(|s| s.buffer_bytes()).sum::<usize>()
    }
}
