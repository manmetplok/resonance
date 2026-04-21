//! Horizontal stereo correlation bar (-1 … +1).

use wayland_plugin_gui::egui;

use super::theme;

pub fn draw(painter: &egui::Painter, rect: egui::Rect, value: f32) {
    painter.rect_filled(rect, 2.0, theme::PANEL);
    painter.rect_stroke(
        rect,
        2.0,
        egui::Stroke::new(1.0, theme::BORDER),
        egui::StrokeKind::Inside,
    );

    let label_rect =
        egui::Rect::from_min_max(rect.left_top(), egui::pos2(rect.right(), rect.top() + 16.0));
    painter.text(
        label_rect.center(),
        egui::Align2::CENTER_CENTER,
        "Correlation",
        egui::FontId::monospace(10.0),
        theme::TEXT_DIM,
    );

    let body = egui::Rect::from_min_max(
        egui::pos2(rect.left() + 6.0, rect.top() + 20.0),
        egui::pos2(rect.right() - 6.0, rect.top() + 38.0),
    );
    painter.rect_filled(body, 1.0, theme::BG);

    let cx = body.center().x;
    // Centre tick.
    painter.line_segment(
        [egui::pos2(cx, body.top()), egui::pos2(cx, body.bottom())],
        egui::Stroke::new(1.0, theme::BORDER),
    );
    // -1 / +1 ticks.
    painter.line_segment(
        [
            egui::pos2(body.left(), body.top()),
            egui::pos2(body.left(), body.bottom()),
        ],
        egui::Stroke::new(1.0, theme::BORDER),
    );
    painter.line_segment(
        [
            egui::pos2(body.right(), body.top()),
            egui::pos2(body.right(), body.bottom()),
        ],
        egui::Stroke::new(1.0, theme::BORDER),
    );

    let v = value.clamp(-1.0, 1.0);
    let color = if v < 0.0 { theme::WARN } else { theme::ACCENT };
    let end_x = cx + v * (body.width() * 0.5);
    let fill = egui::Rect::from_min_max(
        egui::pos2(cx.min(end_x), body.top() + 2.0),
        egui::pos2(cx.max(end_x), body.bottom() - 2.0),
    );
    painter.rect_filled(fill, 1.0, color);

    painter.text(
        egui::pos2(rect.center().x, rect.bottom() - 8.0),
        egui::Align2::CENTER_BOTTOM,
        format!("{v:+.2}"),
        egui::FontId::monospace(10.0),
        theme::TEXT,
    );
}
