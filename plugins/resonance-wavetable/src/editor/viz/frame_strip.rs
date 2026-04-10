//! Wavetable frame overview strip.
//!
//! A thin strip showing every Nth frame of the selected wavetable as a
//! small polyline offset horizontally. The currently-selected frame is
//! highlighted with a vertical bar.

use wayland_plugin_gui::egui;

use crate::editor::display_waves;
use crate::editor::theme;

pub fn draw(ui: &mut egui::Ui, rect: egui::Rect, wavetable_idx: usize, position: f32) {
    let painter = ui.painter_at(rect);

    painter.rect_filled(rect, 2.0, theme::PANEL);
    painter.rect_stroke(
        rect,
        2.0,
        egui::Stroke::new(1.0, theme::BORDER),
        egui::StrokeKind::Inside,
    );

    let frames = display_waves::frame_count(wavetable_idx);
    if frames == 0 {
        return;
    }

    const N_POINTS_PER_FRAME: usize = 32;
    const MAX_STRIPS: usize = 24;
    let stride = frames.div_ceil(MAX_STRIPS).max(1);
    let strip_count = frames.div_ceil(stride);

    let pad = 4.0f32;
    let w = rect.width() - pad * 2.0;
    let strip_w = w / strip_count as f32;
    let mid_y = rect.center().y;
    let h = rect.height() - pad * 2.0;

    for s in 0..strip_count {
        let frame = s * stride;
        let samples = display_waves::display_samples(wavetable_idx, frame, N_POINTS_PER_FRAME);
        let x0 = rect.left() + pad + s as f32 * strip_w;
        let mut points: Vec<egui::Pos2> = Vec::with_capacity(N_POINTS_PER_FRAME);
        for (i, sample) in samples.iter().enumerate() {
            let x = x0 + (i as f32 / (N_POINTS_PER_FRAME - 1) as f32) * (strip_w * 0.9);
            let y = mid_y - sample.clamp(-1.0, 1.0) * (h * 0.4);
            points.push(egui::pos2(x, y));
        }
        painter.add(egui::Shape::line(
            points,
            egui::Stroke::new(0.8, theme::TEXT_DIM),
        ));
    }

    // Highlight marker at the current position.
    let marker_x = rect.left() + pad + position.clamp(0.0, 1.0) * w;
    painter.line_segment(
        [
            egui::pos2(marker_x, rect.top() + 2.0),
            egui::pos2(marker_x, rect.bottom() - 2.0),
        ],
        egui::Stroke::new(1.0, theme::ACCENT),
    );
}
