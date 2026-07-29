//! The egui app: the three-band shell of the redesigned editor
//! (ba todo #1136, design doc #264) — a 46 px header, the central hero
//! buffer-view band (~400 px, `hero.rs`, ba todo #1139) and the 246 px
//! control strip.
//!
//! `GranularEditorApp` is the `EditorApp` the runtime drives each
//! frame. The header carries the title and live readouts (effective
//! delay, tempo + division, voice-lock, grain count, freeze), all from
//! the shared [`GranularViz`] atomics — the editor never touches the
//! DSP state.

use std::sync::Arc;

use wayland_plugin_gui::{egui, EditorApp};

use crate::params::GranularDelayParams;
use crate::sync::DIVISION_LABELS;
use crate::viz::GranularViz;

use super::{controls, hero, theme};

/// Band heights of the three-band layout (design doc #264).
const HEADER_H: f32 = 46.0;
const STRIP_H: f32 = 246.0;

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
            .exact_size(HEADER_H)
            .show_inside(ui, |ui| draw_header(ui, self));

        egui::Panel::bottom("granular_strip")
            .exact_size(STRIP_H)
            .show_inside(ui, |ui| {
                // The strip rework (signal-flow layout) is a separate
                // todo; until then the existing grouped surface may
                // overflow the band — keep it reachable via scroll.
                egui::ScrollArea::vertical().show(ui, |ui| {
                    ui.add_space(6.0);
                    controls::draw(ui, &self.params);
                });
            });

        egui::CentralPanel::default().show_inside(ui, |ui| {
            // Hero buffer view (ba todo #1139): the returned layout
            // carries the tap hit-rect for the direct-manipulation
            // todo.
            let layout = hero::draw(ui, &self.params, &self.viz);
            ui.allocate_rect(layout.canvas, egui::Sense::hover());
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
            ui.label(egui::RichText::new(format!("{bpm:.1} BPM")).color(theme::TEXT_1));
            ui.add_space(12.0);
        }
        let delay_ms = app.viz.read_delay_ms();
        ui.label(egui::RichText::new(format!("{delay_ms:.1} ms")).color(theme::TEXT_2));
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
                (theme::GOOD, format!("VOICE {hz:.0} Hz"))
            } else {
                (theme::TEXT_3, "voice —".to_string())
            };
            ui.label(egui::RichText::new(text).color(color));
            let voices = app.viz.read_psola_voices();
            if voices > 0 {
                ui.add_space(6.0);
                ui.label(egui::RichText::new(format!("{voices}v")).color(theme::TEXT_2));
            }
        }

        // Grain-count readout.
        ui.add_space(12.0);
        let grains = app.viz.read_active_grains();
        ui.label(egui::RichText::new(format!("{grains} grains")).color(theme::TEXT_2));

        // Freeze indicator (right-aligned; warm amber = the audio-domain
        // freeze token, design doc #264).
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.add_space(12.0);
            let frozen = app.params.freeze.value();
            let (dot_color, text_color, label) = if frozen {
                (theme::WARM, theme::WARM, "FREEZE")
            } else {
                (theme::LINE, theme::TEXT_3, "freeze")
            };
            ui.label(egui::RichText::new(label).strong().color(text_color));
            ui.add_space(4.0);
            let (rect, _) = ui.allocate_exact_size(egui::vec2(10.0, 10.0), egui::Sense::hover());
            ui.painter().circle_filled(rect.center(), 5.0, dot_color);
        });
    });
}
