//! Reference-track (A/B) GUI domain. Owns the view-side reference state
//! ([`ReferenceState`]) and the user-intent messages ([`ReferenceMessage`])
//! that the update layer turns into engine `AudioCommand`s. The handlers
//! live in `crate::update::reference` and the engine-event folding in
//! `crate::engine_events::reference`, mirroring the layout the other
//! domains use (compose, tracks).

mod messages;
mod state;

pub use messages::ReferenceMessage;
pub use state::{
    AbMeters, ReferenceEntry, ReferenceMarkerState, ReferenceState, ReferenceStatus, ReferenceUndo,
};
