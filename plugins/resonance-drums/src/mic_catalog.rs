//! Index of the mic setups available in a loaded Drummica manifest.
//!
//! Scanned once at kit-load time and shared with the GUI so mic-pick
//! dropdowns can render without re-reading the JSON on every frame.
//! Keyed by canonical position (`"KickIn"`, `"SNTop"`, `"OHsAB"`, …) so
//! the editor can enumerate exactly the setups the library provides.

use std::collections::BTreeMap;

use crate::kit_loader::KitManifest;

/// For each position key, the list of unique setup keys that appear in
/// the manifest. Setup keys are Drummica's full manifest identifiers
/// like `"01_KickIn_e901"` — they're globally unique across pieces.
#[allow(dead_code)] // consumed by the Phase 6 drum editor GUI
#[derive(Debug, Default, Clone)]
pub struct ManifestMicCatalog {
    /// Map from position key (e.g. `"KickIn"`) → setup keys in the order
    /// they first appear in the manifest.
    pub positions: BTreeMap<String, Vec<String>>,
}

#[allow(dead_code)] // consumed by the Phase 6 drum editor GUI
impl ManifestMicCatalog {
    /// Build a catalog by walking every piece in the manifest and
    /// collecting the unique (position, setup_key) pairs encountered.
    pub fn from_manifest(manifest: &KitManifest) -> Self {
        let mut positions: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for setups in manifest.values() {
            for (setup_key, setup) in setups {
                let entry = positions.entry(setup.position.clone()).or_default();
                if !entry.contains(setup_key) {
                    entry.push(setup_key.clone());
                }
            }
        }
        Self { positions }
    }

    /// All overhead setup keys, in order of first occurrence. Used by
    /// the global overhead picker in the drum editor.
    pub fn overhead_setups(&self) -> Vec<String> {
        let mut v = Vec::new();
        for (pos, keys) in &self.positions {
            if pos.starts_with("OH") {
                v.extend(keys.iter().cloned());
            }
        }
        v
    }

    /// All setup keys for a specific close-mic position (e.g. `"KickIn"`).
    pub fn close_setups(&self, position: &str) -> Vec<String> {
        self.positions.get(position).cloned().unwrap_or_default()
    }
}
