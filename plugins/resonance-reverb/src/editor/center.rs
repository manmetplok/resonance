//! The central area: the impulse-tail hero view on the left, the tank
//! view on the right, the stereo peak meters along the bottom.

use plugin_gui_core::egui;

use super::{impulse_view, meters, tank_view, ReverbEditorApp};

pub(super) fn draw(ui: &mut egui::Ui, app: &mut ReverbEditorApp) {
    let avail = ui.available_rect_before_wrap();

    // Reserve a thin strip along the bottom for the stereo peak meters.
    let meter_h = 28.0f32;
    let gap = 8.0f32;
    let viz_rect = egui::Rect::from_min_max(
        egui::pos2(avail.left() + gap, avail.top() + gap),
        egui::pos2(avail.right() - gap, avail.bottom() - meter_h - gap),
    );
    let meter_rect = egui::Rect::from_min_max(
        egui::pos2(avail.left() + gap, avail.bottom() - meter_h),
        egui::pos2(avail.right() - gap, avail.bottom() - 2.0),
    );

    // Split the viz area: impulse hero (left ~68%) + FDN tank (right ~32%).
    let tank_w = 300.0f32.min(viz_rect.width() * 0.35);
    let impulse_rect = egui::Rect::from_min_max(
        viz_rect.min,
        egui::pos2(viz_rect.right() - tank_w - gap, viz_rect.bottom()),
    );
    let tank_rect = egui::Rect::from_min_max(
        egui::pos2(impulse_rect.right() + gap, viz_rect.top()),
        viz_rect.max,
    );

    let painter = ui.painter_at(avail);
    impulse_view::draw(&painter, impulse_rect, app);
    tank_view::draw(&painter, tank_rect, app);
    meters::draw(&painter, meter_rect, &app.viz);
}
