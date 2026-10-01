//! Loading a kit from the editor.
//!
//! [`load_library_kit`] is **the** entry point every kit pick in the editor
//! goes through — the header's kit dropdown and ◀/▶, the Library overlay's
//! Load and double-click. Today it starts the loader thread on the entry's
//! manifest; K4 (drums-plugin-rework.md §5.1) swaps its body for a write of
//! the `kit_select` slot parameter, and nothing else in the editor has to
//! change.
//!
//! Also here: which kit the editor treats as current while a load is in
//! flight, and the kit-status formatter.

use std::path::PathBuf;
use std::sync::atomic::Ordering;

use resonance_common::drumkit_library::{Entry, EntryStatus};

use crate::kit_loader::{self, KitStatus};
use crate::library::SharedKitLibrary;
use crate::KitBridge;

/// Whether a load is a user **pick** (a dropdown row, a Library Load) —
/// which counts as a use for Recent — or **browsing** with ◀/▶, which does
/// not: recording it would re-sort a "Recently used" view under the
/// stepping and bounce between two kits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LoadKind {
    Pick,
    Browse,
}

/// A kit load the editor asked for: the manifest it is loading, and the
/// load generation it was given.
///
/// The bridge's `kit_path` is written only when a load *succeeds*, and
/// `KitStatus::Loading` only once the loader thread gets going, so for a
/// while after a click neither says which kit is on its way. The header
/// steps ◀/▶ from this in that window ([`kit_path_for_stepping`]),
/// rather than from the kit being replaced.
#[derive(Clone, Debug)]
pub(crate) struct RequestedKit {
    pub(crate) path: PathBuf,
    pub(crate) generation: u64,
}

/// The kit the editor should treat as current: the one being loaded, if
/// any, else the one loaded.
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

/// Load library kit `entry` into this instance — the one load entry point
/// (see the module docs). A pick also records a use for Recent.
pub(crate) fn load_library_kit(
    bridge: &KitBridge,
    library: &SharedKitLibrary,
    entry: &Entry,
    kind: LoadKind,
) -> Result<RequestedKit, String> {
    match &entry.status {
        EntryStatus::ManifestError(reason) => {
            return Err(format!("\"{}\" cannot be loaded: {reason}", entry.name))
        }
        EntryStatus::DuplicateOf(dir) => {
            return Err(format!(
                "\"{}\" is a copy of the kit in {}; load that one",
                entry.name,
                dir.display()
            ))
        }
        EntryStatus::Ok | EntryStatus::MissingFiles(_) => {}
    }
    let requested = spawn(bridge, entry.manifest_path.clone())?;
    if kind == LoadKind::Pick {
        if let Err(e) = library.record_use(&entry.id) {
            tracing::warn!("could not record the kit pick: {e}");
        }
    }
    Ok(requested)
}

/// Start the loader on `manifest_path` and say what was asked for.
fn spawn(bridge: &KitBridge, manifest_path: PathBuf) -> Result<RequestedKit, String> {
    // Refuse to spawn a loader before the host has activated the
    // plugin — without a sample rate we'd decode at the wrong pitch.
    let sr_bits = bridge.sample_rate.load(Ordering::Acquire);
    if sr_bits == 0 {
        let message = "plugin not yet activated by host".to_string();
        *bridge.kit_status.lock() = KitStatus::Error {
            message: message.clone(),
        };
        return Err(message);
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
    Ok(RequestedKit {
        path: manifest_path,
        generation: bridge.load_generation.load(Ordering::Acquire),
    })
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
        KitStatus::Loaded { name, num_pads, .. } => {
            format!("Kit: {name} ({num_pads} pads)")
        }
        KitStatus::Error { message } => {
            let short: String = message.chars().take(80).collect();
            format!("Error: {short}")
        }
    }
}
