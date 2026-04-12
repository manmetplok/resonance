//! Download Kits overlay panel.
//!
//! Rendered on top of the normal drum editor when the user clicks the
//! "Download Kits" button. All network activity is delegated to
//! [`crate::download`], so this file is purely presentation: it reads the
//! shared `State` each frame, lays out egui widgets, and posts `Command`s
//! back.

use std::sync::Arc;

use wayland_plugin_gui::egui;

use super::theme;
use crate::download::{Command, ServerKit, Status, WorkerHandle};
use resonance_common::registry::{self, ContentType};

/// Per-panel UI state, owned by the editor app.
pub struct DownloadPanelState {
    pub open: bool,
    /// Set once after the panel opens so we fetch the index exactly once.
    pub did_initial_fetch: bool,
    /// When set, the kit with this name is awaiting deletion confirmation.
    /// A second click on "Confirm?" will actually delete it.
    pending_delete: Option<String>,
}

impl Default for DownloadPanelState {
    fn default() -> Self {
        Self {
            open: false,
            did_initial_fetch: false,
            pending_delete: None,
        }
    }
}

pub fn draw(ui: &mut egui::Ui, panel: &mut DownloadPanelState, worker: &Arc<WorkerHandle>) {
    // Dim the background behind the overlay. Use a Tooltip-order layer so it
    // sits between the normal UI and the Foreground-order panel, avoiding
    // darkening the panel itself.
    let screen = ui.ctx().content_rect();
    let painter = ui.ctx().layer_painter(egui::LayerId::new(
        egui::Order::Tooltip,
        egui::Id::new("download_kits_backdrop"),
    ));
    painter.rect_filled(screen, 0.0, egui::Color32::from_black_alpha(180));

    let margin = 48.0;
    let rect = screen.shrink(margin);
    let window_id = egui::Id::new("download_kits_panel");

    egui::Area::new(window_id)
        .fixed_pos(rect.min)
        .order(egui::Order::Foreground)
        .show(ui.ctx(), |ui| {
            let frame = egui::Frame::new()
                .fill(theme::PANEL)
                .stroke(egui::Stroke::new(1.0, theme::BORDER))
                .corner_radius(6.0)
                .inner_margin(egui::Margin::same(16));
            frame.show(ui, |ui| {
                ui.set_width(rect.width() - 32.0);
                ui.set_height(rect.height() - 32.0);
                draw_contents(ui, panel, worker);
            });
        });
}

fn draw_contents(
    ui: &mut egui::Ui,
    panel: &mut DownloadPanelState,
    worker: &Arc<WorkerHandle>,
) {
    // Kick off the index fetch on first open.
    if !panel.did_initial_fetch {
        panel.did_initial_fetch = true;
        worker.send(Command::FetchIndex);
    }

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

    if let Some(err) = error {
        ui.add_space(4.0);
        ui.label(
            egui::RichText::new(format!("Error: {err}"))
                .color(theme::DANGER)
                .size(11.0),
        );
    }
}

fn draw_header(
    ui: &mut egui::Ui,
    panel: &mut DownloadPanelState,
    worker: &Arc<WorkerHandle>,
) {
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
                panel.open = false;
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

    // Load the registry once per frame to check installed status.
    let installed = registry::list_installed(&ContentType::Drumkit);

    egui::ScrollArea::vertical()
        .id_salt("download_kits_scroll")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            for kit in &index.drumkits {
                draw_kit_row(ui, panel, worker, kit, &installed, status);
            }
        });
}

fn draw_kit_row(
    ui: &mut egui::Ui,
    panel: &mut DownloadPanelState,
    worker: &Arc<WorkerHandle>,
    kit: &ServerKit,
    installed: &[registry::InstalledItem],
    status: &Status,
) {
    let installed_item = installed.iter().find(|item| item.name == kit.name);
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
                if is_installed {
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
                            // Actually delete: remove directory, then registry entry.
                            if let Some(item) = installed_item {
                                let path = std::path::Path::new(&item.path);
                                if path.exists() {
                                    let _ = std::fs::remove_dir_all(path);
                                }
                            }
                            let _ =
                                registry::remove_installed(&kit.name, &ContentType::Drumkit);
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
