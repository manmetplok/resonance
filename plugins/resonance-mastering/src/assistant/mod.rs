//! One-shot master assistant.
//!
//! Runs alongside the live chain — the audio thread continuously copies
//! post-chain samples into a 10-second stereo ring. On demand the UI
//! thread snapshots the ring, runs every measurement stream over the
//! captured buffer, compares the resulting long-term average spectrum
//! to a built-in genre target band (or a loaded reference track), and
//! produces a small set of parameter
//! suggestions (tonal shelves, glue compressor, limiter, target LUFS).
//! The suggestions carry human-readable rationale and can be applied
//! to the plugin's atomic params with one call.
//!
//! The analysis and the decision engine are the library crate
//! `resonance_mastering_assist` (which the app's `master.assist` uses
//! too); they are re-exported here under their old paths. This module
//! keeps what belongs to the plugin: the capture ring, the panel's state,
//! decoding a reference file, and the param lookup the suggestions are
//! applied through ([`decide::param_by_key`]).

pub use resonance_mastering_assist::{analyze, targets};

pub mod capture;
pub mod decide;
pub mod reference;
pub mod state;

pub use analyze::AnalysisResult;
pub use decide::{Suggestions, Target};
pub use reference::ReferenceTrack;
pub use state::{
    Assistant, AssistantSettings, AssistantStateSaver, TargetMode, CAPTURE_SECONDS, STATE_KEY,
};
pub use targets::Genre;
