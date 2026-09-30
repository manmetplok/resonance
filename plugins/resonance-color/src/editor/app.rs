//! Main Color editor app: state, `EditorApp` impl, header and the
//! centre visualisation row.

use std::sync::Arc;

use plugin_gui_core::{egui, EditorApp};
use resonance_plugin::preset_ui::preset_bar;
use resonance_plugin::presets::{PresetBank, PresetEditor, PresetSession};
use resonance_plugin::Param;

use crate::dsp::Settings;
use crate::params::ColorParams;
use crate::viz::ColorViz;

use super::{controls, curve, harmonics, theme};

pub(crate) struct ColorEditorApp {
    pub(crate) params: Arc<ColorParams>,
    pub(crate) viz: Arc<ColorViz>,
    pub(crate) bank: PresetBank,
    pub(crate) presets: Arc<PresetSession>,
    pub(crate) preset_editor: PresetEditor,
    curve_cache: curve::CurveCache,
    probe_cache: harmonics::ProbeCache,
}

impl ColorEditorApp {
    pub fn new(params: Arc<ColorParams>, viz: Arc<ColorViz>, presets: Arc<PresetSession>) -> Self {
        Self {
            params,
            viz,
            bank: PresetBank::for_plugin::<crate::ResonanceColor>(),
            presets,
            preset_editor: PresetEditor::default(),
            curve_cache: curve::CurveCache::default(),
            probe_cache: harmonics::ProbeCache::default(),
        }
    }
}

impl EditorApp for ColorEditorApp {
    fn ui(&mut self, ui: &mut egui::Ui) {
        theme::apply(ui.ctx());
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(16));

        egui::Panel::top("color_header")
            .exact_size(40.0)
            .show_inside(ui, |ui| draw_header(ui, self));

        egui::Panel::bottom("color_strip")
            .exact_size(150.0)
            .show_inside(ui, |ui| controls::draw_control_strip(ui, &self.params));

        egui::CentralPanel::default().show_inside(ui, |ui| draw_center(ui, self));
    }
}

fn draw_header(ui: &mut egui::Ui, app: &mut ColorEditorApp) {
    ui.horizontal_centered(|ui| {
        ui.add_space(8.0);
        ui.label(
            egui::RichText::new("RESONANCE COLOR")
                .strong()
                .color(theme::ACCENT),
        );
        ui.add_space(16.0);
        ui.separator();
        ui.add_space(8.0);
        ui.label(egui::RichText::new("Preset").color(theme::TEXT_DIM));
        let params: Vec<&dyn Param> = app.params.all();
        preset_bar(
            ui,
            "color_preset",
            &mut app.preset_editor,
            &app.bank,
            &app.presets,
            &params,
            "— select —",
        );
    });
}

const METER_W: f32 = 40.0;
const METER_GAP: f32 = 6.0;

fn draw_center(ui: &mut egui::Ui, app: &mut ColorEditorApp) {
    let avail = ui.available_rect_before_wrap();
    let gap = 8.0f32;
    let settings = Settings::from_params(&app.params);

    let meters_w = 2.0 * METER_W + METER_GAP;
    let curve_w = (avail.width() - meters_w - 2.0 * gap) * 0.45;
    let curve_rect = egui::Rect::from_min_max(
        avail.min,
        egui::pos2(avail.min.x + curve_w, avail.max.y),
    );
    let bars_rect = egui::Rect::from_min_max(
        egui::pos2(curve_rect.max.x + gap, avail.min.y),
        egui::pos2(avail.max.x - meters_w - gap, avail.max.y),
    );
    let meters_rect = egui::Rect::from_min_max(
        egui::pos2(avail.max.x - meters_w, avail.min.y),
        avail.max,
    );

    let painter = ui.painter_at(avail);
    let points = app.curve_cache.points(&settings);
    curve::draw(&painter, curve_rect, points, app.viz.input_db());

    let signature = app.probe_cache.signature(&settings);
    harmonics::draw(&painter, bars_rect, signature);

    let in_rect = egui::Rect::from_min_max(
        meters_rect.min,
        egui::pos2(meters_rect.min.x + METER_W, meters_rect.max.y - 20.0),
    );
    let out_rect = egui::Rect::from_min_max(
        egui::pos2(meters_rect.min.x + METER_W + METER_GAP, meters_rect.min.y),
        egui::pos2(meters_rect.max.x, meters_rect.max.y - 20.0),
    );
    draw_meter(&painter, in_rect, app.viz.input_db(), "IN");
    draw_meter(&painter, out_rect, app.viz.output_db(), "OUT");

    // What auto-gain is doing right now, under the meters: the number
    // that says how much level the drive would otherwise have added.
    let readout = if app.params.auto_gain.value() {
        format!("AUTO {:+.1} dB", app.viz.auto_gain_db())
    } else {
        "AUTO off".to_string()
    };
    painter.text(
        egui::pos2(meters_rect.center().x, meters_rect.max.y - 8.0),
        egui::Align2::CENTER_CENTER,
        readout,
        egui::FontId::proportional(10.0),
        theme::TEXT_DIM,
    );
}

/// A vertical −60…+6 dBFS peak bar.
fn draw_meter(painter: &egui::Painter, rect: egui::Rect, db: f32, label: &str) {
    const MIN_DB: f32 = -60.0;
    const MAX_DB: f32 = 6.0;
    painter.rect_filled(rect, 4.0, theme::PANEL);
    let label_h = 14.0;
    let bar = egui::Rect::from_min_max(
        egui::pos2(rect.left() + 8.0, rect.top() + 6.0),
        egui::pos2(rect.right() - 8.0, rect.bottom() - label_h),
    );
    painter.rect_filled(bar, 2.0, theme::BG_1);
    if db.is_finite() {
        let frac = ((db - MIN_DB) / (MAX_DB - MIN_DB)).clamp(0.0, 1.0);
        let top = bar.bottom() - frac * bar.height();
        let fill = egui::Rect::from_min_max(egui::pos2(bar.left(), top), bar.max);
        let color = if db > 0.0 { theme::DANGER } else { theme::ACCENT };
        painter.rect_filled(fill, 2.0, color);
    }
    painter.text(
        egui::pos2(rect.center().x, rect.bottom() - 6.0),
        egui::Align2::CENTER_CENTER,
        label,
        egui::FontId::proportional(9.0),
        theme::TEXT_DIM,
    );
}
