//! The Library overlay's `plok.org` tab (drums-plugin-rework.md §6.5):
//! one row per kit in the server index — name, size, description, tags,
//! added — and on its right what can be done with it:
//!
//! - `Download`, then a progress bar (bytes, rate, time left) with `Cancel`;
//! - `Installed` and `Load`, when the kit's manifest hash (or, without one,
//!   its name) is in the library;
//! - `Update`, when a kit of that name is installed but the index's
//!   manifest hash differs: it replaces the installed kit in place.
//!
//! The index is fetched each time the tab opens unless the last fetch is
//! under ten minutes old; `Refresh` always fetches. All network work is the
//! shared download worker's (`crate::download`); this file reads its state
//! and posts commands.

use plugin_gui_core::egui;
use resonance_common::drumkit_library::format_bytes;

use super::app::DrumsEditorApp;
use super::kit_browser::LoadKind;
use super::{probe, theme};
use crate::download::{Command, ServerKit, Status};
use crate::library::{plok_row_state, PlokRowState};

/// The tab was opened: fetch the index unless it is fresh.
pub(crate) fn on_tab_open(app: &mut DrumsEditorApp) {
    let stale = app.library.download().state.lock().index_is_stale();
    if stale {
        app.library.download().send(Command::FetchIndex);
    }
}

/// What a row asked for this frame.
enum RowAction {
    Download(ServerKit),
    Update(ServerKit, std::path::PathBuf),
    Cancel(String),
    /// Load the installed kit with this library id.
    Load(String),
}

pub(crate) fn draw_tab(ui: &mut egui::Ui, app: &mut DrumsEditorApp) {
    let snapshot = app.library.download().state.lock().clone();

    ui.horizontal(|ui| {
        let fetching = matches!(snapshot.status, Status::FetchingIndex);
        if ui
            .add_enabled(!fetching, egui::Button::new("Refresh"))
            .clicked()
        {
            app.library.download().send(Command::FetchIndex);
        }
        let line = match (&snapshot.status, &snapshot.index) {
            (Status::FetchingIndex, _) => "Fetching the kit index from plok.org…".to_string(),
            (_, Some(index)) => {
                let n = index.drumkits.len();
                format!("{n} kit{} on plok.org", if n == 1 { "" } else { "s" })
            }
            (_, None) => String::new(),
        };
        ui.label(egui::RichText::new(line).size(11.0).color(theme::TEXT_DIM));
    });
    if let Status::Error(e) = &snapshot.status {
        ui.add(
            egui::Label::new(
                egui::RichText::new(format!("Error: {e}"))
                    .size(11.0)
                    .color(theme::DANGER),
            )
            .wrap(),
        );
    }
    if let Some(n) = app.browser.notice() {
        if !n.is_error() {
            ui.label(
                egui::RichText::new(n.text())
                    .size(11.0)
                    .color(theme::TEXT_DIM),
            );
        }
    }
    ui.add_space(6.0);

    let Some(index) = snapshot.index.clone() else {
        let text = if matches!(snapshot.status, Status::Error(_)) {
            "Could not reach plok.org. Refresh to try again."
        } else {
            "(loading…)"
        };
        ui.label(egui::RichText::new(text).size(11.0).color(theme::TEXT_DIM));
        return;
    };
    if index.drumkits.is_empty() {
        ui.label(
            egui::RichText::new("No kits available on the server.")
                .color(theme::TEXT_DIM)
                .size(11.0),
        );
        return;
    }

    // One library snapshot for every row of the frame.
    let states: Vec<PlokRowState> = {
        let lib = app.library.read();
        index
            .drumkits
            .iter()
            .map(|k| plok_row_state(&lib, k))
            .collect()
    };
    let mut action = None;
    egui::ScrollArea::vertical()
        .id_salt("drums_plok_scroll")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            for (kit, state) in index.drumkits.iter().zip(&states) {
                if let Some(a) = draw_row(ui, kit, state, &snapshot) {
                    action = Some(a);
                }
            }
        });
    match action {
        Some(RowAction::Download(kit)) => {
            app.my_downloads.insert(kit.name.clone());
            app.library.download().send(Command::Download(kit));
        }
        Some(RowAction::Update(kit, dir)) => {
            app.my_downloads.insert(kit.name.clone());
            app.library.download().send(Command::Redownload {
                kit,
                existing_dir: dir,
            });
        }
        Some(RowAction::Cancel(name)) => {
            app.my_downloads.remove(&name);
            app.library.download().send(Command::Cancel(name));
        }
        Some(RowAction::Load(id)) => {
            let entry = app.library.read().entry(&id).cloned();
            match entry {
                Some(entry) => app.load_entry(&entry, LoadKind::Pick),
                None => app
                    .browser
                    .set_error("that kit is no longer in the library"),
            }
        }
        None => {}
    }
}

fn draw_row(
    ui: &mut egui::Ui,
    kit: &ServerKit,
    state: &PlokRowState,
    snapshot: &crate::download::State,
) -> Option<RowAction> {
    let mut action = None;
    let frame = egui::Frame::new()
        .fill(theme::BG_1)
        .stroke(egui::Stroke::new(1.0, theme::BORDER))
        .corner_radius(4.0)
        .inner_margin(egui::Margin::same(8))
        .outer_margin(egui::Margin::symmetric(0, 2));
    frame.show(ui, |ui| {
        ui.set_min_width(ui.available_width());
        ui.horizontal(|ui| {
            // The right-hand side first, at its natural width, so a long
            // description wraps beside it instead of pushing it away.
            let right_w = 230.0_f32.min(ui.available_width() * 0.45);
            let text_w = (ui.available_width() - right_w - 8.0).max(0.0);
            ui.vertical(|ui| {
                ui.set_width(text_w);
                ui.label(
                    egui::RichText::new(&kit.name)
                        .color(theme::TEXT)
                        .size(13.0)
                        .strong(),
                );
                let mut parts = Vec::new();
                if let Some(size) = kit.size_text() {
                    parts.push(size);
                }
                if let Some(n) = kit.pieces {
                    parts.push(format!("{n} pieces"));
                }
                if let Some(n) = kit.mic_setups {
                    parts.push(format!("{n} mic setups"));
                }
                if let Some(added) = &kit.added {
                    parts.push(format!("added {added}"));
                }
                if !parts.is_empty() {
                    ui.label(
                        egui::RichText::new(parts.join(" · "))
                            .color(theme::TEXT_DIM)
                            .size(10.5),
                    );
                }
                if let Some(desc) = kit.description.as_deref().filter(|d| !d.is_empty()) {
                    ui.add(
                        egui::Label::new(egui::RichText::new(desc).color(theme::TEXT_2).size(11.0))
                            .wrap(),
                    );
                }
                if !kit.tags.is_empty() {
                    ui.label(
                        egui::RichText::new(kit.tags.join(" · "))
                            .color(theme::TEXT_3)
                            .size(10.5),
                    );
                }
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.set_width(right_w);
                action = draw_row_action(ui, kit, state, snapshot);
            });
        });
    });
    action
}

/// The right side of a row.
fn draw_row_action(
    ui: &mut egui::Ui,
    kit: &ServerKit,
    state: &PlokRowState,
    snapshot: &crate::download::State,
) -> Option<RowAction> {
    let mut action = None;
    let name = kit.name.as_str();
    if snapshot.status.active_kit() == Some(name) {
        if ui.button("Cancel").clicked() {
            action = Some(RowAction::Cancel(kit.name.clone()));
        }
        let (fraction, text) = progress_of(&snapshot.status);
        let bar = ui.add(
            egui::ProgressBar::new(fraction)
                .desired_width(150.0)
                .text(egui::RichText::new(text).size(10.0)),
        );
        probe(ui, format!("plok.{name}.progress"), bar.rect);
        return action;
    }
    if snapshot.queued.iter().any(|q| q == name) {
        if ui.button("Cancel").clicked() {
            action = Some(RowAction::Cancel(kit.name.clone()));
        }
        let l = ui.label(
            egui::RichText::new("Queued")
                .color(theme::TEXT_DIM)
                .size(11.0),
        );
        probe(ui, format!("plok.{name}.queued"), l.rect);
        return action;
    }
    match state {
        PlokRowState::Installed { id } => {
            // Right to left: Load sits at the row's edge.
            let b = ui
                .button("Load")
                .on_hover_text("Load this kit in this instance");
            probe(ui, format!("plok.{name}.load"), b.rect);
            if b.clicked() {
                action = Some(RowAction::Load(id.clone()));
            }
            let l = ui.label(
                egui::RichText::new("Installed")
                    .color(theme::ACCENT)
                    .size(12.0),
            );
            probe(ui, format!("plok.{name}.installed"), l.rect);
        }
        PlokRowState::Update { dir, .. } => {
            let b = ui
                .button("Update")
                .on_hover_text("A different version of this kit is installed: replace it");
            probe(ui, format!("plok.{name}.update"), b.rect);
            if b.clicked() {
                action = Some(RowAction::Update(kit.clone(), dir.clone()));
            }
        }
        PlokRowState::Download => {
            let b = ui.button("Download");
            probe(ui, format!("plok.{name}.download"), b.rect);
            if b.clicked() {
                action = Some(RowAction::Download(kit.clone()));
            }
        }
    }
    action
}

/// The progress bar's fill and text for the running job.
fn progress_of(status: &Status) -> (f32, String) {
    match status {
        Status::Downloading {
            downloaded_bytes,
            total_bytes,
            bytes_per_sec,
            eta_secs,
            ..
        } => {
            let mut text = if *total_bytes > 0 {
                format!(
                    "{} / {}",
                    format_bytes(*downloaded_bytes),
                    format_bytes(*total_bytes)
                )
            } else {
                format_bytes(*downloaded_bytes)
            };
            if *bytes_per_sec > 0.0 {
                text.push_str(&format!(" · {}/s", format_bytes(*bytes_per_sec as u64)));
            }
            if let Some(eta) = eta_secs {
                text.push_str(&format!(" · {} left", format_eta(*eta)));
            }
            let fraction = if *total_bytes > 0 {
                *downloaded_bytes as f32 / *total_bytes as f32
            } else {
                0.0
            };
            (fraction, text)
        }
        Status::Verifying(_) => (1.0, "verifying…".to_string()),
        Status::Extracting {
            files_done,
            files_total,
            bytes_done,
            bytes_total,
            ..
        } => {
            let fraction = if *bytes_total > 0 {
                *bytes_done as f32 / *bytes_total as f32
            } else {
                0.0
            };
            (
                fraction,
                format!("extracting {files_done} / {files_total} files"),
            )
        }
        _ => (0.0, String::new()),
    }
}

/// "45 s", "4 min", "1 h 05 min".
pub(crate) fn format_eta(secs: f64) -> String {
    let s = secs.max(0.0).round() as u64;
    if s < 60 {
        format!("{s} s")
    } else if s < 3600 {
        format!("{} min", s.div_ceil(60))
    } else {
        format!("{} h {:02} min", s / 3600, (s % 3600) / 60)
    }
}
