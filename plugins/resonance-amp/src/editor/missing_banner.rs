//! The missing-model banner (nam-model-library.md §6.4): over the scope and
//! curve area, which have nothing to draw with no model.
//!
//! ```text
//! ⚠ Missing model: "Friedman BE-100 · standard"
//!   was /home/…/tone3000/Friedman_BE100_(standard)_48121.nam — the amp is passing the signal through clean.
//!   [Re-download from Tone3000]   [Locate file…]   [Choose another model]
//! ```

use std::path::PathBuf;

use plugin_gui_core::egui;
use resonance_common::nam_library::{self, ImportOutcome};

use super::jobs::JobDone;
use super::{actions, theme, AmpEditorApp};
use crate::model_ref::{ModelState, ModelStatus};

/// Banner-local state: a located file whose bytes differ from the saved
/// model, waiting for "Use this file anyway?".
#[derive(Default)]
pub(crate) struct MissingBannerState {
    pub(crate) mismatch: Option<PathBuf>,
    pub(crate) error: Option<String>,
}

/// What Locate should do with a picked file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LocateDecision {
    /// Same bytes as the saved model (or nothing to compare): relink.
    Relink,
    /// Different bytes: ask first.
    AskFirst,
}

/// Compare a located file's content id with the saved one.
pub fn locate_decision(saved_id: Option<&str>, located_id: Option<&str>) -> LocateDecision {
    match (saved_id, located_id) {
        (Some(want), Some(have)) if want != have => LocateDecision::AskFirst,
        _ => LocateDecision::Relink,
    }
}

pub(crate) fn draw(ui: &mut egui::Ui, rect: egui::Rect, app: &mut AmpEditorApp, status: &ModelStatus) {
    let ModelState::Missing {
        name,
        path,
        file_changed,
        source,
        ..
    } = &status.state
    else {
        return;
    };
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect.shrink(12.0)));
    egui::Frame::new()
        .fill(theme::PANEL)
        .stroke(egui::Stroke::new(1.0, theme::WARN))
        .corner_radius(6.0)
        .inner_margin(egui::Margin::same(14))
        .show(&mut child, |ui| {
            ui.set_width(ui.available_width());
            ui.label(
                egui::RichText::new(format!("⚠ Missing model: \"{name}\""))
                    .color(theme::WARN)
                    .size(14.0)
                    .strong(),
            );
            let why = if *file_changed {
                "the file there now holds a different model"
            } else {
                "the file is gone"
            };
            ui.label(
                egui::RichText::new(format!(
                    "was {path} — {why}; the amp is passing the signal through clean."
                ))
                .color(theme::TEXT_DIM)
                .size(11.0),
            );
            ui.add_space(8.0);
            if let Some(located) = app.missing.mismatch.clone() {
                ui.horizontal(|ui| {
                    ui.label(
                        egui::RichText::new(format!(
                            "{} is a different model than the one this project was saved with. Use this file anyway?",
                            located.display()
                        ))
                        .color(theme::WARN)
                        .size(11.0),
                    );
                });
                ui.horizontal(|ui| {
                    if ui.button("Use it").clicked() {
                        app.missing.mismatch = None;
                        relink(app, &located);
                    }
                    if ui.button("Cancel").clicked() {
                        app.missing.mismatch = None;
                    }
                });
            } else {
                ui.horizontal(|ui| {
                    if let Some(nam_library::Source::Tone3000 { tone_id, model_id }) = source {
                        if actions::tone3000_connected(app) {
                            if ui.button("Re-download from Tone3000").clicked() {
                                actions::redownload_and_load(app, *tone_id, *model_id, Some(name.clone()));
                                app.missing.error = None;
                            }
                        } else if ui.button("Connect… to re-download").clicked() {
                            app.tone3000
                                .send(crate::tone3000::worker::Command::Authenticate);
                        }
                    }
                    if *file_changed && ui.button("Use the file at this path").clicked() {
                        relink(app, &PathBuf::from(path));
                    }
                    if ui
                        .add_enabled(!app.jobs.busy(), egui::Button::new("Locate file…"))
                        .clicked()
                    {
                        if let Some(located) = actions::pick_nam_files(false).into_iter().next() {
                            // Hashed on the job thread; the frame decides
                            // when it reports back (`app.rs`).
                            app.jobs.start("checking the file…", move || {
                                let id = nam_library::hash_file(&located).ok();
                                JobDone::Located(located, id)
                            });
                        }
                    }
                    if ui.button("Choose another model").clicked() {
                        app.open_library();
                    }
                });
            }
            if let Some(err) = &app.missing.error {
                ui.label(egui::RichText::new(err).color(theme::DANGER).size(11.0));
            }
        });
}

/// Bring `file` into the library (a copy unless it is already inside the
/// root) and load it, as a job.
pub(crate) fn relink(app: &mut AmpEditorApp, file: &std::path::Path) {
    let library = app.params.library.clone();
    let file = file.to_path_buf();
    let inside = library.root().is_some_and(|r| file.starts_with(r));
    let started = app.jobs.start("relinking…", move || {
        JobDone::Relinked(if inside {
            library
                .rescan()
                .map_err(|e| e.to_string())
                .and_then(|_| {
                    library
                        .read()
                        .by_path(&file)
                        .cloned()
                        .map(|e| (e, false))
                        .ok_or_else(|| format!("{} is not a readable model", file.display()))
                })
        } else {
            library
                .mutate(|lib| lib.import(&file))
                .map(|o| {
                    let already = matches!(o, ImportOutcome::AlreadyPresent(_));
                    (o.entry().clone(), already)
                })
                .map_err(|e| e.to_string())
        })
    });
    if !started {
        app.missing.error = Some("the library is busy; try again in a moment".into());
    }
}
