/// resonance-plugin: Lightweight CLAP plugin framework for the Resonance project.
///
/// Replaces nih-plug with a thin abstraction over clack-plugin.
pub mod clap_bridge;
pub mod features;
pub mod formatters;
pub mod gui;
pub mod host;
pub mod loader;
pub mod param;
pub mod plugin;
pub mod presets;
pub mod range;
pub mod smoother;
pub mod state;

#[cfg(feature = "editor-widgets")]
pub mod editor_widgets;

/// Shared preset bar (picker + Save/Rename/Delete) for plugin editors.
#[cfg(feature = "editor-widgets")]
pub mod preset_ui;

#[cfg(feature = "ui")]
pub mod ui;

// Re-export core types for convenient use
pub use clap_bridge::ClapBridge;
pub use formatters::*;
pub use host::HostHandle;
pub use loader::{rescan_directory, Mailbox};
pub use param::{BoolParam, FloatParam, IntParam, Param};
pub use presets::{
    FactoryPreset, PresetBank, PresetEditor, PresetEvent, PresetRef, PresetSession, PresetSource,
};
pub use state::{ParamRename, STATE_VERSION};
pub use plugin::{
    ControlEvent, EventIterator, ExtraStateSaver, KeyBuffer, NoteEvent, OutputBuffer,
    OutputPortSpec, PluginEvent, ResonancePlugin, TempoInfo,
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
///
/// Alongside the CLAP entry point this exports
/// [`resonance_factory_presets`](crate::FACTORY_PRESETS_SYMBOL), a
/// first-party side channel carrying the plugin's factory bank as JSON.
///
/// The host needs the names to offer them over the control API, and CLAP
/// gives it no way to ask: presets are `include_str!`d into the binary,
/// and a host can only reach them by instantiating the plugin and loading
/// state it cannot enumerate. The standard answer is the
/// `clap.preset-discovery` factory, which is a much larger surface aimed
/// at third-party banks on disk; this is one symbol read out of the
/// library the host already has open (ba todo #1333).
///
/// Third-party plugins simply do not export it, and the host treats its
/// absence as "no factory presets" rather than an error — which is the
/// truthful answer for a plugin we know nothing about.
#[macro_export]
macro_rules! export_clap {
    ($plugin:ty) => {
        $crate::clack_reexport::clack_export_entry!(
            $crate::clack_reexport::entry::SinglePluginEntry::<$crate::ClapBridge<$plugin>>
        );

        /// The factory bank as a NUL-terminated JSON array of
        /// `{"name": .., "json": ..}`, or null when this plugin ships none.
        ///
        /// # Safety
        /// The returned pointer is valid for the lifetime of the process
        /// and must not be freed by the caller: it borrows a `CString`
        /// built once into a `OnceLock`. The host copies out of it.
        #[no_mangle]
        pub extern "C" fn resonance_factory_presets() -> *const ::std::os::raw::c_char {
            static ENCODED: ::std::sync::OnceLock<Option<::std::ffi::CString>> =
                ::std::sync::OnceLock::new();
            let slot = ENCODED.get_or_init(|| {
                let presets =
                    <$plugin as $crate::ResonancePlugin>::FACTORY_PRESETS;
                if presets.is_empty() {
                    return None;
                }
                $crate::presets::encode_factory_bank(presets)
            });
            match slot {
                Some(text) => text.as_ptr(),
                None => ::std::ptr::null(),
            }
        }
    };
}
