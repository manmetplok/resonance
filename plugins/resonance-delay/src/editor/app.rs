//! The actual egui app: state and update/view orchestration for the delay editor.
//!
//! `DelayEditorApp` is the `EditorApp` the runtime drives each frame. It paints
//! the header (title + preset picker + readouts + freeze indicator), the bottom
//! control strip, and dispatches the centre to the echo view.

use std::sync::Arc;

use wayland_plugin_gui::{egui, EditorApp};

use crate::params::{DelayParams, PARAM_COUNT};
use crate::presets::PRESETS;
use crate::sync::DIVISION_LABELS;
use crate::viz::DelayViz;

use super::{controls, echo_view, theme};

pub(crate) struct DelayEditorApp {
    pub(crate) params: Arc<DelayParams>,
    pub(crate) viz: Arc<DelayViz>,
}

impl DelayEditorApp {
    pub fn new(params: Arc<DelayParams>, viz: Arc<DelayViz>) -> Self {
        Self { params, viz }
    }
}

impl EditorApp for DelayEditorApp {
    fn ui(&mut self, ui: &mut egui::Ui) {
        theme::apply(ui.ctx());
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(16));

        egui::Panel::top("delay_header")
            .exact_size(42.0)
            .show_inside(ui, |ui| draw_header(ui, self));

        egui::Panel::bottom("delay_strip")
            .exact_size(180.0)
            .show_inside(ui, |ui| controls::draw(ui, &self.params));

        egui::CentralPanel::default().show_inside(ui, |ui| draw_center(ui, self));
    }
}

fn draw_header(ui: &mut egui::Ui, app: &mut DelayEditorApp) {
    ui.horizontal_centered(|ui| {
        ui.add_space(12.0);
        ui.label(
            egui::RichText::new("RESONANCE DELAY")
                .strong()
                .color(theme::ACCENT),
        );
        ui.add_space(16.0);
        ui.separator();
        ui.add_space(8.0);

        ui.label(egui::RichText::new("Preset").color(theme::TEXT_DIM));
        egui::ComboBox::from_id_salt("delay_preset_combo")
            .width(180.0)
            .selected_text("— select —")
            .show_ui(ui, |ui| {
                for entry in PRESETS {
                    if ui.selectable_label(false, entry.name).clicked() {
                        load_preset(&app.params, entry.json);
                    }
                }
            });

        ui.add_space(16.0);
        ui.separator();
        ui.add_space(8.0);

        let bpm = app.viz.read_bpm();
        if bpm > 0.0 {
            ui.label(egui::RichText::new(format!("{bpm:.1} BPM")).color(theme::TEXT));
            ui.add_space(12.0);
        }

        let delay_ms = app.viz.read_delay_time_ms();
        ui.label(egui::RichText::new(format!("{delay_ms:.1} ms")).color(theme::TEXT_DIM));

        if app.params.sync.value() {
            let div = app.params.division.value() as usize;
            if let Some(label) = DIVISION_LABELS.get(div) {
                ui.add_space(8.0);
                ui.label(egui::RichText::new(*label).color(theme::ACCENT));
            }
        }

        // Character + Routing readout.
        ui.add_space(12.0);
        let char_label = if app.params.character.value() == 1 {
            "Analog"
        } else {
            "Digital"
        };
        let route_label = match app.params.routing.value() {
            1 => "Ping-Pong",
            2 => "Dual",
            _ => "Stereo",
        };
        ui.label(
            egui::RichText::new(format!("{char_label} · {route_label}")).color(theme::TEXT_DIM),
        );

        // Freeze indicator.
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

fn draw_center(ui: &mut egui::Ui, app: &mut DelayEditorApp) {
    let avail = ui.available_rect_before_wrap();
    let gap = 8.0f32;
    let viz_rect = egui::Rect::from_min_max(
        egui::pos2(avail.left() + gap, avail.top() + gap),
        egui::pos2(avail.right() - gap, avail.bottom() - gap),
    );
    let painter = ui.painter_at(avail);
    echo_view::draw(&painter, viz_rect, &app.viz);
}

fn load_preset(params: &DelayParams, json: &str) {
    resonance_plugin::presets::load(json, PARAM_COUNT, |i| params.param_at(i));
}
