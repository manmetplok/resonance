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

#[cfg(test)]
mod tests;

pub use drumroll::{DrumGroup, DrumrollViewState};
#[allow(unused_imports)]
pub use drumroll::{
    default_kit_pads, grid_label, DrumGroupPad, KitPadInfo, GROUP_PALETTE,
};
pub use generate::{DeriveKind, GenerateParams};
pub use lane_generator::{
    DrumLaneConfig, DrumVoiceMode, LaneGeneratorConfig, LaneGeneratorKind, LaneGeneratorKindTag,
};
pub use messages::ComposeMessage;
#[allow(unused_imports)]
pub use messages::DrumGroupsMessage;
pub use section::{
    ChordState, EditSectionForm, NewSectionForm, SectionDefinitionState, SectionPlacementState,
    SelectedLane,
};
pub use state::ComposeState;
