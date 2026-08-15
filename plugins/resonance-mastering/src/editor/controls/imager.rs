//! Stereo imager control panel.

use wayland_plugin_gui::egui;

use crate::params::ImagerParams;

use super::theme;
use super::widgets;

pub fn draw(ui: &mut egui::Ui, params: &ImagerParams) {
    ui.vertical(|ui| {
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.add_space(12.0);
            ui.label(
                egui::RichText::new("Stereo Imager")
                    .strong()
                    .size(14.0)
                    .color(theme::ACCENT),
            );
        });
        ui.add_space(4.0);

        ui.horizontal(|ui| {
            ui.add_space(8.0);
            widgets::bool_checkbox(ui, &params.on, "On");
            ui.add_space(8.0);

            widgets::float_knob(ui, &params.width, "Width", "0 mono .. 2 wide");

            widgets::bool_checkbox(ui, &params.side_hpf_on, "Side HPF");
            ui.add_space(8.0);

            widgets::float_knob(ui, &params.side_hpf_freq, "HPF Freq", "keep bass mono");
        });
    });
}
