//! Vertical LUFS strip showing Momentary / Short-term / Integrated bars
//! alongside a target-LUFS reference line.

use wayland_plugin_gui::egui;

use super::theme;

/// Meter floor in LUFS.
const FLOOR_LUFS: f32 = -60.0;
/// Meter ceiling in LUFS.
const CEILING_LUFS: f32 = 0.0;

pub fn draw(
    painter: &egui::Painter,
    rect: egui::Rect,
    momentary: f32,
    short_term: f32,
    integrated: f32,
    target: f32,
) {
    painter.rect_filled(rect, 2.0, theme::PANEL);
    painter.rect_stroke(
        rect,
        2.0,
        egui::Stroke::new(1.0, theme::BORDER),
        egui::StrokeKind::Inside,
    );

    painter.text(
        egui::pos2(rect.center().x, rect.top() + 6.0),
        egui::Align2::CENTER_TOP,
        "LUFS",
        egui::FontId::monospace(10.0),
        theme::TEXT_DIM,
    );

    let body = egui::Rect::from_min_max(
        egui::pos2(rect.left() + 4.0, rect.top() + 22.0),
        egui::pos2(rect.right() - 4.0, rect.bottom() - 30.0),
    );

    draw_scale_ticks(painter, body);
    draw_target_line(painter, body, target);

    let col_w = (body.width() - 8.0) / 3.0;
    let l_rect = egui::Rect::from_min_max(
        egui::pos2(body.left(), body.top()),
        egui::pos2(body.left() + col_w, body.bottom()),
    );
    let m_rect = egui::Rect::from_min_max(
        egui::pos2(l_rect.right() + 4.0, body.top()),
        egui::pos2(l_rect.right() + 4.0 + col_w, body.bottom()),
    );
    let r_rect = egui::Rect::from_min_max(
        egui::pos2(m_rect.right() + 4.0, body.top()),
        egui::pos2(m_rect.right() + 4.0 + col_w, body.bottom()),
    );

    draw_bar(painter, l_rect, momentary, "M");
    draw_bar(painter, m_rect, short_term, "S");
    draw_bar(painter, r_rect, integrated, "I");

    // Numeric readouts.
    let readout_y = rect.bottom() - 22.0;
    let third = rect.width() / 3.0;
    draw_readout(
        painter,
        egui::pos2(rect.left() + third * 0.5, readout_y),
        "M",
        momentary,
    );
    draw_readout(
        painter,
        egui::pos2(rect.left() + third * 1.5, readout_y),
        "S",
        short_term,
    );
    draw_readout(
        painter,
        egui::pos2(rect.left() + third * 2.5, readout_y),
        "I",
        integrated,
    );
}

fn draw_bar(painter: &egui::Painter, rect: egui::Rect, lufs: f32, label: &str) {
    painter.rect_filled(rect, 1.0, theme::BG);
    if lufs.is_finite() && lufs > FLOOR_LUFS {
        let norm = ((lufs - FLOOR_LUFS) / (CEILING_LUFS - FLOOR_LUFS)).clamp(0.0, 1.0);
        let h = norm * rect.height();
        let fill = egui::Rect::from_min_max(
            egui::pos2(rect.left(), rect.bottom() - h),
            egui::pos2(rect.right(), rect.bottom()),
        );
        painter.rect_filled(fill, 1.0, color_for_lufs(lufs));
    }
    painter.text(
        egui::pos2(rect.center().x, rect.top() - 10.0),
        egui::Align2::CENTER_BOTTOM,
        label,
        egui::FontId::monospace(9.0),
        theme::TEXT_DIM,
    );
}

fn color_for_lufs(lufs: f32) -> egui::Color32 {
    if lufs > -6.0 {
        theme::DANGER
    } else if lufs > -14.0 {
        theme::WARN
    } else {
        theme::ACCENT
    }
}

fn draw_scale_ticks(painter: &egui::Painter, rect: egui::Rect) {
    for tick in [-6.0, -14.0, -23.0, -40.0, -60.0] {
        let norm = ((tick - FLOOR_LUFS) / (CEILING_LUFS - FLOOR_LUFS)).clamp(0.0, 1.0);
        let y = rect.bottom() - norm * rect.height();
        painter.line_segment(
            [egui::pos2(rect.left(), y), egui::pos2(rect.right(), y)],
            egui::Stroke::new(0.5, theme::BORDER),
        );
    }
}

fn draw_target_line(painter: &egui::Painter, rect: egui::Rect, target: f32) {
    let norm = ((target - FLOOR_LUFS) / (CEILING_LUFS - FLOOR_LUFS)).clamp(0.0, 1.0);
    let y = rect.bottom() - norm * rect.height();
    painter.line_segment(
        [egui::pos2(rect.left(), y), egui::pos2(rect.right(), y)],
        egui::Stroke::new(1.5, theme::GOOD),
    );
}

fn draw_readout(painter: &egui::Painter, pos: egui::Pos2, label: &str, value: f32) {
    let text = if value.is_finite() && value > FLOOR_LUFS {
        format!("{label}: {value:>5.1}")
    } else {
        format!("{label}: —")
    };
    painter.text(
        pos,
        egui::Align2::CENTER_CENTER,
        text,
        egui::FontId::monospace(10.0),
        theme::TEXT,
    );
}
