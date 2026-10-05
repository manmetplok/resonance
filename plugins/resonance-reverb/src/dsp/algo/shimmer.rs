//! R8: hall with a pitch shifter in the loop (§4.4).
//!
//! **Placeholder.** Wired into the dispatch so phases can land in
//! parallel without touching `algo/mod.rs` or `engine.rs`; renders silence
//! and is not in [`super::Algorithm::BUILT`] until its phase merges.

use super::super::er::ER_TAPS;
use super::super::CHANNELS;
use super::{Extras, Wet};

pub struct ShimmerEngine {
    _sample_rate: f32,
}

impl ShimmerEngine {
    pub fn new(sample_rate: f32) -> Self {
        Self {
            _sample_rate: sample_rate,
        }
    }

    pub fn set_size(&mut self, _v: f32) {}
    pub fn set_decay(&mut self, _v: f32) {}
    pub fn set_freeze(&mut self, _v: bool) {}
    pub fn set_damping(&mut self, _v: f32) {}
    pub fn set_er_level(&mut self, _v: f32) {}
    pub fn set_er_time(&mut self, _v: f32) {}
    pub fn set_mod_rate(&mut self, _v: f32) {}
    pub fn set_mod_depth(&mut self, _v: f32) {}
    pub fn set_decay_shape(&mut self, _low_mult: f32, _low_xover_hz: f32, _high_mult: f32) {}
    pub fn set_build(&mut self, _v: f32) {}
    pub fn set_extras(&mut self, _extras: &Extras) {}

    #[inline]
    pub fn process(&mut self, _l: f32, _r: f32, _diffusion: f32) -> Wet {
        Wet::default()
    }

    pub fn clear(&mut self) {}

    pub fn channel_energies(&self) -> [f32; CHANNELS] {
        [0.0; CHANNELS]
    }

    pub fn fdn_delay_ms(&self) -> [f32; CHANNELS] {
        [0.0; CHANNELS]
    }

    pub fn er_tap_times_ms(&self) -> [(f32, f32); ER_TAPS] {
        [(0.0, 0.0); ER_TAPS]
    }

    pub fn er_tap_gains(&self) -> [(f32, f32); ER_TAPS] {
        [(0.0, 0.0); ER_TAPS]
    }
}
