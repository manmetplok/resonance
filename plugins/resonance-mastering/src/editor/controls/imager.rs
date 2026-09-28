//! Stereo imager control panel.

use plugin_gui_core::egui;

use crate::params::ImagerParams;

use super::multiband::BAND_NAMES;
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

        // Per-band width on the multiband's crossover bands (whose
        // splits are set on the Multiband tab).
        ui.horizontal(|ui| {
            ui.add_space(8.0);
            ui.label(
                egui::RichText::new("Per band")
                    .size(10.0)
                    .color(theme::TEXT_DIM),
            );
            ui.add_space(8.0);
            for (b, name) in BAND_NAMES.iter().enumerate() {
                widgets::float_knob(ui, &params.band_width[b], name, "multiband split");
            }
        });
    });
}
