//! Gate plugin editor: an egui UI hosted in `wayland-plugin-gui`.
//!
//! Deliberately plain — a title band and three captioned clusters of
//! parameter knobs. The gate has no visualisation worth the frame
//! budget: its interesting state is a single open/closed bit and a
//! gain-reduction number, both of which the host's own meters already
//! show on the channel it sits on.
//!
//! Plain is not the same as unstyled, though. Until ba todo #1275 this
//! editor installed no `egui::Visuals` at all and opened in stock egui
//! grey, the only plugin in the fleet that did; it now applies the
//! shared lavender palette ([`theme`]) once per frame and lays its
//! chrome out on the same header band + central body as its
//! neighbours (compressor, granular delay).

mod factory;
pub mod theme;
mod widgets;

pub use factory::GateEditorFactory;

use std::sync::Arc;

use wayland_plugin_gui::{egui, EditorApp};

use crate::params::{GateParams, PARAM_COUNT};

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
}

impl GateEditorApp {
    pub fn new(params: Arc<GateParams>) -> Self {
        Self { params }
    }
}

impl EditorApp for GateEditorApp {
    fn ui(&mut self, ui: &mut egui::Ui) {
        theme::apply(ui.ctx());
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(33));

        egui::Panel::top("gate_header")
            .exact_size(HEADER_H)
            .show_inside(ui, draw_header);

        egui::CentralPanel::default().show_inside(ui, |ui| draw_body(ui, &self.params));

        debug_assert_eq!(PARAM_COUNT, 8, "editor knob list is out of date");
    }
}

/// The title band: product name in the brand accent, then the one line
/// that explains what the key port does.
fn draw_header(ui: &mut egui::Ui) {
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
        ui.label(
            egui::RichText::new(
                "Keys off its own input, or off the sidechain source the host connects.",
            )
            .color(theme::TEXT_2),
        );
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
