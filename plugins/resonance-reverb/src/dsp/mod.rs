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
//! - [`return_eq`] — the wet HPF/LPF on the reverb input (before the tank)
//! - [`duck`] — the wet-return ducker, keyed by the sidechain or the dry input

mod chain;
mod diffusion;
pub(crate) mod duck;
mod er;
mod fdn;
mod modulation;
mod return_eq;

/// Internal channel count for the diffusion + FDN buses. Shared by
/// every submodule that processes multi-channel signal arrays.
pub(crate) const CHANNELS: usize = 8;

/// Number of cascaded diffusion steps in the input chain.
pub(crate) const DIFFUSION_STEPS: usize = 4;

/// Per-sample limit on how fast a gliding delay-line read tap may move,
/// in samples per sample. A moving read tap resamples its content — the
/// classic size-morph Doppler — and this bound keeps the pitch swing
/// inside roughly ±4 semitones (read rate 0.75x..1.25x) while a
/// full-range size throw still completes in well under two seconds.
/// Shared by the FDN bank and the diffusion steps, which glide their
/// taps instead of relocating them per block (which clicked).
pub(crate) const TAP_SLEW_PER_SAMPLE: f32 = 0.25;

pub use chain::ReverbDsp;
pub use duck::{Ducker, DUCK_MAX_GR_DB};
pub use er::ER_TAPS;
