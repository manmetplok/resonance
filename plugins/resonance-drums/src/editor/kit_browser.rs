//! Loading a kit from the editor.
//!
//! [`load_library_kit`] is **the** entry point every kit pick in the editor
//! goes through — the header's kit dropdown and ◀/▶, the Library overlay's
//! Load and double-click, the missing-kit banner's relink. It writes the
//! `kit_select` slot parameter (drums-plugin-rework.md §5.1) and acts on it
//! at once ([`selection::select_now`]): the same path a host or the control
//! API takes, so the pick is the parameter's value and a host can undo it.
//!
//! Also here: which kit the editor treats as current while a load is in
//! flight, and the kit-status formatter.

use std::path::PathBuf;
use std::sync::atomic::Ordering;

use resonance_common::drumkit_library::Entry;

use crate::kit_loader::KitStatus;
use crate::library::SharedKitLibrary;
use crate::selection::{self, StartedLoad};
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
    selection::loadable(entry)?;
    let requested = match entry.slot {
        Some(slot) => match selection::select_now(bridge, slot as i32) {
            Ok(Some(started)) => requested(started),
            // The shared library has no kit in that slot any more (the
            // row is from a snapshot the index has moved on from): load
            // the entry by its manifest.
            _ => spawn(bridge, entry.manifest_path.clone())?,
        },
        // No slot to write (the library holds every kit it lists in one;
        // this is a row the index has not slotted yet): by manifest.
        None => spawn(bridge, entry.manifest_path.clone())?,
    };
    if kind == LoadKind::Pick {
        if let Err(e) = library.record_use(&entry.id) {
            tracing::warn!("could not record the kit pick: {e}");
        }
    }
    Ok(requested)
}

fn requested(started: StartedLoad) -> RequestedKit {
    RequestedKit {
        path: started.path,
        generation: started.generation,
    }
}

/// Load `manifest_path` with no slot to name it by: `kit_select` parks at
/// "none", and the kit is loaded now — or, before the host activated the
/// plugin, at activation.
fn spawn(bridge: &KitBridge, manifest_path: PathBuf) -> Result<RequestedKit, String> {
    Ok(requested(selection::load_unslotted_now(bridge, manifest_path)))
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
