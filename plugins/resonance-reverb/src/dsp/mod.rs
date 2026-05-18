//! Core reverb DSP: 8-channel diffusion network + feedback delay network.
//!
//! Architecture (Signalsmith/Geraint Luff style):
//!   Input -> Pre-delay -> 4-step Diffusion Network -> FDN Feedback Loop -> Stereo Output
//!
//! The diffusion network blurs input into dense reflections using Hadamard mixing.
//! The FDN provides the decaying tail with Householder feedback and frequency-dependent damping.
//!
//! This module is split into:
//! - [`chain`] — top-level [`ReverbDsp`] orchestrator wiring all the stages together
//! - [`diffusion`] — input diffusion network (cascaded Hadamard-mixed delay lines)
//! - [`er`] — early reflections (parallel multi-tap stereo delay)
//! - [`fdn`] — late-tail Feedback Delay Network: delay bank + Householder feedback
//! - [`modulation`] — chorus/modulation LFO bank for the FDN read positions

mod chain;
mod diffusion;
mod er;
mod fdn;
mod modulation;

/// Internal channel count for the diffusion + FDN buses. Shared by
/// every submodule that processes multi-channel signal arrays.
pub(crate) const CHANNELS: usize = 8;

/// Number of cascaded diffusion steps in the input chain.
pub(crate) const DIFFUSION_STEPS: usize = 4;

pub use chain::ReverbDsp;
pub use er::ER_TAPS;
