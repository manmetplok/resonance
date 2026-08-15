//! Gate plugin editor: an egui UI hosted in `wayland-plugin-gui`.
//!
//! Deliberately plain — a title band with the factory-preset combo and
//! the detector readout, then three captioned clusters of parameter
//! knobs. The gate has no *signal* visualisation worth the frame
//! budget; the host's own meters already show the level on the channel
//! it sits on. What the host cannot show is the detector — which
//! signal it is reading, and what it is doing with it — which is the
//! whole plugin (see [`status`], ba todo #1314).
//!
//! Plain is not the same as unstyled, though. Until ba todo #1275 this
//! editor installed no `egui::Visuals` at all and opened in stock egui
//! grey, the only plugin in the fleet that did; it now applies the
//! shared lavender palette ([`theme`]) once per frame and lays its
//! chrome out on the same header band + central body as its
//! neighbours (compressor, granular delay).

mod factory;
pub mod status;
pub mod theme;
mod widgets;

pub use factory::GateEditorFactory;

use std::sync::Arc;

use wayland_plugin_gui::{egui, EditorApp};

use crate::params::{GateParams, PARAM_COUNT};
use crate::presets::{load_preset, PRESETS};
use crate::viz::GateViz;

use status::DetectorSummary;
use widgets::param_knob;

/// Height of the title band, in points. The fleet's header bands run
/// 40–46 px (compressor 40, granular delay 46); the gate takes the
/// same 46 px band as the most recently designed editor.
pub const HEADER_H: f32 = 46.0;

/// One captioned cluster of knobs in the body.
pub struct KnobGroup {
    /// Caption drawn above the cluster.
    pub caption: &'static str,
    /// Indices into [`GateParams::param_at`].
    pub params: &'static [usize],
}

/// The clusters, in the order the signal meets them: what opens the
/// gate, how fast it moves, how hard it closes.
///
/// Covers every declared parameter exactly once — asserted by
/// `tests/editor_layout.rs`, which is what keeps this table honest when
/// a parameter is added.
pub const GROUPS: &[KnobGroup] = &[
    KnobGroup {
        caption: "DETECTION",
        // threshold, hysteresis, key_hpf
        params: &[0, 6, 7],
    },
    KnobGroup {
        caption: "TIMING",
        // attack, hold, release
        params: &[2, 3, 4],
    },
    KnobGroup {
        caption: "DEPTH",
        // ratio, range
        params: &[1, 5],
    },
];

pub(crate) struct GateEditorApp {
    pub(crate) params: Arc<GateParams>,
    /// Detector status published by the audio thread (ba todo #1314).
    pub(crate) viz: Arc<GateViz>,
    /// Index into [`crate::presets::PRESETS`] of the preset last loaded
    /// from the header combo. Display only — the combo shows what was
    /// loaded rather than a permanent placeholder (ba todo #1280);
    /// editing a knob afterwards does not clear it, because the plugin
    /// has no way to tell a user edit from a host automation write.
    selected_preset: Option<usize>,
}

impl GateEditorApp {
    pub fn new(params: Arc<GateParams>, viz: Arc<GateViz>) -> Self {
        Self {
            params,
            viz,
            selected_preset: None,
        }
    }
}

impl EditorApp for GateEditorApp {
    fn ui(&mut self, ui: &mut egui::Ui) {
        theme::apply(ui.ctx());
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(33));

        egui::Panel::top("gate_header")
            .exact_size(HEADER_H)
            .show_inside(ui, |ui| draw_header(ui, self));

        egui::CentralPanel::default().show_inside(ui, |ui| draw_body(ui, &self.params));

        debug_assert_eq!(PARAM_COUNT, 8, "editor knob list is out of date");
    }
}

/// The title band: product name in the brand accent, the factory preset
/// combo — the same affordance, in the same place, as the compressor's
/// and the granular delay's — and, right-aligned, the live detector
/// readout.
fn draw_header(ui: &mut egui::Ui, app: &mut GateEditorApp) {
    ui.horizontal_centered(|ui| {
        ui.add_space(12.0);
        ui.label(
            egui::RichText::new("RESONANCE GATE")
                .strong()
                .color(theme::ACCENT),
        );
        ui.add_space(14.0);
        ui.separator();
        ui.add_space(8.0);

        ui.label(egui::RichText::new("Preset").color(theme::TEXT_3));
        let selected_text = app
            .selected_preset
            .and_then(|i| PRESETS.get(i))
            .map_or("— preset —", |e| e.name);
        egui::ComboBox::from_id_salt("gate_preset_combo")
            .width(200.0)
            .selected_text(selected_text)
            .show_ui(ui, |ui| {
                for (i, entry) in PRESETS.iter().enumerate() {
                    let selected = app.selected_preset == Some(i);
                    if ui.selectable_label(selected, entry.name).clicked()
                        && load_preset(&app.params, entry.json)
                    {
                        app.selected_preset = Some(i);
                    }
                }
            });

        // Right-aligned: which detector is running and what it is doing.
        status::draw(ui, &DetectorSummary::from_viz(&app.viz));
    });
}

fn draw_body(ui: &mut egui::Ui, params: &GateParams) {
    ui.add_space(8.0);
    ui.horizontal(|ui| {
        ui.add_space(4.0);
        for group in GROUPS {
            draw_group(ui, params, group);
            ui.add_space(8.0);
        }
    });
    ui.add_space(10.0);
    ui.horizontal(|ui| {
        ui.add_space(8.0);
        ui.label(
            egui::RichText::new(
                "Keys off its own input, or off the sidechain source the host connects.",
            )
            .color(theme::TEXT_3)
            .size(11.0),
        );
    });
}

/// One captioned card of knobs, in the design system's panel chrome.
fn draw_group(ui: &mut egui::Ui, params: &GateParams, group: &KnobGroup) {
    let frame = egui::Frame::default()
        .fill(theme::BG_2)
        .stroke(egui::Stroke::new(1.0, theme::LINE_2))
        .corner_radius(theme::RADIUS_PANEL)
        .inner_margin(egui::Margin::symmetric(12, 10));
    frame.show(ui, |ui| {
        ui.vertical(|ui| {
            ui.label(
                egui::RichText::new(group.caption)
                    .color(theme::TEXT_3)
                    .size(10.5)
                    .strong(),
            );
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                for &index in group.params {
                    param_knob(ui, params, index);
                }
            });
        });
    });
}
