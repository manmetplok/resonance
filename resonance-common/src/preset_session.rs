//! `com.resonance.preset-session` — the first-party CLAP extension pair
//! that carries the loaded-preset identity between a Resonance plugin and
//! the Resonance host (plugin-preset-library.md §7, slice P5).
//!
//! CLAP's own `clap_host_preset_load.loaded()` tells a host *which* preset
//! a plugin loaded, but nothing else. This adds the rest, both ways:
//!
//! - **Host side** ([`HostPresetSession`], served by the host):
//!   `report(host, json)`, a `[main-thread]` call the plugin makes (from
//!   `on_main_thread`) whenever its identity or its modified flag changes.
//!   `json` is a UTF-8 object: `{"source": "factory"|"user", "id", "name",
//!   "modified": bool}`, or `{}` when nothing is loaded.
//! - **Plugin side** ([`PluginPresetSession`], served by the plugin):
//!   `set_ignored_params(plugin, json)`, `[main-thread]`: the CLAP ids
//!   (`[u32, …]`) of the params the host automates, which the plugin leaves
//!   out of its modified comparison (D8) — a param under a playing lane
//!   always differs from the preset, and that is not an edit.
//!
//! The one place both ends read the id and the layout from, so they cannot
//! drift. Pointers are `c_void` so neither end needs the other's CLAP
//! bindings; they are a `clap_host` / `clap_plugin` pointer respectively.

use std::ffi::{c_char, c_void, CStr};

/// The extension id, on both sides.
pub const EXTENSION_ID: &CStr = c"com.resonance.preset-session/1";

/// The host's half.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct HostPresetSession {
    /// `host` is the plugin's `clap_host` pointer; `json` a NUL-terminated
    /// UTF-8 identity report (see the module docs).
    pub report: Option<unsafe extern "C" fn(host: *const c_void, json: *const c_char)>,
}

/// The plugin's half.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct PluginPresetSession {
    /// `plugin` is the `clap_plugin` pointer; `json` a NUL-terminated JSON
    /// array of CLAP param ids.
    pub set_ignored_params:
        Option<unsafe extern "C" fn(plugin: *const c_void, json: *const c_char)>,
}

/// The `set_ignored_params` argument for `ids`.
pub fn ignored_params_json(ids: &[u32]) -> String {
    serde_json::to_string(ids).unwrap_or_else(|_| "[]".to_string())
}

/// Parse a `set_ignored_params` argument; garbage reads as "none".
pub fn parse_ignored_params(text: &str) -> Vec<u32> {
    serde_json::from_str(text).unwrap_or_default()
}

/// The identity a report carries, parsed. `None` for a report of "nothing
/// loaded" (`{}`) or one that does not parse.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IdentityReport {
    pub source: String,
    pub id: String,
    pub name: String,
    pub modified: bool,
}

impl IdentityReport {
    /// The report as JSON text.
    pub fn to_json(&self) -> String {
        serde_json::json!({
            "source": self.source,
            "id": self.id,
            "name": self.name,
            "modified": self.modified,
        })
        .to_string()
    }

    /// Parse a report as the host takes it: `Some(None)` for "nothing
    /// loaded" (`{}`), `Some(Some(_))` for an identity, `None` for a report
    /// that does not parse — which the host drops rather than reading as
    /// "nothing loaded".
    pub fn parse_report(text: &str) -> Option<Option<Self>> {
        let v: serde_json::Value = serde_json::from_str(text).ok()?;
        let obj = v.as_object()?;
        if obj.is_empty() {
            return Some(None);
        }
        Self::parse(text).map(Some)
    }

    /// Parse a report; `None` for "nothing loaded" or malformed.
    pub fn parse(text: &str) -> Option<Self> {
        let v: serde_json::Value = serde_json::from_str(text).ok()?;
        Some(Self {
            source: v.get("source")?.as_str()?.to_string(),
            id: v.get("id").and_then(|i| i.as_str()).unwrap_or("").to_string(),
            name: v.get("name")?.as_str()?.to_string(),
            modified: v.get("modified").and_then(|m| m.as_bool()).unwrap_or(false),
        })
    }
}
