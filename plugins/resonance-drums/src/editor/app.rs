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
use plugin_gui_core::{egui, widgets, EditorApp};

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
            bank: resonance_plugin::presets::PresetBank::for_plugin::<crate::ResonanceDrums>(),
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


/// Height of the fixed bottom row (KIT + GLOBAL cards): the 12px gap from
/// the region above plus the cards' own 110px.
const KIT_GLOBAL_ROW_HEIGHT: f32 = 122.0;

/// A fixed-size column inside a horizontal row, laid out top to bottom.
///
/// `ui.allocate_ui(size, ..)` would be the obvious call, and it is wrong
/// here: it reuses the *parent's* layout (egui's `allocate_ui` is
/// `allocate_ui_with_layout(size, *self.layout(), ..)`), and every
/// column in this body sits in a horizontal row. Each card's contents
/// then ran left to right — the KIT header, MASTER and ROUTING side by
/// side, the GLOBAL card pushed past the window's right edge under an
/// inverted clip, the pad rows given 0 px of width, the inspector laid
/// out 1878 px wide. Every column states its own direction instead.
fn column<R>(
    ui: &mut egui::Ui,
    size: egui::Vec2,
    add: impl FnOnce(&mut egui::Ui) -> R,
) -> R {
    ui.allocate_ui_with_layout(size, egui::Layout::top_down(egui::Align::Min), add)
        .inner
}

/// Pads tab body: a fixed-height bottom row for the KIT + GLOBAL cards,
/// and above it the pad list (320 px column) + pad detail, each scrolling
/// on its own so nothing in either one — or the row below — can be pushed
/// out of reach (ba drums-plugin-rework.md §1.3, §6.1).
///
/// This replaces an `available_height() - 200.0` guess that gave the top
/// row 102px at the window's old default size and 22px at its minimum,
/// with nothing below it able to scroll into view.
fn draw_pads_body(ui: &mut egui::Ui, app: &mut DrumsEditorApp) {
    // Snapshot the catalog once per frame; cheap clone avoids re-locking
    // inside the inspector's nested combo callbacks.
    let catalog = app.bridge.catalog.lock().clone();
    let gap = 12.0;

    // Bottom row first, so it reserves its own space regardless of how
    // tall the pad list or inspector end up — drawn via a nested Panel,
    // which (like the chrome's top-level ones in `ui()`) claims its slice
    // of whatever `ui` it is shown inside before anything else sees it.
    egui::Panel::bottom("drums_kit_global_row")
        .exact_size(KIT_GLOBAL_ROW_HEIGHT)
        .frame(egui::Frame::NONE)
        .show_inside(ui, |ui| {
            ui.add_space(gap);
            ui.horizontal(|ui| {
                // `gap` is the one space between the two cards: the
                // row's item spacing puts it there, so the cards share
                // what is left once it is taken off. GLOBAL gets the
                // larger share — three labelled controls against KIT's
                // two — so its readouts still fit at the minimum width.
                ui.spacing_mut().item_spacing = egui::vec2(gap, 0.0);
                let shared = super::body_width(ui, gap);
                let kit_w = shared * KIT_CARD_SHARE;
                column(ui, egui::vec2(kit_w, 110.0), |ui| draw_kit_row_card(ui, app));
                column(ui, egui::vec2(shared - kit_w, 110.0), |ui| {
                    draw_global_row_card(ui, &app.params)
                });
            });
        });

    // Top row shares whatever height is left after the bottom row above.
    // `right_w` is floored at zero, not at some comfortable minimum: a
    // floor wider than what is left would push the inspector off the
    // window's edge instead of letting it squeeze.
    let avail_w = ui.available_width();
    let left_w = 320.0_f32.min(avail_w * 0.42);
    let right_w = (avail_w - left_w - gap).max(0.0);
    let body_h = ui.available_height();

    let mut clicked_pad: Option<usize> = None;
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing = egui::vec2(gap, 0.0);

        // The pad list scrolls internally (`pad_grid.rs`), so it degrades
        // by scrolling rather than clipping when `body_h` is tight.
        column(ui, egui::vec2(left_w, body_h), |ui| {
            let mut selected = app.selected_pad;
            pad_grid::draw(ui, &app.params, &app.bridge, &mut app.pad_filter, &mut selected);
            if selected != app.selected_pad {
                clicked_pad = Some(selected);
            }
        });

        // The inspector has no internal scroll area of its own, so one is
        // wrapped around it here.
        column(ui, egui::vec2(right_w, body_h), |ui| {
            egui::ScrollArea::vertical()
                .id_salt("pad_inspector_scroll")
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    pad_inspector::draw(ui, &app.params, &app.bridge, &catalog, app.selected_pad);
                });
        });
    });
    if let Some(p) = clicked_pad {
        app.selected_pad = p;
    }
}

/// The frame both bottom-row cards are drawn in.
fn row_card_frame() -> egui::Frame {
    egui::Frame::default()
        .fill(theme::BG_2)
        .stroke(egui::Stroke::new(1.0, theme::LINE_2))
        .corner_radius(theme::RADIUS_PANEL)
        .inner_margin(egui::Margin::symmetric(14, 12))
}

/// Gap between the columns inside a bottom-row card.
const CARD_COLUMN_GAP: f32 = 18.0;

/// The KIT card's share of the bottom row; GLOBAL takes the rest.
const KIT_CARD_SHARE: f32 = 0.42;

/// The widths of `N` columns that share `ui`'s width in proportion to
/// `weights` (which sum to 1), with a [`CARD_COLUMN_GAP`] between
/// neighbours.
///
/// Only right while the row's own `item_spacing.x` is zero — the cards
/// set it so (`card_body`), because the bottom row's 12 px spacing would
/// otherwise be inherited and added on top of every explicit gap: three
/// columns sized this way overflowed their card by 48 px.
fn card_columns<const N: usize>(ui: &egui::Ui, weights: [f32; N]) -> [f32; N] {
    let gaps = CARD_COLUMN_GAP * N.saturating_sub(1) as f32;
    let width = super::body_width(ui, gaps);
    weights.map(|w| width * w)
}

/// Fill the card's width, and zero the horizontal item spacing the
/// bottom row hands down: inside a card, every horizontal gap is an
/// explicit `add_space`.
fn card_body(ui: &mut egui::Ui) {
    ui.spacing_mut().item_spacing.x = 0.0;
    ui.set_min_width(ui.available_width());
}

fn draw_kit_row_card(ui: &mut egui::Ui, app: &mut DrumsEditorApp) {
    let shown = row_card_frame().show(ui, |ui| {
        card_body(ui);

        let status = kit_browser::format_kit_status(&app.bridge.kit_status.lock().clone());
        label_value_row(ui, "KIT", theme::TEXT_3, 10.5, &status, theme::TEXT_3, 10.5);
        ui.add_space(4.0);

        // Two-column field row: master volume and the routing readout,
        // which is the wider of the two strings.
        let [master_w, routing_w] = card_columns(ui, [0.4, 0.6]);
        ui.horizontal(|ui| {
            // Master.
            ui.vertical(|ui| {
                ui.set_width(master_w);
                let v = app.params.master_volume.value();
                label_value_row(
                    ui,
                    "MASTER",
                    theme::TEXT_3,
                    10.0,
                    &format!("{v:.2}"),
                    theme::TEXT_1,
                    11.0,
                );
                if let Some(nv) =
                    super::probed(ui, "kit.master", |ui| widgets::slider_unipolar(ui, master_w, v))
                {
                    app.params.master_volume.set_value(nv);
                }
            });
            ui.add_space(CARD_COLUMN_GAP);
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
                ui.set_width(routing_w);
                label_value_row(
                    ui,
                    "ROUTING",
                    theme::TEXT_3,
                    10.0,
                    &kit::routing_summary(),
                    theme::TEXT_1,
                    11.0,
                );
                // Truncated rather than left to wrap or overflow: at the
                // card's narrow half-width this monospace line is wider
                // than its column. The full list is still one hover away.
                ui.add(
                    egui::Label::new(
                        egui::RichText::new(kit::routing_port_list())
                            .color(theme::TEXT_3)
                            .size(9.5)
                            .monospace(),
                    )
                    .truncate(),
                )
                .on_hover_text(
                    "Every drum group has its own stereo output port. Route them \
                     in the host's mixer — the plugin always declares all of them.",
                );
            });
        });
    });
    super::probe(ui, "card.kit", shown.response.rect);
}

/// Gap kept between a row's label and its value.
const LABEL_VALUE_GAP: f32 = 8.0;

/// A "LABEL … value" row: the label on the left at its natural width,
/// the value right-aligned in whatever is left.
///
/// The value is a single line, elided with "…" when it does not fit,
/// with the full text on hover — a `KitStatus::Error` runs to ~550 px,
/// and painted unbounded it ran straight over its own label. The label
/// is never shortened: it says what the row is.
///
/// Both halves are real `Label`s, so they are laid out, sensed and
/// reported like any other widget. This used to paint both strings by
/// hand, on the theory that a right-to-left `with_layout` nested in
/// sibling columns starved every later column of width; what actually
/// starved them was the columns themselves running sideways — see
/// [`column`] — and with that fixed the ordinary idiom is fine.
fn label_value_row(
    ui: &mut egui::Ui,
    label: &str,
    label_color: egui::Color32,
    label_size: f32,
    value: &str,
    value_color: egui::Color32,
    value_size: f32,
) {
    ui.horizontal(|ui| {
        let l = ui.label(egui::RichText::new(label).color(label_color).size(label_size));
        super::probe(ui, format!("{label}.label"), l.rect);
        ui.add_space(LABEL_VALUE_GAP);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let v = ui
                .add(
                    egui::Label::new(
                        egui::RichText::new(value)
                            .color(value_color)
                            .size(value_size)
                            .monospace(),
                    )
                    .truncate(),
                )
                .on_hover_text(value);
            super::probe(ui, format!("{label}.value"), v.rect);
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
    let shown = row_card_frame().show(ui, |ui| {
        card_body(ui);
        ui.label(
            egui::RichText::new("GLOBAL")
                .color(theme::TEXT_3)
                .size(10.5)
                .strong(),
        );
        ui.add_space(4.0);

        // Weighted by what each heading has to show: VELOCITY CURVE is
        // the longest label, POLYPHONY's value is two digits.
        let [poly_w, curve_w, rr_w] = card_columns(ui, [0.28, 0.38, 0.34]);
        ui.horizontal(|ui| {
            // Polyphony — voice ceiling, 1..MAX_VOICES.
            ui.vertical(|ui| {
                ui.set_width(poly_w);
                let voices = params.polyphony.value();
                global_control_head(ui, "POLYPHONY", &voices.to_string());
                let span = (MAX_VOICES - 1) as f32;
                let unit = (voices - 1) as f32 / span;
                if let Some(new_unit) = super::probed(ui, "global.polyphony", |ui| {
                    widgets::slider_unipolar(ui, poly_w, unit)
                }) {
                    params
                        .polyphony
                        .set_value(1 + (new_unit * span).round() as i32);
                }
            });
            ui.add_space(CARD_COLUMN_GAP);
            // Velocity curve — bipolar, centred on linear.
            ui.vertical(|ui| {
                ui.set_width(curve_w);
                let curve = params.velocity_curve.value();
                global_control_head(ui, "VELOCITY CURVE", &velocity::curve_label(curve));
                if let Some(new_curve) = super::probed(ui, "global.velocity_curve", |ui| {
                    widgets::slider_bipolar(ui, curve_w, curve)
                }) {
                    params.velocity_curve.set_value(new_curve);
                }
            });
            ui.add_space(CARD_COLUMN_GAP);
            // Round robin — how a layer's takes are walked.
            ui.vertical(|ui| {
                ui.set_width(rr_w);
                let mode = params.round_robin_mode.value();
                global_control_head(ui, "ROUND ROBIN", params.round_robin_mode.label());
                if let Some(picked) = super::probed(ui, "global.round_robin", |ui| {
                    widgets::segmented(ui, ROUND_ROBIN_LABELS, mode.max(0) as usize)
                }) {
                    params.round_robin_mode.set_value(picked as i32);
                }
            });
        });
    });
    super::probe(ui, "card.global", shown.response.rect);
}

/// Label + right-aligned value readout, the header every GLOBAL control
/// shares. The readout is the parameter's own text, so the card and the
/// host's automation lane can never disagree about what is set.
fn global_control_head(ui: &mut egui::Ui, label: &str, value: &str) {
    label_value_row(
        ui,
        label,
        theme::TEXT_3,
        10.0,
        &value.to_lowercase(),
        theme::TEXT_3,
        11.0,
    );
}
