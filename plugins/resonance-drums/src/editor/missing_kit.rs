//! The missing-kit banner (drums-plugin-rework.md §5.3), over the pad
//! area while the kit the project asked for is not on this machine:
//!
//! ```text
//! ⚠ Missing kit "Drummica" — playing the built-in kit.
//!   [Download from plok.org]   [Locate folder…]   [Choose another kit]
//! ```
//!
//! - **Download** is offered only when the plok.org index (the worker's,
//!   else the copy cached on disk) has the kit — by its manifest hash, or
//!   by name. The download lands in the library and loads.
//! - **Locate folder…** opens a folder dialog (on the picker's thread, never
//!   this one). A folder whose manifest hashes to the saved id relinks
//!   silently; another kit asks "Use this kit anyway?" first. Either way
//!   the folder goes through the library's import — a copy into the root
//!   (D2) unless it is already inside it — and the kit loads from there.
//! - **Choose another kit** opens the Library.

use std::path::PathBuf;

use plugin_gui_core::egui;
use resonance_common::drumkit_library;

use crate::download::{self, Command, ServerKit};
use crate::selection::KitRef;

use super::app::DrumsEditorApp;
use super::jobs::{JobDone, JobKind, PickKind};
use super::kit_browser::LoadKind;
use super::{library_panel, probe, theme};

/// The banner's own state.
#[derive(Default)]
pub(crate) struct MissingKitState {
    /// The picker is up for `Locate folder…` (not for an import).
    pub(crate) locating: bool,
    /// A located folder holding another kit than the missing one, waiting
    /// for "Use this kit anyway?".
    pub(crate) mismatch: Option<PathBuf>,
    /// An import started by Locate: whatever it resolves to is loaded,
    /// an already-present kit included.
    pub(crate) relinking: bool,
    /// The index name of a download started from the banner, loaded once
    /// it lands.
    pub(crate) downloading: Option<String>,
    /// The last thing Locate or Download could not do.
    pub(crate) error: Option<String>,
    /// [`download_candidate`]'s last answer, and what it was for — the
    /// banner draws every frame, and the index (cloned, or read from disk
    /// and parsed) is looked up again only when the question changes.
    candidate: Option<(CandidateKey, Option<ServerKit>)>,
}

/// What a download candidate depends on: the missing reference, and which
/// index the worker holds (when it was fetched, and how many kits it
/// lists — an index put in place without a fetch changes that). With no
/// index in the worker, the copy cached on disk: written only by a fetch,
/// which puts one in the worker too.
#[derive(Clone, Debug, PartialEq)]
struct CandidateKey {
    missing: KitRef,
    index: Option<(Option<std::time::Instant>, usize)>,
}

/// The plok.org index entry the missing kit can be downloaded as: its
/// manifest hash, else its name, in the worker's index or the one cached
/// on disk. Cached ([`MissingKitState::candidate`]).
pub(crate) fn download_candidate(app: &mut DrumsEditorApp, missing: &KitRef) -> Option<ServerKit> {
    let key = {
        let state = app.library.download().state.lock();
        CandidateKey {
            missing: missing.clone(),
            index: state
                .index
                .as_ref()
                .map(|i| (state.index_fetched_at, i.drumkits.len())),
        }
    };
    if let Some((cached, found)) = &app.missing_kit.candidate {
        if *cached == key {
            return found.clone();
        }
    }
    let found = look_up_candidate(app, missing);
    app.missing_kit.candidate = Some((key, found.clone()));
    found
}

fn look_up_candidate(app: &DrumsEditorApp, missing: &KitRef) -> Option<ServerKit> {
    let index = app
        .library
        .download()
        .state
        .lock()
        .index
        .clone()
        .or_else(|| app.library.root().and_then(download::read_index_cache))?;
    if let Some(id) = &missing.id {
        let by_hash = index.drumkits.iter().find(|k| {
            k.manifest_sha256
                .as_deref()
                .is_some_and(|h| h.trim().eq_ignore_ascii_case(id))
        });
        if let Some(kit) = by_hash {
            return Some(kit.clone());
        }
    }
    index.find(&missing.display_name()).cloned()
}

/// Draw the banner if this instance's kit is missing. Returns whether it
/// was drawn.
pub(crate) fn draw(ui: &mut egui::Ui, app: &mut DrumsEditorApp) -> bool {
    let Some(missing) = app.params.selection.missing() else {
        app.missing_kit.mismatch = None;
        return false;
    };
    let name = missing.display_name();
    let candidate = download_candidate(app, &missing);
    let shown = egui::Frame::new()
        .fill(theme::BG_2)
        .stroke(egui::Stroke::new(1.0, theme::WARN))
        .corner_radius(theme::RADIUS_PANEL)
        .inner_margin(egui::Margin::symmetric(14, 10))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            let title = ui.add(
                egui::Label::new(
                    egui::RichText::new(format!(
                        "⚠ Missing kit \"{name}\" — playing the built-in kit."
                    ))
                    .color(theme::WARN)
                    .size(13.0)
                    .strong(),
                )
                .truncate(),
            );
            probe(ui, "missing.title", title.rect);
            if let Some(path) = missing.abs_path.as_ref().or(missing.rel_path.as_ref()) {
                ui.add(
                    egui::Label::new(
                        egui::RichText::new(format!("was {}", path.display()))
                            .color(theme::TEXT_3)
                            .size(10.5),
                    )
                    .truncate(),
                );
            }
            ui.add_space(6.0);
            match app.missing_kit.mismatch.clone() {
                Some(located) => draw_mismatch(ui, app, &located),
                None => draw_actions(ui, app, candidate),
            }
            if let Some(err) = &app.missing_kit.error {
                ui.add(
                    egui::Label::new(egui::RichText::new(err).color(theme::BAD).size(10.5))
                        .truncate(),
                );
            }
        });
    probe(ui, "missing.banner", shown.response.rect);
    true
}

fn draw_actions(ui: &mut egui::Ui, app: &mut DrumsEditorApp, candidate: Option<ServerKit>) {
    ui.horizontal(|ui| {
        if let Some(kit) = candidate {
            let busy = app.library.download().state.lock().is_working_on(&kit.name);
            let label = if busy {
                "Downloading…"
            } else {
                "Download from plok.org"
            };
            let b = ui.add_enabled(!busy, egui::Button::new(label));
            probe(ui, "missing.download", b.rect);
            if b.clicked() {
                app.missing_kit.error = None;
                app.missing_kit.downloading = Some(kit.name.clone());
                app.my_downloads.insert(kit.name.clone());
                app.library.download().send(Command::Download(kit));
            }
        }
        let can_locate = !app.picker.busy() && !app.jobs.writing();
        let b = ui.add_enabled(can_locate, egui::Button::new("Locate folder…"));
        probe(ui, "missing.locate", b.rect);
        if b.clicked() && app.picker.start(PickKind::MissingKitFolder) {
            app.missing_kit.error = None;
            app.missing_kit.locating = true;
        }
        let b = ui.button("Choose another kit");
        probe(ui, "missing.choose", b.rect);
        if b.clicked() {
            app.open_library();
        }
    });
}

fn draw_mismatch(ui: &mut egui::Ui, app: &mut DrumsEditorApp, located: &std::path::Path) {
    ui.add(
        egui::Label::new(
            egui::RichText::new(format!(
                "{} holds a different kit than the one this project was saved with. \
                 Use this kit anyway?",
                located.display()
            ))
            .color(theme::WARN)
            .size(11.0),
        )
        .wrap(),
    );
    ui.horizontal(|ui| {
        let b = ui.button("Use it");
        probe(ui, "missing.use_anyway", b.rect);
        if b.clicked() {
            app.missing_kit.mismatch = None;
            relink(app, located.to_path_buf());
        }
        let b = ui.button("Cancel");
        probe(ui, "missing.cancel", b.rect);
        if b.clicked() {
            app.missing_kit.mismatch = None;
        }
    });
}

/// The picker closed on a `Locate folder…`: hash the folder's manifest on
/// a job (the frame applies [`located`] when it reports back).
pub(crate) fn picked(app: &mut DrumsEditorApp, dir: PathBuf) {
    let started = app.jobs.start(
        JobKind::Check,
        "checking the kit folder…",
        false,
        move |_| {
            let id = match drumkit_library::find_manifest(&dir) {
                None => Err(format!(
                    "{} holds no drum_samples.json (at its top or one folder down)",
                    dir.display()
                )),
                Some(manifest) => drumkit_library::hash_manifest(&manifest)
                    .map_err(|e| format!("could not read {}: {e}", manifest.display())),
            };
            JobDone::Located { dir, id }
        },
    );
    if !started {
        app.missing_kit.error = Some(format!(
            "could not check the folder: {}",
            library_panel::BUSY_WHY
        ));
    }
}

/// A located folder's manifest id is in: relink when it is the missing
/// kit (or there is no id to compare), else ask first.
pub(crate) fn located(app: &mut DrumsEditorApp, dir: PathBuf, id: Result<String, String>) {
    let id = match id {
        Ok(id) => id,
        Err(e) => {
            app.missing_kit.error = Some(e);
            return;
        }
    };
    let saved = app.params.selection.missing().and_then(|m| m.id);
    match saved {
        Some(want) if !want.eq_ignore_ascii_case(&id) => app.missing_kit.mismatch = Some(dir),
        _ => relink(app, dir),
    }
}

/// Bring `dir` into the library (the import copies it unless it is
/// already inside the root, where its rescan finds it) and load it.
pub(crate) fn relink(app: &mut DrumsEditorApp, dir: PathBuf) {
    app.missing_kit.error = None;
    app.missing_kit.relinking = true;
    library_panel::start_import(app, dir);
}

/// The banner's download landed: load it.
pub(crate) fn installed(app: &mut DrumsEditorApp, name: &str, id: &str) -> bool {
    if app.missing_kit.downloading.as_deref() != Some(name) {
        return false;
    }
    app.missing_kit.downloading = None;
    let entry = app.library.read().entry(id).cloned();
    match entry {
        Some(entry) => {
            app.load_entry(&entry, LoadKind::Pick);
            true
        }
        None => false,
    }
}
