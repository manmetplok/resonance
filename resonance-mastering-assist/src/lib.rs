//! The mastering assistant's engine (warmth-width-depth.md §7.4), with no
//! plugin in it.
//!
//! Given two channels of audio, [`analyze::run`] measures what the
//! assistant reasons about (BS.1770 loudness, true peak, crest,
//! correlation, a 1/6-octave long-term average spectrum), all through
//! `resonance-metering`. [`decide::build`] compares that analysis with a
//! target, a built-in genre band ([`targets`]) or a reference track
//! ([`reference`]), and returns suggestions stage by stage, each with its
//! rationale and the exact mastering-plugin param writes it amounts to.
//!
//! Two callers share it: the mastering plugin's assistant panel, which
//! analyses a live capture and applies the writes to its own params, and
//! the app's `master.assist`, which analyses an offline render and
//! answers with the writes. Both depend on this crate rather than one on
//! the other, so the app links no plugin. The plugin owns everything that
//! touches its params (the key lookup behind [`decide::ParamSink`]) and a
//! test there checks that every key this crate emits resolves.

pub mod analyze;
pub mod decide;
pub mod reference;
pub mod targets;

pub use analyze::AnalysisResult;
pub use decide::{ParamSink, Suggestions, Target};
pub use reference::ReferenceTrack;
pub use targets::Genre;
