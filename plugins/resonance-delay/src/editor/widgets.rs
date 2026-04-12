use wayland_plugin_gui::egui;
use egui::Ui;

use crate::params::DelayParams;
use super::theme;

pub fn param_knob(ui: &mut Ui, params: &DelayParams, index: usize) {
    let p = params.param_at(index);
    let val = p.get_plain() as f32;
    let min = p.min_plain() as f32;
    let max = p.max_plain() as f32;

    ui.vertical(|ui| {
        ui.set_width(65.0);
        let mut current = val;
        let slider = egui::Slider::new(&mut current, min..=max).show_value(false);
        if ui.add(slider).changed() {
            p.set_plain(current as f64);
        }
        ui.label(
            egui::RichText::new(p.display(val as f64))
                .small()
                .color(theme::TEXT),
        );
        ui.label(
            egui::RichText::new(p.name())
                .small()
                .color(theme::TEXT_DIM),
        );
    });
}
