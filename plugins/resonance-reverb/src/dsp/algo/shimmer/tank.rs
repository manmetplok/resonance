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
//!   m_k = ⟨u_k, y⟩                                      k = 0..3
//!   z_k = cap · HP(LP(shift_k(m_k)))
//!   y'  = y + a · Σ_k (z_k − m_k)·u_k
//!   line ← H·y' + input                                 (H: Hadamard)
//! ```
//!
//! The `u_k` are four orthonormal sign vectors over the 16 lines, each a
//! bent function of the line index plus a different linear term
//! (`(−1)^(b0·b1 ⊕ b2·b3 ⊕ ⟨α_k, i⟩)/4`): orthogonal because their linear
//! terms differ, and each spread evenly over every line by the Hadamard,
//! so no shifted component returns into a single line (a comb at its
//! length). `a` is `shimmer_amount`: the share of those four components
//! that goes through the shifters, a quarter of the loop at `a = 1`.
//!
//! **Why the energy stays bounded.** Write `y = Σ m_k·u_k + y_⊥`. Then
//! `‖y'‖² = ‖y_⊥‖² + Σ_k ((1 − a)·m_k + a·z_k)²`, and per sample
//! `((1 − a)m + az)² ≤ (1 − a)m² + az²` (Jensen). The shifter path is
//! non-expansive on every prefix of time: the shifter's gain is at most
//! `√c_max` (see `shifter.rs`), the one-pole low-pass and the
//! `x − LP(x)` high-pass have `|H| ≤ 1`, and `cap = 0.97/√c_max`. So
//! `Σ_{t≤T} z_k² ≤ Σ_{t≤T} m_k²`, hence `Σ_{t≤T} ‖y'‖² ≤ Σ_{t≤T} ‖y‖²`:
//! for fixed settings the map from the absorbed line outputs to the
//! next line inputs is non-expansive, whatever the pitch. The
//! absorption is a strict contraction (`|A_i(f)| ≤ ρ < 1` at any finite
//! T60), so by the small-gain theorem the loop's total output energy is
//! at most `‖input‖² / (1 − ρ)²`: no setting of pitch and amount turns
//! the loop into an oscillator. A parameter move is a finite transient
//! (the amount and the cap slew over 50 ms) after which the same bound
//! holds from the state it left.
//!
//! **Freeze.** The engine ramps `a` to 0 before it freezes the tank: a
//! frozen tank is the plain lossless network (orthogonal `H`, unity
//! absorption, integer reads once the modulation has faded), which holds
//! its energy exactly. The halo that has built up is held; it stops
//! climbing (a still-shifting frozen loop could only keep its level with
//! a gain above 1 on the shifted path, which is how shimmer freezes run
//! away).

use resonance_dsp::reverb::{
    allpass_read, hadamard_in_place, next_prime, Absorption, DecayBands, SmoothRandom,
};
use resonance_dsp::{DelayLine, SimpleRng};

use super::shifter::{self, PitchShifter, ReadWeights};

pub(super) const LINES: usize = 16;
/// Pitch-shifted feedback components.
pub(super) const SHIFTERS: usize = 4;
/// The linear terms `α_k` of the shifted directions (see the module docs).
const ALPHA: [usize; SHIFTERS] = [0b0000, 0b0010, 0b0100, 0b0110];
/// Low-pass after each shifter (stops octave-on-octave build-up), Hz.
const SHIFT_LP_HZ: f32 = 5_000.0;
/// High-pass after each shifter (stops −12 piling up sub-bass), Hz.
const SHIFT_HP_HZ: f32 = 80.0;
/// Headroom under the proven bound.
const CAP_MARGIN: f32 = 0.97;
/// Slew time of `a` and the cap, ms.
const AMOUNT_SLEW_MS: f32 = 50.0;
/// Modulation fade when frozen, s (as `Fdn`).
const MOD_FADE_S: f32 = 0.05;

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
    bands: DecayBands,
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
        let weights = ReadWeights::new(sample_rate);
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
            weights,
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

    pub(super) fn decay(&self) -> DecayBands {
        self.bands
    }

    pub(super) fn set_decay(&mut self, bands: DecayBands) {
        self.bands = bands;
        self.redesign();
    }

    pub(super) fn set_modulation(&mut self, rate_hz: f32, depth_samples: f32) {
        self.mod_depth = depth_samples.clamp(0.0, self.max_mod_depth);
        for (i, m) in self.mods.iter_mut().enumerate() {
            m.set_rate(rate_hz * (1.0 + 0.07 * i as f32 / LINES as f32), self.sample_rate);
            m.set_depth(self.mod_depth);
        }
    }

    /// Lossless loop, input muted, modulation faded out. The caller ramps
    /// the shifted share to 0 first (see the module docs).
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

    /// Shifted share of the loop, `0..=1`; slews over 50 ms.
    pub(super) fn set_amount(&mut self, amount: f32) {
        self.amount_target = amount.clamp(0.0, 1.0);
    }

    /// True once the shifted share has reached 0.
    pub(super) fn shift_silent(&self) -> bool {
        self.amount == 0.0
    }

    /// Put the amount and the cap on their targets (nothing in flight).
    pub(super) fn snap_shift(&mut self) {
        self.amount = self.amount_target;
        self.cap = self.cap_target;
    }

    /// The current loop gain on the shifted path (diagnostic).
    pub(super) fn cap(&self) -> f32 {
        self.cap_target
    }

    pub(super) fn line_lengths(&self) -> [usize; LINES] {
        self.len
    }

    #[inline]
    pub(super) fn tick(&mut self, input: &[f32; LINES]) -> &[f32; LINES] {
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

        // The shifted path. The shifters always run (so raising the
        // amount fades in current audio, not a stale buffer).
        for k in 0..SHIFTERS {
            let dir = &self.dirs[k];
            let m: f32 = dir.iter().zip(&self.out).map(|(u, v)| u * v).sum();
            let s = self.shifters[k].process(m);
            self.lp[k] += self.lp_a * (s - self.lp[k]);
            let lp = self.lp[k];
            self.hp[k] += self.hp_a * (lp - self.hp[k]);
            let z = self.cap * (lp - self.hp[k]);
            let delta = self.amount * (z - m);
            for (v, u) in y.iter_mut().zip(dir) {
                *v += delta * u;
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

    fn redesign(&mut self) {
        for i in 0..LINES {
            if self.frozen {
                self.absorb[i].set_lossless();
            } else {
                self.absorb[i].design(&self.bands, self.len[i] as f32, self.sample_rate);
            }
        }
    }

    /// Bytes held in delay buffers.
    pub(super) fn buffer_bytes(&self) -> usize {
        let f = std::mem::size_of::<f32>();
        LINES * (self.max_len + 2).next_power_of_two() * f
            + self.shifters.iter().map(|s| s.buffer_bytes()).sum::<usize>()
    }
}
