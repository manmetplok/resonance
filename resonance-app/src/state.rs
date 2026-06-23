//! GUI-side state types for the Resonance application.
//!
//! The types here mirror the engine-side configuration in a shape the
//! Iced view layer can borrow into directly. They're split per domain
//! under `state/<domain>.rs` and re-exported from this module so the
//! rest of the codebase keeps using `crate::state::TypeName` without
//! caring about the file layout.

// Inherent-impl extensions on `Resonance` that operate on state held in
// this module. Each one lives next to the data it touches instead of
// piling onto the top-level `impl Resonance` block in `lib.rs`.
pub mod arrange;
pub mod plugin_index;

// Data types, grouped by domain.
pub mod aux_sends;
pub mod clips;
pub mod export;
pub mod global;
pub mod interaction;
pub mod midi_map;
pub mod mixer;
pub mod project_io;
pub mod tracks;
pub mod transport;
pub mod viewport;

pub use aux_sends::*;
pub use clips::*;
pub use export::*;
pub use global::*;
pub use interaction::*;
pub use midi_map::*;
pub use mixer::*;
pub use project_io::*;
pub use tracks::*;
pub use transport::*;
pub use viewport::*;
