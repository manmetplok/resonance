//! `vocal` test group.
//!
//! One binary per group instead of one per file. Each of these files used
//! to be its own integration-test target, and every target re-monomorphizes
//! the whole app + iced generic surface — 240 of them cost 193s to relink
//! after a one-line change (ba doc #285 §3, todo #1368). They are still
//! ordinary test files; only the target boundary moved.
//!
//! The `#[path]` attributes are load-bearing: `mod foo;` in a crate-root
//! file resolves against that file's own directory (`tests/`), not against
//! a `tests/<group>/` subdirectory.

#[path = "vocal/expression_curve_edits.rs"]
mod expression_curve_edits;
#[path = "vocal/expression_curves_model.rs"]
mod expression_curves_model;
#[path = "vocal/pronunciation_resolution.rs"]
mod pronunciation_resolution;
#[path = "vocal/vocal_articulation.rs"]
mod vocal_articulation;
#[path = "vocal/vocal_audio_io.rs"]
mod vocal_audio_io;
#[path = "vocal/vocal_audio_placement.rs"]
mod vocal_audio_placement;
#[path = "vocal/vocal_expression_segment.rs"]
mod vocal_expression_segment;
#[path = "vocal/vocal_phoneme_accessor.rs"]
mod vocal_phoneme_accessor;
#[path = "vocal/vocal_render_cache.rs"]
mod vocal_render_cache;
#[path = "vocal/vocal_render_plan.rs"]
mod vocal_render_plan;
#[path = "vocal/vocal_segment_pronunciation.rs"]
mod vocal_segment_pronunciation;
#[path = "vocal/vocal_tuning_mirror.rs"]
mod vocal_tuning_mirror;
#[path = "vocal/voicebank_curve_support.rs"]
mod voicebank_curve_support;
