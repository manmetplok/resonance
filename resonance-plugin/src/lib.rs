/// resonance-plugin: Lightweight CLAP plugin framework for the Resonance project.
///
/// Replaces nih-plug with a thin abstraction over clack-plugin.
pub mod clap_bridge;
pub mod formatters;
pub mod gui;
pub mod loader;
pub mod param;
pub mod plugin;
pub mod presets;
pub mod range;
pub mod smoother;
pub mod state;

#[cfg(feature = "editor-widgets")]
pub mod editor_widgets;

#[cfg(feature = "ui")]
pub mod ui;

// Re-export core types for convenient use
pub use clap_bridge::ClapBridge;
pub use formatters::*;
pub use loader::{rescan_directory, Mailbox};
pub use param::{BoolParam, FloatParam, IntParam, Param};
pub use plugin::{
    EventIterator, ExtraStateSaver, NoteEvent, OutputBuffer, OutputPortSpec, ResonancePlugin,
    TempoInfo,
};
pub use range::{FloatRange, IntRange};
pub use smoother::{Smoother, SmoothingStyle};

// Re-export clack-plugin crate so the export_clap! macro can reference it
pub use clack_plugin as clack_reexport;

// Re-export Match for the bridge's note event handling
pub use clack_plugin::events::Match;

/// Compute a stable u32 hash from a string ID (for CLAP param IDs).
/// Uses FNV-1a hash for simplicity and speed.
pub fn stable_hash(s: &str) -> u32 {
    let mut hash: u32 = 2166136261;
    for byte in s.bytes() {
        hash ^= byte as u32;
        hash = hash.wrapping_mul(16777619);
    }
    hash
}

/// Export a ResonancePlugin as a CLAP plugin.
///
/// Usage: `resonance_plugin::export_clap!(MyPlugin);`
#[macro_export]
macro_rules! export_clap {
    ($plugin:ty) => {
        $crate::clack_reexport::clack_export_entry!(
            $crate::clack_reexport::entry::SinglePluginEntry::<$crate::ClapBridge<$plugin>>
        );
    };
}
