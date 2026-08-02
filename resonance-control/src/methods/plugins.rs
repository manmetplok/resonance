//! `plugins.*` — the catalog of installed plugins.
//!
//! This is a query about the APP, not about a track: it lists what can
//! be loaded, and takes no parameters at all. It used to be called
//! `track.plugins`, and the name was a trap — a field agent read it as
//! "the plugins on a track", concluded there was no way to inspect a
//! track's chain, and reported that as a missing feature. The chain
//! query has existed all along under `track.plugin_params`, which
//! reports the real loaded chain with full parameter metadata (ba doc
//! #273, todo #1236).
//!
//! [`CATALOG`] is read-only and needs NO open project — the scanner
//! populates the catalog at startup, so a client can decide what to
//! build with before it creates or opens anything.

use crate::methods::track::PluginKind;
use serde::{Deserialize, Serialize};

/// `plugins.catalog` — every installed plugin, read-only (no params ->
/// [`PluginCatalog`]).
pub const CATALOG: &str = "plugins.catalog";

/// The old name for [`CATALOG`], kept reachable for one release so an
/// existing client does not break on the rename. It resolves to the
/// same handler and returns the same [`PluginCatalog`].
///
/// **New clients must not use it.** It is scheduled for removal in a
/// following release; there is deliberately no MCP tool under this name,
/// so an agent sees exactly one spelling (`plugins_catalog`). This
/// constant is the single place the alias is documented.
#[deprecated(note = "renamed to `plugins.catalog` (CATALOG); this alias goes in a later release")]
pub const PLUGINS_DEPRECATED_ALIAS: &str = "track.plugins";

/// All `plugins.*` method names, including the deprecated alias — so
/// `control.hello` tells the truth about every name the app answers.
#[allow(deprecated)]
pub const METHODS: &[&str] = &[CATALOG, PLUGINS_DEPRECATED_ALIAS];

/// Result of `plugins.catalog`: the built-in plugin catalog.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct PluginCatalog {
    pub plugins: Vec<PluginCatalogEntry>,
}

/// One catalog entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct PluginCatalogEntry {
    /// Stable id used in
    /// [`AddPluginParams::plugin_id`](crate::methods::track::AddPluginParams::plugin_id).
    pub id: String,
    pub name: String,
    pub kind: PluginKind,
}
