//! Rolling LUFS-momentary trace.

use wayland_plugin_gui::egui;

use super::theme;
use crate::viz::MasteringViz;

const FLOOR_LUFS: f32 = -60.0;
const CEILING_LUFS: f32 = 0.0;

pub fn draw(painter: &egui::Painter, rect: egui::Rect, viz: &MasteringViz, target: f32) {
    painter.rect_filled(rect, 2.0, theme::PANEL);
    painter.rect_stroke(
        rect,
        2.0,
        egui::Stroke::new(1.0, theme::BORDER),
        egui::StrokeKind::Inside,
    );

    painter.text(
        egui::pos2(rect.left() + 8.0, rect.top() + 4.0),
        egui::Align2::LEFT_TOP,
        "LUFS-M history  (30 s)",
        egui::FontId::monospace(10.0),
        theme::TEXT_DIM,
    );

    // Target line.
    let target_y = lufs_to_y(target, rect);
    painter.line_segment(
        [
            egui::pos2(rect.left(), target_y),
            egui::pos2(rect.right(), target_y),
        ],
        egui::Stroke::new(1.0, theme::GOOD),
    );

    let history = viz.lufs_history.lock();
    let n = crate::viz::LUFS_HISTORY_LEN;
    let samples: Vec<f32> = history.iter_chrono().collect();
    drop(history);

    if samples.is_empty() {
        return;
    }
    let step = rect.width() / (n as f32 - 1.0).max(1.0);
    let mut prev: Option<egui::Pos2> = None;
    for (i, &v) in samples.iter().enumerate() {
        if !v.is_finite() {
            prev = None;
            continue;
        }
        let x = rect.left() + i as f32 * step;
        let y = lufs_to_y(v, rect);
        let p = egui::pos2(x, y);
        if let Some(pp) = prev {
            painter.line_segment([pp, p], egui::Stroke::new(1.0, theme::ACCENT));
        }
        prev = Some(p);
    }
}

fn lufs_to_y(lufs: f32, rect: egui::Rect) -> f32 {
    let norm = ((lufs - FLOOR_LUFS) / (CEILING_LUFS - FLOOR_LUFS)).clamp(0.0, 1.0);
    rect.bottom() - norm * (rect.height() - 16.0) - 4.0
}
