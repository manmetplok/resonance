//! Main Compressor editor app: state struct, `EditorApp` impl, header,
//! center visualisation orchestration, and preset loading.

use std::sync::Arc;

use wayland_plugin_gui::{egui, EditorApp};

use crate::params::CompressorParams;
use crate::presets::PRESETS;
use crate::viz::CompressorViz;

use super::{control_strip, curve, history, meters, theme};

// ---------------------------------------------------------------------------
// App
// ---------------------------------------------------------------------------

pub(crate) struct CompressorEditorApp {
    pub(crate) params: Arc<CompressorParams>,
    pub(crate) viz: Arc<CompressorViz>,
}

impl CompressorEditorApp {
    pub fn new(params: Arc<CompressorParams>, viz: Arc<CompressorViz>) -> Self {
        Self { params, viz }
    }
}

impl EditorApp for CompressorEditorApp {
    fn ui(&mut self, ui: &mut egui::Ui) {
        theme::apply(ui.ctx());
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(16));

        egui::Panel::top("comp_header")
            .exact_size(40.0)
            .show_inside(ui, |ui| draw_header(ui, self));

        egui::Panel::bottom("comp_strip")
            .exact_size(110.0)
            .show_inside(ui, |ui| control_strip::draw_control_strip(ui, self));

        egui::CentralPanel::default().show_inside(ui, |ui| draw_center(ui, self));
    }
}

fn draw_header(ui: &mut egui::Ui, app: &mut CompressorEditorApp) {
    ui.horizontal_centered(|ui| {
        ui.add_space(8.0);
        ui.label(
            egui::RichText::new("RESONANCE COMPRESSOR")
                .strong()
                .color(theme::ACCENT),
        );
        ui.add_space(16.0);
        ui.separator();
        ui.add_space(8.0);

        ui.label(egui::RichText::new("Preset").color(theme::TEXT_DIM));
        egui::ComboBox::from_id_salt("comp_preset_combo")
            .width(200.0)
            .selected_text("— select —")
            .show_ui(ui, |ui| {
                for entry in PRESETS {
                    if ui.selectable_label(false, entry.name).clicked() {
                        load_preset(&app.params, entry.json);
                    }
                }
            });
    });
}

fn draw_center(ui: &mut egui::Ui, app: &mut CompressorEditorApp) {
    let avail = ui.available_rect_before_wrap();
    let meter_block_width = 150.0f32;
    let gap = 8.0f32;

    // Split the center row horizontally: transfer curve (~40%),
    // GR history (~flex), and three meters on the right.
    let curve_width = avail.width() * 0.36;
    let curve_rect = egui::Rect::from_min_max(
        avail.min,
        egui::pos2(avail.min.x + curve_width, avail.max.y),
    );
    let history_rect = egui::Rect::from_min_max(
        egui::pos2(curve_rect.max.x + gap, avail.min.y),
        egui::pos2(avail.max.x - meter_block_width - gap, avail.max.y),
    );
    let meters_rect = egui::Rect::from_min_max(
        egui::pos2(avail.max.x - meter_block_width, avail.min.y),
        avail.max,
    );

    let painter = ui.painter_at(avail);

    curve::draw(
        &painter,
        curve_rect,
        curve::CurveParams {
            threshold: app.params.threshold.value(),
            ratio: app.params.ratio.value(),
            knee: app.params.knee.value(),
            makeup: app.params.makeup.value(),
            current_gr_db: app.viz.read_gr_db(),
            current_input_db: app.viz.read_input_db(),
        },
    );

    history::draw(&painter, history_rect, &app.viz);

    // Three meters side by side in meters_rect.
    let meter_gap = 6.0f32;
    let meter_w = (meters_rect.width() - 2.0 * meter_gap) / 3.0;
    let in_rect = egui::Rect::from_min_max(
        meters_rect.min,
        egui::pos2(meters_rect.min.x + meter_w, meters_rect.max.y),
    );
    let gr_rect = egui::Rect::from_min_max(
        egui::pos2(in_rect.max.x + meter_gap, meters_rect.min.y),
        egui::pos2(in_rect.max.x + meter_gap + meter_w, meters_rect.max.y),
    );
    let out_rect = egui::Rect::from_min_max(
        egui::pos2(gr_rect.max.x + meter_gap, meters_rect.min.y),
        egui::pos2(gr_rect.max.x + meter_gap + meter_w, meters_rect.max.y),
    );
    meters::draw_input_meter(&painter, in_rect, app.viz.read_input_db());
    meters::draw_gr_meter(&painter, gr_rect, app.viz.read_gr_db());
    meters::draw_output_meter(&painter, out_rect, app.viz.read_output_db());
}

fn load_preset(params: &CompressorParams, json: &str) {
    resonance_plugin::presets::load(json, crate::params::PARAM_COUNT, |i| params.param_at(i));
}
