//! The actual egui app: state and update/view orchestration for the drums editor.
//!
//! `DrumsEditorApp` is the `EditorApp` the runtime drives each frame. It
//! paints the chrome (brand + tab bar + status bar) on the outside and the
//! Pads body in the middle: the canonical two-column layout (pad list +
//! per-pad detail) plus a bottom row of KIT and GLOBAL cards.
//!
//! Pads is the only view. The editor used to offer four more tabs, each
//! rendering a placeholder that said the feature was not built yet —
//! including Mics and Articulations, whose pickers already ship inside the
//! pad inspector, so those two tabs denied features the plugin has. They
//! were removed rather than left lying (ba todo #1327);
//! `chrome::draw_tab_bar` points at where the pickers live.

use std::sync::atomic::Ordering;
use std::sync::Arc;

use resonance_common::registry::InstalledItem;
use wayland_plugin_gui::{egui, widgets, EditorApp};

use crate::download::WorkerHandle;
use crate::kit;
use crate::params::{DrumParams, ROUND_ROBIN_LABELS};
use crate::velocity;
use crate::voice::MAX_VOICES;
use crate::KitBridge;

use super::{chrome, download_panel, kit_browser, pad_grid, pad_inspector, theme};

pub(crate) struct DrumsEditorApp {
    pub(crate) params: Arc<DrumParams>,
    pub(crate) bridge: KitBridge,
    pub(crate) selected_pad: usize,
    pub(crate) pad_filter: String,
    pub(crate) download_worker: Arc<WorkerHandle>,
    pub(crate) download_panel: download_panel::DownloadPanelState,
    /// Cached list of installed drum kits from the shared registry.
    pub(crate) installed_kits: Vec<InstalledItem>,
    installed_kits_refresh: u32,
    /// Displayed OUT meter level per channel. Rises instantly to the peak
    /// the audio thread published and falls back with a fixed decay, so
    /// the bar tracks real output instead of sitting dead.
    out_meter: [f32; 2],
    /// This plugin ships no factory presets, so the bank is the user's
    /// own directory alone (ba todo #1358).
    pub(crate) bank: resonance_plugin::presets::PresetBank,
    /// Shared with the plugin struct, so what the bar shows is what
    /// `save_state` persists.
    pub(crate) presets: Arc<resonance_plugin::presets::PresetSession>,
    /// Transient bar state (open combo, in-progress rename), editor-only.
    pub(crate) preset_editor: resonance_plugin::presets::PresetEditor,
}

impl DrumsEditorApp {
    pub(super) fn new(
        params: Arc<DrumParams>,
        bridge: KitBridge,
        download_worker: Arc<WorkerHandle>,
        presets: Arc<resonance_plugin::presets::PresetSession>,
    ) -> Self {
        let installed_kits = kit_browser::refresh_installed_kits();
        Self {
            params,
            bridge,
            selected_pad: 0,
            pad_filter: String::new(),
            download_worker,
            download_panel: download_panel::DownloadPanelState::default(),
            installed_kits,
            installed_kits_refresh: 0,
            out_meter: [0.0; 2],
            bank: resonance_plugin::presets::PresetBank::new(
                <crate::ResonanceDrums as resonance_plugin::ResonancePlugin>::CLAP_ID,
                <crate::ResonanceDrums as resonance_plugin::ResonancePlugin>::FACTORY_PRESETS,
            ),
            presets,
            preset_editor: resonance_plugin::presets::PresetEditor::default(),
        }
    }

    /// Fold the audio thread's latest block peak into the displayed OUT
    /// meter and return the level to draw. The editor repaints at ~10 Hz
    /// while the audio thread publishes every block, so the peak is taken
    /// as an instant rise and a 0.75×-per-frame fall — a real reading with
    /// readable ballistics, never a value we made up.
    pub(crate) fn tick_out_meter(&mut self) -> [f32; 2] {
        const DECAY: f32 = 0.75;
        for (channel, level) in self.out_meter.iter_mut().enumerate() {
            let published =
                f32::from_bits(self.bridge.out_peak[channel].load(Ordering::Relaxed));
            let published = if published.is_finite() && published > 0.0 {
                published
            } else {
                0.0
            };
            *level = if published >= *level {
                published
            } else {
                (*level * DECAY).max(published)
            };
            if *level < 1.0e-5 {
                *level = 0.0;
            }
        }
        self.out_meter
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
            .show_inside(ui, |ui| draw_pads_body(ui, self));

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
        let half = super::body_width(ui, gap) * 0.5;
        ui.allocate_ui(egui::vec2(half, 110.0), |ui| {
            draw_kit_row_card(ui, app);
        });
        ui.allocate_ui(egui::vec2(half, 110.0), |ui| {
            draw_global_row_card(ui, &app.params);
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
        ui.set_min_width(super::body_width(ui, 28.0));
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

        // Two-column field row: master volume and the routing readout,
        // separated by one 18 px gap.
        let avail = ui.available_width();
        let col = (avail - 18.0) / 2.0;

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
            // BUS TONE used to sit here: a bipolar slider reading
            // "+0.00" that discarded every drag, because the plugin has
            // no bus tone control anywhere in its DSP. It is the same
            // defect as the GLOBAL card's three (ba todo #1326) and the
            // audit register missed it, so it goes the way the other
            // unimplemented controls went — out, rather than left drawn
            // for a user to drag at.

            // Routing — a readout, not a control. The plugin declares all
            // `kit::NUM_OUTPUT_PORTS` ports unconditionally (see
            // `ResonanceDrums::output_layout`); there is no stereo-only mode
            // to switch to, so nothing here is clickable.
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
                                egui::RichText::new(kit::routing_summary())
                                    .color(theme::TEXT_1)
                                    .size(11.0)
                                    .monospace(),
                            );
                        },
                    );
                });
                ui.label(
                    egui::RichText::new(kit::routing_port_list())
                        .color(theme::TEXT_3)
                        .size(9.5)
                        .monospace(),
                )
                .on_hover_text(
                    "Every drum group has its own stereo output port. Route them \
                     in the host's mixer — the plugin always declares all of them.",
                );
            });
        });
    });
}

/// The GLOBAL card: polyphony, velocity curve and round-robin mode.
///
/// Every control here writes a parameter (ba todo #1326) — before that
/// they were drawn from constants and threw their interaction away. The
/// parameters are what the sampler reads, so these three are reachable
/// from a host automation lane and `set_plugin_param` as well.
fn draw_global_row_card(ui: &mut egui::Ui, params: &DrumParams) {
    let frame = egui::Frame::default()
        .fill(theme::BG_2)
        .stroke(egui::Stroke::new(1.0, theme::LINE_2))
        .corner_radius(theme::RADIUS_PANEL)
        .inner_margin(egui::Margin::symmetric(14, 12));
    frame.show(ui, |ui| {
        ui.set_min_width(super::body_width(ui, 28.0));
        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new("GLOBAL")
                    .color(theme::TEXT_3)
                    .size(10.5)
                    .strong(),
            );
        });
        ui.add_space(4.0);

        let avail = ui.available_width();
        let col = (avail - 36.0) / 3.0;
        ui.horizontal(|ui| {
            // Polyphony — voice ceiling, 1..MAX_VOICES.
            ui.vertical(|ui| {
                ui.set_min_width(col);
                ui.set_max_width(col);
                let voices = params.polyphony.value();
                global_control_head(ui, "POLYPHONY", &voices.to_string());
                let span = (MAX_VOICES - 1) as f32;
                let unit = (voices - 1) as f32 / span;
                if let Some(new_unit) = widgets::slider_unipolar(ui, col, unit) {
                    params
                        .polyphony
                        .set_value(1 + (new_unit * span).round() as i32);
                }
            });
            ui.add_space(18.0);
            // Velocity curve — bipolar, centred on linear.
            ui.vertical(|ui| {
                ui.set_min_width(col);
                ui.set_max_width(col);
                let curve = params.velocity_curve.value();
                global_control_head(ui, "VELOCITY CURVE", &velocity::curve_label(curve));
                if let Some(new_curve) = widgets::slider_bipolar(ui, col, curve) {
                    params.velocity_curve.set_value(new_curve);
                }
            });
            ui.add_space(18.0);
            // Round robin — how a layer's takes are walked.
            ui.vertical(|ui| {
                ui.set_min_width(col);
                ui.set_max_width(col);
                let mode = params.round_robin_mode.value();
                global_control_head(ui, "ROUND ROBIN", params.round_robin_mode.label());
                if let Some(picked) =
                    widgets::segmented(ui, ROUND_ROBIN_LABELS, mode.max(0) as usize)
                {
                    params.round_robin_mode.set_value(picked as i32);
                }
            });
        });
    });
}

/// Label + right-aligned value readout, the header every GLOBAL control
/// shares. The readout is the parameter's own text, so the card and the
/// host's automation lane can never disagree about what is set.
fn global_control_head(ui: &mut egui::Ui, label: &str, value: &str) {
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new(label).color(theme::TEXT_3).size(10.0));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label(
                egui::RichText::new(value.to_lowercase())
                    .color(theme::TEXT_3)
                    .size(11.0)
                    .monospace(),
            );
        });
    });
}
