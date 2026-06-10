//! Compose-tab state and behaviour. Owns sections, placements, the chord
//! lane, drumroll editor state, and the table of derived MIDI clips.

pub mod drumroll;
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

pub use drumroll::{DrumGroup, DrumPattern, DrumrollViewState};
pub use generate::{DeriveKind, GenerateParams};
pub use lane_generator::{
    DrumVoiceMode, LaneGeneratorConfig, LaneGeneratorKind, LaneGeneratorKindTag,
};
pub use messages::{ComposeMessage, WorkspaceGroup};
pub use section::{
    ChordState, EditSectionForm, NewSectionForm, SectionDefinitionState, SectionPlacementState,
    SelectedLane,
};
pub use state::{ComposeState, RailPanelKey};
