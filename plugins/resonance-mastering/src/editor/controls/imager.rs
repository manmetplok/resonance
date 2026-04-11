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
            widgets::control_column(ui, "On", "enable", |ui| {
                widgets::bool_checkbox(ui, &params.on, "");
            });
            widgets::control_column(ui, "Width", "0 mono .. 2 wide", |ui| {
                widgets::float_slider(ui, &params.width, 0.0..=2.0, 2, "");
            });
            widgets::control_column(ui, "Side HPF", "enable", |ui| {
                widgets::bool_checkbox(ui, &params.side_hpf_on, "");
            });
            widgets::control_column(ui, "HPF Freq", "keep bass mono", |ui| {
                widgets::float_slider_log(ui, &params.side_hpf_freq, 20.0..=400.0, 0, " Hz");
            });
        });
    });
}
