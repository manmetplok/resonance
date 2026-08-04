//! Gate plugin editor: an egui UI hosted in `wayland-plugin-gui`.
//!
//! Deliberately plain — a title row and one strip of parameter knobs.
//! The gate has no visualisation worth the frame budget: its interesting
//! state is a single open/closed bit and a gain-reduction number, both of
//! which the host's own meters already show on the channel it sits on.

mod factory;
mod widgets;

pub use factory::GateEditorFactory;

use std::sync::Arc;

use wayland_plugin_gui::{egui, EditorApp};

use crate::params::{GateParams, PARAM_COUNT};

use widgets::param_knob;

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
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(33));

        ui.vertical(|ui| {
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                ui.add_space(12.0);
                ui.heading("Resonance Gate");
            });
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.add_space(12.0);
                ui.label(
                    "Keys off its own input, or off the sidechain source the host connects.",
                );
            });
            ui.add_space(12.0);

            ui.horizontal(|ui| {
                ui.add_space(8.0);
                // Detection: what opens the gate.
                ui.group(|ui| {
                    ui.horizontal(|ui| {
                        param_knob(ui, &self.params, 0); // threshold
                        param_knob(ui, &self.params, 6); // hysteresis
                        param_knob(ui, &self.params, 7); // key_hpf
                    });
                });
                ui.add_space(6.0);
                // Timing: how it opens and closes.
                ui.group(|ui| {
                    ui.horizontal(|ui| {
                        param_knob(ui, &self.params, 2); // attack
                        param_knob(ui, &self.params, 3); // hold
                        param_knob(ui, &self.params, 4); // release
                    });
                });
                ui.add_space(6.0);
                // Depth: how hard it closes.
                ui.group(|ui| {
                    ui.horizontal(|ui| {
                        param_knob(ui, &self.params, 1); // ratio
                        param_knob(ui, &self.params, 5); // range
                    });
                });
            });
        });

        debug_assert_eq!(PARAM_COUNT, 8, "editor knob list is out of date");
    }
}
