//! Device-definition registry (architecture doc #201 §2, epic #40).
//!
//! Collects [`DeviceDefinition`]s from two sources, **last-wins by `id`**: the
//! read-only definitions bundled with the app, then a user/project definitions
//! folder. A user definition therefore shadows a bundled one sharing its `id`,
//! letting users override a factory device without editing the shipped files.
//!
//! Each definition is one JSON file (extension [`DEVICE_DEFINITION_EXT`]) in a
//! directory, mirroring the plugin/preset scanning already in the crate
//! ([`crate::scan::scan_directory`], [`crate::midi_map::load_controller_maps`]).
//! A file that cannot be read, parsed, or validated is **skipped** — its problem
//! is collected into [`DeviceDefinitionRegistry::errors`] and the rest still
//! load. Scanning never panics on bad input.

use std::path::{Path, PathBuf};

use crate::device_definition::DeviceDefinition;
use crate::scan::scan_directory;

/// On-disk file extension for device-definition files (one definition per file).
pub const DEVICE_DEFINITION_EXT: &str = "json";

/// The read-only device definitions shipped with the app, embedded into the
/// binary as JSON so they are always present regardless of the install layout
/// (the `bundled` source for [`DeviceDefinitionRegistry::scan_bundled`]). Each
/// entry is the verbatim on-disk JSON of one bundled definition; add a device by
/// dropping its `.json` next to the others and listing it here.
const BUNDLED_DEFINITION_JSON: &[&str] =
    &[include_str!("../bundled/device_definitions/moog-muse.json")];

/// Parse the [`BUNDLED_DEFINITION_JSON`] into definitions — the read-only set
/// shipped with the app, used to seed a [`DeviceDefinitionRegistry`] before the
/// user folder is layered on top. The embedded content is our own and is covered
/// by a load/validate/round-trip test, so a parse failure here is a build bug,
/// not a runtime condition; it panics rather than silently dropping a device.
pub fn bundled_definitions() -> Vec<DeviceDefinition> {
    BUNDLED_DEFINITION_JSON
        .iter()
        .map(|json| {
            DeviceDefinition::from_json(json.as_bytes())
                .expect("bundled device definition must parse")
        })
        .collect()
}

/// A device-definition file that was skipped during a scan, with the reason.
///
/// Collected rather than fatal: one malformed file never blocks the others (same
/// never-block-on-corrupt policy as [`crate::registry`] and
/// [`crate::midi_map`]). The `message` is human-readable (read/parse/validate).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceScanError {
    /// The file that could not be loaded.
    pub path: PathBuf,
    /// Why it was skipped.
    pub message: String,
}

/// The default user/project device-definitions folder:
/// `$XDG_DATA_HOME/resonance/device_definitions` (alongside `installed.json`
/// and `controller_maps.json`). `None` when no data dir can be determined.
pub fn user_definitions_dir() -> Option<PathBuf> {
    dirs::data_dir().map(|d| d.join("resonance/device_definitions"))
}

/// A scanned set of [`DeviceDefinition`]s, deduplicated last-wins by `id`.
#[derive(Debug, Clone, Default)]
pub struct DeviceDefinitionRegistry {
    /// Definitions in insertion order, at most one per `id`.
    defs: Vec<DeviceDefinition>,
    /// Files skipped during scanning, in encounter order.
    errors: Vec<DeviceScanError>,
}

impl DeviceDefinitionRegistry {
    /// Build a registry by scanning the `bundled` (read-only, shipped) directory
    /// first, then the `user` directory, so a user definition overrides a
    /// bundled one with the same `id`. Missing directories scan as empty.
    pub fn scan(bundled: &Path, user: &Path) -> Self {
        let mut reg = Self::default();
        reg.scan_dir(bundled);
        reg.scan_dir(user);
        reg
    }

    /// Seed the registry with the embedded read-only [`bundled_definitions`].
    /// Call before [`Self::scan_dir`] on the user folder so a user definition
    /// shadows a bundled one with the same `id` (last-wins). Unlike
    /// [`Self::scan`], this needs no on-disk bundled directory — the definitions
    /// ship inside the binary.
    pub fn scan_bundled(&mut self) {
        for def in bundled_definitions() {
            self.insert(def);
        }
    }

    /// Scan one directory of definition files into the registry, later files
    /// overriding earlier ones by `id`. Useful to layer more than two sources;
    /// [`Self::scan`] is the common bundled-then-user case.
    pub fn scan_dir(&mut self, dir: &Path) {
        for path in scan_directory(dir, DEVICE_DEFINITION_EXT) {
            let path = PathBuf::from(path);
            match Self::load_from_path(&path) {
                Ok(def) => self.insert(def),
                Err(message) => self.errors.push(DeviceScanError { path, message }),
            }
        }
    }

    /// Insert a definition, replacing any existing one with the same `id`
    /// (last-wins) and preserving the existing slot's position.
    fn insert(&mut self, def: DeviceDefinition) {
        match self.defs.iter_mut().find(|d| d.id == def.id) {
            Some(slot) => *slot = def,
            None => self.defs.push(def),
        }
    }

    /// All definitions, in insertion order (at most one per `id`).
    pub fn list(&self) -> Vec<&DeviceDefinition> {
        self.defs.iter().collect()
    }

    /// The definition with the given `id`, if any.
    pub fn get(&self, id: &str) -> Option<&DeviceDefinition> {
        self.defs.iter().find(|d| d.id == id)
    }

    /// Files skipped during scanning, with the reason each was rejected.
    pub fn errors(&self) -> &[DeviceScanError] {
        &self.errors
    }

    /// Load and validate a single user-authored definition from a JSON file.
    /// Returns a human-readable error if the file can't be read, parsed, or
    /// fails [`DeviceDefinition::validate`] — callers (and [`Self::scan_dir`])
    /// treat that as "skip this file".
    pub fn load_from_path(path: &Path) -> Result<DeviceDefinition, String> {
        let bytes = std::fs::read(path).map_err(|e| format!("read {}: {e}", path.display()))?;
        let def = DeviceDefinition::from_json(&bytes)
            .map_err(|e| format!("{}: {e}", path.display()))?;
        def.validate()
            .map_err(|e| format!("invalid device definition {}: {e}", path.display()))?;
        Ok(def)
    }

    /// Save a user-authored definition to a JSON file, creating parent
    /// directories as needed. Validates before writing so a broken definition is
    /// never persisted. Pretty-printed, matching [`DeviceDefinition::to_json`].
    pub fn save_to_path(def: &DeviceDefinition, path: &Path) -> Result<(), String> {
        def.validate()
            .map_err(|e| format!("refusing to save invalid device definition: {e}"))?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("mkdir {}: {e}", parent.display()))?;
        }
        let json = def.to_json()?;
        std::fs::write(path, json.as_bytes())
            .map_err(|e| format!("write {}: {e}", path.display()))?;
        Ok(())
    }
}
