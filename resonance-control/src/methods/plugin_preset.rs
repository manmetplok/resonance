//! Shapes shared by the plugin-preset methods on all three chain
//! surfaces (ba todo #1333).
//!
//! The methods themselves are declared per surface — `track.*`, `bus.*`,
//! `master.*` — because that is how a plugin is *addressed*, and it is
//! the same split `plugin_params` / `set_plugin_param` already use. What
//! a preset *is* does not vary by surface, so it is declared once here.
//!
//! # Factory and user presets
//!
//! A plugin's presets come from two places and behave differently:
//!
//! * **Factory** presets are compiled into the plugin binary. Every
//!   installation has the same ones, they are read-only, and they are the
//!   set that exists before anyone has saved anything.
//! * **User** presets are files under
//!   `$XDG_DATA_HOME/resonance/plugin-presets/<plugin-id>/`. They are
//!   whatever this user has saved, and only they can be overwritten.
//!
//! Both are listed together, tagged with their [`PluginPresetSource`], and
//! a user preset may deliberately shadow a factory name — so a preset is
//! identified by the *pair*, not by the name alone.
//!
//! Factory presets are only visible for Resonance's own plugins (and,
//! from slice P8, for third-party plugins that publish a
//! `preset-discovery` factory).

use serde::{Deserialize, Serialize};

/// Where a preset came from.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(rename_all = "lowercase")]
pub enum PluginPresetSource {
    /// Baked into the plugin binary. Read-only.
    #[default]
    Factory,
    /// A file this user saved. Can be overwritten.
    User,
}

/// One preset in a plugin's bank.
///
/// Every field after `source` is additive (plugin-preset-library.md
/// §12.1): `id` is the stable identity (a UUID for a user preset, a slug
/// for a factory one) that survives renames, and the metadata is the
/// preset's own (content) plus this user's marks (favourite, personal
/// tags, last use).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct PluginPresetEntry {
    /// Display name, and what the load/save methods take. Unique within
    /// its own source, but a user preset may share a factory preset's
    /// name.
    pub name: String,
    pub source: PluginPresetSource,
    /// Stable id: pass it as `preset_id` to load, mark or edit this preset
    /// whatever it is called.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub id: String,
    /// One of the seeded categories (`Bass`, `Pad`, … for instruments;
    /// `Track`, `Bus`, `Master`, `Creative`, `Utility`, `Init` for effects).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
    /// What it is for (`vocal`, `drum-bus`, `synth-bass`, …).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub instrument: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub genres: Vec<String>,
    /// Timbre words (`warm`, `dark`, `wide`, …).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub character: Vec<String>,
    /// The preset's own tags and this user's personal tags, merged.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    /// This user's personal tags only (what `presets.set_marks` edits).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub personal_tags: Vec<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub favorite: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// The plugin version the preset was saved with, when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plugin_version: Option<String>,
    /// RFC 3339.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub modified_at: Option<String>,
    /// Id of the preset this one was saved from.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub derived_from: Option<String>,
    /// When this user last picked it (RFC 3339).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_used: Option<String>,
}

/// How a preset list is ordered.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum PresetSort {
    /// Factory presets in the plugin's order, then user presets by name.
    #[default]
    Bank,
    Name,
    Category,
    /// Most recently used first.
    Recent,
    /// Most recently modified first.
    Modified,
}

/// Filters for a preset list (the `*.plugin_presets` methods and
/// `presets.search`). Every filter is optional; they AND, and values
/// within one facet OR.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct PresetFilter {
    /// Search text, the same syntax as the preset browsers: tokens are
    /// ANDed and matched (case- and accent-insensitively) against name,
    /// author, description, category and tags; `is:fav`, `is:recent`,
    /// `is:user` / `is:factory`, `tag:<t>`, `genre:<g>`, `cat:<c>`,
    /// `for:<instrument>`, `char:<character>` and `by:<author>` scope a
    /// token. Example: `"reese is:fav genre:drum-and-bass"`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub query: Option<String>,
    /// Only this user's favourites.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub favorites_only: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<PluginPresetSource>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub instrument: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub genres: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub character: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sort: Option<PresetSort>,
    /// At most this many presets (default 100); `total` counts them all.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub offset: Option<u32>,
}

impl PresetFilter {
    /// Whether any filter, sort or paging is set (a bare list otherwise).
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }
}

/// One facet value and how many presets in the result carry it (counted
/// with every *other* filter applied).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct FacetCount {
    pub value: String,
    pub count: u32,
}

/// Facet counts for a preset list.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct PresetFacets {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub source: Vec<FacetCount>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub category: Vec<FacetCount>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub instrument: Vec<FacetCount>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub genres: Vec<FacetCount>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub character: Vec<FacetCount>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<FacetCount>,
}

/// Descriptive metadata to write into a user preset (every field
/// optional: an omitted one is left as it is, or seeded from the loaded
/// preset on a save).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct PresetMetaInput {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instrument: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub genres: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub character: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tags: Option<Vec<String>>,
}

/// Result of the `*.save_plugin_preset` methods: the ack plus the id the
/// preset will have (minted up front, so it can be referred to before the
/// capture lands).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct SavePluginPresetResult {
    pub revision: u64,
    pub id: String,
}

/// Result of the `*.plugin_presets` methods.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct PluginPresetsView {
    /// The plugin these presets belong to.
    pub plugin_id: String,
    /// Factory presets in the order the plugin declares them, then the
    /// user's own by name.
    pub presets: Vec<PluginPresetEntry>,
    /// The loaded preset, when one is known. `None` after a plain
    /// parameter edit, or when the project was made before the plugin
    /// tracked preset identity.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current: Option<PluginPresetEntry>,
    /// Whether a parameter has moved since `current` was loaded.
    pub modified: bool,
    /// Whether the plugin itself reports `modified` (a Resonance plugin
    /// compares its sound with the preset, leaving automated params out).
    /// False when the host only knows its own edits — then `modified`
    /// misses changes made in the plugin's window.
    #[serde(default)]
    pub modified_known: bool,
    /// How many presets match before `limit` / `offset`.
    #[serde(default)]
    pub total: u32,
    /// Facet counts over the matches.
    #[serde(default)]
    pub facets: PresetFacets,
}
