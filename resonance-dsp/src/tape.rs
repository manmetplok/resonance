//! Tape-machine filters: the speed-dependent head bump, level-dependent
//! HF loss (self-erasure) and a flutter delay modulator.
//!
//! These are the linear and time-varying parts of a tape stage; the
//! nonlinearity is a [`crate::saturate::Curve`]. Every type here is
//! fixed-size after construction and allocation-free per sample. The
//! only allocation is [`Flutter::new`]'s delay buffer, which belongs in
//! an owner's `initialize`.
//!
//! Speeds are in inches per second (ips). The voicing constants
//! (bump at 55 Hz for 15 ips, HF corner at 10 kHz for 15 ips) are
//! defaults, not measurements of a particular machine, and both scale in
//! proportion to speed.

use crate::Biquad;

/// Head-bump frequency at 15 ips. The bump scales in proportion to speed
/// (≈ 55 Hz at 15 ips, ≈ 110 Hz at 30 ips).
pub const HEAD_BUMP_HZ_AT_15_IPS: f32 = 55.0;

/// Q of the head-bump peak.
pub const HEAD_BUMP_Q: f32 = 1.2;

/// Corner of the HF loss shelf at 15 ips; scales in proportion to speed.
pub const HF_LOSS_HZ_AT_15_IPS: f32 = 10_000.0;

/// Head-bump centre frequency for a tape speed in ips.
pub fn head_bump_hz(speed_ips: f32) -> f32 {
    HEAD_BUMP_HZ_AT_15_IPS * speed_ips.max(0.1) / 15.0
}

/// HF-loss shelf corner for a tape speed in ips.
pub fn hf_loss_corner_hz(speed_ips: f32) -> f32 {
    HF_LOSS_HZ_AT_15_IPS * speed_ips.max(0.1) / 15.0
}

/// Speed-dependent head bump: a peak at [`head_bump_hz`] (Q ≈ 1.2) plus a
/// dip an octave above it, the pair a playback head's low-frequency
/// contour produces. Mono; use one per channel.
///
/// Defaults to the identity (0 dB bump and dip), which is bit-exact.
#[derive(Clone, Copy, Debug, Default)]
pub struct HeadBump {
    peak: Biquad,
    dip: Biquad,
    active: bool,
}

impl HeadBump {
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the voicing. `bump_db` is the peak gain, `dip_db` the gain of
    /// the dip one octave up (normally negative, about half the bump).
    /// Both 0 makes the stage an exact passthrough. Block-rate: it
    /// recomputes coefficients (a few transcendentals), not per sample.
    pub fn set(&mut self, sample_rate: f32, speed_ips: f32, bump_db: f32, dip_db: f32) {
        let f = head_bump_hz(speed_ips);
        self.active = bump_db != 0.0 || dip_db != 0.0;
        self.peak.set_bell(sample_rate, f, HEAD_BUMP_Q, bump_db);
        self.dip.set_bell(sample_rate, 2.0 * f, HEAD_BUMP_Q, dip_db);
    }

    pub fn reset(&mut self) {
        self.peak.reset();
        self.dip.reset();
    }

    #[inline]
    pub fn process(&mut self, x: f32) -> f32 {
        if !self.active {
            return x;
        }
        self.dip.process(self.peak.process(x))
    }

    /// Magnitude response at `freq` Hz (offline analysis, e.g. an editor
    /// curve). Pure function of the coefficients.
    pub fn magnitude(&self, freq: f32, sample_rate: f32) -> f32 {
        self.peak.magnitude(freq, sample_rate) * self.dip.magnitude(freq, sample_rate)
    }
}

/// Level-dependent HF loss (tape self-erasure): a first-order high shelf
/// whose cut deepens as the high-frequency envelope rises.
///
/// The shelf is the complementary split `x = lp + hp` of one fixed
/// one-pole lowpass, recombined as `lp + g·hp`, so the gain `g` can move
/// every sample without recomputing a coefficient and without any
/// instability. The detector follows `|hp|` (the band that self-erases)
/// with an attack/release follower; the cut in dB is
///
/// `static_db + dynamic_db · min(env / reference, 1)`
///
/// with `static_db` and `dynamic_db` ≤ 0. Both 0 makes the stage an exact
/// passthrough (the default).
#[derive(Clone, Copy, Debug)]
pub struct HfLoss {
    lp_coef: f32,
    lp_state: f32,
    env: f32,
    attack: f32,
    release: f32,
    static_db: f32,
    dynamic_db: f32,
    inv_reference: f32,
}

impl Default for HfLoss {
    fn default() -> Self {
        let mut s = Self {
            lp_coef: 0.0,
            lp_state: 0.0,
            env: 0.0,
            attack: 0.0,
            release: 0.0,
            static_db: 0.0,
            dynamic_db: 0.0,
            inv_reference: 1.0,
        };
        s.set_corner(48_000.0, HF_LOSS_HZ_AT_15_IPS);
        s.set_times(48_000.0, 1.0, 50.0);
        s
    }
}

impl HfLoss {
    pub fn new() -> Self {
        Self::default()
    }

    /// Shelf corner in Hz; see [`hf_loss_corner_hz`] for the speed
    /// mapping.
    pub fn set_corner(&mut self, sample_rate: f32, corner_hz: f32) {
        let w = (std::f32::consts::TAU * corner_hz.max(1.0) / sample_rate.max(1.0))
            .min(std::f32::consts::PI);
        self.lp_coef = (-w).exp();
    }

    /// Detector attack and release in milliseconds.
    pub fn set_times(&mut self, sample_rate: f32, attack_ms: f32, release_ms: f32) {
        self.attack = time_coef(sample_rate, attack_ms);
        self.release = time_coef(sample_rate, release_ms);
    }

    /// Static and level-dependent cut in dB (both clamped to ≤ 0), and the
    /// HF envelope level (linear, e.g. 0.25) at which the full dynamic
    /// cut applies.
    pub fn set_amounts(&mut self, static_db: f32, dynamic_db: f32, reference: f32) {
        self.static_db = static_db.min(0.0);
        self.dynamic_db = dynamic_db.min(0.0);
        self.inv_reference = 1.0 / reference.max(1.0e-6);
    }

    pub fn reset(&mut self) {
        self.lp_state = 0.0;
        self.env = 0.0;
    }

    /// The current detector envelope (linear).
    pub fn envelope(&self) -> f32 {
        self.env
    }

    #[inline]
    pub fn process(&mut self, x: f32) -> f32 {
        if self.static_db == 0.0 && self.dynamic_db == 0.0 {
            return x;
        }
        self.lp_state = x + self.lp_coef * (self.lp_state - x);
        let lp = self.lp_state;
        let hp = x - lp;
        let level = hp.abs();
        let c = if level > self.env { self.attack } else { self.release };
        self.env = level + c * (self.env - level);
        let amount = (self.env * self.inv_reference).min(1.0);
        let db = self.static_db + self.dynamic_db * amount;
        let g = crate::db_to_linear(db);
        lp + g * hp
    }
}

fn time_coef(sample_rate: f32, ms: f32) -> f32 {
    let n = (ms.max(0.0) * 0.001 * sample_rate.max(1.0)).max(1.0e-3);
    (-1.0 / n).exp()
}

/// Longest modulation depth [`Flutter`] supports, in milliseconds.
pub const FLUTTER_MAX_DEPTH_MS: f32 = 2.0;

/// Wow at `amount` 1: 0.8 Hz, ±1.0 ms.
const WOW_HZ: f64 = 0.8;
const WOW_DEPTH_MS: f64 = 1.0;
/// Flutter at `amount` 1: 7.3 Hz, ±0.06 ms (together ≈ 0.5 % peak speed
/// deviation, a worn machine).
const FLUTTER_HZ: f64 = 7.3;
const FLUTTER_DEPTH_MS: f64 = 0.06;

/// Wow-and-flutter delay modulator: two slow sines move a fractional read
/// tap (4-point Hermite) around a centre delay.
///
/// **At `amount` 0 it is bypassed**: the input comes back unchanged,
/// bit-exact, with no delay and no interpolation (it still records into
/// its buffer, so switching on has history). Above 0 the signal is
/// delayed by the centre delay, [`Flutter::centre_delay_samples`], so going
/// from 0 to non-zero is a latency step; owners should switch while
/// silent or crossfade. The centre delay (≈ 1.1 ms) is part of the
/// effect, like a tape path's own delay, and is not host latency.
///
/// The modulation is deterministic (fixed phases), so two instances with
/// the same settings modulate identically, as both channels of one tape
/// should.
pub struct Flutter {
    buf: Vec<f32>,
    mask: usize,
    write: usize,
    sample_rate: f64,
    amount: f64,
    wow_phase: f64,
    flutter_phase: f64,
    wow_inc: f64,
    flutter_inc: f64,
    centre: f64,
}

impl Flutter {
    /// Allocates the delay buffer (call from `initialize`).
    pub fn new(sample_rate: f32) -> Self {
        let sr = sample_rate.max(1.0) as f64;
        let max = (2.0 * FLUTTER_MAX_DEPTH_MS as f64 * 0.001 * sr).ceil() as usize + 8;
        let size = max.next_power_of_two();
        Self {
            buf: vec![0.0; size],
            mask: size - 1,
            write: 0,
            sample_rate: sr,
            amount: 0.0,
            wow_phase: 0.0,
            flutter_phase: 0.25,
            wow_inc: WOW_HZ / sr,
            flutter_inc: FLUTTER_HZ / sr,
            centre: (WOW_DEPTH_MS + FLUTTER_DEPTH_MS) * 0.001 * sr + 2.0,
        }
    }

    /// Modulation amount, clamped to 0..=1. 0 bypasses.
    pub fn set_amount(&mut self, amount: f32) {
        self.amount = if amount > 0.0 { (amount as f64).min(1.0) } else { 0.0 };
    }

    /// The fixed centre delay (samples) applied while `amount` > 0.
    pub fn centre_delay_samples(&self) -> f32 {
        self.centre as f32
    }

    pub fn reset(&mut self) {
        self.buf.fill(0.0);
        self.write = 0;
        self.wow_phase = 0.0;
        self.flutter_phase = 0.25;
    }

    #[inline]
    pub fn process(&mut self, x: f32) -> f32 {
        let x = if x.is_finite() { x } else { 0.0 };
        self.buf[self.write] = x;
        self.write = (self.write + 1) & self.mask;
        if self.amount == 0.0 {
            return x;
        }
        self.wow_phase = (self.wow_phase + self.wow_inc).fract();
        self.flutter_phase = (self.flutter_phase + self.flutter_inc).fract();
        let ms_to_samples = 0.001 * self.sample_rate;
        let m = WOW_DEPTH_MS * (std::f64::consts::TAU * self.wow_phase).sin()
            + FLUTTER_DEPTH_MS * (std::f64::consts::TAU * self.flutter_phase).sin();
        // Delay in samples behind the newest sample (index 0 = newest).
        let d = self.centre + self.amount * m * ms_to_samples;
        let di = d.floor();
        let frac = (d - di) as f32;
        let i = di as usize;
        // newest sample sits at write − 1.
        let at = |k: usize| self.buf[self.write.wrapping_sub(1 + k) & self.mask];
        // Hermite between delays i and i+1 (older), neighbours i−1, i+2.
        crate::hermite4(at(i - 1), at(i), at(i + 1), at(i + 2), frac)
    }
}
