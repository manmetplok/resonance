use egui::{Painter, Rect, Stroke, StrokeKind};
use wayland_plugin_gui::egui;

use super::theme;
use crate::viz::DelayViz;

pub fn draw(painter: &Painter, rect: Rect, viz: &DelayViz) {
    painter.rect_filled(rect, 4.0, theme::PANEL);
    painter.rect_stroke(
        rect,
        4.0,
        Stroke::new(1.0, theme::BORDER),
        StrokeKind::Inside,
    );

    let (times_l, levels_l, times_r, levels_r) = viz.read_echo_taps();

    let max_time_ms = 2000.0f32;
    let mid_y = rect.center().y;
    let half_h = rect.height() * 0.4;

    for i in 0..times_l.len() {
        let t = times_l[i];
        let level = levels_l[i];
        if t <= 0.0 || level < -60.0 {
            continue;
        }
        let x = rect.left() + (t / max_time_ms).min(1.0) * rect.width();
        let amp = ((level + 60.0) / 60.0).clamp(0.0, 1.0);
        let y_top = mid_y - amp * half_h;
        painter.line_segment(
            [egui::pos2(x, mid_y), egui::pos2(x, y_top)],
            Stroke::new(3.0, theme::ECHO_L),
        );
        painter.circle_filled(egui::pos2(x, y_top), 4.0, theme::ECHO_L);
    }

    for i in 0..times_r.len() {
        let t = times_r[i];
        let level = levels_r[i];
        if t <= 0.0 || level < -60.0 {
            continue;
        }
        let x = rect.left() + (t / max_time_ms).min(1.0) * rect.width();
        let amp = ((level + 60.0) / 60.0).clamp(0.0, 1.0);
        let y_bot = mid_y + amp * half_h;
        painter.line_segment(
            [egui::pos2(x, mid_y), egui::pos2(x, y_bot)],
            Stroke::new(3.0, theme::ECHO_R),
        );
        painter.circle_filled(egui::pos2(x, y_bot), 4.0, theme::ECHO_R);
    }

    painter.line_segment(
        [
            egui::pos2(rect.left() + 4.0, mid_y),
            egui::pos2(rect.right() - 4.0, mid_y),
        ],
        Stroke::new(1.0, theme::BORDER),
    );

    for &ms in &[0, 250, 500, 1000, 1500, 2000] {
        let x = rect.left() + (ms as f32 / max_time_ms) * rect.width();
        if x > rect.right() - 20.0 {
            break;
        }
        painter.text(
            egui::pos2(x, rect.bottom() - 12.0),
            egui::Align2::CENTER_BOTTOM,
            format!("{ms}"),
            egui::FontId::proportional(9.0),
            theme::TEXT_DIM,
        );
    }
}
