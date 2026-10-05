//! **Room** (R4, reverb-algorithms.md §4.4): shoebox early reflections
//! and a short, dense 16-line FDN. Places a source in a believable space
//! with a fast build and a short decay.
//!
//! The engine is [`RoomCore`] (in `room/tank.rs`, with the reflections in
//! `room/early.rs`); Chamber and Ambience are other [`Voicing`]s of the
//! same core. What Room's voicing fixes:
//!
//! - **Box** 3 × 4 × 2.5 m at `size` 0 up to 15 × 20 × 8 m at 1 (per axis,
//!   log). Source at (0.38, 0.64, 0.46) of the box, listener at
//!   (0.57, 0.31, 0.41): off-centre on every axis, the source ahead and
//!   a little left, so the 24 image paths per ear all differ. Walls absorb
//!   30 % of the energy. First- and second-order taps at their physical
//!   weights, scattered by 1–4 ms allpasses (coefficient 0.65).
//! - **Lines** 8–60 ms at size scale 1, scaled 0.8× (`size` 0:
//!   6.4–48 ms) to 1.25× (`size` 1: 10–75 ms). The lower end is kept
//!   above the spec's 5 ms: the shortest settings scored 2–3 dB more
//!   modal colour (L3) for no gain in density, which the diffuser and
//!   the direct diffuse path (0.3) already deliver by 5–13 ms. Light
//!   random modulation, up to 8 samples at `mod_depth` 1.
//! - **Diffuser** six 0.3–2.7 ms allpasses per side, coefficient up to
//!   0.7.
//! - **Levels**: ER 0.25, tail 0.63 — at the global defaults the wet
//!   energy matches Classic's (−9.6 dB re a unit impulse) and the
//!   reflections carry about a third of it, low enough that a short decay
//!   in a big box still reads as one slope (the ER envelope follows
//!   `decay`, see `room/early.rs`).

mod early;
mod freeze;
mod tank;

pub(in crate::dsp::algo) use self::early::EarlyVoicing;
pub(in crate::dsp::algo) use self::freeze::{stretch, FreezeRamp, FreezeTick};
pub(in crate::dsp::algo) use self::tank::{RoomCore, Voicing};

/// The inherent method set every engine exposes (see `engine.rs`),
/// forwarded to the wrapped [`RoomCore`]. `set_build` is a no-op: the
/// room family has no build envelope.
macro_rules! room_family_engine {
    ($name:ident, $lines:expr, $voicing:expr) => {
        pub struct $name {
            core: $crate::dsp::algo::room::RoomCore<$lines>,
        }

        impl $name {
            pub fn new(sample_rate: f32) -> Self {
                Self {
                    core: $crate::dsp::algo::room::RoomCore::new(sample_rate, $voicing),
                }
            }

            pub fn set_size(&mut self, v: f32) {
                self.core.set_size(v)
            }
            pub fn set_decay(&mut self, v: f32) {
                self.core.set_decay(v)
            }
            pub fn set_freeze(&mut self, v: bool) {
                self.core.set_freeze(v)
            }
            pub fn set_damping(&mut self, v: f32) {
                self.core.set_damping(v)
            }
            pub fn set_er_level(&mut self, v: f32) {
                self.core.set_er_level(v)
            }
            pub fn set_er_time(&mut self, v: f32) {
                self.core.set_er_time(v)
            }
            pub fn set_mod_rate(&mut self, v: f32) {
                self.core.set_mod_rate(v)
            }
            pub fn set_mod_depth(&mut self, v: f32) {
                self.core.set_mod_depth(v)
            }
            pub fn set_decay_shape(&mut self, low_mult: f32, low_xover_hz: f32, high_mult: f32) {
                self.core.set_decay_shape(low_mult, low_xover_hz, high_mult)
            }
            pub fn set_build(&mut self, _v: f32) {}
            pub fn set_extras(&mut self, _extras: &$crate::dsp::algo::Extras) {}

            #[inline]
            pub fn process(&mut self, l: f32, r: f32, diffusion: f32) -> $crate::dsp::algo::Wet {
                self.core.process(l, r, diffusion)
            }

            pub fn clear(&mut self) {
                self.core.clear()
            }

            pub fn channel_energies(&self) -> [f32; $crate::dsp::CHANNELS] {
                self.core.channel_energies()
            }

            pub fn fdn_delay_ms(&self) -> [f32; $crate::dsp::CHANNELS] {
                self.core.fdn_delay_ms()
            }

            pub fn er_tap_times_ms(&self) -> [(f32, f32); $crate::dsp::ER_TAPS] {
                self.core.er_tap_times_ms()
            }

            pub fn er_tap_gains(&self) -> [(f32, f32); $crate::dsp::ER_TAPS] {
                self.core.er_tap_gains()
            }
        }
    };
}
pub(in crate::dsp::algo) use room_family_engine;

/// Room's voicing (see the module docs).
pub(in crate::dsp::algo) const ROOM: Voicing = Voicing {
    early: EarlyVoicing {
        small: [3.0, 4.0, 2.5],
        large: [15.0, 20.0, 8.0],
        source: [0.38, 0.64, 0.46],
        listener: [0.57, 0.31, 0.41],
        absorption: 0.3,
        second_order_gain: 1.0,
        scatter_ms: [[1.1, 2.3, 3.7], [0.9, 2.5, 3.9]],
        scatter_gain: 0.65,
        level: 0.25,
        envelope_scale: 1.0,
    },
    line_ms: (8.0, 60.0),
    size_scale: (0.8, 1.25),
    diffuser_ms: [
        [0.31, 0.47, 0.83, 1.13, 1.79, 2.71],
        [0.29, 0.53, 0.79, 1.07, 1.91, 2.53],
    ],
    diffuser_gain: 0.7,
    diffused_early: false,
    mod_depth_max: 8.0,
    decay_range: (0.1, 30.0),
    direct_diffuse: 0.3,
    late_level: 0.63,
    late_decay_norm: 0.0,
    seed: 0x0000_0000_524F_4F4D,
};

room_family_engine!(RoomEngine, 16, ROOM);
