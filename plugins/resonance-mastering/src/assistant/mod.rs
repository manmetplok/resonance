//! One-shot master assistant.
//!
//! Runs alongside the live chain — the audio thread continuously copies
//! post-chain samples into a 10-second stereo ring. On demand the UI
//! thread snapshots the ring, runs every measurement stream over the
//! captured buffer, compares the resulting long-term average spectrum
//! to a stored genre target, and produces a small set of parameter
//! suggestions (tonal shelves, glue compressor, limiter, target LUFS).
//! The suggestions carry human-readable rationale and can be applied
//! to the plugin's atomic params with one call.

pub mod analyze;
pub mod capture;
pub mod decide;
pub mod reference;
pub mod state;
pub mod targets;

pub use analyze::AnalysisResult;
pub use decide::{Suggestions, Target};
pub use reference::ReferenceTrack;
pub use state::{Assistant, CAPTURE_SECONDS};
pub use targets::Genre;
