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
pub mod nonlinear;
pub mod plate;
pub mod room;
pub mod shimmer;
pub mod spring;
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

/// The creative algorithms' own parameters (§4.1, indices 29-34), set as
/// one value every block; an engine that does not use them ignores it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Extras {
    /// Shimmer pitch shift, semitones (from `shimmer_pitch`).
    pub shimmer_semitones: f32,
    /// Share of the Shimmer loop that is pitch-shifted, `0..=1`.
    pub shimmer_amount: f32,
    /// `nl_shape` index: 0 Gated, 1 Reverse, 2 Flat.
    pub nl_shape: i32,
    /// Nonlinear envelope length, ms.
    pub nl_length_ms: f32,
    /// Spring chirp rate (dispersion), `0..=1`.
    pub spring_tension: f32,
    /// Spring transient "drip", `0..=1`.
    pub spring_drip: f32,
}

impl Default for Extras {
    /// The parameters' defaults.
    fn default() -> Self {
        Self {
            shimmer_semitones: 12.0,
            shimmer_amount: 0.3,
            nl_shape: 0,
            nl_length_ms: 300.0,
            spring_tension: 0.5,
            spring_drip: 0.3,
        }
    }
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
    Spring = 6,
    Nonlinear = 7,
    Shimmer = 8,
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
        Algorithm::Spring,
        Algorithm::Nonlinear,
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
        matches!(self, Algorithm::Hall | Algorithm::Shimmer)
    }

    /// Whether this algorithm reads `low_decay_mult`, `low_xover` and
    /// `high_decay_mult`. The editor greys the three out when it does not.
    pub fn uses_decay_shape(self) -> bool {
        match self {
            Algorithm::Classic | Algorithm::Spring | Algorithm::Nonlinear => false,
            Algorithm::Plate
            | Algorithm::Room
            | Algorithm::Chamber
            | Algorithm::Hall
            | Algorithm::Ambience
            | Algorithm::Shimmer => true,
        }
    }
}
