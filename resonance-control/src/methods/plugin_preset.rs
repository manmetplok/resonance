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
//! Factory presets are only visible for Resonance's own plugins. A
//! third-party CLAP has no way to publish its baked-in bank to a host
//! (CLAP's `preset-discovery` factory is not implemented here), so it
//! reports none rather than a guess.

use serde::{Deserialize, Serialize};

/// Where a preset came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(rename_all = "lowercase")]
pub enum PluginPresetSource {
    /// Baked into the plugin binary. Read-only.
    Factory,
    /// A file this user saved. Can be overwritten.
    User,
}

/// One preset in a plugin's bank.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct PluginPresetEntry {
    /// Display name, and what the load/save methods take. Unique within
    /// its own source, but a user preset may share a factory preset's
    /// name.
    pub name: String,
    pub source: PluginPresetSource,
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
}
