//! The actual egui app: state and update/view orchestration for the drums editor.
//!
//! `DrumsEditorApp` is the `EditorApp` the runtime drives each frame. It
//! paints the chrome (brand + tab bar + status bar) on the outside, and
//! dispatches the central body to whichever tab is selected. The Pads tab
//! renders the canonical two-column layout (pad list + per-pad detail)
//! plus a bottom row of KIT and GLOBAL cards.

use std::sync::Arc;

use resonance_common::registry::InstalledItem;
use wayland_plugin_gui::{egui, EditorApp};

use crate::download::WorkerHandle;
use crate::params::DrumParams;
use crate::KitBridge;

use super::{
    chrome, download_panel, kit_browser, pad_grid, pad_inspector, theme, widgets,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DrumsTab {
    Pads,
    Mics,
    Articulations,
    Mod,
    Fx,
}

pub(crate) struct DrumsEditorApp {
    pub(crate) params: Arc<DrumParams>,
    pub(crate) bridge: KitBridge,
    pub(crate) selected_pad: usize,
    pub(crate) selected_tab: DrumsTab,
    pub(crate) pad_filter: String,
    pub(crate) download_worker: Arc<WorkerHandle>,
    pub(crate) download_panel: download_panel::DownloadPanelState,
    /// Cached list of installed drum kits from the shared registry.
    pub(crate) installed_kits: Vec<InstalledItem>,
    installed_kits_refresh: u32,
}

impl DrumsEditorApp {
    pub(super) fn new(
        params: Arc<DrumParams>,
        bridge: KitBridge,
        download_worker: Arc<WorkerHandle>,
    ) -> Self {
        let installed_kits = kit_browser::refresh_installed_kits();
        let selected_tab = match std::env::var("DRUMS_TAB").as_deref() {
            Ok("mics") => DrumsTab::Mics,
            Ok("articulations") => DrumsTab::Articulations,
            Ok("mod") => DrumsTab::Mod,
            Ok("fx") => DrumsTab::Fx,
            _ => DrumsTab::Pads,
        };
        Self {
            params,
            bridge,
            selected_pad: 0,
            selected_tab,
            pad_filter: String::new(),
            download_worker,
            download_panel: download_panel::DownloadPanelState::default(),
            installed_kits,
            installed_kits_refresh: 0,
        }
    }

    fn maybe_refresh_installed_kits(&mut self) {
        self.installed_kits_refresh += 1;
        if self.installed_kits_refresh >= 60 {
            self.installed_kits_refresh = 0;
            self.installed_kits = kit_browser::refresh_installed_kits();
        }
    }
}

impl EditorApp for DrumsEditorApp {
    fn ui(&mut self, ui: &mut egui::Ui) {
        theme::apply(ui.ctx());
        self.maybe_refresh_installed_kits();

        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(100));

        // Chrome.
        egui::Panel::top("drums_chrome")
            .exact_size(38.0)
            .frame(
                egui::Frame::default()
                    .fill(theme::BG_1)
                    .inner_margin(egui::Margin::symmetric(14, 6))
                    .stroke(egui::Stroke::new(1.0, theme::LINE_2)),
            )
            .show_inside(ui, |ui| chrome::draw_chrome(ui, self));

        // Tab bar.
        egui::Panel::top("drums_tabs")
            .exact_size(48.0)
            .frame(
                egui::Frame::default()
                    .fill(theme::BG_1)
                    .inner_margin(egui::Margin::symmetric(14, 6))
                    .stroke(egui::Stroke::new(1.0, theme::LINE_2)),
            )
            .show_inside(ui, |ui| chrome::draw_tab_bar(ui, self));

        // Status bar.
        egui::Panel::bottom("drums_status")
            .exact_size(28.0)
            .frame(
                egui::Frame::default()
                    .fill(theme::BG_1)
                    .inner_margin(egui::Margin::symmetric(16, 6))
                    .stroke(egui::Stroke::new(1.0, theme::LINE_2)),
            )
            .show_inside(ui, |ui| chrome::draw_status_bar(ui, self));

        // Body.
        egui::CentralPanel::default()
            .frame(
                egui::Frame::default()
                    .fill(theme::BG_0)
                    .inner_margin(egui::Margin::same(12)),
            )
            .show_inside(ui, |ui| match self.selected_tab {
                DrumsTab::Pads => draw_pads_body(ui, self),
                DrumsTab::Mics => draw_placeholder_tab(ui, "Mics"),
                DrumsTab::Articulations => draw_placeholder_tab(ui, "Articulations"),
                DrumsTab::Mod => draw_placeholder_tab(ui, "Mod"),
                DrumsTab::Fx => draw_placeholder_tab(ui, "FX"),
            });

        if self.download_panel.open {
            download_panel::draw(ui, &mut self.download_panel, &self.download_worker);
        }
    }
}

/// Pads tab body: 320 px left column (pad list) + right column (detail) +
/// bottom row of KIT + GLOBAL cards.
fn draw_pads_body(ui: &mut egui::Ui, app: &mut DrumsEditorApp) {
    // Snapshot the catalog once per frame; cheap clone avoids re-locking
    // inside the inspector's nested combo callbacks.
    let catalog = app.bridge.catalog.lock().clone();

    let avail_w = ui.available_width();
    let left_w = 320.0_f32.min(avail_w * 0.42);
    let gap = 12.0;
    let right_w = (avail_w - left_w - gap).max(200.0);

    // Top row: split into 2 columns of fixed/proportional width.
    let mut clicked_pad: Option<usize> = None;
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing = egui::vec2(gap, 0.0);

        ui.allocate_ui(egui::vec2(left_w, ui.available_height() - 200.0), |ui| {
            let mut selected = app.selected_pad;
            pad_grid::draw(
                ui,
                &app.params,
                &app.bridge,
                &mut app.download_panel,
                &mut app.pad_filter,
                &mut selected,
                &app.download_worker,
            );
            if selected != app.selected_pad {
                clicked_pad = Some(selected);
            }
        });

        ui.allocate_ui(egui::vec2(right_w, ui.available_height() - 200.0), |ui| {
            pad_inspector::draw(
                ui,
                &app.params,
                &app.bridge,
                &catalog,
                app.selected_pad,
            );
        });
    });
    if let Some(p) = clicked_pad {
        app.selected_pad = p;
    }

    ui.add_space(12.0);

    // Bottom row: KIT card + GLOBAL card.
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing = egui::vec2(gap, 0.0);
        let half = (ui.available_width() - gap) * 0.5;
        ui.allocate_ui(egui::vec2(half, 110.0), |ui| {
            draw_kit_row_card(ui, app);
        });
        ui.allocate_ui(egui::vec2(half, 110.0), |ui| {
            draw_global_row_card(ui);
        });
    });
}

fn draw_placeholder_tab(ui: &mut egui::Ui, name: &str) {
    let frame = egui::Frame::default()
        .fill(theme::BG_2)
        .stroke(egui::Stroke::new(1.0, theme::LINE_2))
        .corner_radius(theme::RADIUS_PANEL)
        .inner_margin(egui::Margin::same(32));
    frame.show(ui, |ui| {
        ui.vertical_centered(|ui| {
            ui.add_space(20.0);
            ui.label(
                egui::RichText::new(name)
                    .italics()
                    .color(theme::TEXT_2)
                    .size(20.0),
            );
            ui.add_space(8.0);
            ui.label(
                egui::RichText::new("Coming soon — open the Pads tab to edit kit and pads.")
                    .color(theme::TEXT_3)
                    .size(11.0),
            );
        });
    });
}

fn draw_kit_row_card(ui: &mut egui::Ui, app: &mut DrumsEditorApp) {
    let frame = egui::Frame::default()
        .fill(theme::BG_2)
        .stroke(egui::Stroke::new(1.0, theme::LINE_2))
        .corner_radius(theme::RADIUS_PANEL)
        .inner_margin(egui::Margin::symmetric(14, 12));
    frame.show(ui, |ui| {
        ui.set_min_width(ui.available_width() - 28.0);
        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new("KIT")
                    .color(theme::TEXT_3)
                    .size(10.5)
                    .strong(),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let status = kit_browser::format_kit_status(&app.bridge.kit_status.lock().clone());
                ui.label(
                    egui::RichText::new(status)
                        .color(theme::TEXT_3)
                        .size(10.5)
                        .monospace(),
                );
            });
        });
        ui.add_space(4.0);

        // Three-column field row.
        let avail = ui.available_width();
        let col = (avail - 36.0) / 3.0;

        ui.horizontal(|ui| {
            // Master.
            ui.vertical(|ui| {
                ui.set_min_width(col);
                ui.set_max_width(col);
                let v = app.params.master_volume.value();
                ui.horizontal(|ui| {
                    ui.label(
                        egui::RichText::new("MASTER")
                            .color(theme::TEXT_3)
                            .size(10.0),
                    );
                    ui.with_layout(
                        egui::Layout::right_to_left(egui::Align::Center),
                        |ui| {
                            ui.label(
                                egui::RichText::new(format!("{:.2}", v))
                                    .color(theme::TEXT_1)
                                    .size(11.0)
                                    .monospace(),
                            );
                        },
                    );
                });
                if let Some(nv) = widgets::slider_unipolar(ui, col, v) {
                    app.params.master_volume.set_value(nv);
                }
            });
            ui.add_space(18.0);
            // Bus tone (preview only).
            ui.vertical(|ui| {
                ui.set_min_width(col);
                ui.set_max_width(col);
                ui.horizontal(|ui| {
                    ui.label(
                        egui::RichText::new("BUS TONE")
                            .color(theme::TEXT_3)
                            .size(10.0),
                    );
                    ui.with_layout(
                        egui::Layout::right_to_left(egui::Align::Center),
                        |ui| {
                            ui.label(
                                egui::RichText::new("+0.00")
                                    .color(theme::TEXT_3)
                                    .size(11.0)
                                    .monospace(),
                            );
                        },
                    );
                });
                let _ = widgets::slider_bipolar(ui, col, 0.0);
            });
            ui.add_space(18.0);
            // Routing (preview only).
            ui.vertical(|ui| {
                ui.set_min_width(col);
                ui.set_max_width(col);
                ui.horizontal(|ui| {
                    ui.label(
                        egui::RichText::new("ROUTING")
                            .color(theme::TEXT_3)
                            .size(10.0),
                    );
                    ui.with_layout(
                        egui::Layout::right_to_left(egui::Align::Center),
                        |ui| {
                            ui.label(
                                egui::RichText::new("stereo")
                                    .color(theme::TEXT_3)
                                    .size(11.0)
                                    .monospace(),
                            );
                        },
                    );
                });
                let _ = widgets::segmented(ui, &["Stereo", "Multi-out"], 0, false);
            });
        });
    });
}

fn draw_global_row_card(ui: &mut egui::Ui) {
    let frame = egui::Frame::default()
        .fill(theme::BG_2)
        .stroke(egui::Stroke::new(1.0, theme::LINE_2))
        .corner_radius(theme::RADIUS_PANEL)
        .inner_margin(egui::Margin::symmetric(14, 12));
    frame.show(ui, |ui| {
        ui.set_min_width(ui.available_width() - 28.0);
        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new("GLOBAL")
                    .color(theme::TEXT_3)
                    .size(10.5)
                    .strong(),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(
                    egui::RichText::new("preview")
                        .color(theme::TEXT_3)
                        .size(10.5)
                        .monospace(),
                );
            });
        });
        ui.add_space(4.0);

        let avail = ui.available_width();
        let col = (avail - 36.0) / 3.0;
        ui.horizontal(|ui| {
            // Polyphony.
            ui.vertical(|ui| {
                ui.set_min_width(col);
                ui.set_max_width(col);
                ui.horizontal(|ui| {
                    ui.label(
                        egui::RichText::new("POLYPHONY")
                            .color(theme::TEXT_3)
                            .size(10.0),
                    );
                    ui.with_layout(
                        egui::Layout::right_to_left(egui::Align::Center),
                        |ui| {
                            ui.label(
                                egui::RichText::new("64")
                                    .color(theme::TEXT_3)
                                    .size(11.0)
                                    .monospace(),
                            );
                        },
                    );
                });
                let _ = widgets::slider_unipolar(ui, col, 0.5);
            });
            ui.add_space(18.0);
            // Velocity curve.
            ui.vertical(|ui| {
                ui.set_min_width(col);
                ui.set_max_width(col);
                ui.horizontal(|ui| {
                    ui.label(
                        egui::RichText::new("VELOCITY CURVE")
                            .color(theme::TEXT_3)
                            .size(10.0),
                    );
                    ui.with_layout(
                        egui::Layout::right_to_left(egui::Align::Center),
                        |ui| {
                            ui.label(
                                egui::RichText::new("linear")
                                    .color(theme::TEXT_3)
                                    .size(11.0)
                                    .monospace(),
                            );
                        },
                    );
                });
                let _ = widgets::slider_bipolar(ui, col, 0.0);
            });
            ui.add_space(18.0);
            // Round robin.
            ui.vertical(|ui| {
                ui.set_min_width(col);
                ui.set_max_width(col);
                ui.horizontal(|ui| {
                    ui.label(
                        egui::RichText::new("ROUND ROBIN")
                            .color(theme::TEXT_3)
                            .size(10.0),
                    );
                    ui.with_layout(
                        egui::Layout::right_to_left(egui::Align::Center),
                        |ui| {
                            ui.label(
                                egui::RichText::new("cycle")
                                    .color(theme::TEXT_3)
                                    .size(11.0)
                                    .monospace(),
                            );
                        },
                    );
                });
                let _ = widgets::segmented(ui, &["Cycle", "Random"], 0, false);
            });
        });
    });
}
