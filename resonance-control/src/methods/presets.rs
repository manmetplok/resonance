//! `presets.*` — the per-user plugin preset library across every plugin
//! (plugin-preset-library.md §12.2).
//!
//! Library state, not the project: these methods need no open project and
//! no plugin instance, and they never touch the undo history or the
//! project `revision`. The app reads and writes the same files and the
//! same marks store the plugins' own preset browsers do.

use serde::{Deserialize, Serialize};

use super::plugin_preset::{PluginPresetEntry, PresetMetaInput};

/// `presets.set_marks` — favourite or personally tag one preset, factory
/// presets included ([`SetMarksParams`] -> [`EntryResult`]). Per-user
/// state: no undo entry, no `revision` bump.
pub const SET_MARKS: &str = "presets.set_marks";

/// `presets.update_meta` — edit a **user** preset's own metadata
/// ([`UpdateMetaParams`] -> [`EntryResult`]). Refused on a factory
/// preset (use `set_marks` for personal tags). Not undoable: it edits the
/// library, not the project.
pub const UPDATE_META: &str = "presets.update_meta";

/// `presets.vocabulary` — the seeded facet values plus the ones in use
/// ([`VocabularyParams`] -> [`Vocabulary`]), so an agent tags
/// consistently instead of inventing near-duplicates.
pub const VOCABULARY: &str = "presets.vocabulary";

/// All `presets.*` method names.
pub const METHODS: &[&str] = &[SET_MARKS, UPDATE_META, VOCABULARY];

/// Params for `presets.set_marks`. At least one of `favorite` / `tags`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct SetMarksParams {
    /// CLAP id of the plugin the preset belongs to.
    pub plugin_id: String,
    /// The preset's stable id (from `*.plugin_presets`).
    pub preset_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub favorite: Option<bool>,
    /// Replaces the personal tags (normalised to lowercase `[a-z0-9-]`);
    /// `[]` clears them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tags: Option<Vec<String>>,
}

/// Params for `presets.update_meta`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct UpdateMetaParams {
    pub plugin_id: String,
    pub preset_id: String,
    /// Fields to replace (an omitted field is left alone).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub set: Option<PresetMetaInput>,
    /// Content tags to add.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub add_tags: Vec<String>,
    /// Content tags to remove.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub remove_tags: Vec<String>,
}

/// The updated preset.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct EntryResult {
    pub plugin_id: String,
    pub entry: PluginPresetEntry,
}

/// Params for `presets.vocabulary` (none).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct VocabularyParams {}

/// The metadata vocabulary: seeded values first, then values in use.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct Vocabulary {
    /// Categories for instrument plugins.
    pub categories_instrument: Vec<String>,
    /// Categories for effect plugins.
    pub categories_effect: Vec<String>,
    pub instrument: Vec<String>,
    pub genres: Vec<String>,
    pub character: Vec<String>,
    /// Tags in use (content and personal), most used first.
    pub tags: Vec<String>,
}
