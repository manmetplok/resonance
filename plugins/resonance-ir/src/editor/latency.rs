//! Latency cell of the bottom strip: the latency-mode picker and the
//! readout of what the plugin is actually costing the user (ba todo
//! #1300, audit finding I1).
//!
//! Two things this fixes. The convolution block size *is* the reported
//! latency, and it used to be derived from the sample rate alone: the
//! user could neither choose it nor see it, so the ~2.9 ms it spends
//! were invisible — bad enough while mixing, wrong while tracking a
//! guitar through a cabinet IR. It is now
//! [`crate::params::IrParams::latency_mode`], a real parameter, so the
//! picker here, a host automation lane and `track.set_plugin_param` are
//! the same control.
//!
//! Nothing here restates a fact about that parameter: the entries come
//! from its own choice table (`Param::choices`), their text from its own
//! `display`, and the milliseconds from [`crate::latency::readout`],
//! which converts through the same `dsp::latency_ms` the engine and the
//! tests use. `tests/editor_bindings.rs` scans for regressions.
//!
//! # Why the picker does not talk to the host itself
//!
//! Changing the block size means reallocating the bypass delay lines and
//! re-partitioning the convolver, and CLAP only lets a plugin's reported
//! latency change while it is deactivated. So the picker only writes the
//! parameter: the audio thread notices on its next block and reports the
//! new latency to the host, which cycles the plugin (deactivate →
//! reactivate) and re-reads it, and `initialize()` applies it. Pushing
//! from here instead would race that cycle — the bridge copies
//! editor-driven parameter writes into its shared atomics from
//! `process()`, so a restart serviced before the next block would
//! re-activate with the *old* value and quietly undo the user's choice.
//!
//! Until the cycle completes the readout says so, rather than pretending
//! the new mode is already in effect.

use resonance_plugin::Param;
use wayland_plugin_gui::egui;

use crate::latency;

use super::theme;
use super::IrEditorApp;

pub fn draw(ui: &mut egui::Ui, app: &IrEditorApp) {
    let param = &app.params.latency_mode;

    ui.vertical(|ui| {
        ui.add_space(6.0);
        ui.label(
            egui::RichText::new("Latency")
                .strong()
                .size(13.0)
                .color(theme::ACCENT),
        );
        ui.add_space(6.0);

        let current = param.value();
        egui::ComboBox::from_id_salt("ir_latency_mode")
            .selected_text(param.display(current as f64))
            .show_ui(ui, |ui| {
                // The entries are the parameter's own choice table, in its
                // own range order — this editor knows no mode names.
                let min = param.range().min();
                for offset in 0..param.choices().map_or(0, <[&str]>::len) {
                    let value = min + offset as i32;
                    let label = param.display(value as f64);
                    if ui.selectable_label(value == current, label).clicked() {
                        param.set_value(value);
                    }
                }
            });

        ui.add_space(6.0);
        let readout = latency::readout(&app.params, &app.viz);
        match &readout.active {
            Some(active) => {
                ui.label(egui::RichText::new(active).size(11.0).color(theme::TEXT));
            }
            None => {
                ui.label(
                    egui::RichText::new("not activated yet")
                        .size(11.0)
                        .color(theme::TEXT_DIM),
                );
            }
        }
        if let Some(pending) = &readout.pending {
            ui.label(
                egui::RichText::new(pending)
                    .size(11.0)
                    .color(theme::TEXT_DIM),
            );
            ui.label(
                egui::RichText::new("pending — applied when the host restarts the plugin")
                    .size(11.0)
                    .color(theme::TEXT_DIM),
            );
        }
    });
}
