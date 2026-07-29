//! Per-parameter control widgets. Every setter goes through
//! `Param::set_plain`, the same GUI→host path the delay editor uses
//! (the CLAP bridge picks the new value up and emits the host param
//! event), so host automation and editor edits stay consistent.
//!
//! Knobs use the shared lavender helpers from
//! `wayland_plugin_gui::widgets` (ba todo #1136: the crate renders in
//! lavender tokens only).

use egui::Ui;
use wayland_plugin_gui::egui;
use wayland_plugin_gui::widgets;

use crate::params::GranularDelayParams;

use super::theme;

/// Shared rotary knob for continuous parameters: the lavender themed
/// knob, bipolar (centre-out arc) when the param range spans zero.
pub fn param_knob(ui: &mut Ui, params: &GranularDelayParams, index: usize) {
    let p = params.param_at(index);
    let min = p.min_plain() as f32;
    let max = p.max_plain() as f32;
    let span = (max - min).max(f32::EPSILON);
    let val = p.get_plain() as f32;
    let default = p.default_plain() as f32;
    let display = p.display(val as f64);

    let bipolar = min < 0.0 && max > 0.0;
    let new_plain = if bipolar {
        widgets::knob_bipolar(
            ui,
            p.name(),
            ((val - min) / span) * 2.0 - 1.0,
            &display,
            ((default - min) / span) * 2.0 - 1.0,
        )
        .map(|u| min + (u + 1.0) * 0.5 * span)
    } else {
        widgets::knob_unipolar(
            ui,
            p.name(),
            (val - min) / span,
            &display,
            (default - min) / span,
        )
        .map(|u| min + u * span)
    };
    if let Some(v) = new_plain {
        p.set_plain(f64::from(v.clamp(min, max)));
    }
}

/// Labelled toggle for boolean parameters.
pub fn param_toggle(ui: &mut Ui, params: &GranularDelayParams, index: usize) {
    let p = params.param_at(index);
    let on = p.get_plain() >= 0.5;
    ui.vertical(|ui| {
        ui.set_width(64.0);
        ui.label(egui::RichText::new(p.name()).small().color(theme::TEXT_3));
        let (text, color) = if on {
            (egui::RichText::new("ON").strong().color(theme::ACCENT), true)
        } else {
            (egui::RichText::new("off").color(theme::TEXT_3), false)
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
        ui.label(egui::RichText::new(p.name()).small().color(theme::TEXT_3));
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
