//! The actual egui app: state and update/view orchestration for the amp editor.
//!
//! `AmpEditorApp` is the `EditorApp` the runtime drives each frame. It paints
//! the chrome panels (header, tuner, control strip) and dispatches the centre
//! to the scope/curve/meters views.

use std::sync::atomic::AtomicI32;
use std::sync::Arc;

use parking_lot::Mutex;
use wayland_plugin_gui::{egui, EditorApp};

use crate::params::AmpParams;
use crate::tone3000::worker::WorkerHandle;
use crate::viz::AmpViz;

use super::tone3000_panel::Tone3000PanelState;
use super::{controls, curve_view, header, meters, scope_view, theme, tone3000_panel, tuner_view};

pub(crate) struct AmpEditorApp {
    pub(crate) params: Arc<AmpParams>,
    pub(crate) model_name: Arc<Mutex<String>>,
    pub(crate) load_request: Arc<AtomicI32>,
    pub(crate) viz: Arc<AmpViz>,
    pub(crate) tone3000: Arc<WorkerHandle>,
    pub(crate) tone3000_panel: Tone3000PanelState,
}

impl EditorApp for AmpEditorApp {
    fn ui(&mut self, ui: &mut egui::Ui) {
        theme::apply(ui.ctx());
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(16));

        egui::Panel::top("amp_header")
            .exact_size(38.0)
            .show_inside(ui, |ui| header::draw(ui, self));

        egui::Panel::top("amp_tuner")
            .exact_size(72.0)
            .show_inside(ui, |ui| {
                let rect = ui.available_rect_before_wrap();
                let painter = ui.painter_at(rect);
                tuner_view::draw(&painter, rect, &self.viz);
            });

        egui::Panel::bottom("amp_strip")
            .exact_size(140.0)
            .show_inside(ui, |ui| controls::draw(ui, &self.params));

        egui::CentralPanel::default().show_inside(ui, |ui| draw_center(ui, self));

        if self.tone3000_panel.open {
            tone3000_panel::draw(ui, &mut self.tone3000_panel, &self.tone3000);
        }
    }
}

fn draw_center(ui: &mut egui::Ui, app: &mut AmpEditorApp) {
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

    // Split the viz area: scope (left ~65%) + transfer curve (right ~35%).
    let curve_w = 280.0f32.min(viz_rect.width() * 0.4);
    let scope_rect = egui::Rect::from_min_max(
        viz_rect.min,
        egui::pos2(viz_rect.right() - curve_w - gap, viz_rect.bottom()),
    );
    let curve_rect = egui::Rect::from_min_max(
        egui::pos2(scope_rect.right() + gap, viz_rect.top()),
        viz_rect.max,
    );

    let painter = ui.painter_at(avail);
    scope_view::draw(&painter, scope_rect, &app.viz);
    curve_view::draw(&painter, curve_rect, &app.viz);
    meters::draw(&painter, meter_rect, &app.viz);
}
