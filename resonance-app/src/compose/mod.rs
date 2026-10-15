//! Compose-tab state and behaviour. Owns sections, placements, the chord
//! lane, drumroll editor state, and the table of derived MIDI clips.

pub mod arrangement;
pub mod drumroll;
pub mod expression;
pub mod expression_lanes;
pub mod generate;
pub mod invariants;
pub mod messages;
pub mod vocal_melody;
pub mod vocal_render_status;
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
pub use expression_lanes::{project_vocal_lanes, VocalLane, VocalLanes};
pub use generate::{DeriveKind, GenerateParams};
pub use lane_generator::{
    DrumVoiceMode, LaneGeneratorConfig, LaneGeneratorKind, LaneGeneratorKindTag,
};
pub use messages::{ComposeMessage, SectionChordSpec, WorkspaceGroup};
pub use section::{
    ChordState, EditSectionForm, EntryLength, NewSectionForm, PatternEntry, SectionDefinitionState,
    SectionPlacementState, SelectedLane,
};
pub use state::{ComposeState, RailPanelKey, VocalTakeLayout, VocalTakeSources};
pub use vocal_melody::{LaneMelodyFlags, VocalMelodyChoice};
pub use vocal_render_status::{
    VocalLaneErrorKind, VocalLaneKey, VocalLaneRenderState, VocalLaneRenderStatus,
};

/// A section whose one vocal render does not fit every bar it plays at
/// (code review FU-M11b): see `update::compose::vocal_tempo_mismatch`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VocalTempoMismatch {
    /// The tempo the vocal is rendered at (`section_meter`'s).
    pub rendered_bpm: f32,
    /// The placed tempo that differs most from it.
    pub other_bpm: f32,
}
