//! What a plugin's `clap.preset-discovery-factory` described (slice P8).
//! Plain data: the indexer that fills it is `clap_host::discovery`.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// `CLAP_PRESET_DISCOVERY_IS_FACTORY_CONTENT`.
pub const DISCOVERY_IS_FACTORY_CONTENT: u32 = 1 << 0;
/// `CLAP_PRESET_DISCOVERY_IS_USER_CONTENT`.
pub const DISCOVERY_IS_USER_CONTENT: u32 = 1 << 1;
/// `CLAP_PRESET_DISCOVERY_IS_FAVORITE`.
pub const DISCOVERY_IS_FAVORITE: u32 = 1 << 3;

/// Where a discovered preset lives, as `clap.preset-load` names it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum DiscoveredLocation {
    Plugin,
    File(PathBuf),
}

/// One preset a provider described.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiscoveredPreset {
    pub name: String,
    pub location: DiscoveredLocation,
    pub load_key: Option<String>,
    /// CLAP plugin ids (`abi == "clap"`) the preset is for. Empty means the
    /// provider named none; the indexer then gives it to a bundle's only
    /// plugin, and to no plugin of a bundle with several.
    pub plugin_ids: Vec<String>,
    pub creators: Vec<String>,
    pub description: Option<String>,
    pub features: Vec<String>,
    /// `CLAP_PRESET_DISCOVERY_IS_*`: the preset's own, or its location's
    /// when the provider never set any.
    pub flags: u32,
}

impl DiscoveredPreset {
    pub fn is_favorite(&self) -> bool {
        self.flags & DISCOVERY_IS_FAVORITE != 0
    }

    /// A stable id for the preset within its plugin: the load key for a
    /// `PLUGIN` location, the path (plus the key) for a file.
    pub fn stable_id(&self) -> String {
        match (&self.location, &self.load_key) {
            (DiscoveredLocation::Plugin, Some(k)) => format!("plugin:{k}"),
            (DiscoveredLocation::Plugin, None) => format!("plugin:{}", self.name),
            (DiscoveredLocation::File(p), Some(k)) => format!("file:{}#{k}", p.display()),
            (DiscoveredLocation::File(p), None) => format!("file:{}", p.display()),
        }
    }

    /// Whether `location` + `load_key` (a `loaded()` echo) name this preset.
    pub fn is_at(&self, location: &DiscoveredLocation, load_key: Option<&str>) -> bool {
        &self.location == location && self.load_key.as_deref() == load_key
    }
}
