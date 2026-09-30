//! The app's view of the plugin preset library (plugin-preset-library.md
//! §6.6): the same `resonance_plugin::presets::PresetLibrary` the plugins'
//! own editors use, over the same preset root and the same marks store, so
//! a preset saved, starred or renamed on either side is there on the other.
//!
//! - **Root.** `presets.plugin_preset_root` when set, else the user's
//!   (`RESONANCE_PLUGIN_PRESET_DIR` or the data dir). A test app gets a
//!   private directory under the test process's hermetic root at
//!   construction, so no test reads or writes the user's presets.
//! - **Factory banks** come from the scan (`ScannedPlugin::factory_presets`,
//!   read from each first-party binary's `resonance_factory_presets`
//!   symbol, ids and metadata included) and are registered with the
//!   library once per plugin.
//! - **Marks** are the shared `library_marks` store the NAM library uses
//!   (`control.amp_library.roots.marks`), opened once.

use std::path::PathBuf;
use std::sync::Arc;

use resonance_common::library_marks::SharedMarks;
use resonance_plugin::presets::{FactoryEntry, PresetBank, PresetLibrary};

use crate::Resonance;

/// A private preset root for a test app (`Host::None`), under the process's
/// hermetic data dir.
pub(crate) fn hermetic_preset_root() -> Option<PathBuf> {
    Some(crate::user_dirs::hermetic_subdir("plugin-presets"))
}

/// The preset root the app reads and writes.
pub(crate) fn preset_root(app: &Resonance) -> Option<PathBuf> {
    app.presets
        .plugin_preset_root
        .clone()
        .or_else(resonance_plugin::presets::user_preset_root)
}

/// The shared marks store, opened on first use.
pub(crate) fn marks(app: &Resonance) -> Arc<SharedMarks> {
    app.presets
        .library_marks
        .get_or_init(|| {
            Arc::new(match &app.control.amp_library.roots.marks {
                Some(dir) => SharedMarks::open(dir).unwrap_or_else(|e| {
                    tracing::warn!("plugin presets: marks unavailable: {e}");
                    SharedMarks::detached()
                }),
                None => SharedMarks::detached(),
            })
        })
        .clone()
}

/// The library for the app's preset root, with the marks store installed
/// and every scanned plugin's factory bank registered: built once and
/// cached (the bar reads it every frame; the process-wide root map lock
/// and the marks install are not per-frame work). A plugin scan registers
/// what is new ([`register_scanned`]).
pub(crate) fn library(app: &Resonance) -> Arc<PresetLibrary> {
    app.presets
        .library_cache
        .get_or_init(|| {
            let lib = match preset_root(app) {
                Some(root) => PresetLibrary::shared_for_root(&root),
                None => PresetLibrary::shared(),
            };
            lib.set_marks(marks(app));
            register_into(app, &lib);
            lib
        })
        .clone()
}

/// Register the factory bank of every scanned plugin the library does not
/// have yet. Called on a plugin scan.
pub(crate) fn register_scanned(app: &Resonance) {
    register_into(app, &library(app));
}

fn register_into(app: &Resonance, lib: &PresetLibrary) {
    for plugin in &app.plugin_catalog.available_plugins {
        if plugin.factory_presets.is_empty() || lib.factory_len(&plugin.clap_plugin_id) > 0 {
            continue;
        }
        lib.register_factory_entries(
            &plugin.clap_plugin_id,
            plugin.factory_presets.iter().map(|e| {
                FactoryEntry::from_parts(&e.id, &e.name, &e.json, e.meta.as_deref())
            }),
        );
    }
}

/// One plugin's bank over [`library`].
pub(crate) fn bank(app: &Resonance, clap_id: &str) -> PresetBank {
    let name = app
        .plugin_catalog
        .available_plugins
        .iter()
        .find(|p| p.clap_plugin_id == clap_id)
        .map(|p| p.name.clone());
    let bank = PresetBank::new(clap_id, &[]).with_library(library(app));
    match name {
        Some(n) => bank.with_plugin_name(&n),
        None => bank,
    }
}
