//! Download Kits overlay panel.
//!
//! Rendered on top of the normal drum editor when the user clicks the
//! header's "Download kits…" button (`chrome.rs`). All network activity
//! is delegated to [`crate::download`], so this file is purely
//! presentation: it reads the shared `State` each frame, lays out egui
//! widgets, and posts `Command`s back.
//!
//! It is an `egui::Modal`: the backdrop is painted on the modal's own
//! foreground layer, same as the panel content, so it can never end up
//! drawn above the panel the way the old backdrop (a layer order above
//! the panel's) did (ba drums-plugin-rework.md §1.2). The modal also
//! makes it properly modal — its backdrop senses clicks, so they no
//! longer fall through to the pads behind it — and closes on Esc for
//! free.

use std::collections::HashMap;
use std::sync::Arc;

use parking_lot::Mutex;

use plugin_gui_core::egui;

use super::theme;
use crate::download::{Command, ServerKit, Status, WorkerHandle};
use resonance_common::registry::{self, ContentType};

/// Per-panel UI state, owned by the editor app.
#[derive(Default)]
pub struct DownloadPanelState {
    pub open: bool,
    /// Set once after the panel opens so we fetch the index exactly once.
    pub did_initial_fetch: bool,
    /// When set, the kit with this name is awaiting deletion confirmation.
    /// A second click on "Confirm?" will actually delete it.
    ///
    /// Cleared whenever the panel opens or closes: a "Confirm?" left
    /// armed across a close used to come back on reopen, one click away
    /// from deleting a kit the user had walked away from.
    pub(super) pending_delete: Option<String>,
    /// Deletions running on their own threads — `remove_dir_all` on a
    /// multi-gigabyte kit is far too slow for the UI thread.
    deletions: Vec<Deletion>,
    /// Why the last deletion failed, if it did.
    delete_error: Option<String>,
}

/// One kit being removed in the background.
struct Deletion {
    name: String,
    /// `None` while running; then `Ok` or the error to show.
    outcome: Arc<Mutex<Option<Result<(), String>>>>,
}

/// Open the overlay and, unless the worker is mid-job, force a fresh
/// index fetch.
///
/// Without resetting `did_initial_fetch`, the index was fetched once per
/// editor lifetime (ba drums-plugin-rework.md §1.1): a kit installed or
/// removed elsewhere — by another editor's download, or by hand — never
/// showed up here until the whole editor was reopened.
///
/// A busy worker is left alone, though. The fetch would queue behind the
/// running download and, the moment it finished, replace its "Download
/// complete" with "Fetching available kits…" before the user reopening
/// the panel to check on it ever saw it.
pub(super) fn open(panel: &mut DownloadPanelState, worker: &WorkerHandle) {
    panel.open = true;
    panel.pending_delete = None;
    panel.did_initial_fetch = worker.state.lock().status.is_busy();
}

/// Close the overlay, disarming any half-confirmed delete.
pub(super) fn close(panel: &mut DownloadPanelState) {
    panel.open = false;
    panel.pending_delete = None;
}

const MARGIN: f32 = 48.0;
const FRAME_MARGIN: f32 = 16.0;

pub fn draw(ui: &mut egui::Ui, panel: &mut DownloadPanelState, worker: &Arc<WorkerHandle>) {
    let screen = ui.ctx().content_rect();
    let content_size = egui::vec2(
        (screen.width() - 2.0 * MARGIN - 2.0 * FRAME_MARGIN).max(0.0),
        (screen.height() - 2.0 * MARGIN - 2.0 * FRAME_MARGIN).max(0.0),
    );

    let frame = egui::Frame::new()
        .fill(theme::PANEL)
        .stroke(egui::Stroke::new(1.0, theme::BORDER))
        .corner_radius(6.0)
        .inner_margin(egui::Margin::same(16));

    let modal = egui::Modal::new(egui::Id::new("download_kits_panel"))
        .backdrop_color(egui::Color32::from_black_alpha(180))
        .frame(frame);

    let response = modal.show(ui.ctx(), |ui| {
        ui.set_width(content_size.x);
        ui.set_height(content_size.y);
        draw_contents(ui, panel, worker);
    });

    // Esc closes, as does the Close button; a click on the backdrop does
    // not. `ModalResponse::should_close` counts backdrop clicks too, and a
    // click that strays outside a panel this size is far likelier to be a
    // miss than a request to dismiss it.
    let escape = response.is_top_modal
        && !response.any_popup_open
        && ui
            .ctx()
            .input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape));
    if escape || response.response.should_close() {
        close(panel);
    }
}

fn draw_contents(ui: &mut egui::Ui, panel: &mut DownloadPanelState, worker: &Arc<WorkerHandle>) {
    // Kick off the index fetch on first open.
    if !panel.did_initial_fetch {
        panel.did_initial_fetch = true;
        worker.send(Command::FetchIndex);
    }

    poll_deletions(panel);
    draw_header(ui, panel, worker);
    ui.add_space(6.0);
    ui.separator();
    ui.add_space(6.0);

    // Snapshot state for this frame.
    let status;
    let index;
    let error;
    {
        let s = worker.state.lock();
        status = s.status.clone();
        index = s.index.clone();
        error = s.last_error.clone();
    }

    draw_status_line(ui, &status);
    ui.add_space(6.0);

    draw_kit_list(ui, panel, worker, index.as_ref(), &status);

    for err in error.iter().chain(panel.delete_error.iter()) {
        ui.add_space(4.0);
        ui.label(
            egui::RichText::new(format!("Error: {err}"))
                .color(theme::DANGER)
                .size(11.0),
        );
    }
}

/// Retire finished deletions, keeping the last failure to show.
fn poll_deletions(panel: &mut DownloadPanelState) {
    let mut failed = None;
    panel.deletions.retain(|d| match d.outcome.lock().take() {
        None => true,
        Some(Ok(())) => false,
        Some(Err(e)) => {
            failed = Some(format!("could not delete {}: {e}", d.name));
            false
        }
    });
    if failed.is_some() {
        panel.delete_error = failed;
    }
}

/// Delete an installed kit on its own thread: its directory first, then —
/// only once that worked — its registry entry, so a failed delete never
/// leaves a kit on disk the library no longer knows about.
fn start_deletion(panel: &mut DownloadPanelState, name: &str, path: std::path::PathBuf) {
    let outcome = Arc::new(Mutex::new(None));
    let slot = outcome.clone();
    let kit = name.to_string();
    let spawned = std::thread::Builder::new()
        .name("drums-kit-delete".into())
        .spawn(move || {
            let removed = match std::fs::remove_dir_all(&path) {
                Ok(()) => Ok(()),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
                Err(e) => Err(e.to_string()),
            };
            let result = removed.and_then(|()| {
                registry::remove_installed(&kit, &ContentType::Drumkit)
                    .map_err(|e| e.to_string())
            });
            *slot.lock() = Some(result);
        });
    match spawned {
        Ok(_) => {
            panel.delete_error = None;
            panel.deletions.push(Deletion {
                name: name.to_string(),
                outcome,
            });
        }
        Err(e) => panel.delete_error = Some(format!("could not delete {name}: {e}")),
    }
}

fn draw_header(ui: &mut egui::Ui, panel: &mut DownloadPanelState, worker: &Arc<WorkerHandle>) {
    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new("DOWNLOAD KITS")
                .strong()
                .color(theme::ACCENT)
                .size(14.0),
        );
        ui.add_space(10.0);

        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.button("Close").clicked() {
                close(panel);
            }
            ui.add_space(6.0);
            let busy = worker.state.lock().status.is_busy();
            ui.add_enabled_ui(!busy, |ui| {
                if ui.button("Refresh").clicked() {
                    worker.send(Command::FetchIndex);
                }
            });
        });
    });
}

fn draw_status_line(ui: &mut egui::Ui, status: &Status) {
    let text = match status {
        Status::Idle => return,
        Status::FetchingIndex => "Fetching available kits...".to_string(),
        Status::Downloading {
            name,
            downloaded_bytes,
            total_bytes,
        } => {
            let dl = format_bytes(*downloaded_bytes);
            if *total_bytes > 0 {
                let total = format_bytes(*total_bytes);
                format!("Downloading {name} ({dl} / {total})...")
            } else {
                format!("Downloading {name} ({dl})...")
            }
        }
        Status::Extracting(name) => format!("Extracting {name}..."),
        Status::Done(name) => format!("Download complete: {name}"),
        Status::Error(_) => return, // shown separately
    };

    let color = match status {
        Status::Done(_) => theme::ACCENT,
        _ => theme::TEXT_DIM,
    };

    ui.label(egui::RichText::new(text).color(color).size(11.0));
}

fn draw_kit_list(
    ui: &mut egui::Ui,
    panel: &mut DownloadPanelState,
    worker: &Arc<WorkerHandle>,
    index: Option<&crate::download::ServerIndex>,
    status: &Status,
) {
    let Some(index) = index else {
        ui.label(
            egui::RichText::new("(loading...)")
                .color(theme::TEXT_DIM)
                .size(11.0),
        );
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

    // Load the registry once per frame to check installed status, and
    // index it by name so each kit row is an O(1) lookup instead of a
    // linear scan per row.
    let installed = registry::list_installed(&ContentType::Drumkit);
    let installed_by_name: HashMap<&str, &registry::InstalledItem> =
        installed.iter().map(|item| (item.name.as_str(), item)).collect();

    egui::ScrollArea::vertical()
        .id_salt("download_kits_scroll")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            for kit in &index.drumkits {
                draw_kit_row(ui, panel, worker, kit, &installed_by_name, status);
            }
        });
}

fn draw_kit_row(
    ui: &mut egui::Ui,
    panel: &mut DownloadPanelState,
    worker: &Arc<WorkerHandle>,
    kit: &ServerKit,
    installed: &HashMap<&str, &registry::InstalledItem>,
    status: &Status,
) {
    let installed_item = installed.get(kit.name.as_str()).copied();
    let is_installed = installed_item.is_some();

    let frame = egui::Frame::new()
        .fill(theme::PANEL)
        .stroke(egui::Stroke::new(1.0, theme::BORDER))
        .inner_margin(egui::Margin::same(8))
        .outer_margin(egui::Margin::symmetric(0, 2));

    frame.show(ui, |ui| {
        ui.set_min_width(ui.available_width());
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.label(
                    egui::RichText::new(&kit.name)
                        .color(theme::TEXT)
                        .size(13.0)
                        .strong(),
                );
                let mut subtitle_parts = Vec::new();
                if let Some(size) = &kit.size {
                    subtitle_parts.push(size.clone());
                }
                if let Some(desc) = &kit.description {
                    if !desc.is_empty() {
                        subtitle_parts.push(desc.clone());
                    }
                }
                if let Some(added) = &kit.added {
                    subtitle_parts.push(format!("added {added}"));
                }
                if !subtitle_parts.is_empty() {
                    ui.label(
                        egui::RichText::new(subtitle_parts.join(" \u{00b7} "))
                            .color(theme::TEXT_DIM)
                            .size(10.0),
                    );
                }
            });

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let deleting = panel.deletions.iter().any(|d| d.name == kit.name);
                if deleting {
                    ui.label(
                        egui::RichText::new("Deleting…")
                            .color(theme::TEXT_DIM)
                            .size(11.0),
                    );
                } else if is_installed {
                    let confirming = panel
                        .pending_delete
                        .as_ref()
                        .is_some_and(|name| name == &kit.name);

                    if confirming {
                        // Show confirm/cancel buttons.
                        if ui
                            .button(
                                egui::RichText::new("Confirm?")
                                    .color(theme::DANGER)
                                    .size(11.0),
                            )
                            .clicked()
                        {
                            if let Some(item) = installed_item {
                                let path = std::path::PathBuf::from(&item.path);
                                start_deletion(panel, &kit.name, path);
                            }
                            panel.pending_delete = None;
                        }
                        if ui.button("Cancel").clicked() {
                            panel.pending_delete = None;
                        }
                    } else {
                        // Show delete button + installed label.
                        if ui
                            .button(
                                egui::RichText::new("Delete")
                                    .color(theme::DANGER)
                                    .size(11.0),
                            )
                            .clicked()
                        {
                            panel.pending_delete = Some(kit.name.clone());
                        }
                        ui.label(
                            egui::RichText::new("Installed")
                                .color(theme::ACCENT)
                                .size(12.0),
                        );
                    }
                } else {
                    let busy = status.is_busy();
                    ui.add_enabled_ui(!busy, |ui| {
                        if ui.button("Download").clicked() {
                            worker.send(Command::Download(kit.clone()));
                        }
                    });
                }
            });
        });
    });
}

/// Format a byte count as a human-readable string with appropriate unit.
fn format_bytes(bytes: u64) -> String {
    const KIB: u64 = 1024;
    const MIB: u64 = 1024 * KIB;
    const GIB: u64 = 1024 * MIB;

    if bytes >= GIB {
        format!("{:.1} GiB", bytes as f64 / GIB as f64)
    } else if bytes >= MIB {
        format!("{:.1} MiB", bytes as f64 / MIB as f64)
    } else if bytes >= KIB {
        format!("{:.0} KiB", bytes as f64 / KIB as f64)
    } else {
        format!("{bytes} B")
    }
}
