//! `presets.*` — the per-user plugin preset library across every plugin
//! (plugin-preset-library.md §12.2).
//!
//! Library state, not the project: these methods need no open project and
//! no plugin instance, and they never touch the undo history or the
//! project `revision`. The app reads and writes the same files and the
//! same marks store the plugins' own preset browsers do.

use serde::{Deserialize, Serialize};

use super::plugin_preset::{PluginPresetEntry, PresetFacets, PresetFilter, PresetMetaInput};

/// `presets.search` — find presets across every plugin, or one
/// ([`SearchParams`] -> [`SearchResult`]), before a plugin is even on a
/// track. Read-only.
pub const SEARCH: &str = "presets.search";

/// `presets.rename` — rename a **user** preset ([`RenameParams`] ->
/// [`EntryResult`]). Its id does not change, so stars, tags and every
/// loaded identity follow it.
pub const RENAME: &str = "presets.rename";

/// `presets.delete` — move a **user** preset to the trash (recoverable for
/// 30 days) ([`DeleteParams`] -> [`DeleteResult`]). Destructive: refused
/// with a summary until `confirm: true`.
pub const DELETE: &str = "presets.delete";

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
pub const METHODS: &[&str] = &[SET_MARKS, UPDATE_META, VOCABULARY, SEARCH, RENAME, DELETE];

/// Params for `presets.search`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct SearchParams {
    /// Only this plugin's presets (its CLAP id); omitted searches every
    /// plugin the app knows.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plugin_id: Option<String>,
    #[serde(flatten)]
    pub filter: PresetFilter,
}

/// One search hit: the preset and the plugin it belongs to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct SearchHit {
    pub plugin_id: String,
    #[serde(flatten)]
    pub entry: PluginPresetEntry,
}

/// Result of `presets.search`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct SearchResult {
    /// Matches before `limit` / `offset`.
    pub total: u32,
    pub hits: Vec<SearchHit>,
    pub facets: PresetFacets,
    /// The marks store's write counter, for a caller caching results.
    pub library_generation: u64,
}

/// Params for `presets.rename`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct RenameParams {
    pub plugin_id: String,
    pub preset_id: String,
    pub name: String,
}

/// Params for `presets.delete`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct DeleteParams {
    pub plugin_id: String,
    pub preset_id: String,
    /// Required: without it the call is refused with what would be lost.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub confirm: bool,
}

/// Result of `presets.delete`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct DeleteResult {
    /// Where the file went (recoverable for 30 days).
    pub trashed_path: String,
}

/// Params for `presets.set_marks`. At least one of `favorite` / `tags`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
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
#[serde(deny_unknown_fields)]
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
#[serde(deny_unknown_fields)]
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
