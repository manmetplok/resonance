//! Ducking of the reverb's wet return.
//!
//! The classic ducked vocal reverb (warmth-width-depth.md §3.3): the wet
//! return is pulled down while the lead sings and blooms in the gaps, so
//! the vocal stays upfront without the reverb being turned down overall.
//!
//! The detector is the external sidechain **key** when the host has
//! connected one — the usual setup is a shared reverb on a return bus,
//! keyed from the dry lead — and otherwise this plugin's own dry input
//! (self-ducking, the same behaviour as `resonance-delay`'s `duck_*`).
//! Only the wet signal is ducked; the dry passes untouched.
//!
//! The ducker itself is the one `resonance-delay` uses,
//! [`resonance_dsp::dynamics::Ducker`]: with the key held well over the
//! threshold the wet settles exactly `amount × DUCK_MAX_GR_DB` dB down.

pub use resonance_dsp::dynamics::{Ducker, DUCK_MAX_GR_DB};

/// Attack and release the ducker starts with, before the first block
/// applies the params.
pub(crate) const INITIAL_ATTACK_MS: f32 = 15.0;
pub(crate) const INITIAL_RELEASE_MS: f32 = 200.0;
