//! Per-parameter control widgets. Every setter goes through
//! `Param::set_plain`, the same GUI→host path the delay editor uses
//! (the CLAP bridge picks the new value up and emits the host param
//! event), so host automation and editor edits stay consistent.

use egui::Ui;
use wayland_plugin_gui::egui;
use wayland_plugin_gui::widgets;

use crate::params::GranularDelayParams;

use super::theme;

/// Shared rotary knob for continuous parameters (identical to the
/// delay editor's `param_knob`).
pub fn param_knob(ui: &mut Ui, params: &GranularDelayParams, index: usize) {
    let p = params.param_at(index);
    let mut val = p.get_plain() as f32;
    let min = p.min_plain() as f32;
    let max = p.max_plain() as f32;
    let default = p.default_plain() as f32;
    let display = p.display(val as f64);

    if widgets::knob(
        ui,
        &mut val,
        min..=max,
        default,
        p.name(),
        "",
        &display,
        false,
    ) {
        p.set_plain(val as f64);
    }
}

/// Labelled toggle for boolean parameters.
pub fn param_toggle(ui: &mut Ui, params: &GranularDelayParams, index: usize) {
    let p = params.param_at(index);
    let on = p.get_plain() >= 0.5;
    ui.vertical(|ui| {
        ui.set_width(64.0);
        ui.label(
            egui::RichText::new(p.name())
                .small()
                .color(theme::TEXT_DIM),
        );
        let (text, color) = if on {
            (egui::RichText::new("ON").strong().color(theme::ACCENT), true)
        } else {
            (egui::RichText::new("off").color(theme::TEXT_DIM), false)
        };
        if ui.selectable_label(color, text).clicked() {
            p.set_plain(if on { 0.0 } else { 1.0 });
        }
    });
}

/// Labelled combo box for enumerated parameters. `labels` must be a
/// cached static list (view-performance rules: no per-frame option
/// building) covering the param's plain range `0..labels.len()`.
pub fn param_choice(
    ui: &mut Ui,
    params: &GranularDelayParams,
    index: usize,
    labels: &'static [&'static str],
) {
    let p = params.param_at(index);
    let current = (p.get_plain().round() as usize).min(labels.len().saturating_sub(1));
    ui.vertical(|ui| {
        ui.label(
            egui::RichText::new(p.name())
                .small()
                .color(theme::TEXT_DIM),
        );
        egui::ComboBox::from_id_salt(p.id())
            .width(96.0)
            .selected_text(labels[current])
            .show_ui(ui, |ui| {
                for (i, label) in labels.iter().enumerate() {
                    if ui.selectable_label(i == current, *label).clicked() {
                        p.set_plain(i as f64);
                    }
                }
            });
    });
}
