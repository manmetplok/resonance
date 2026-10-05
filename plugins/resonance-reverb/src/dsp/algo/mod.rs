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
//! Plate, Room, Chamber, Hall and Ambience are wired here ahead of their
//! phases (placeholder engines in their own files), so they are built in
//! parallel without touching this module. An algorithm is selectable once
//! it joins [`Algorithm::BUILT`], which must stay a prefix of the labels.

pub mod ambience;
pub mod chamber;
pub mod classic;
mod engine;
pub mod hall;
pub mod plate;
pub mod room;
pub(crate) mod switch;

pub use engine::Engine;

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
    Plate = 1,
    Room = 2,
    Chamber = 3,
    Hall = 4,
    Ambience = 5,
}

impl Algorithm {
    /// The algorithms this build has, in parameter order. The `algorithm`
    /// parameter's range ends at the last of these, so an unbuilt
    /// algorithm is never selectable.
    pub const BUILT: &'static [Algorithm] = &[
        Algorithm::Classic,
        Algorithm::Plate,
        Algorithm::Room,
        Algorithm::Chamber,
        Algorithm::Hall,
        Algorithm::Ambience,
    ];

    /// The algorithm a parameter value selects. Out-of-range indices fall
    /// back to Classic, but the `algorithm` parameter clamps to its range
    /// first, so a newer build's index arrives here as the last of
    /// [`Algorithm::BUILT`].
    pub fn from_index(index: i32) -> Self {
        usize::try_from(index)
            .ok()
            .and_then(|i| Self::BUILT.get(i).copied())
            .unwrap_or(Algorithm::Classic)
    }

    /// Whether this algorithm reads `build` (the ER-to-late crossfade).
    pub fn uses_build(self) -> bool {
        matches!(self, Algorithm::Hall)
    }

    /// Whether this algorithm reads `low_decay_mult`, `low_xover` and
    /// `high_decay_mult`. The editor greys the three out when it does not.
    pub fn uses_decay_shape(self) -> bool {
        match self {
            Algorithm::Classic => false,
            Algorithm::Plate
            | Algorithm::Room
            | Algorithm::Chamber
            | Algorithm::Hall
            | Algorithm::Ambience => true,
        }
    }
}
