//! [`PresetBank`]: one plugin's view of the [`PresetLibrary`].

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use super::format::{PresetMeta, PresetPluginInfo};
use super::library::{PresetLibrary, PresetRecord, SaveRequest};
use super::query::{Query, QueryResult};
use super::{FactoryPreset, PresetRef, PresetSource};
use crate::param::Param;
use crate::state::ParamRename;

/// How "Save as…" seeds a new preset.
#[derive(Debug, Clone, Default)]
pub struct SaveOptions {
    /// Descriptive metadata for the new preset (category, tags, …).
    pub meta: Option<PresetMeta>,
    /// The id of the preset this one is saved from.
    pub derived_from: Option<String>,
}

/// The browsable preset set for one plugin: its factory bank plus the
/// user's saved presets, backed by the process-wide [`PresetLibrary`] for
/// its root. Cheap to construct (the host builds one per call).
pub struct PresetBank {
    library: Arc<PresetLibrary>,
    plugin_id: String,
    factory: &'static [FactoryPreset],
    renames: &'static [ParamRename],
    plugin: PresetPluginInfo,
}

impl PresetBank {
    /// `plugin_id` is the plugin's CLAP id; it names the per-plugin
    /// subdirectory. `factory` is registered with the library.
    pub fn new(plugin_id: impl Into<String>, factory: &'static [FactoryPreset]) -> Self {
        Self::on_library(PresetLibrary::shared(), plugin_id.into(), factory)
    }

    /// The bank for plugin `P`: its CLAP id, factory bank, name and
    /// version (recorded as `plugin.version` in every preset it saves).
    pub fn for_plugin<P: crate::ResonancePlugin>() -> Self {
        Self::new(P::CLAP_ID, P::FACTORY_PRESETS).with_plugin_info(P::NAME, P::VERSION)
    }

    fn on_library(
        library: Arc<PresetLibrary>,
        plugin_id: String,
        factory: &'static [FactoryPreset],
    ) -> Self {
        if !factory.is_empty() {
            library.register_factory(&plugin_id, factory);
        }
        Self {
            plugin: PresetPluginInfo {
                id: plugin_id.clone(),
                ..PresetPluginInfo::default()
            },
            library,
            plugin_id,
            factory,
            renames: &[],
        }
    }

    /// Declare the plugin's parameter-id renames so presets written
    /// before a rename still recall the renamed parameter.
    pub fn with_renames(mut self, renames: &'static [ParamRename]) -> Self {
        self.renames = renames;
        self
    }

    /// Point this bank at an explicit user-preset root (the process-wide
    /// library for that root). Tests use it to stay hermetic.
    pub fn with_root(self, root: impl Into<PathBuf>) -> Self {
        let root = root.into();
        self.with_library(PresetLibrary::shared_for_root(&root))
    }

    /// Use a specific library (tests with an injected clock or marks).
    pub fn with_library(self, library: Arc<PresetLibrary>) -> Self {
        let mut bank = Self::on_library(library, self.plugin_id, self.factory);
        bank.renames = self.renames;
        bank.plugin = self.plugin;
        bank
    }

    /// Record the plugin's display name and version in saved presets.
    pub fn with_plugin_info(mut self, name: &str, version: &str) -> Self {
        self.plugin.name = Some(name.to_string());
        self.plugin.version = Some(version.to_string());
        self
    }

    pub fn plugin_id(&self) -> &str {
        &self.plugin_id
    }

    pub fn factory(&self) -> &'static [FactoryPreset] {
        self.factory
    }

    pub fn library(&self) -> &Arc<PresetLibrary> {
        &self.library
    }

    /// Directory this plugin's user presets live in.
    pub fn user_dir(&self) -> Option<PathBuf> {
        self.library.plugin_dir(&self.plugin_id)
    }

    // -----------------------------------------------------------------
    // Listing
    // -----------------------------------------------------------------

    /// Every record in bank order, checking the directory now.
    pub fn records(&self) -> Arc<Vec<PresetRecord>> {
        self.library.records(&self.plugin_id, Duration::ZERO)
    }

    /// Every record in bank order, checking the directory's fingerprint
    /// at most once per `max_age`. What a widget drawn every frame uses.
    pub fn records_cached(&self, max_age: Duration) -> Arc<Vec<PresetRecord>> {
        self.library.records(&self.plugin_id, max_age)
    }

    /// Every preset the user can pick: factory bank first (declared
    /// order), then user presets sorted by name.
    pub fn list(&self) -> Vec<PresetRef> {
        self.records().iter().map(|r| r.preset.clone()).collect()
    }

    /// Just the user half of [`list`](Self::list).
    pub fn list_user(&self) -> Vec<PresetRef> {
        self.records()
            .iter()
            .filter(|r| r.preset.source == PresetSource::User)
            .map(|r| r.preset.clone())
            .collect()
    }

    /// The record behind `preset` (an unresolved reference resolves by
    /// name).
    pub fn record(&self, preset: &PresetRef) -> Option<PresetRecord> {
        self.library.record(&self.plugin_id, preset)
    }

    /// `preset` with its id and current name filled in from the index, or
    /// `None` if it no longer exists.
    pub fn resolve(&self, preset: &PresetRef) -> Option<PresetRef> {
        self.record(preset).map(|r| r.preset)
    }

    /// [`resolve`](Self::resolve) from memory only; see
    /// [`PresetLibrary::peek_record`]. Never touches the disk.
    pub fn resolve_in_memory(&self, preset: &PresetRef) -> Option<PresetRef> {
        self.library
            .peek_record(&self.plugin_id, preset)
            .map(|r| r.preset)
    }

    /// [`resolve`](Self::resolve) against the index as last read, checking
    /// the directory at most once per `max_age` (for per-frame callers).
    pub fn resolve_cached(&self, preset: &PresetRef, max_age: Duration) -> Option<PresetRef> {
        let records = self.records_cached(max_age);
        super::library::find_record(&records, preset).map(|r| r.preset.clone())
    }

    /// Search this plugin's presets (`q.plugins` is overridden).
    pub fn query(&self, q: &Query) -> QueryResult {
        let q = Query {
            plugins: vec![self.plugin_id.clone()],
            ..q.clone()
        };
        self.library.query(&q)
    }

    // -----------------------------------------------------------------
    // Loading
    // -----------------------------------------------------------------

    /// The state document behind a preset, as JSON text, or `None` if it
    /// no longer exists.
    pub fn json_for(&self, preset: &PresetRef) -> Option<String> {
        self.library.state_json(&self.plugin_id, preset)
    }

    /// Apply a preset onto `params`. Returns `false` when the preset is
    /// gone or unreadable — the params are left untouched.
    pub fn apply(&self, preset: &PresetRef, params: &[&dyn Param]) -> bool {
        match self.json_for(preset) {
            Some(json) => super::apply(&json, params, self.renames),
            None => false,
        }
    }

    // -----------------------------------------------------------------
    // Writing
    // -----------------------------------------------------------------

    /// Save the current value of every parameter as a user preset. A user
    /// preset of the same name (case-insensitively) is overwritten in
    /// place, keeping its id; factory presets are never touched.
    ///
    /// The snapshot is [`crate::state::params_to_json`], which writes
    /// **every** declared parameter, so a preset can never be a partial
    /// recall (audit finding P7).
    pub fn save(&self, name: &str, params: &[&dyn Param]) -> Result<PresetRef, String> {
        self.save_with(name, params, SaveOptions::default())
    }

    /// [`save`](Self::save) with metadata for a new preset.
    pub fn save_with(
        &self,
        name: &str,
        params: &[&dyn Param],
        options: SaveOptions,
    ) -> Result<PresetRef, String> {
        self.write(name, crate::state::params_to_json(params), options)
    }

    /// Write an already-formed state document as a user preset — the blob
    /// the plugin produced through `save_state`, which is what the host
    /// has (ba todo #1333). Same naming and overwrite rules as
    /// [`save`](Self::save); the plugin's `"preset"` session key and a
    /// legacy top-level `"name"` are stripped.
    pub fn write_user_preset(
        &self,
        name: &str,
        document: &serde_json::Value,
    ) -> Result<PresetRef, String> {
        let mut doc = document.clone();
        if let Some(obj) = doc.as_object_mut() {
            obj.remove("name");
        }
        self.write(name, doc, SaveOptions::default())
    }

    fn write(
        &self,
        name: &str,
        doc: serde_json::Value,
        options: SaveOptions,
    ) -> Result<PresetRef, String> {
        self.library
            .save(
                &self.plugin_id,
                SaveRequest {
                    name: name.to_string(),
                    doc,
                    meta: options.meta,
                    derived_from: options.derived_from,
                    plugin: self.plugin.clone(),
                },
            )
            .map(|r| r.preset)
    }

    /// Rename a user preset. Its id does not change.
    pub fn rename(&self, preset: &PresetRef, new_name: &str) -> Result<PresetRef, String> {
        self.library
            .rename(&self.plugin_id, preset, new_name)
            .map(|r| r.preset)
    }

    /// Delete a user preset: it moves to the trash (D10).
    pub fn delete(&self, preset: &PresetRef) -> Result<(), String> {
        self.trash(preset).map(|_| ())
    }

    /// [`delete`](Self::delete), reporting where the file went.
    pub fn trash(&self, preset: &PresetRef) -> Result<PathBuf, String> {
        self.library.trash(&self.plugin_id, preset)
    }
}
