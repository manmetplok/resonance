//! Persistent application settings, stored as JSON at
//! `dirs::config_dir()/resonance/settings.json`. Mirrors `recent.rs`:
//! loaded once at app startup, all I/O errors swallowed (logged to
//! stderr), and a broken or missing file must never prevent the app
//! from starting — it falls back to [`AppSettings::default`].
//!
//! Today the only section is [`AutosaveSettings`]; it lives inside a
//! top-level [`AppSettings`] wrapper so future settings sections can be
//! added without a format migration. Every struct is `#[serde(default)]`
//! so an older on-disk file missing a field (or a whole section) loads
//! cleanly with that field defaulted.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

const FILE_NAME: &str = "settings.json";
const APP_DIR: &str = "resonance";

/// User-configurable autosave + versioned-backup settings, persisted
/// across sessions. Defaults: autosave on, every 30 s, keep 10 backups
/// (see epic #32 / doc #171). This struct holds *configuration only* —
/// the runtime status the UI shows (last-saved time, save-in-progress)
/// lives on `ProjectIoState`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct AutosaveSettings {
    /// Whether periodic autosave is enabled at all.
    pub enabled: bool,
    /// Seconds between autosaves. The trigger is also change-gated, so
    /// this is the *minimum* spacing, not a guaranteed cadence.
    pub interval_secs: u32,
    /// Number of timestamped backups to retain under `backups/`; older
    /// snapshots are pruned past this count.
    pub backup_retention: u32,
}

impl Default for AutosaveSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            interval_secs: 30,
            backup_retention: 10,
        }
    }
}

/// Root persisted settings document. Wrapping each section (rather than
/// persisting [`AutosaveSettings`] at the top level) leaves room for
/// future settings groups without a format migration.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct AppSettings {
    pub autosave: AutosaveSettings,
}

fn settings_file_path() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join(APP_DIR).join(FILE_NAME))
}

/// Load settings from disk, falling back to defaults on any error
/// (missing file, unreadable file, malformed JSON). Never panics and
/// never blocks boot — mirrors `recent::load`.
pub fn load() -> AppSettings {
    let Some(file) = settings_file_path() else {
        return AppSettings::default();
    };
    let bytes = match std::fs::read(&file) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return AppSettings::default(),
        Err(e) => {
            eprintln!("settings.json read failed: {e}");
            return AppSettings::default();
        }
    };
    match serde_json::from_slice::<AppSettings>(&bytes) {
        Ok(settings) => settings,
        Err(e) => {
            eprintln!("settings.json parse failed: {e}");
            AppSettings::default()
        }
    }
}

/// Persist `settings` to disk as pretty JSON, creating the parent
/// directory if needed. All I/O errors are swallowed (logged), so a
/// failed write never disrupts the session — mirrors `recent::persist`.
pub fn persist(settings: &AppSettings) {
    let Some(file) = settings_file_path() else {
        return;
    };
    if let Some(parent) = file.parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            eprintln!("settings.json mkdir failed: {e}");
            return;
        }
    }
    match serde_json::to_vec_pretty(settings) {
        Ok(bytes) => {
            if let Err(e) = std::fs::write(&file, bytes) {
                eprintln!("settings.json write failed: {e}");
            }
        }
        Err(e) => eprintln!("settings.json serialize failed: {e}"),
    }
}
