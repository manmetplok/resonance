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

/// All `plugins.*` method names — what `control.hello` advertises.
///
/// The `track.plugins` alias that #1236 kept reachable for one release
/// was removed in todo #1240; `plugins.catalog` is the only spelling.
pub const METHODS: &[&str] = &[CATALOG];

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
