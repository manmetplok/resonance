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

use resonance_common::{atomic_write, quarantine_corrupt};
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

/// Media-browser user state that is *project-independent* and therefore
/// belongs in the app's settings rather than any project file (doc #175):
/// the favourite folders pinned to the top of the browser and the
/// recently-visited folders shelf. Persisted here so they survive across
/// sessions and follow the user from one project to the next. The live
/// working copies are held on `state::MediaPool`; this is the durable
/// store synced to/from it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct MediaBrowserSettings {
    /// User-pinned favourite folder paths, in pin order.
    pub favourites: Vec<PathBuf>,
    /// Recently-visited folder paths, most-recent first (the pool caps
    /// this list when pushing; persisted verbatim).
    pub recent_folders: Vec<PathBuf>,
}

/// Arrange-view preferences.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ArrangeSettings {
    /// Page the arrange view along with the playhead during playback
    /// (review FU-D1). Switchable in Settings (code review FU-V3b).
    pub follow_playhead: bool,
}

impl Default for ArrangeSettings {
    fn default() -> Self {
        Self {
            follow_playhead: true,
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
    /// Media-browser favourites + recent folders (doc #175). Absent on
    /// settings files written before this section existed; `#[serde(default)]`
    /// fills it with empty lists.
    pub media: MediaBrowserSettings,
    /// Arrange-view preferences; absent on older files, defaulted.
    pub arrange: ArrangeSettings,
    /// Audio-engine startup options; absent on older files, defaulted.
    pub audio: AudioSettings,
}

/// Audio-engine options read once at startup, before the engine exists.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct AudioSettings {
    /// Threads the live mixer renders tracks on, the audio thread
    /// included; 1 = one thread, as before multithreading. `None` lets
    /// the engine pick (physical cores − 1 workers plus the audio
    /// thread). The `RESONANCE_RENDER_THREADS` environment variable wins
    /// over this. Takes effect on the next launch.
    pub render_threads: Option<usize>,
}

fn settings_file_path() -> Option<PathBuf> {
    crate::user_dirs::config_dir().map(|d| d.join(APP_DIR).join(FILE_NAME))
}

/// Load settings from disk, falling back to defaults on any error
/// (missing file, unreadable file, malformed JSON). Never panics and
/// never blocks boot — mirrors `recent::load`.
pub fn load() -> AppSettings {
    let Some(file) = settings_file_path() else {
        return AppSettings::default();
    };
    load_from(&file)
}

/// Load from a specific path (useful for testing, mirroring
/// `registry::load_registry_from` / `midi_map::load_controller_maps_from`).
pub fn load_from(file: &std::path::Path) -> AppSettings {
    let bytes = match std::fs::read(file) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return AppSettings::default(),
        Err(e) => {
            tracing::warn!("settings.json read failed: {e}");
            return AppSettings::default();
        }
    };
    match serde_json::from_slice::<AppSettings>(&bytes) {
        Ok(settings) => settings,
        Err(e) => {
            tracing::warn!(
                "settings.json parse failed: {e}; quarantining {} and starting from defaults",
                file.display()
            );
            quarantine_corrupt(file);
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
    persist_to(&file, settings);
}

/// Persist to a specific path (useful for testing).
pub fn persist_to(file: &std::path::Path, settings: &AppSettings) {
    if let Some(parent) = file.parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            tracing::warn!("settings.json mkdir failed: {e}");
            return;
        }
    }
    match serde_json::to_vec_pretty(settings) {
        Ok(bytes) => {
            if let Err(e) = atomic_write(file, &bytes) {
                tracing::warn!("settings.json write failed: {e}");
            }
        }
        Err(e) => tracing::warn!("settings.json serialize failed: {e}"),
    }
}
