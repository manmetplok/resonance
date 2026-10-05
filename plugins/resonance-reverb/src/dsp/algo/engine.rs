//! [`Engine`]: one instance of one algorithm, and the dispatch over the
//! algorithms (see the parent module).

use super::super::er::ER_TAPS;
use super::super::CHANNELS;
use super::ambience::AmbienceEngine;
use super::chamber::ChamberEngine;
use super::classic::ClassicEngine;
use super::hall::HallEngine;
use super::plate::PlateEngine;
use super::room::RoomEngine;
use super::{Algorithm, Wet};

/// Forward one call to whichever engine `$self` holds. Every engine
/// exposes the same inherent method set (the decay-shape setter is a
/// no-op where unused), so a new variant is one arm here.
macro_rules! dispatch {
    ($self:ident, $e:ident => $call:expr) => {
        match $self {
            Engine::Classic($e) => $call,
            Engine::Plate($e) => $call,
            Engine::Room($e) => $call,
            Engine::Chamber($e) => $call,
            Engine::Hall($e) => $call,
            Engine::Ambience($e) => $call,
        }
    };
}

/// One instance of one algorithm.
pub enum Engine {
    Classic(ClassicEngine),
    Plate(PlateEngine),
    Room(RoomEngine),
    Chamber(ChamberEngine),
    Hall(HallEngine),
    Ambience(AmbienceEngine),
}

impl Engine {
    pub fn new(algorithm: Algorithm, sample_rate: f32) -> Self {
        match algorithm {
            Algorithm::Classic => Engine::Classic(ClassicEngine::new(sample_rate)),
            Algorithm::Plate => Engine::Plate(PlateEngine::new(sample_rate)),
            Algorithm::Room => Engine::Room(RoomEngine::new(sample_rate)),
            Algorithm::Chamber => Engine::Chamber(ChamberEngine::new(sample_rate)),
            Algorithm::Hall => Engine::Hall(HallEngine::new(sample_rate)),
            Algorithm::Ambience => Engine::Ambience(AmbienceEngine::new(sample_rate)),
        }
    }

    pub fn algorithm(&self) -> Algorithm {
        match self {
            Engine::Classic(_) => Algorithm::Classic,
            Engine::Plate(_) => Algorithm::Plate,
            Engine::Room(_) => Algorithm::Room,
            Engine::Chamber(_) => Algorithm::Chamber,
            Engine::Hall(_) => Algorithm::Hall,
            Engine::Ambience(_) => Algorithm::Ambience,
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

    /// The tail's build-up time, `0..=1` (Hall; a no-op elsewhere).
    pub fn set_build(&mut self, v: f32) {
        dispatch!(self, e => e.set_build(v))
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
