//! What the editor's buttons do to the instance and the library: every
//! model pick goes through `file_select` (so it is automatable, undoable in
//! the host and visible over MCP) and the loader thread. Anything that
//! hashes, copies or waits on the library's writer lock runs as a
//! [`Jobs`](super::jobs::Jobs) job, off the editor thread.

use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::{Duration, Instant};

use resonance_common::nam_library::{Entry, ImportOutcome};

use super::jobs::JobDone;
use super::AmpEditorApp;
use crate::tone3000::worker::DownloadDone;

/// How long a re-download notice stays in the header when nothing else
/// replaces it.
pub(crate) const NOTICE_TTL: Duration = Duration::from_secs(12);

/// Whether a load is a user **pick** (a Library row, a Tone3000 Load, an
/// import) — which counts as a use for Recent — or **browsing** with ◀/▶,
/// which does not: recording it would re-sort a "Recently used" view under
/// the stepping and bounce between two models.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LoadKind {
    Pick,
    Browse,
}

/// Point `file_select` at `slot` and ask the loader for it. A pick also
/// records a use (D10: a project-open restore never does) and clears a
/// stale re-download notice.
pub(crate) fn load_slot(app: &AmpEditorApp, slot: u32, kind: LoadKind) {
    let changed = app.params.file_select.value() != slot as i32;
    app.params.file_select.set_value(slot as i32);
    app.load_request.store(slot as i32, Ordering::Release);
    // The user's pick: one undoable edit for the host (PUX-01).
    if let (true, Some(a)) = (changed, &app.announcer) {
        a.announce(resonance_plugin::Param::id(&app.params.file_select));
    }
    *app.redownload_notice.lock() = None;
    if kind == LoadKind::Pick {
        let id = app.params.library.read().by_slot(slot).map(|e| e.id.clone());
        if let Some(id) = id {
            if let Err(e) = app.params.library.record_use(&id) {
                tracing::warn!("could not record the model pick: {e}");
            }
        }
    }
}

/// Load a library entry (a pick). `Err` for an entry without a slot (a
/// duplicate, or past the 1000th model).
pub(crate) fn load_entry(app: &AmpEditorApp, entry: &Entry) -> Result<(), String> {
    match entry.slot {
        Some(slot) => {
            load_slot(app, slot, LoadKind::Pick);
            Ok(())
        }
        None => Err(format!("\"{}\" has no slot to load it through", entry.name)),
    }
}

/// Start importing `files` (copied into the library, deduplicated by
/// content) off the editor thread; a single file is also loaded when the
/// job reports back.
pub(crate) fn start_import(app: &mut AmpEditorApp, files: Vec<PathBuf>) {
    if files.is_empty() {
        return;
    }
    let library = app.params.library.clone();
    let load_single = files.len() == 1;
    let started = app.jobs.start("importing…", move || JobDone::Imported {
        results: files
            .iter()
            .map(|f| library.mutate(|lib| lib.import(f)).map_err(|e| e.to_string()))
            .collect(),
        load_single,
    });
    if !started {
        app.browser.set_error("the library is busy; try again in a moment");
    }
}

/// The callback a Tone3000 download from this editor finishes with (on the
/// worker thread): remember it as this editor's own download, and load the
/// new entry into this instance.
pub(crate) fn download_done(app: &AmpEditorApp) -> DownloadDone {
    let params = app.params.clone();
    let load_request = app.load_request.clone();
    let mine = app.my_download.clone();
    let announcer = app.announcer.clone();
    Arc::new(move |entry: &Entry| {
        *mine.lock() = Some(entry.path.clone());
        if let Some(slot) = entry.slot {
            params.file_select.set_value(slot as i32);
            load_request.store(slot as i32, Ordering::Release);
            let _ = params.library.record_use(&entry.id);
            if let Some(a) = &announcer {
                a.announce(resonance_plugin::Param::id(&params.file_select));
            }
        }
    })
}

/// Re-download a Tone3000 model into this instance (the missing banner):
/// when the new bytes are not the model the instance's reference was saved
/// with (the author re-uploaded), the header says so once it lands.
pub(crate) fn redownload_and_load(
    app: &AmpEditorApp,
    tone_id: i64,
    model_id: i64,
    title_hint: Option<String>,
) {
    let load = download_done(app);
    let saved_id = app.params.model_ref.lock().id.clone();
    let notice = app.redownload_notice.clone();
    let done: DownloadDone = Arc::new(move |entry: &Entry| {
        if saved_id.as_deref().is_some_and(|s| s != entry.id) {
            *notice.lock() = Some((
                "re-downloaded model differs from the one this project was saved with".into(),
                Instant::now(),
            ));
        }
        load(entry);
    });
    app.tone3000.send(crate::tone3000::worker::Command::Redownload {
        tone_id,
        model_id,
        title_hint,
        done,
    });
}

/// Re-download a Tone3000 model from the detail pane: refresh the file in
/// the library WITHOUT loading it into this amp (the user was looking at a
/// row, not asking to switch), and compare against that entry's id.
pub(crate) fn redownload_only(app: &AmpEditorApp, entry: &Entry, tone_id: i64, model_id: i64) {
    let done = redownload_only_done(app, entry);
    app.tone3000.send(crate::tone3000::worker::Command::Redownload {
        tone_id,
        model_id,
        title_hint: Some(entry.name.clone()),
        done,
    });
}

/// The callback a detail-pane re-download of `entry` finishes with: a
/// notice comparing against THAT entry's id, and no load.
pub(crate) fn redownload_only_done(app: &AmpEditorApp, entry: &Entry) -> DownloadDone {
    let installed_id = entry.id.clone();
    let name = entry.name.clone();
    let notice = app.redownload_notice.clone();
    let mine = app.my_download.clone();
    Arc::new(move |fresh: &Entry| {
        *mine.lock() = Some(fresh.path.clone());
        let text = if fresh.id == installed_id {
            format!("re-downloaded \"{name}\" (unchanged)")
        } else {
            format!("re-downloaded \"{name}\": the file differs from the one installed before")
        };
        *notice.lock() = Some((text, Instant::now()));
    })
}

/// Whether the Tone3000 worker has a session (re-download needs one).
pub(crate) fn tone3000_connected(app: &AmpEditorApp) -> bool {
    !matches!(
        app.tone3000.state.lock().status,
        crate::tone3000::worker::Status::Disconnected | crate::tone3000::worker::Status::Error(_)
    )
}

/// The re-download notice, if it has not expired.
pub(crate) fn live_notice(app: &AmpEditorApp) -> Option<String> {
    let mut n = app.redownload_notice.lock();
    if n.as_ref().is_some_and(|(_, at)| at.elapsed() > NOTICE_TTL) {
        *n = None;
    }
    n.as_ref().map(|(t, _)| t.clone())
}

/// What a finished import reports: the entry to select, and a message.
pub(crate) fn import_summary(results: &[Result<ImportOutcome, String>]) -> (Option<Entry>, Result<String, String>) {
    let mut added = 0usize;
    let mut last = None;
    let mut errors = Vec::new();
    for r in results {
        match r {
            Ok(o) => {
                if matches!(o, ImportOutcome::Added(_)) {
                    added += 1;
                }
                last = Some(o.entry().clone());
            }
            Err(e) => errors.push(e.clone()),
        }
    }
    let message = if !errors.is_empty() {
        Err(errors.join("; "))
    } else if results.len() == 1 && added == 0 {
        Ok("already in library".to_string())
    } else {
        Ok(format!("imported {added} of {}", results.len()))
    };
    (last, message)
}

/// A modal file dialog for `.nam` files. On Cocoa this stays a direct,
/// synchronous `rfd` call — it runs inside the guarded AppKit modal run
/// loop (macos-editor-plan.md §3h). On Linux, PUX-07: see
/// [`start_nam_picker`]/[`poll_nam_picker`] below instead — a modal
/// dialog call inside `ui()` would block the Wayland editor thread (no
/// repaint, no Wayland dispatch) for as long as it is up.
#[cfg(target_os = "macos")]
pub(crate) fn pick_nam_files(multiple: bool) -> Vec<PathBuf> {
    let dialog = rfd::FileDialog::new().add_filter("NAM model", &["nam"]);
    if multiple {
        dialog.pick_files().unwrap_or_default()
    } else {
        dialog.pick_file().into_iter().collect()
    }
}

/// Non-macOS: start a `.nam` file dialog on its own thread, kept alive
/// across frames at `id` until [`poll_nam_picker`] collects the answer.
/// `id` distinguishes the editor's two pickers (`Import .nam…` picks
/// many; the missing-model banner's `Locate file…` picks one) so
/// either can be in flight without disturbing the other.
#[cfg(not(target_os = "macos"))]
pub(crate) fn start_nam_picker(ctx: &plugin_gui_core::egui::Context, id: plugin_gui_core::egui::Id, multiple: bool) {
    use resonance_plugin::file_picker::{CtxPicker, FileDialogRequest};
    let request = if multiple {
        FileDialogRequest::open_files()
    } else {
        FileDialogRequest::open_file()
    }
    .title("Import a NAM model")
    .filter("NAM model", &["nam"]);
    CtxPicker::<()>::get(ctx, id).start(request, ());
}

/// The picker at `id`'s answer, once its dialog has closed. `None` means
/// still open or dismissed-and-not-yet-polled; an empty `Vec` means the
/// user dismissed it without picking anything.
#[cfg(not(target_os = "macos"))]
pub(crate) fn poll_nam_picker(
    ctx: &plugin_gui_core::egui::Context,
    id: plugin_gui_core::egui::Id,
) -> Option<Vec<PathBuf>> {
    use resonance_plugin::file_picker::CtxPicker;
    let (answer, ()) = CtxPicker::<()>::get(ctx, id).poll()?;
    Some(answer.into_many())
}

#[cfg(not(target_os = "macos"))]
pub(crate) fn import_nam_picker_id() -> plugin_gui_core::egui::Id {
    plugin_gui_core::egui::Id::new("amp_import_nam_picker")
}

#[cfg(not(target_os = "macos"))]
pub(crate) fn locate_nam_picker_id() -> plugin_gui_core::egui::Id {
    plugin_gui_core::egui::Id::new("amp_locate_nam_picker")
}
