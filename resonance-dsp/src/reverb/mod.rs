//! Shared reverb primitives (reverb-algorithms.md §4.3, phase R2).
//!
//! - [`Fdn`]: an N-line feedback delay network with an orthogonal
//!   ([`MatrixKind`]) feedback matrix, per-line [`Absorption`] designed from
//!   each line's own length, mutually prime line lengths and per-line
//!   [`SmoothRandom`] modulation.
//! - [`Absorption`]: Jot's per-line low-shelf × high-shelf loop filter for a
//!   three-band T60 ([`DecayBands`]).
//! - [`Allpass`]: a Schroeder allpass, modulatable and nestable (the
//!   plate's building block).
//! - [`SmoothRandom`]: a seeded, smoothed random modulator.
//! - [`ShoeboxEr`]: first/second-order image-source early reflections.
//! - [`DispersiveAllpass`]: a first-order allpass cascade, the spring's
//!   chirp (R8).
//!
//! Everything is real-time safe after construction (no allocation in any
//! per-sample path), deterministic for a seed, and `clear`/`reset` returns
//! a primitive to its freshly constructed state bit-exactly.

mod absorption;
mod allpass;
mod dispersive;
mod fdn;
mod matrix;
mod shoebox;
mod smooth_random;

pub use absorption::{loop_gain, Absorption, DecayBands};
pub use allpass::{allpass_read, Allpass};
pub use dispersive::{stage_group_delay, DispersiveAllpass, MAX_DISPERSION_COEFFICIENT};
pub use fdn::{next_prime, Fdn, FdnConfig};
pub use matrix::{hadamard_in_place, householder_in_place, MatrixKind};
pub use shoebox::{ErTap, ShoeboxEr, FIRST_ORDER_TAPS, MAX_TAPS};
pub use smooth_random::SmoothRandom;
