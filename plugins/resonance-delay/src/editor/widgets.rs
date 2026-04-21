use wayland_plugin_gui::egui;
use wayland_plugin_gui::widgets;
use egui::Ui;

use crate::params::DelayParams;

pub fn param_knob(ui: &mut Ui, params: &DelayParams, index: usize) {
    let p = params.param_at(index);
    let mut val = p.get_plain() as f32;
    let min = p.min_plain() as f32;
    let max = p.max_plain() as f32;
    let default = p.default_plain() as f32;
    let display = p.display(val as f64);

    if widgets::knob(
        ui, &mut val, min..=max, default,
        p.name(), "", &display, false,
    ) {
        p.set_plain(val as f64);
    }
}
