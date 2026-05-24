//! Kit browser / loader helpers.
//!
//! Provides the shared "load a kit" code path used by:
//!   • The Load kit button on the left-panel kit card (`load_kit_clicked`).
//!   • The KIT preset pill in the tab bar (`load_installed_kit`).
//!
//! Drives the loader thread via [`crate::kit_loader::spawn_loader`]. The
//! actual UI for selecting / loading is rendered by `pad_grid` and
//! `chrome`; this module exposes the imperative actions and the kit-status
//! formatter so they stay in one place.

use std::sync::atomic::Ordering;

use resonance_common::registry::{self, ContentType, InstalledItem};

use crate::kit_loader::{self, KitStatus};
use crate::KitBridge;

/// Find the `drum_samples.json` manifest inside a kit directory. The
/// downloaded kits have a nested subdirectory, so we search one level
/// deep as well as the root.
fn find_manifest(kit_dir: &std::path::Path) -> Option<std::path::PathBuf> {
    let direct = kit_dir.join("drum_samples.json");
    if direct.exists() {
        return Some(direct);
    }
    // Search one level of subdirectories.
    if let Ok(entries) = std::fs::read_dir(kit_dir) {
        for entry in entries.flatten() {
            if entry.path().is_dir() {
                let nested = entry.path().join("drum_samples.json");
                if nested.exists() {
                    return Some(nested);
                }
            }
        }
    }
    None
}

/// Load a kit from an installed registry entry.
pub(super) fn load_installed_kit(bridge: &KitBridge, item: &InstalledItem) {
    let kit_dir = std::path::PathBuf::from(&item.path);
    let Some(manifest_path) = find_manifest(&kit_dir) else {
        *bridge.kit_status.lock() = KitStatus::Error {
            message: format!("no drum_samples.json found in {}", kit_dir.display()),
        };
        return;
    };
    let sr_bits = bridge.sample_rate.load(Ordering::Acquire);
    if sr_bits == 0 {
        *bridge.kit_status.lock() = KitStatus::Error {
            message: "plugin not yet activated by host".to_string(),
        };
        return;
    }
    let target_sr = f32::from_bits(sr_bits);
    let overhead_key = bridge.overhead_setup_key.lock().clone();
    let choices = bridge.pad_choices.lock().clone();
    let articulations = *bridge.articulations.lock();
    kit_loader::spawn_loader(
        manifest_path,
        target_sr,
        bridge,
        overhead_key,
        choices,
        articulations,
    );
}

pub(super) fn load_kit_clicked(bridge: &KitBridge) {
    // Sync rfd dialog on the editor thread. Blocks briefly while the
    // native file picker is up; the loader thread then does all the
    // heavy work off the UI thread.
    let picked = rfd::FileDialog::new()
        .add_filter("Drum kit manifest", &["json"])
        .pick_file();
    let Some(path) = picked else { return };

    // Refuse to spawn a loader before the host has activated the
    // plugin — without a sample rate we'd decode at the wrong pitch.
    let sr_bits = bridge.sample_rate.load(Ordering::Acquire);
    if sr_bits == 0 {
        *bridge.kit_status.lock() = KitStatus::Error {
            message: "plugin not yet activated by host".to_string(),
        };
        return;
    }
    let target_sr = f32::from_bits(sr_bits);
    let overhead_key = bridge.overhead_setup_key.lock().clone();
    let choices = bridge.pad_choices.lock().clone();
    let articulations = *bridge.articulations.lock();
    kit_loader::spawn_loader(
        path,
        target_sr,
        bridge,
        overhead_key,
        choices,
        articulations,
    );
}

pub(super) fn format_kit_status(status: &KitStatus) -> String {
    match status {
        KitStatus::Empty => "Defaults (no kit loaded)".to_string(),
        KitStatus::Loading { path } => format!(
            "Loading {}...",
            path.file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| "kit".to_string())
        ),
        KitStatus::Loaded { name, num_pads } => {
            format!("Kit: {name} ({num_pads} pads)")
        }
        KitStatus::Error { message } => {
            let short: String = message.chars().take(80).collect();
            format!("Error: {short}")
        }
    }
}

/// Refresh the installed-kits cache from the registry.
pub fn refresh_installed_kits() -> Vec<InstalledItem> {
    registry::list_installed(&ContentType::Drumkit)
}
