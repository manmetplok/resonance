//! Shared installed-content registry for Resonance plugins.
//!
//! Tracks which downloadable assets (drum kits, amp models, etc.) have been
//! installed, persisted as `$XDG_DATA_HOME/resonance/installed.json`. Both
//! the drum and amp plugins can read/write this file via the helpers here.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Type of installed content.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ContentType {
    Drumkit,
    AmpModel,
}

/// One installed asset.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstalledItem {
    pub name: String,
    #[serde(rename = "type")]
    pub content_type: ContentType,
    /// Absolute path on disk where the content lives.
    pub path: String,
    /// ISO-8601 date string of when the item was installed.
    pub installed_at: String,
}

/// The on-disk JSON shape.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct InstalledRegistry {
    #[serde(default)]
    pub items: Vec<InstalledItem>,
}

impl InstalledRegistry {
    /// Iterate all installed items of a given type.
    pub fn items_of<'a>(
        &'a self,
        content_type: &'a ContentType,
    ) -> impl Iterator<Item = &'a InstalledItem> {
        self.items
            .iter()
            .filter(move |item| item.content_type == *content_type)
    }

    /// Check whether an item with the given name and type is present.
    pub fn is_installed(&self, name: &str, content_type: &ContentType) -> bool {
        self.items
            .iter()
            .any(|item| item.name == name && item.content_type == *content_type)
    }

    /// Build a set of installed names for a given type, for answering
    /// many membership queries against one registry load. Call sites
    /// that loop over N entries should `load_registry()` once and use
    /// this (or [`Self::is_installed`]) instead of the free
    /// [`is_installed`], which re-reads the JSON file per call.
    pub fn installed_set<'a>(
        &'a self,
        content_type: &ContentType,
    ) -> std::collections::HashSet<&'a str> {
        self.items
            .iter()
            .filter(|item| item.content_type == *content_type)
            .map(|item| item.name.as_str())
            .collect()
    }
}

/// Return the path to the registry file:
/// `$XDG_DATA_HOME/resonance/installed.json`.
pub fn registry_path() -> Option<PathBuf> {
    dirs::data_dir().map(|d| d.join("resonance/installed.json"))
}

/// Load the registry from disk. Returns an empty registry if the file
/// doesn't exist or can't be parsed (we never want to block on a corrupt
/// file).
pub fn load_registry() -> InstalledRegistry {
    let Some(path) = registry_path() else {
        return InstalledRegistry::default();
    };
    load_registry_from(&path)
}

/// Load from a specific path (useful for testing).
pub fn load_registry_from(path: &Path) -> InstalledRegistry {
    match std::fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_default(),
        Err(_) => InstalledRegistry::default(),
    }
}

/// Persist the registry to disk. Creates parent directories as needed.
pub fn save_registry(registry: &InstalledRegistry) -> Result<(), String> {
    let path = registry_path().ok_or_else(|| "no data dir".to_string())?;
    save_registry_to(registry, &path)
}

/// Save to a specific path (useful for testing).
pub fn save_registry_to(registry: &InstalledRegistry, path: &Path) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("mkdir {}: {e}", parent.display()))?;
    }
    let json =
        serde_json::to_string_pretty(registry).map_err(|e| format!("serialize registry: {e}"))?;
    std::fs::write(path, json.as_bytes()).map_err(|e| format!("write {}: {e}", path.display()))?;
    Ok(())
}

/// List all installed items of a given type. Loads the registry from
/// disk; callers needing multiple views of the registry should
/// `load_registry()` once and use the [`InstalledRegistry`] methods.
pub fn list_installed(content_type: &ContentType) -> Vec<InstalledItem> {
    load_registry()
        .items
        .into_iter()
        .filter(|item| item.content_type == *content_type)
        .collect()
}

/// Check whether an item with the given name and type is installed.
///
/// Convenience for one-off checks only — this re-reads the JSON file on
/// every call. To answer N queries (e.g. when looping over a server
/// index), `load_registry()` once and use
/// [`InstalledRegistry::is_installed`] or
/// [`InstalledRegistry::installed_set`].
pub fn is_installed(name: &str, content_type: &ContentType) -> bool {
    load_registry().is_installed(name, content_type)
}

/// Mark an item as installed. Replaces any existing entry with the same
/// name + type so we don't accumulate duplicates from re-downloads.
pub fn mark_installed(item: InstalledItem) -> Result<(), String> {
    let mut reg = load_registry();
    reg.items.retain(|existing| {
        !(existing.name == item.name && existing.content_type == item.content_type)
    });
    reg.items.push(item);
    save_registry(&reg)
}

/// Remove an installed item by name and type.
pub fn remove_installed(name: &str, content_type: &ContentType) -> Result<(), String> {
    let mut reg = load_registry();
    reg.items
        .retain(|item| !(item.name == name && item.content_type == *content_type));
    save_registry(&reg)
}

/// Return today's date (UTC) as an ISO-8601 string (YYYY-MM-DD). Uses
/// the system clock; clamps to the epoch date if the clock reports a
/// time before 1970.
pub fn today_iso() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    iso_date_from_unix_secs(secs)
}

/// Convert Unix seconds (UTC) to a YYYY-MM-DD string via the `time`
/// crate. Split out of [`today_iso`] so the conversion is testable
/// against pinned timestamps. Returns `"unknown"` for timestamps
/// outside `time`'s supported range (roughly ±9999 years).
pub fn iso_date_from_unix_secs(secs: i64) -> String {
    match time::OffsetDateTime::from_unix_timestamp(secs) {
        Ok(dt) => {
            let date = dt.date();
            format!(
                "{:04}-{:02}-{:02}",
                date.year(),
                u8::from(date.month()),
                date.day()
            )
        }
        Err(_) => "unknown".to_string(),
    }
}

