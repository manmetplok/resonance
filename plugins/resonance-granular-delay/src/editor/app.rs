//! The egui app: header readouts + the grouped control surface.
//!
//! `GranularEditorApp` is the `EditorApp` the runtime drives each
//! frame. The header mirrors the delay editor's: title, live readouts
//! (effective delay, tempo + division, voice-lock, grain count) and a
//! right-aligned freeze indicator. All readouts come from the shared
//! [`GranularViz`] atomics — the editor never touches the DSP state.

use std::sync::Arc;

use wayland_plugin_gui::{egui, EditorApp};

use crate::params::GranularDelayParams;
use crate::sync::DIVISION_LABELS;
use crate::viz::GranularViz;

use super::{controls, theme};

pub(crate) struct GranularEditorApp {
    pub(crate) params: Arc<GranularDelayParams>,
    pub(crate) viz: Arc<GranularViz>,
}

impl GranularEditorApp {
    pub fn new(params: Arc<GranularDelayParams>, viz: Arc<GranularViz>) -> Self {
        Self { params, viz }
    }
}

impl EditorApp for GranularEditorApp {
    fn ui(&mut self, ui: &mut egui::Ui) {
        theme::apply(ui.ctx());
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(16));

        egui::Panel::top("granular_header")
            .exact_size(42.0)
            .show_inside(ui, |ui| draw_header(ui, self));

        egui::CentralPanel::default().show_inside(ui, |ui| {
            ui.add_space(6.0);
            controls::draw(ui, &self.params);
        });
    }
}

fn draw_header(ui: &mut egui::Ui, app: &mut GranularEditorApp) {
    ui.horizontal_centered(|ui| {
        ui.add_space(12.0);
        ui.label(
            egui::RichText::new("RESONANCE GRANULAR DELAY")
                .strong()
                .color(theme::ACCENT),
        );
        ui.add_space(16.0);
        ui.separator();
        ui.add_space(8.0);

        // Tempo + effective-delay readout (the Repitch glide / Fade
        // commit value, ba todo #1076).
        let bpm = app.viz.read_bpm();
        if bpm > 0.0 {
            ui.label(egui::RichText::new(format!("{bpm:.1} BPM")).color(theme::TEXT));
            ui.add_space(12.0);
        }
        let delay_ms = app.viz.read_delay_ms();
        ui.label(egui::RichText::new(format!("{delay_ms:.1} ms")).color(theme::TEXT_DIM));
        if app.params.sync.value() {
            let div = app.params.division.value() as usize;
            if let Some(label) = DIVISION_LABELS.get(div) {
                ui.add_space(8.0);
                ui.label(egui::RichText::new(*label).color(theme::ACCENT));
            }
        }

        // Voice-lock indicator (ba todo #1082): shown while the
        // Pitch-Sync scheduler is selected; lit with the tracked
        // fundamental while the PSOLA path is engaged.
        if app.params.scheduler.value() == 2 {
            ui.add_space(12.0);
            ui.separator();
            ui.add_space(8.0);
            let engaged = app.viz.read_engaged();
            let hz = app.viz.read_period_hz();
            let (color, text) = if engaged && hz > 0.0 {
                (theme::ACCENT, format!("VOICE {hz:.0} Hz"))
            } else {
                (theme::TEXT_DIM, "voice —".to_string())
            };
            ui.label(egui::RichText::new(text).color(color));
            let voices = app.viz.read_psola_voices();
            if voices > 0 {
                ui.add_space(6.0);
                ui.label(
                    egui::RichText::new(format!("{voices}v")).color(theme::TEXT_DIM),
                );
            }
        }

        // Grain-count readout.
        ui.add_space(12.0);
        let grains = app.viz.read_active_grains();
        ui.label(egui::RichText::new(format!("{grains} grains")).color(theme::TEXT_DIM));

        // Freeze indicator (right-aligned, like the delay editor).
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.add_space(12.0);
            let frozen = app.params.freeze.value();
            let (dot_color, text_color, label) = if frozen {
                (theme::ACCENT, theme::ACCENT, "FREEZE")
            } else {
                (theme::BORDER, theme::TEXT_DIM, "freeze")
            };
            ui.label(egui::RichText::new(label).strong().color(text_color));
            ui.add_space(4.0);
            let (rect, _) = ui.allocate_exact_size(egui::vec2(10.0, 10.0), egui::Sense::hover());
            ui.painter().circle_filled(rect.center(), 5.0, dot_color);
        });
    });
}
