//! The horizontal control strip at the bottom of the reverb editor.
//! A single row of `control_column`s, one per parameter, matching the
//! compressor / mastering family aesthetic.

use wayland_plugin_gui::egui;

use crate::params::ReverbParams;

use super::theme;
use super::widgets;

pub fn draw(ui: &mut egui::Ui, params: &ReverbParams) {
    ui.vertical(|ui| {
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.add_space(12.0);
            ui.label(
                egui::RichText::new("Reverb Controls")
                    .strong()
                    .size(13.0)
                    .color(theme::ACCENT),
            );
        });
        ui.add_space(4.0);

        ui.horizontal(|ui| {
            ui.add_space(8.0);

            widgets::control_column(ui, "Pre-delay", "before tail", |ui| {
                widgets::float_slider(ui, &params.predelay, 0.0..=250.0, 1, " ms");
            });
            widgets::control_column(ui, "ER Level", "early refl.", |ui| {
                widgets::float_slider(ui, &params.er_level, 0.0..=1.0, 2, "");
            });
            widgets::control_column(ui, "ER Time", "tap spread", |ui| {
                widgets::float_slider(ui, &params.er_time, 0.0..=1.0, 2, "");
            });
            widgets::control_column(ui, "Size", "", |ui| {
                widgets::float_slider(ui, &params.size, 0.0..=1.0, 2, "");
            });
            widgets::control_column(ui, "Decay", "RT60", |ui| {
                widgets::float_slider_log(ui, &params.decay, 0.1..=30.0, 2, " s");
            });
            widgets::control_column(ui, "Damping", "HF cutoff", |ui| {
                widgets::float_slider_log(ui, &params.damping, 200.0..=20000.0, 0, " Hz");
            });
            widgets::control_column(ui, "Diffusion", "", |ui| {
                widgets::float_slider(ui, &params.diffusion, 0.0..=1.0, 2, "");
            });
            widgets::control_column(ui, "Mod Rate", "chorus", |ui| {
                widgets::float_slider(ui, &params.mod_rate, 0.0..=5.0, 2, " Hz");
            });
            widgets::control_column(ui, "Mod Depth", "", |ui| {
                widgets::float_slider(ui, &params.mod_depth, 0.0..=1.0, 2, "");
            });
            widgets::control_column(ui, "Width", "stereo", |ui| {
                widgets::float_slider(ui, &params.width, 0.0..=1.0, 2, "");
            });
            widgets::control_column(ui, "Mix", "dry/wet", |ui| {
                widgets::float_slider(ui, &params.mix, 0.0..=1.0, 2, "");
            });
            widgets::control_column(ui, "Freeze", "infinite tail", |ui| {
                widgets::bool_checkbox(ui, &params.freeze, "");
            });
        });
    });
}
