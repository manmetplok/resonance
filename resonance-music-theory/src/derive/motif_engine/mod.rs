// ---------------------------------------------------------------------------
// Motif-based melody engine
//
// Submodule layout:
//   - `types`   : shared `MotifNote`, `Transform`, `PhrasePlan`, `Contour`
//   - `build`   : construct a motif cell and apply transformations to it
//   - `phrase`  : phrase planning + the per-phrase realizer
//   - `harmony` : align raw motif pitches to chord/scale + leap recovery
//   - `cadence` : validated goal-cadence overlay on phrase endings
//   - `melody`  : top-level entry points for the melody lane
//
// Only the items re-exported below are visible to other `derive::*`
// modules and (for the two `pub` re-exports) outside the crate. Anything
// not in this re-export list is genuinely private to the engine.
// ---------------------------------------------------------------------------

mod build;
mod cadence;
mod harmony;
mod melody;
mod phrase;
mod types;

// External API consumed outside the `derive` module (re-exported from
// `derive::mod`).
pub use melody::{derive_motif_melody_with_section, motif_intervals};

// Sibling-module API: visible to `derive::motif_bass`, `derive::motif_rhythm`,
// `derive::motif_source`, and `derive::melody`. Each item is here because at
// least one of those siblings needs it.
pub(super) use build::{build_motif, transform_motif};
pub(super) use harmony::align_to_harmony;
pub(super) use melody::derive_motif_melody;
pub(super) use phrase::{plan_motif_transforms, plan_phrases};
pub(super) use types::{MotifNote, Transform};
