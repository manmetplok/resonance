//! What the editor's buttons do to the instance and the library: every
//! model pick goes through `file_select` (so it is automatable, undoable in
//! the host and visible over MCP) and the loader thread.

use std::path::Path;
use std::sync::atomic::Ordering;
use std::sync::Arc;

use resonance_common::nam_library::{Entry, ImportOutcome};

use super::AmpEditorApp;
use crate::tone3000::worker::DownloadDone;

/// Point `file_select` at `slot` and ask the loader for it.
pub(crate) fn load_slot(app: &AmpEditorApp, slot: u32) {
    app.params.file_select.set_value(slot as i32);
    app.load_request.store(slot as i32, Ordering::Release);
}

/// Load a library entry. `Err` for an entry without a slot (a duplicate,
/// or past the 1000th model).
pub(crate) fn load_entry(app: &AmpEditorApp, entry: &Entry) -> Result<(), String> {
    match entry.slot {
        Some(slot) => {
            load_slot(app, slot);
            Ok(())
        }
        None => Err(format!("\"{}\" has no slot to load it through", entry.name)),
    }
}

/// Import `path` into the library (a copy, deduplicated by content) and
/// load it. Returns what the import did.
pub(crate) fn import_and_load(app: &AmpEditorApp, path: &Path) -> Result<ImportOutcome, String> {
    let outcome = app
        .params
        .library
        .mutate(|lib| lib.import(path))
        .map_err(|e| e.to_string())?;
    load_entry(app, outcome.entry())?;
    Ok(outcome)
}

/// The callback a Tone3000 download from this editor finishes with: load
/// the new entry into this instance.
pub(crate) fn download_done(app: &AmpEditorApp) -> DownloadDone {
    let params = app.params.clone();
    let load_request = app.load_request.clone();
    Arc::new(move |entry: &Entry| {
        if let Some(slot) = entry.slot {
            params.file_select.set_value(slot as i32);
            load_request.store(slot as i32, Ordering::Release);
        }
    })
}

/// A modal file dialog for `.nam` files. Sync on the UI thread — the
/// Wayland runtime's editor thread, or the AppKit main thread under the
/// Cocoa runtime, where a modal panel is the supported path and the
/// runtime's reentrancy guard skips nested paints (macos-editor-plan.md
/// §3h).
pub(crate) fn pick_nam_files(multiple: bool) -> Vec<std::path::PathBuf> {
    let dialog = rfd::FileDialog::new().add_filter("NAM model", &["nam"]);
    if multiple {
        dialog.pick_files().unwrap_or_default()
    } else {
        dialog.pick_file().into_iter().collect()
    }
}
