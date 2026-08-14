//! Melody-side helpers and post-processing for the vocal generator.
//!
//! The actual per-syllable walk lives in `super::style` and runs through
//! `walk_with_profile`. This module is a facade over one submodule per
//! responsibility, each of which is an independent reason to change:
//!
//! - [`syllables`] — the public `count_syllables` text helper, used by
//!   the SVS pipeline and by `VocalContext`;
//! - [`primitives`] — the contour / scale / chord lookups shared by the
//!   walker, the motif pass and `VocalContext::build`;
//! - [`motif`] — the motif re-skin pass;
//! - [`srdc`] — the statement–restatement–departure–conclusion section
//!   layout;
//! - [`climax`] — the section-climax pass;
//! - [`cadence`] — the per-line goal-cadence formula pass;
//! - [`spans`] — the public `vocal_phrase_spans` helper for the synth
//!   fill;
//! - [`cleanup`] — the post-walk overlap trim.
//!
//! `super::derive_vocal_with_motif` runs the passes in that order
//! (motif → srdc → per-line climax → section climax → cadence →
//! cleanup); several of them are seeded, so the order and the number of
//! their RNG draws are part of the persisted-output contract. See
//! `tests/vocal_melody_golden.rs`.

mod cadence;
mod cleanup;
mod climax;
mod motif;
mod primitives;
mod spans;
mod srdc;
mod syllables;

pub use spans::vocal_phrase_spans;
pub use syllables::count_syllables;

pub(in crate::derive::vocal) use cadence::apply_line_cadence_formulas;
pub(in crate::derive::vocal) use cleanup::enforce_no_overlap;
pub(in crate::derive::vocal) use climax::apply_section_climax;
pub(in crate::derive::vocal) use motif::{apply_motif_pitches, MotifPitchContext};
pub(in crate::derive::vocal) use primitives::{
    chord_at_beat, contour_height, scale_from_chords, snap_to_scale, total_beats,
};
pub(in crate::derive::vocal) use srdc::apply_srdc_layout;
