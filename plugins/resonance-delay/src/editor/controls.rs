use wayland_plugin_gui::egui;

use super::widgets::param_knob;
use crate::params::DelayParams;

pub fn draw(ui: &mut egui::Ui, params: &DelayParams) {
    ui.horizontal(|ui| {
        ui.add_space(8.0);
        ui.group(|ui| {
            ui.horizontal(|ui| {
                param_knob(ui, params, 0); // sync
                param_knob(ui, params, 1); // division
                param_knob(ui, params, 2); // time_ms
            });
        });
        ui.add_space(4.0);
        ui.group(|ui| {
            ui.horizontal(|ui| {
                param_knob(ui, params, 3); // feedback
                param_knob(ui, params, 4); // mix
            });
        });
        ui.add_space(4.0);
        ui.group(|ui| {
            ui.horizontal(|ui| {
                param_knob(ui, params, 5); // character
                param_knob(ui, params, 6); // routing
                param_knob(ui, params, 7); // stereo_offset
            });
        });
        ui.add_space(4.0);
        ui.group(|ui| {
            ui.horizontal(|ui| {
                param_knob(ui, params, 8); // hi_cut
                param_knob(ui, params, 9); // lo_cut
                param_knob(ui, params, 10); // drive
            });
        });
        ui.add_space(4.0);
        ui.group(|ui| {
            ui.horizontal(|ui| {
                param_knob(ui, params, 11); // mod_rate
                param_knob(ui, params, 12); // mod_depth
            });
        });
        ui.add_space(4.0);
        param_knob(ui, params, 13); // freeze
    });
}
