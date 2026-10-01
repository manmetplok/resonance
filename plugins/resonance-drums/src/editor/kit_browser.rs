//! Kit browser / loader helpers.
//!
//! Provides the shared "load a kit" code path used by:
//!   • The header's "Open kit file…" button (`load_kit_clicked`).
//!   • The KIT pill's ◀ / ▶ and dropdown (`load_installed_kit`).
//!
//! Drives the loader thread via [`crate::kit_loader::spawn_loader`]. The
//! UI for selecting / loading is rendered by `chrome`; this module exposes
//! the imperative actions and the kit-status formatter so they stay in
//! one place.

use std::path::PathBuf;
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

/// A kit load the editor asked for: the manifest it is loading, and the
/// load generation it was given.
///
/// The bridge's `kit_path` is written only when a load *succeeds*, and
/// `KitStatus::Loading` only once the loader thread gets going, so for a
/// while after a click neither says which kit is on its way. The kit
/// pill steps ◀/▶ from this in that window ([`kit_path_for_stepping`]),
/// rather than from the kit being replaced.
#[derive(Clone, Debug)]
pub(crate) struct RequestedKit {
    pub(crate) path: PathBuf,
    pub(crate) generation: u64,
}

/// The kit the editor should treat as current when stepping through the
/// installed kits: the one being loaded, if any, else the one loaded.
///
/// In order: the manifest a `Loading` status names; else the editor's own
/// last request, while it is still the newest load and has not failed;
/// else the last kit that loaded successfully.
pub(crate) fn kit_path_for_stepping(
    bridge: &KitBridge,
    requested: Option<&RequestedKit>,
) -> Option<PathBuf> {
    let status_says = match &*bridge.kit_status.lock() {
        KitStatus::Loading { path } => Some(Some(path.clone())),
        KitStatus::Error { .. } => Some(None),
        _ => None,
    };
    match status_says {
        Some(Some(loading)) => return Some(loading),
        // A failed load leaves the last good kit in place.
        Some(None) => {}
        None => {
            if let Some(req) = requested {
                if bridge.load_generation.load(Ordering::Acquire) == req.generation {
                    return Some(req.path.clone());
                }
            }
        }
    }
    bridge.kit_path.lock().clone()
}

/// Start the loader on `manifest_path` and say what was asked for.
fn spawn(bridge: &KitBridge, manifest_path: PathBuf) -> Option<RequestedKit> {
    // Refuse to spawn a loader before the host has activated the
    // plugin — without a sample rate we'd decode at the wrong pitch.
    let sr_bits = bridge.sample_rate.load(Ordering::Acquire);
    if sr_bits == 0 {
        *bridge.kit_status.lock() = KitStatus::Error {
            message: "plugin not yet activated by host".to_string(),
        };
        return None;
    }
    let target_sr = f32::from_bits(sr_bits);
    let overhead_key = bridge.overhead_setup_key.lock().clone();
    let choices = bridge.pad_choices.lock().clone();
    let articulations = bridge.articulations();
    kit_loader::spawn_loader(
        manifest_path.clone(),
        target_sr,
        bridge,
        overhead_key,
        choices,
        articulations,
    );
    Some(RequestedKit {
        path: manifest_path,
        generation: bridge.load_generation.load(Ordering::Acquire),
    })
}

/// Load a kit from an installed registry entry.
pub(super) fn load_installed_kit(bridge: &KitBridge, item: &InstalledItem) -> Option<RequestedKit> {
    let kit_dir = PathBuf::from(&item.path);
    let Some(manifest_path) = find_manifest(&kit_dir) else {
        *bridge.kit_status.lock() = KitStatus::Error {
            message: format!("no drum_samples.json found in {}", kit_dir.display()),
        };
        return None;
    };
    spawn(bridge, manifest_path)
}

pub(super) fn load_kit_clicked(bridge: &KitBridge) -> Option<RequestedKit> {
    // Sync rfd dialog on the UI thread — the Wayland runtime's editor
    // thread, or the AppKit main thread under the Cocoa runtime, where a
    // modal panel is the supported path and the runtime's reentrancy
    // guard skips nested paints (macos-editor-plan.md §3h). Blocks
    // briefly while the native file picker is up; the loader thread then
    // does all the heavy work off the UI thread.
    let picked = rfd::FileDialog::new()
        .add_filter("Drum kit manifest", &["json"])
        .pick_file();
    let path = picked?;
    spawn(bridge, path)
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
