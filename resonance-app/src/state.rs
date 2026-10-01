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
pub mod automation;
pub mod aux_sends;
pub mod banners;
pub mod browser;
pub mod clips;
pub mod control;
pub mod devices;
pub mod drag;
pub mod export;
pub mod external_instrument;
pub mod freeze;
pub mod global;
pub mod ids;
pub mod markers;
pub mod import;
pub mod input_devices;
pub mod interaction;
pub mod master;
pub mod media;
pub mod midi_devices;
pub mod midi_map;
pub mod missing_plugins;
pub mod mixer;
pub mod modal;
pub mod overlay;
pub mod performance;
pub mod plugin_catalog;
pub mod plugin_mirror;
pub mod plugin_window;
pub mod pool;
pub mod pool_import;
pub mod presets;
pub mod project_io;
pub mod quantize;
pub mod relink;
pub mod session;
pub mod sidechain;
pub mod takes;
pub mod track_group_registry;
pub mod tracks;
pub mod transport;
pub mod ui_transient;
pub mod viewport;

pub use automation::*;
pub use aux_sends::*;
pub use banners::*;
pub use browser::*;
pub use clips::*;
pub use control::*;
pub use devices::*;
pub use drag::*;
pub use export::*;
pub use external_instrument::*;
pub use freeze::*;
pub use global::*;
pub use markers::*;
pub use import::*;
pub use input_devices::*;
pub use interaction::*;
pub use master::*;
pub use media::*;
pub use midi_devices::*;
pub use midi_map::*;
pub use missing_plugins::*;
pub use mixer::*;
pub use modal::*;
pub use overlay::Overlay;
pub use performance::*;
pub use plugin_catalog::*;
pub use plugin_mirror::*;
pub use plugin_window::*;
pub use pool::*;
pub use pool_import::*;
pub use presets::*;
pub use project_io::*;
pub use quantize::*;
pub use relink::*;
pub use session::*;
pub use sidechain::*;
pub use takes::*;
pub use tracks::*;
pub use track_group_registry::*;
pub use transport::*;
pub use ui_transient::*;
pub use viewport::*;
