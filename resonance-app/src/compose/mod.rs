//! Compose-tab state and behaviour. Owns sections, placements, the chord
//! lane, drumroll editor state, and the table of derived MIDI clips.

pub mod arrangement;
pub mod drumroll;
pub mod expression;
pub mod expression_edit;
pub mod generate;
pub mod invariants;
pub mod messages;
pub mod vocal_svs;

mod lane_generator;
mod section;
mod state;

// Inline tests: `resonance-app` is a binary crate with no `lib.rs`, so an
// integration test under `tests/` can't construct or load `ComposeState`.
// See ARCHITECTURE.md → Test Layout → Binary-crate exception.
#[cfg(test)]
mod tests;

pub use arrangement::{
    entry_span_label, entry_stepper_label, entry_stepper_value, resolve_arrangement,
    step_entry_length, ArrangementCoverage, ArrangementSpan, ResolvedArrangement,
};
pub use drumroll::{
    apply_density, builtin_pattern_catalog, builtin_pattern_names, instantiate_builtin,
    is_builtin_pattern, DrumGroup, DrumGroupPad, DrumPattern, DrumrollViewState,
};
pub use expression::{Breakpoint, CurveStatus, ExpressionCurve, ExpressionCurves};
pub use expression_edit::{ExpressionDockState, PenMode};
pub use generate::{DeriveKind, GenerateParams};
pub use lane_generator::{
    DrumVoiceMode, LaneGeneratorConfig, LaneGeneratorKind, LaneGeneratorKindTag,
};
pub use messages::{ComposeMessage, SectionChordSpec, WorkspaceGroup};
pub use section::{
    ChordState, EditSectionForm, EntryLength, NewSectionForm, PatternEntry, SectionDefinitionState,
    SectionPlacementState, SelectedLane,
};
pub use state::{ComposeState, RailPanelKey};
