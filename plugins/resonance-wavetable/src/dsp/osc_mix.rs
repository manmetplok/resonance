//! How the two oscillators interact: summed, osc2 phase-modulating osc1,
//! ring-modulated, or osc2 hard-synced to osc1.
//!
//! The one `osc_mod_amount` parameter means something per mode — the PM
//! index, the ring depth, or how far the synced slave is swept above its own
//! pitch — so a single modulation destination drives the characteristic
//! sweep of whichever mode is selected.
//!
//! In every mode but [`OscMixMode::Sum`] osc2 runs whenever its wavetable
//! resolves, even with its `enabled` switch off: `enabled` then only
//! decides whether osc2 is *heard*. That is how a patch gets a pure FM
//! modulator or a silent ring carrier. Its level and pan likewise shape only
//! what is heard: the modulator is osc2's raw (warped) output. Osc1's
//! `enabled` only decides whether it is heard too; as sync master its phase
//! always runs.

/// Values are the `osc_mix_mode` parameter's integers.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
#[repr(u8)]
pub enum OscMixMode {
    /// `osc1 + osc2`. The default, and bit-identical to the synth before
    /// the interaction modes existed.
    #[default]
    Sum = 0,
    /// Osc2 phase-modulates osc1 (see [`PM_DEPTH_CYCLES`]).
    Fm = 1,
    /// Osc1 is multiplied by osc2, crossfaded from dry by the amount.
    Ring = 2,
    /// Osc1's phase wrap restarts osc2 (see [`SYNC_SWEEP_SEMITONES`]).
    Sync = 3,
}

impl OscMixMode {
    /// Display names, indexed by the parameter's integer value.
    pub const LABELS: [&'static str; 4] = ["Sum", "FM", "Ring", "Sync"];

    pub fn from_int(v: i32) -> Self {
        match v {
            1 => Self::Fm,
            2 => Self::Ring,
            3 => Self::Sync,
            _ => Self::Sum,
        }
    }

    pub fn label(self) -> &'static str {
        Self::LABELS[self as usize]
    }

    /// True when osc2 must run for osc1's sake (as modulator or slave),
    /// independently of whether it is heard.
    #[inline]
    pub fn drives_osc2(self) -> bool {
        self != Self::Sum
    }
}

/// Phase deviation, in cycles, of a full-scale (±1) modulator at an amount
/// of 1.0: a peak index of `2π` radians.
///
/// "FM" here is phase modulation, as on every classic "FM" synth: the
/// modulator is added to osc1's *read phase*, not to its increment. Two
/// reasons. The carrier's average frequency is untouched at any index, so
/// the pitch never drifts — true FM integrates the modulator, and a
/// wavetable frame is not guaranteed DC-free (a pulse is not), which would
/// detune the carrier by the modulator's mean. And the modulation is
/// memoryless: a moving amount changes the timbre, never the phase
/// accumulator, so sweeping it cannot knock the carrier out of tune.
pub const PM_DEPTH_CYCLES: f64 = 1.0;

/// How far above its own pitch a full amount sweeps the synced slave, in
/// semitones. Sweeping the slave against a fixed master is *the* sync sound;
/// wiring it to the amount lets a mod envelope or LFO do it through the
/// `OscModAmount` destination.
pub const SYNC_SWEEP_SEMITONES: f32 = 36.0;

/// Worst-case instantaneous-frequency factor of a phase-modulated carrier:
/// a sinusoidal modulator at `f_mod` with peak deviation `d` cycles moves
/// the carrier's instantaneous frequency by up to `2π d f_mod`, so its
/// partials reach `1 + 2π d f_mod / f_car` times their resting frequency.
/// This is Carson's rule without the modulator's own bandwidth term, used
/// the same way as a warp's slope: to pick the carrier's mip level. It is
/// exact for a sine modulator; a brighter one widens the sidebands beyond
/// it, which is the known cost of FM on a non-oversampled oscillator.
#[inline]
pub fn pm_bandwidth(depth_cycles: f64, f_mod: f32, f_car: f32) -> f32 {
    if depth_cycles <= 0.0 || f_car <= 0.0 {
        return 1.0;
    }
    1.0 + (std::f64::consts::TAU * depth_cycles) as f32 * f_mod / f_car
}
