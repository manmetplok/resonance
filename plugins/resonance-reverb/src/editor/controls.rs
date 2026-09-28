//! The control strip at the bottom of the reverb editor.
//! Uses rotary knobs in a grid layout for compact, consistent display.
//!
//! Every knob is built straight from its `FloatParam` (ba todo #1281):
//! range, default, skew and the value readout come from `params.rs`, so
//! the four knobs that had drifted from their own declarations cannot
//! drift again.
//!
//! The third row is the return channel (warmth-width-depth.md §6.4):
//! the wet HPF/LPF before the tank, the ducker with a readout of what it
//! is keyed from and how far it is pulling the return down, and the
//! ER/tail depth balance.

use resonance_plugin::editor_widgets;
use plugin_gui_core::egui;

use crate::params::ReverbParams;
use crate::viz::ReverbViz;

use super::theme;

pub fn draw(ui: &mut egui::Ui, params: &ReverbParams, viz: &ReverbViz) {
    ui.vertical(|ui| {
        ui.add_space(4.0);
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

        // Two rows of 6 knobs each.
        ui.horizontal(|ui| {
            ui.add_space(8.0);
            editor_widgets::float_knob(ui, &params.predelay, "Pre-delay", "before tail");
            ui.add_space(4.0);
            editor_widgets::float_knob(ui, &params.er_level, "ER Level", "early refl.");
            ui.add_space(4.0);
            editor_widgets::float_knob(ui, &params.er_time, "ER Time", "tap spread");
            ui.add_space(4.0);
            editor_widgets::float_knob(ui, &params.size, "Size", "");
            ui.add_space(4.0);
            editor_widgets::float_knob(ui, &params.decay, "Decay", "RT60");
            ui.add_space(4.0);
            editor_widgets::float_knob(ui, &params.damping, "Damping", "HF cutoff");
        });

        ui.add_space(2.0);

        ui.horizontal(|ui| {
            ui.add_space(8.0);
            editor_widgets::float_knob(ui, &params.diffusion, "Diffusion", "");
            ui.add_space(4.0);
            editor_widgets::float_knob(ui, &params.mod_rate, "Mod Rate", "chorus");
            ui.add_space(4.0);
            editor_widgets::float_knob(ui, &params.mod_depth, "Mod Depth", "");
            ui.add_space(4.0);
            editor_widgets::float_knob(ui, &params.width, "Width", "stereo");
            ui.add_space(4.0);
            editor_widgets::float_knob(ui, &params.mix, "Mix", "dry/wet");
            ui.add_space(4.0);

            // Freeze is a toggle, not a knob — render as a checkbox.
            ui.vertical(|ui| {
                ui.add_space(16.0);
                editor_widgets::bool_checkbox(ui, &params.freeze, "Freeze");
            });
        });

        ui.add_space(2.0);

        // Return channel: return EQ, ducking, depth.
        ui.horizontal(|ui| {
            ui.add_space(8.0);
            ui.vertical(|ui| {
                ui.add_space(10.0);
                editor_widgets::bool_checkbox(ui, &params.wet_hpf_on, "Wet HPF");
                editor_widgets::bool_checkbox(ui, &params.wet_lpf_on, "Wet LPF");
                ui.add_space(2.0);
                editor_widgets::int_choice(ui, &params.wet_filter_slope, 86.0);
            });
            ui.add_space(4.0);
            editor_widgets::float_knob(ui, &params.wet_hpf_freq, "HPF", "before tank");
            ui.add_space(4.0);
            editor_widgets::float_knob(ui, &params.wet_lpf_freq, "LPF", "before tank");
            ui.add_space(12.0);
            ui.separator();
            ui.add_space(8.0);
            editor_widgets::float_knob(ui, &params.duck_amount, "Duck", "wet return");
            ui.add_space(4.0);
            editor_widgets::float_knob(ui, &params.duck_threshold, "Threshold", "");
            ui.add_space(4.0);
            editor_widgets::float_knob(ui, &params.duck_attack, "Attack", "");
            ui.add_space(4.0);
            editor_widgets::float_knob(ui, &params.duck_release, "Release", "");
            ui.add_space(4.0);
            ui.vertical(|ui| {
                ui.add_space(16.0);
                // Name the detector, so a return that ducks while the
                // track's own input is idle has an explanation on screen.
                let source = if viz.key_connected() {
                    "key: sidechain"
                } else {
                    "key: dry input"
                };
                ui.label(egui::RichText::new(source).color(theme::TEXT_DIM));
                let gr = viz.duck_gr_db();
                let text = if gr > 0.05 {
                    format!("GR -{gr:.1} dB")
                } else {
                    "GR 0.0 dB".to_string()
                };
                ui.label(egui::RichText::new(text).color(theme::TEXT));
            });
            ui.add_space(12.0);
            ui.separator();
            ui.add_space(8.0);
            editor_widgets::float_knob(ui, &params.er_tail_balance, "ER / Tail", "depth");
        });
    });
}
