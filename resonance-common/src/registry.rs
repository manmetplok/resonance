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

/// List all installed items of a given type.
pub fn list_installed(content_type: &ContentType) -> Vec<InstalledItem> {
    load_registry()
        .items
        .into_iter()
        .filter(|item| item.content_type == *content_type)
        .collect()
}

/// Check whether an item with the given name and type is installed.
pub fn is_installed(name: &str, content_type: &ContentType) -> bool {
    load_registry()
        .items
        .iter()
        .any(|item| item.name == name && item.content_type == *content_type)
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

/// Return today's date as an ISO-8601 string (YYYY-MM-DD). Uses the
/// system clock. Falls back to "unknown" if the clock is unavailable.
pub fn today_iso() -> String {
    // We only need a date, not a full timestamp. Compute from the Unix
    // epoch to avoid pulling in a datetime crate.
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let days = secs / 86400;
    // Algorithm from http://howardhinnant.github.io/date_algorithms.html
    let z = days + 719468;
    let era = z / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!("{y:04}-{m:02}-{d:02}")
}

