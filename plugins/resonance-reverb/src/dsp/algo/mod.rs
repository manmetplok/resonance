//! The reverb engines and the dispatch over them (reverb-algorithms.md
//! §4.2).
//!
//! An engine is the algorithm-specific middle of the chain: it takes the
//! pre-delayed, return-EQ'd stereo input and returns its early and late
//! parts separately ([`Wet`]), so the shared ER/tail balance in
//! `chain.rs` works on every algorithm. Everything else (return EQ,
//! pre-delay, balance, width, ducker, mix) stays outside.
//!
//! Dispatch is a plain `enum` + `match`: no `dyn`, no allocation on the
//! audio thread. Every engine is built in `initialize` at the session
//! sample rate (see [`switch::EngineBank`]) and lives for the instance.
//!
//! Adding an algorithm (R3 onward) is: a variant in [`Algorithm`] and in
//! [`Engine`], one arm in each `match` below and in `dispatch!`, a label
//! in `params::ALGORITHM_LABELS_ALL` order, and growing
//! [`Algorithm::BUILT`]. The switch logic in [`switch`] needs no change.

pub mod classic;
pub(crate) mod switch;

use super::er::ER_TAPS;
use super::CHANNELS;
use classic::ClassicEngine;

/// One engine output sample: early reflections and late tail, per side.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Wet {
    pub er_l: f32,
    pub er_r: f32,
    pub late_l: f32,
    pub late_r: f32,
}

/// The `algorithm` parameter's values, in label order. The order is fixed
/// by the spec's table (§4.1): labels are appended even when phases land
/// out of order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Algorithm {
    Classic = 0,
}

impl Algorithm {
    /// The algorithms this build has, in parameter order. The `algorithm`
    /// parameter's range ends at the last of these, so an unbuilt
    /// algorithm is never selectable.
    pub const BUILT: &'static [Algorithm] = &[Algorithm::Classic];

    /// The algorithm a parameter value selects. Out-of-range values
    /// (a hand-edited state, a newer build's index) fall back to Classic.
    pub fn from_index(index: i32) -> Self {
        usize::try_from(index)
            .ok()
            .and_then(|i| Self::BUILT.get(i).copied())
            .unwrap_or(Algorithm::Classic)
    }

    /// Whether this algorithm reads `low_decay_mult`, `low_xover` and
    /// `high_decay_mult`. The editor greys the three out when it does not.
    pub fn uses_decay_shape(self) -> bool {
        match self {
            Algorithm::Classic => false,
        }
    }
}

/// Forward one call to whichever engine `$self` holds. Every engine
/// exposes the same inherent method set (the decay-shape setter is a
/// no-op where unused), so a new variant is one arm here.
macro_rules! dispatch {
    ($self:ident, $e:ident => $call:expr) => {
        match $self {
            Engine::Classic($e) => $call,
        }
    };
}

/// One instance of one algorithm.
pub enum Engine {
    Classic(ClassicEngine),
}

impl Engine {
    pub fn new(algorithm: Algorithm, sample_rate: f32) -> Self {
        match algorithm {
            Algorithm::Classic => Engine::Classic(ClassicEngine::new(sample_rate)),
        }
    }

    pub fn algorithm(&self) -> Algorithm {
        match self {
            Engine::Classic(_) => Algorithm::Classic,
        }
    }

    pub fn set_size(&mut self, v: f32) {
        dispatch!(self, e => e.set_size(v))
    }

    pub fn set_decay(&mut self, v: f32) {
        dispatch!(self, e => e.set_decay(v))
    }

    pub fn set_freeze(&mut self, v: bool) {
        dispatch!(self, e => e.set_freeze(v))
    }

    pub fn set_damping(&mut self, v: f32) {
        dispatch!(self, e => e.set_damping(v))
    }

    pub fn set_er_level(&mut self, v: f32) {
        dispatch!(self, e => e.set_er_level(v))
    }

    pub fn set_er_time(&mut self, v: f32) {
        dispatch!(self, e => e.set_er_time(v))
    }

    pub fn set_mod_rate(&mut self, v: f32) {
        dispatch!(self, e => e.set_mod_rate(v))
    }

    pub fn set_mod_depth(&mut self, v: f32) {
        dispatch!(self, e => e.set_mod_depth(v))
    }

    /// Bass/treble decay multipliers and the bass crossover. Classic has
    /// one loop gain and ignores them (its method is a no-op).
    pub fn set_decay_shape(&mut self, low_mult: f32, low_xover_hz: f32, high_mult: f32) {
        dispatch!(self, e => e.set_decay_shape(low_mult, low_xover_hz, high_mult))
    }

    #[inline]
    pub fn process(&mut self, l: f32, r: f32, diffusion: f32) -> Wet {
        dispatch!(self, e => e.process(l, r, diffusion))
    }

    pub fn clear(&mut self) {
        dispatch!(self, e => e.clear())
    }

    pub fn channel_energies(&self) -> [f32; CHANNELS] {
        dispatch!(self, e => e.channel_energies())
    }

    pub fn fdn_delay_ms(&self) -> [f32; CHANNELS] {
        dispatch!(self, e => e.fdn_delay_ms())
    }

    pub fn er_tap_times_ms(&self) -> [(f32, f32); ER_TAPS] {
        dispatch!(self, e => e.er_tap_times_ms())
    }

    pub fn er_tap_gains(&self) -> [(f32, f32); ER_TAPS] {
        dispatch!(self, e => e.er_tap_gains())
    }
}
