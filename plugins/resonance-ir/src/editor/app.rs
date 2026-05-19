//! The actual egui app: state and update/view orchestration for the IR editor.
//!
//! `IrEditorApp` is the `EditorApp` the runtime drives each frame. It paints
//! the chrome panels (header, control strip) and dispatches the centre to the
//! waveform/response/meters views.

use std::sync::atomic::AtomicI32;
use std::sync::Arc;

use parking_lot::Mutex;
use wayland_plugin_gui::{egui, EditorApp};

use crate::params::IrParams;
use crate::viz::IrViz;

use super::{controls, header, meters, response_view, theme, waveform_view};

pub(crate) struct IrEditorApp {
    pub(crate) params: Arc<IrParams>,
    pub(crate) ir_name: Arc<Mutex<String>>,
    pub(crate) ir_info: Arc<Mutex<String>>,
    pub(crate) load_request: Arc<AtomicI32>,
    pub(crate) viz: Arc<IrViz>,
}

impl EditorApp for IrEditorApp {
    fn ui(&mut self, ui: &mut egui::Ui) {
        theme::apply(ui.ctx());
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(33));

        egui::Panel::top("ir_header")
            .exact_size(42.0)
            .show_inside(ui, |ui| header::draw(ui, self));

        egui::Panel::bottom("ir_strip")
            .exact_size(120.0)
            .show_inside(ui, |ui| controls::draw(ui, &self.params));

        egui::CentralPanel::default().show_inside(ui, |ui| draw_center(ui, self));
    }
}

fn draw_center(ui: &mut egui::Ui, app: &mut IrEditorApp) {
    let avail = ui.available_rect_before_wrap();
    let gap = 8.0f32;
    let meter_h = 28.0f32;

    let viz_rect = egui::Rect::from_min_max(
        egui::pos2(avail.left() + gap, avail.top() + gap),
        egui::pos2(avail.right() - gap, avail.bottom() - meter_h - gap),
    );
    let meter_rect = egui::Rect::from_min_max(
        egui::pos2(avail.left() + gap, avail.bottom() - meter_h),
        egui::pos2(avail.right() - gap, avail.bottom() - 2.0),
    );

    // Split viz: waveform (left ~55%), response (right ~45%).
    let resp_w = (viz_rect.width() * 0.45).clamp(240.0, 520.0);
    let wave_rect = egui::Rect::from_min_max(
        viz_rect.min,
        egui::pos2(viz_rect.right() - resp_w - gap, viz_rect.bottom()),
    );
    let resp_rect = egui::Rect::from_min_max(
        egui::pos2(wave_rect.right() + gap, viz_rect.top()),
        viz_rect.max,
    );

    let painter = ui.painter_at(avail);
    waveform_view::draw(&painter, wave_rect, &app.viz, &app.ir_name.lock());
    response_view::draw(&painter, resp_rect, &app.viz);
    meters::draw(&painter, meter_rect, &app.viz);
}
