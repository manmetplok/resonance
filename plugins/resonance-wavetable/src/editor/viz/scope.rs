//! Output oscilloscope reading from the shared viz state.

use wayland_plugin_gui::egui;

use crate::editor::theme;
use crate::viz::SCOPE_FRAMES;

pub fn draw(ui: &mut egui::Ui, rect: egui::Rect, samples: &[f32]) {
    let painter = ui.painter_at(rect);

    painter.rect_filled(rect, 4.0, theme::PANEL);
    painter.rect_stroke(
        rect,
        4.0,
        egui::Stroke::new(1.0, theme::BORDER),
        egui::StrokeKind::Inside,
    );

    let mid_y = rect.center().y;
    painter.line_segment(
        [
            egui::pos2(rect.left() + 6.0, mid_y),
            egui::pos2(rect.right() - 6.0, mid_y),
        ],
        egui::Stroke::new(0.5, theme::BORDER),
    );

    // samples is interleaved [L, R, L, R, ...] of length SCOPE_FRAMES*2.
    if samples.len() < SCOPE_FRAMES * 2 {
        return;
    }

    let pad = 6.0;
    let left = rect.left() + pad;
    let right = rect.right() - pad;
    let width = right - left;
    let amp = (rect.height() - pad * 2.0) * 0.45;

    let mut l_points: Vec<egui::Pos2> = Vec::with_capacity(SCOPE_FRAMES);
    let mut r_points: Vec<egui::Pos2> = Vec::with_capacity(SCOPE_FRAMES);
    for i in 0..SCOPE_FRAMES {
        let x = left + (i as f32 / (SCOPE_FRAMES - 1) as f32) * width;
        let l = samples[i * 2].clamp(-1.0, 1.0);
        let r = samples[i * 2 + 1].clamp(-1.0, 1.0);
        l_points.push(egui::pos2(x, mid_y - l * amp));
        r_points.push(egui::pos2(x, mid_y - r * amp));
    }

    painter.add(egui::Shape::line(
        l_points,
        egui::Stroke::new(1.0, theme::ACCENT),
    ));
    painter.add(egui::Shape::line(
        r_points,
        egui::Stroke::new(1.0, theme::WARN),
    ));
}
